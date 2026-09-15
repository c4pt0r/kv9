#!/usr/bin/env python3
"""Independent compressed representation audit; never imports the producer."""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import selectors
import shutil
import stat
import subprocess
import time

CODEC='/usr/bin/zstd'
CODEC_SHA='7c5468b370f7c47eda07281e3437fafc568f95d10420051e3aa522709f9342c5'
CPUS=list(range(6,16))+list(range(22,32))
COHORT_ROOTS=[Path('/tmp/kv9-fnv-writer-performance-budget-v2-smoke-20260914-first'),
              Path('/tmp/kv9-fnv-writer-performance-budget-v2-timing-20260914-first/cohorts')]
LIMITS={'member_bytes':8*1024**3,'cohort_bytes':16*1024**3,'campaign_bytes':128*1024**3,
        'metadata_bytes':8*1024**2,'files':65536,'codec_seconds':600,'cohort_seconds':1800,
        'io_bytes':1024**2,'decode_memory_bytes':64*1024**2,'retention_floor_bytes':48*1024**3}

STORAGE_POLICY = {'policy_id': 'kv9-local-benchmark-storage-v2', 'available_space': 'f_bavail * f_frsize', 'pre_cohort': {'tmpfs_bytes': 34359738368, 'host_bytes': 68719476736}, 'measurement': {'tmpfs_bytes': 17179869184, 'host_bytes': 68719476736}, 'post_run': {'host_floor_bytes': 51539607552, 'fresh_restore_reserve_bytes': 17179869184, 'metadata_margin_bytes': 1073741824}, 'representation': 'zstd-files-v1'}
STORAGE_POLICY_SHA = '6662377f04c3fec3e27b1ffdb275f8589c3fb009ebc38bcf4a52c8fab9a2dd9a'

def check_storage_policy(document):
    check(json.dumps(document.get('storage_policy'),sort_keys=True,separators=(',',':')) ==
            json.dumps(STORAGE_POLICY,sort_keys=True,separators=(',',':')) and
            document.get('storage_policy_sha256') == STORAGE_POLICY_SHA, 'storage policy binding differs')

# Prospective readback-only availability floor; historical LIMITS and recorded budget checks stay exact.
READBACK_AVAILABLE_FLOOR_BYTES=8*1024**3

def check(ok,message):
    if not ok:raise ValueError(message)

def producer_lifetime_absent(record):
    """Check the recorded lifetime, refusing unavailable or ambiguous identity evidence."""
    def boot(value):
        check(type(value)is str and len(value)==36 and
              all(value[i]=='-' for i in (8,13,18,23)) and
              all(c in '0123456789abcdef' for i,c in enumerate(value) if i not in (8,13,18,23)),
              'producer boot identity malformed')
        return value
    pid=record.get('pid');ticks=record.get('start_ticks')
    check(type(pid)is int and pid>0 and type(ticks)is int and ticks>0,
          'producer PID/start identity missing or malformed')
    recorded_boot=boot(record.get('boot_id'))
    try:
        current_boot=boot(Path('/proc/sys/kernel/random/boot_id').read_text().strip())
        try:
            raw=Path(f'/proc/{pid}/stat').read_text()
        except (FileNotFoundError,ProcessLookupError):
            return True
        check(len(raw)<=16384 and raw.startswith(str(pid)+' ('), 'current producer stat identity malformed')
        head,separator,tail=raw.rpartition(') ');fields=tail.split()
        check(separator and len(fields)>=20 and len(fields[0])==1 and fields[0] in 'RSDZTtXxKWPI' and
              fields[19].isascii() and fields[19].isdecimal(), 'current producer stat identity ambiguous')
        current_ticks=int(fields[19]);check(current_ticks>0, 'current producer start identity malformed')
        check(boot(Path('/proc/sys/kernel/random/boot_id').read_text().strip())==current_boot,
              'current producer boot identity changed')
    except (OSError,UnicodeError) as exc:
        raise ValueError('current producer lifetime identity unreadable') from exc
    return (current_boot,pid,current_ticks)!=(recorded_boot,pid,ticks)


def unique_pairs(pairs):
    result={}
    for key,value in pairs:
        check(key not in result,'duplicate JSON key');result[key]=value
    return result

def canonical(value):return json.dumps(value,sort_keys=True,separators=(',',':'))

def name_ok(value):
    check(type(value)is str and value and '\\' not in value and '\0' not in value,'unsafe member/object path')
    p=PurePosixPath(value)
    check(not p.is_absolute() and str(p)==value and all(x not in ('','.','..') for x in value.split('/')),'unsafe member/object path')

def digest_file(path,records):
    path=Path(path);s=path.lstat()
    check(stat.S_ISREG(s.st_mode) and s.st_nlink==1,'nonordinary physical input')
    h=hashlib.sha256();count=0
    with path.open('rb') as f:
        check((os.fstat(f.fileno()).st_dev,os.fstat(f.fileno()).st_ino)==(s.st_dev,s.st_ino),'physical input replaced')
        while True:
            b=f.read(1024**2)
            if not b:break
            count+=len(b);h.update(b)
    t=path.lstat()
    check((s.st_dev,s.st_ino,s.st_size,s.st_mtime_ns,s.st_ctime_ns)==
          (t.st_dev,t.st_ino,t.st_size,t.st_mtime_ns,t.st_ctime_ns),'physical input changed')
    value={'bytes':count,'sha256':h.hexdigest()};records[str(path)]=value
    return value

def metadata(path,records):
    path=Path(path);check(path.stat().st_size<=8*1024**2,'metadata cap')
    pin=digest_file(path,records);raw=path.read_bytes()
    check(hashlib.sha256(raw).hexdigest()==pin['sha256'],'metadata changed after hash')
    return json.loads(raw,object_pairs_hook=unique_pairs)

def validate_frame(path):
    # Independent frame extent parser. Does not use producer flags/decoder helpers.
    rawsize=path.stat().st_size
    with path.open('rb') as stream:
        def read(n):
            b=stream.read(n);check(len(b)==n,'short frame extent');return b
        check(int.from_bytes(read(4),'little')==0xfd2fb528,'nonstandard frame')
        descriptor=read(1)[0]
        check((descriptor & 0x1c)==4,'frame checksum/reserved flags')
        single=(descriptor>>5)&1;sizeflag=descriptor>>6;dictflag=descriptor&3
        check(dictflag==0,'dictionary frame')
        if not single:
            w=read(1)[0];window=(1<<(10+w//8))*(8+w%8)//8
            check(window<=67108864,'decoder memory window')
        sizes=[single,2,4,8];sizebytes=read(sizes[sizeflag])
        if single:check(int.from_bytes(sizebytes,'little')+(256 if sizeflag==1 else 0)<=67108864,'single frame memory')
        end=False
        while not end:
            block=int.from_bytes(read(3),'little');end=bool(block&1);kind=(block>>1)&3;length=block>>3
            check(kind<3 and length<=131072,'invalid block descriptor')
            extent=1 if kind==1 else length
            check(stream.tell()+extent<=rawsize,'truncated block payload');stream.seek(extent,1)
        read(4);check(stream.tell()==rawsize,'concatenated/trailing frame')

def full_decode(path,expected,observation):
    check(type(expected['bytes'])is int and 0<=expected['bytes']<=LIMITS['member_bytes'],'decode output bound')
    validate_frame(path)
    check(hashlib.sha256(Path(CODEC).read_bytes()).hexdigest()==CODEC_SHA,'decoder pin')
    input_stream=path.open('rb');child=None;selector=None;h=hashlib.sha256();count=0;stderr=bytearray()
    start=time.monotonic();minimum=2**64
    try:
        child=subprocess.Popen([CODEC,'--decompress','--stdout','--quiet','--memory=64MB','--single-thread'],
            stdin=input_stream,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
            preexec_fn=lambda:os.sched_setaffinity(0,CPUS),start_new_session=True)
        identity=Path(f'/proc/{child.pid}/stat').read_text().rsplit(')',1)[1].split()
        observation.update(pid=child.pid,start_ticks=int(identity[19]),boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
                           started_unix_ns=time.time_ns(),cpu_affinity=sorted(os.sched_getaffinity(child.pid)),
                           codec_sha256=CODEC_SHA,complete=False)
        selector=selectors.DefaultSelector()
        selector.register(child.stdout,selectors.EVENT_READ,'output');selector.register(child.stderr,selectors.EVENT_READ,'error')
        while selector.get_map() or child.poll() is None:
            free=shutil.disk_usage(path.parent).free;minimum=min(minimum,free)
            check(free>=READBACK_AVAILABLE_FLOOR_BYTES,'decode retention floor')
            check(time.monotonic()-start<=LIMITS['codec_seconds'],'independent decoder deadline')
            for key,_ in selector.select(0.05):
                b=os.read(key.fileobj.fileno(),1024**2)
                if not b:selector.unregister(key.fileobj);continue
                if key.data=='error':stderr.extend(b);check(len(stderr)<=65536,'decoder stderr bound')
                else:
                    count+=len(b);check(count<=expected['bytes'],'decoded extra bytes');h.update(b)
        check(child.wait()==0,'independent decoder failure: '+stderr.decode(errors='replace'))
        check(count==expected['bytes'] and h.hexdigest()==expected['sha256'],'independent original size/hash differs')
        observation.update(complete=True,bytes=count,sha256=h.hexdigest(),minimum_available_bytes=minimum)
    finally:
        if child:
            if child.poll() is None:child.kill()
            child.wait(timeout=10)
            observation.update(exit_code=child.returncode,absent=not Path(f'/proc/{child.pid}').exists(),ended_unix_ns=time.time_ns())
            child.stdout.close();child.stderr.close()
        if selector:selector.close()
        input_stream.close()

def verify(fixture,expected_children,records,logical,decoder_receipts,*,allow_synthetic=False):
    fixture=Path(fixture);started=time.monotonic()
    check(not fixture.is_symlink() and fixture.is_dir(),'fixture directory type')
    receipt=metadata(fixture/'tmpfs-retention.json',records)
    check_storage_policy(receipt)
    check(receipt['schema']==2 and receipt['representation']=='zstd-files-v1' and receipt['resident_original_paths']is False,
          'compressed representation declaration')
    check(receipt['complete']is True and receipt['children_exited']is True and receipt['scratch_removed']is True,'retention incomplete')
    check(not Path(receipt['original_scratch']).exists() and not (fixture/'data').exists() and not (fixture/'data').is_symlink(),'original path still resident')
    check(receipt['logical_data']==str(fixture/'data') and receipt['retained_objects']==str(fixture/'retention-objects'),'retention paths')
    catalog=metadata(fixture/'retention-catalog.json',records);original=metadata(fixture/'retention-original.json',records)
    intent=metadata(fixture/'retention-cleanup-intent.json',records)
    check(records[str(fixture/'retention-catalog.json')]['sha256']==receipt['catalog_sha256']==intent['catalog_sha256'],'catalog hash binding')
    check(records[str(fixture/'retention-original.json')]['sha256']==receipt['original_inventory_sha256']==catalog['original_inventory_sha256']==intent['source_inventory_sha256'],'original inventory binding')
    check_storage_policy(catalog)
    check(catalog.get('synthetic_only') is allow_synthetic,'synthetic/runtime scope differs')
    check(catalog['schema']==1 and catalog['representation']=='zstd-files-v1' and catalog['resident_original_paths']is False,'catalog scope')
    check(canonical(catalog['limits'])==canonical(LIMITS) and catalog['codec']=={'path':CODEC,'sha256':CODEC_SHA},'catalog budget/codec')
    for document in (catalog,original):
        check(document['logical_data']==str(fixture/'data') and document['original_scratch']==receipt['original_scratch'],'original source path binding')
        check(canonical(document['children'])==canonical(expected_children),'retention exited writer binding')
        check(document['reference_check']['complete']is True and not document['reference_check']['references'] and not document['reference_check']['errors'],'reference evidence incomplete')
    check(catalog['fixture']==str(fixture),'catalog fixture binding')
    if not allow_synthetic:
        check(catalog['logical_campaign_roots']==list(map(str,COHORT_ROOTS)), 'combined campaign scope')
    check(type(catalog['prior_logical_bytes'])is int and catalog['prior_logical_bytes']>=0,'prior logical count')
    check(receipt['started_unix_ns']==catalog['started_unix_ns']==original['started_unix_ns'] and
          catalog['started_unix_ns']<=catalog['verified_unix_ns']<=intent['started_unix_ns']<=receipt['completed_unix_ns'] and
          receipt['completed_unix_ns']-receipt['started_unix_ns']<=LIMITS['cohort_seconds']*10**9,'retention interval')
    budget=receipt['budget'];check(budget['complete']is True and budget['samples'] and
        budget['minimum_available_bytes']>=LIMITS['retention_floor_bytes'] and all(
        s['available_bytes']>=LIMITS['retention_floor_bytes'] and s['allocated_bytes']<=LIMITS['campaign_bytes']
        for s in budget['samples']),'retention space guard evidence')
    members=catalog['members'];check(len(members)<=LIMITS['files'],'catalog member bound')
    seen=set();objects=set();total=0;compressed=0
    dirs=catalog['directories'];check(dirs==original['inventory']['directories'] and '' in dirs,'original directory inventory')
    for name,m in dirs.items():
        if name:name_ok(name)
        check(stat.S_ISDIR(m['mode']),'original directory type')
    for number,row in enumerate(members):
        check(time.monotonic()-started<=LIMITS['cohort_seconds'],'independent cohort decode deadline')
        name=row['name'];name_ok(name);name_ok(row['object'])
        check(name not in seen and name not in dirs and row['object'] not in objects,'duplicate/colliding member')
        seen.add(name);objects.add(row['object'])
        check(row['object']==f'retention-objects/{number:06d}.zst','object identity/order')
        check(row['original_path']==str(fixture/'data'/name),'logical original path')
        item=row['original'];m=item['metadata']
        check(item==original['inventory']['files'][name] and
              receipt['files'][name]=={k:item[k] for k in ('bytes','sha256')},'original member inventory differs')
        check(type(item['bytes'])is int and 0<=item['bytes']<=LIMITS['member_bytes'] and
              m['bytes']==item['bytes'] and stat.S_ISREG(m['mode']) and m['nlink']==1 and not(m['mode']&0o111),'original member metadata')
        parent=str(PurePosixPath(name).parent);check(('' if parent=='.' else parent) in dirs,'missing original parent')
        obj=fixture/row['object'];check(not obj.parent.is_symlink(),'object parent symlink')
        pin=digest_file(obj,records)
        check(pin=={'bytes':row['compressed_bytes'],'sha256':row['compressed_sha256']},'compressed hash/size differs')
        observation={'original_path':row['original_path'],'object':str(obj)};decoder_receipts.append(observation)
        full_decode(obj,item,observation)
        check(digest_file(obj,records)==pin,'object changed during decode')
        logical[row['original_path']]=dict(bytes=item['bytes'],sha256=item['sha256'],metadata=m,
                                         representation='zstd-files-v1',physical_object=str(obj),resident_original_path=False)
        total+=item['bytes'];compressed+=row['compressed_bytes']
    check(set(original['inventory']['files'])==set(receipt['files'])==seen,'full original member set differs')
    check(total==original['inventory']['logical_bytes'] and total<=LIMITS['cohort_bytes'] and
          total+catalog['prior_logical_bytes']<=LIMITS['campaign_bytes'],'decoded cohort/combined logical sum cap')
    physical=fixture/'retention-objects';check(physical.is_dir() and not physical.is_symlink(),'object directory')
    check({str(x.relative_to(fixture)) for x in physical.iterdir()}==objects,'unexpected/missing physical object set')
    codecs=catalog['codecs'];check(len(codecs)==2*len(members),'producer codec lifetime count')
    lives=set()
    for i,c in enumerate(codecs):
        item=members[i//2];mode='compress' if i%2==0 else 'decode'
        argv=[CODEC,'-q','--no-progress','--single-thread']+(['-3','--check','--zstd=wlog=26','-c'] if mode=='compress' else ['-d','--memory=64MB','-c'])
        check(c['mode']==mode and c['argv']==argv and c['complete']is True and c['exit_code']==0 and c['absent']is True and
              producer_lifetime_absent(c) and c['cpu_affinity']==c['observed_cpu_affinity']==CPUS and c['launched_executable_sha256']==CODEC_SHA,'producer codec ownership/exit')
        check(c['source']==str(Path(catalog['original_scratch'])/item['name'] if mode=='compress' else fixture/item['object']) and
              c['destination']==(str(fixture/item['object']) if mode=='compress' else None),'producer codec input/output binding')
        life=(c['pid'],c['start_ticks'],c['boot_id']);check(life not in lives,'duplicate codec lifetime');lives.add(life)
        check(catalog['started_unix_ns']<=c['started_unix_ns']<=c['ended_unix_ns']<=catalog['verified_unix_ns'] and
              c['ended_unix_ns']-c['started_unix_ns']<=LIMITS['codec_seconds']*10**9,'producer codec interval')
        check(c['bytes']==(item['compressed_bytes'] if mode=='compress' else item['original']['bytes']),'producer codec byte record')
        if mode=='decode':check(c['sha256']==item['original']['sha256'],'producer decode digest record')
    return dict(files=len(seen),logical_bytes=total,compressed_bytes=compressed,codec_lifetimes=len(codecs),
                started_unix_ns=receipt['started_unix_ns'],completed_unix_ns=receipt['completed_unix_ns'],
                decoded_all_original_bytes=True,resident_original_paths=False,prior_logical_bytes=catalog['prior_logical_bytes'])


def combined_physical(cohort_roots):
    """Independent allocated-byte census of both completed run trees, including dirs."""
    roots=[p.parent if p.name=='cohorts' else p for p in map(Path,cohort_roots)]
    check(len(roots)==2 and not roots[0].is_relative_to(roots[1]) and not roots[1].is_relative_to(roots[0]),'physical scope overlap')
    total=0;files=0;directories=0;seen=set();entries={}
    for root in roots:
        check(root.is_dir() and not root.is_symlink(),'completed physical root absent/linked')
        for base,dirs,names in os.walk(root,followlinks=False):
            for path in [Path(base)]+[Path(base)/name for name in names]:
                st=path.lstat();check(stat.S_ISREG(st.st_mode) or stat.S_ISDIR(st.st_mode),'unsupported physical entry')
                key=(st.st_dev,st.st_ino)
                if key not in seen:total+=st.st_blocks*512;seen.add(key)
                if stat.S_ISREG(st.st_mode):files+=1
                else:directories+=1
                entries[str(path)]={'device':st.st_dev,'inode':st.st_ino,'bytes':st.st_size,'blocks':st.st_blocks,
                                    'mode':st.st_mode,'mtime_ns':st.st_mtime_ns,'ctime_ns':st.st_ctime_ns}
            check(all(not (Path(base)/name).is_symlink() for name in dirs),'physical directory symlink')
    check(total<=LIMITS['campaign_bytes'],'combined independent physical cap')
    for path,m in entries.items():
        st=Path(path).lstat();check(m=={'device':st.st_dev,'inode':st.st_ino,'bytes':st.st_size,'blocks':st.st_blocks,
            'mode':st.st_mode,'mtime_ns':st.st_mtime_ns,'ctime_ns':st.st_ctime_ns},'physical snapshot changed')
    return {'complete':True,'roots':list(map(str,roots)),'allocated_bytes':total,'files':files,'directories':directories,
            'hardlinked_allocations_counted_once':True,'observed_unix_ns':time.time_ns(),'metadata':entries}
