#!/usr/bin/env python3
"""Root-preauthorized finite remaining-cohort migration. No ambiguous automatic retry."""
import argparse,contextlib,ctypes,fcntl,hashlib,importlib.util,json,os,re,signal,stat,subprocess,sys,time
from pathlib import Path
P=Path(__file__).resolve().parent
OLD=Path('/tmp/kv9-cross-voter-multicohort-migration-preparation-20260915-first')
OUT=Path('/tmp/kv9-cross-voter-multicohort-campaign-root-20260915-first')
EXEC=Path('/tmp/kv9-cross-voter-multicohort-execution-20260915-first')
CODE='66ac9d2f4e5fa8fc59f4fe77ca158ebba8fa40d183ed71b5ff1a6deab1920d7e'
GROUP='e0e9956c1334149ded0f23f2989f90e2ef54d3510a25293d30f1d5b7d66a86de'
COMMANDS='35db1b51a0ae37a7ba7c9958763ca6a90068a3724c568f725ac0c0b1ebdc554b'
ZERO_VERIFIED='56707101902185a165e18705a260a87d919a939a9772f27391d015f319285898'
GIB=1024**3;MIB=1024**2;FLOOR=8*GIB;CAP=256*MIB;EVENT_CAP=64*MIB
CPUS=set(range(6,16))|set(range(22,32));STOP=100000000000
DEADLINES={'stage-verify':4500,'finish':9000};CAMPAIGN_SECONDS=172800


def need(ok,message):
 if not ok:raise RuntimeError(message)
def canonical(p):
 p=Path(p);need(p.is_absolute()and str(p)==os.path.normpath(str(p)),'noncanonical path')
 for q in [p,*p.parents]:need(not q.is_symlink(),'symlink path/ancestor')
 return p
def absent(p):return not p.exists()and not p.is_symlink()
def identity(p):
 s=canonical(p).lstat();need(stat.S_ISREG(s.st_mode)and s.st_nlink==1,'nonordinary file')
 return dict(device=s.st_dev,inode=s.st_ino,mode=s.st_mode,uid=s.st_uid,gid=s.st_gid,nlink=s.st_nlink,bytes=s.st_size,mtime_ns=s.st_mtime_ns,ctime_ns=s.st_ctime_ns,allocated_bytes=s.st_blocks*512)
def pin(p,limit=128*MIB):
 before=identity(p);need(before['bytes']<=limit,'bounded file exceeded');h=hashlib.sha256();count=0
 fd=os.open(p,os.O_RDONLY|os.O_NOFOLLOW|os.O_NOATIME)
 with os.fdopen(fd,'rb')as f:
  st=os.fstat(f.fileno());need((st.st_dev,st.st_ino)==(before['device'],before['inode']),'opened file changed')
  while b:=f.read(MIB):count+=len(b);h.update(b)
 need(identity(p)==before,'file changed while read');return dict(bytes=count,sha256=h.hexdigest())
def check(p,expected):
 got=pin(p);need(got=={k:expected[k]for k in ['bytes','sha256']},'file SHA/size binding differs');return got
def js(p):
 def unique(rows):
  d={}
  for k,v in rows:need(k not in d,'duplicate JSON key');d[k]=v
  return d
 need(identity(p)['bytes']<=16*MIB,'JSON size cap');return json.loads(Path(p).read_text(),object_pairs_hook=unique)
def syncdir(p):
 fd=os.open(p,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
 try:os.fsync(fd)
 finally:os.close(fd)
def mkdir(p):canonical(p).mkdir(mode=0o700,exist_ok=True);syncdir(p.parent)
def save(p,data):
 canonical(p)
 with p.open('x')as f:json.dump(data,f,sort_keys=True);f.write('\n');f.flush();os.fsync(f.fileno())
 syncdir(p.parent)
def allocated(root):
 total=0
 for base,dirs,names in os.walk(root,followlinks=False):
  for p in [Path(base)]+[Path(base)/n for n in names]:
   s=p.lstat();need(not stat.S_ISLNK(s.st_mode),'metadata tree symlink');total+=s.st_blocks*512
  need(not any((Path(base)/d).is_symlink()for d in dirs),'metadata directory symlink')
 return total

def lifetime(pid):
 try:
  raw=Path(f'/proc/{pid}/stat').read_text();fields=raw[raw.rfind(')')+2:].split()
  return dict(pid=int(pid),start_ticks=int(fields[19]),boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())
 except (FileNotFoundError,ProcessLookupError):return None

def gone(row):
 current=lifetime(row['pid']);need(current is None or any(current[k]!=row[k]for k in ['start_ticks','boot_id']),'same owned child lifetime still present')
def outcome(row):
 need(row.get('complete')is True and row.get('exit_code')==0 and row.get('reaped',row.get('child_reaped'))is True,'unsuccessful or unknown child outcome');gone(row)
def exact_rows(rows,expected):
 by={r['id']:r for r in rows};need(len(by)==len(rows)==len(expected)and set(by)==set(expected),'missing/extra/duplicate exact target rows');return by

def pair_proofs(s,stage,verified):
 pairs={p['id']:p for p in s['pairs']};need(len(pairs)==s['target_count'],'pair cardinality');sr=exact_rows(stage['rows'],pairs);vr=exact_rows(verified['rows'],pairs);mm={m['id']:m for m in s['members']}
 need(len(mm)==s['original_object_count']==len(s['members']),'exact original count')
 for p in s['pairs']:
  a=sr[p['id']];b=vr[p['id']];need(a['pair']==p and b['base']==p['base']and b['patch']==a['patch'],'pair/base/patch lineage')
  need(b['decoded_to_eof']is True and b['original_compressed_exact']is True and b['original']==mm[p['target']]['original']and b['compressed']==mm[p['target']]['compressed'],'independent raw/old-encoded proof binding')
 return sr,vr,mm

def stop_reason(free,next_ordinal):
 if free>=STOP:return 'actual_available_target'
 if next_ordinal==96:return 'finite_exhaustion'
 return None

def original_modules():
 need(pin(OLD/'code-pins.json')['sha256']==CODE and pin(OLD/'group-plan.json')['sha256']==GROUP and pin(OLD/'ROOT-COMMANDS.json')['sha256']==COMMANDS,'frozen original preparation changed')
 for name,expected in js(OLD/'code-pins.json').items():check(OLD/name,expected)
 sys.path.insert(0,str(OLD));spec=importlib.util.spec_from_file_location('frozen_group_dispatcher',OLD/'run.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
 return m

@contextlib.contextmanager
def dispatcher_read_lock():
 # Existing dispatcher owns this same lock during child execution. Never hold it while launching that dispatcher.
 fd=os.open(EXEC/'serial.lock',os.O_RDWR|os.O_NOFOLLOW);fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
 try:yield
 finally:os.close(fd)

def selected(group,i):
 need(type(i)is int and 0<=i<96,'finite ordinal');row=group['rows'][i];need(row['ordinal']==i,'frozen ordinal ordering');check(row['selection']['path'],row['selection']);s=js(row['selection']['path']);need(s['group_ordinal']==i and s['root']==row['root'],'selection root binding');return row,s

def phase_receipt(i,name,path,expected):
 doc=js(EXEC/f'{i:03d}'/(name+'-complete.json'));need(doc['complete']and doc['exit_code']==0 and doc['child_reaped']and doc['result_path']==str(path),'dispatcher phase terminal differs');gone(doc)
 need(pin(path)['sha256']==doc['result_sha256'],'phase result changed');r=js(path);need(r['complete'],'phase result incomplete')
 for k,v in expected.items():need(r.get(k)==v,'phase lineage differs: '+k)
 inv=js(doc['invocation']);need(inv['argv']==doc['argv']and inv['result_path']==str(path),'phase invocation differs')
 return r

def verify_release(group,i):
 """Independent bounded readback of authority/identities/outcomes; no second codec run."""
 row,s=selected(group,i);root=Path(s['root']);sp=row['selection']['sha256'];same=dict(selection_sha256=sp,code_pins_sha256=CODE)
 need(absent(root/'retire-1'),'cannot release an already retired cohort')
 stage=phase_receipt(i,'stage',root/'staged.json',dict(same,state='STAGED'))
 verified=phase_receipt(i,'verify',root/'verification-first/result.json',dict(same,state='VERIFIED',staged_sha256=pin(root/'staged.json')['sha256']))
 need(stage['all_children_reaped']and verified['all_children_reaped']and stage['toolchain_sha256']==verified['toolchain_sha256'],'stage/verify child/toolchain binding')
 sr,vr,mm=pair_proofs(s,stage,verified);hashed=set();validation_started=time.monotonic()
 for p in s['pairs']:
  resource_guard(validation_started,1200);a=sr[p['id']];b=vr[p['id']]
  for mid in [p['base'],p['target']]:
   m=mm[mid];need(identity(m['path'])==m['identity'],'original/base identity differs')
   if mid not in hashed:check(m['path'],m['compressed']);hashed.add(mid)
  patchpath=root/'patches'/(p['id']+'.zst');need(a['patch']['path']==str(patchpath)and identity(patchpath)==a['patch']['identity'],'patch destination/identity differs');check(patchpath,a['patch']['pin'])
  need(a['patch']['pin']['bytes']<=p['patch_limit'],'patch output cap')
 for m in s['members']:
  need(identity(m['path'])==m['identity'],'complete resident original inventory differs')
 for r in s['metadata']+s['authorities']:check(r['path'],r)
 tc=root/'toolchain.json';need(pin(tc)['sha256']==verified['toolchain_sha256'],'toolchain manifest pin');tool=js(tc);need(tool['complete']and tool['selection_sha256']==sp,'toolchain lineage')
 for f in tool['files']:need(identity(f['path'])==f['identity'],'retained toolchain identity');check(f['path'],f['pin'])
 expected_names={r['name']for r in s['retained_codec_dependencies']};need({r['name']for r in tool['files']}==expected_names and len(tool['files'])==len(expected_names),'complete exact codec dependency set')
 child_paths=sorted(root.rglob('*.child.json'));need(len(child_paths)==6*s['target_count'],'exact successful stage/verify codec count')
 for f in child_paths:
  r=js(f);outcome(r);need(r['output_bytes']<=r['limit_bytes']<=128*MIB,'codec output limit');need(set(r['cpu_affinity'])==CPUS and r['rlimit_as_bytes']==512*MIB,'codec runtime limits')
  intent=f.with_name(f.name.replace('.child.json','.child-intent.json'));start=f.with_name(f.name.replace('.child.json','.child-start.json'));a=js(intent);b=js(start)
  need(a['argv']==r['argv']==b['argv']and a['output']==r['output']==b['output']and all(b[k]==r[k]for k in ['pid','start_ticks','boot_id']),'durable codec invocation/lifetime chain')
 need(not list(root.rglob('*failure*.json')),'failed attempt remains; no automatic release')
 return dict(complete=True,ordinal=i,selection=row['selection'],staged_sha256=pin(root/'staged.json')['sha256'],verified_path=str(root/'verification-first/result.json'),verified_sha256=pin(root/'verification-first/result.json')['sha256'],toolchain_sha256=verified['toolchain_sha256'],targets=len(s['pairs']),checked_resident_objects=len(mm),successful_codec_lifetimes=len(child_paths),scope='Independent exact metadata/payload digest and existing verifier proof readback; full decoded WAL and old encoded proof comes from the pinned independent verifier, not another codec pass.')

def completed(group,i,verification_sha):
 row,s=selected(group,i);root=Path(s['root']);sp=row['selection']['sha256'];binding=dict(selection_sha256=sp,verified_sha256=verification_sha)
 restore=phase_receipt(i,'restore-1',root/'restore-1/result.json',dict(binding,state='RESTORED'))
 rsha=pin(root/'restore-1/result.json')['sha256'];reader=phase_receipt(i,'readback-1',root/'readback-1/result.json',dict(selection_sha256=sp,restore_result_sha256=rsha,readback_reader=s['readback_reader'],original_reader=s['legacy_reader'],readback_scope=s['readback_scope']))
 cold=phase_receipt(i,'retire-2',root/'retire-2/result.json',dict(binding,state='COLD',cycle=2))
 targets={p['target']for p in s['pairs']};exact_rows(cold['rows'],{p['id']for p in s['pairs']});restored=exact_rows(restore['rows'],targets);need(all(x['original_bytes_exact']for x in restored.values()),'restore byte evidence')
 stage=js(root/'staged.json');verified=js(root/'verification-first/result.json');need(pin(root/'verification-first/result.json')['sha256']==verification_sha and pin(root/'staged.json')['sha256']==verified['staged_sha256'],'completed verification/stage pins changed');pair_proofs(s,stage,verified)
 for r in stage['rows']:need(identity(r['patch']['path'])==r['patch']['identity'],'completed patch identity changed')
 for r in s['metadata']:check(r['path'],r)
 need(reader['observed']['files']==s['original_object_count']and reader['observed']['logical_bytes']==s['logical_cohort_bytes']and reader['observed']['compressed_bytes']==s['encoded_cohort_bytes']and reader['observed']['decoded_all_original_bytes'],'whole restored-cohort scope')
 for m in s['members']:
  if m['id']in targets:need(absent(Path(m['path'])),'claimed COLD object still resident/restored')
  else:need(identity(m['path'])==m['identity'],'preserved object identity differs')
 for d in reader['decoder_receipts']:need(d['complete']and d['exit_code']==0 and d['absent'],'readback codec incomplete');gone(d)
 for f in root.rglob('*.child.json'):outcome(js(f))
 return dict(ordinal=i,selection_sha256=sp,verified_sha256=verification_sha,restore_sha256=rsha,readback_sha256=pin(root/'readback-1/result.json')['sha256'],cold_sha256=pin(root/'retire-2/result.json')['sha256'],state='COLD',complete=True)

def zero_gate(group,terminal,terminal_sha,child,child_sha):
 need(pin(terminal)['sha256']==terminal_sha and pin(child)['sha256']==child_sha,'cohort000 terminal pins');t=js(terminal);c=js(child);root=Path(group['rows'][0]['root']);result=root/'retire-2/result.json'
 need(t['session_id']==56949 and t['initial_chunk']=='aa2cc1'and re.fullmatch('[0-9a-f]+',t['final_chunk'])and t['exit_code']==0,'actual cohort000 tool terminal')
 need(c['complete']and c['exit_code']==0 and t['result_path']==c['result_path']==str(result)and t['result_sha256']==c['result_sha256']==pin(result)['sha256'],'cohort000 tool/child/result binding')
 need(str(OLD/'run.py')in c['argv']and '--mode'in c['argv']and c['argv'][c['argv'].index('--mode')+1]=='finish'and c['argv'][c['argv'].index('--ordinal')+1]=='0'and c['argv'][c['argv'].index('--release-verified-sha256')+1]==ZERO_VERIFIED,'cohort000 actual finish command')
 need(pin(root/'verification-first/result.json')['sha256']==ZERO_VERIFIED,'cohort000 verified binding')
 return completed(group,0,ZERO_VERIFIED)

def zero_audit_binding(group,a):
 b=js(P/'cohort-zero-binding.json')
 for r in b.values():check(r['path'],r)
 for supplied,sha,key in [(a.cohort_zero_terminal,a.cohort_zero_terminal_sha256,'group-000-finish-tool-terminal.json'),(a.cohort_zero_child_terminal,a.cohort_zero_child_terminal_sha256,'group-000-finish-child-terminal.json')]:
  need(supplied==b[key]['path']and sha==b[key]['sha256'],'actual frozen cohort000 terminal path/pin required')
 summary=js(b['group-000-accepted-summary.json']['path']);terminal=js(b['group-000-audit-tool-terminal.json']['path'])
 need(terminal['exit_code']==0 and terminal['session_id']==88896 and terminal['final_chunk']=='109415'and terminal['result_sha256']==b['group-000-accepted-summary.json']['sha256'],'actual cohort000 independent audit terminal')
 row,selection=selected(group,0)
 need(summary['complete']and summary['state']=='COLD'and summary['ordinal']==0 and summary['selection_sha256']==row['selection']['sha256']and summary['targets']==226 and summary['objects']==369 and summary['logical_bytes']==11249173926,'cohort000 accepted exact scope')
 need(summary['all_original_metadata_unchanged']and len(summary['preserved_objects'])==143 and len(summary['codec_lifetimes'])==2403 and all(not x['same_lifetime_present']for x in summary['codec_lifetimes']),'cohort000 independent preservation/lifetime acceptance')
 need(summary['readback_reader']==selection['readback_reader']and summary['original_reader']==selection['legacy_reader']and summary['readback_scope']==selection['readback_scope'],'cohort000 honest reader binding')
 for name,sha in summary['phase_hashes'].items():need(pin(Path(row['root'])/name)['sha256']==sha,'actual cohort000 accepted phase changed')
 return zero_gate(group,a.cohort_zero_terminal,a.cohort_zero_terminal_sha256,a.cohort_zero_child_terminal,a.cohort_zero_child_terminal_sha256)

def resume_prefix(root):
 """Only an exact completed prefix can be resumed. Unknown/partial ordinals are never rerun."""
 done=[]
 for i in range(1,96):
  d=root/f'{i:03d}'
  if absent(d):
   need(not any(not absent(root/f'{j:03d}')for j in range(i+1,96)),'noncontiguous campaign state');return done,i
  need((d/'complete.json').is_file(),'incomplete/unknown ordinal: preserve and reconcile manually; no automatic retry')
  r=js(d/'complete.json');need(r['complete']and r['ordinal']==i,'completed ordinal binding');done.append(r)
 return done,96

def pdeath(parent):
 libc=ctypes.CDLL(None,use_errno=True);need(libc.prctl(1,signal.SIGKILL,0,0,0)==0,'parent-death guard unavailable')
 if os.getppid()!=parent:os.kill(os.getpid(),signal.SIGKILL)

def available():
 v=os.statvfs(OUT);return v.f_bavail*v.f_frsize

def resource_guard(start=None,deadline=None):
 need(available()>=FLOOR,'8 GiB actual available floor');need(allocated(OUT)<=CAP,'256 MiB controller metadata/output cap')
 if start is not None:need(time.monotonic()-start<=deadline,'controller deadline')

def event(data):
 p=OUT/'events.jsonl';row=dict(observed_ns=time.time_ns(),**data);b=(json.dumps(row,sort_keys=True)+'\n').encode();need(len(b)<=64*1024,'event member cap')
 fd=os.open(p,os.O_CREAT|os.O_WRONLY|os.O_APPEND|os.O_NOFOLLOW,0o600)
 try:need(os.fstat(fd).st_size+len(b)<=EVENT_CAP,'event aggregate cap');os.write(fd,b);os.fsync(fd)
 finally:os.close(fd)
 # The only replaced file is a progress pointer; every prior observation remains in events.jsonl.
 tmp=OUT/'status.next.json';need(absent(tmp),'incomplete prior progress publication');save(tmp,row);os.replace(tmp,OUT/'status.json');syncdir(OUT)

def invoke(i,mode,release=None):
 folder=OUT/f'{i:03d}'/mode;need(absent(folder),'never restart an existing phase handle');mkdir(folder)
 argv=[sys.executable,str(OLD/'run.py'),'--code-pins-sha256',CODE,'--mode',mode,'--ordinal',str(i)]
 if release is not None:argv+=['--release-verified-sha256',release]
 save(folder/'invocation.json',dict(argv=argv,ordinal=i,mode=mode,started_ns=time.time_ns(),deadline_seconds=DEADLINES[mode]));child=None;failure=None;start=time.monotonic();parent=os.getpid()
 try:
  with (folder/'stdout').open('xb')as out,(folder/'stderr').open('xb')as err:
   child=subprocess.Popen(argv,stdin=subprocess.DEVNULL,stdout=out,stderr=err,start_new_session=True,preexec_fn=lambda:pdeath(parent));birth=lifetime(child.pid);need(birth is not None,'unknown child start identity');save(folder/'child.json',dict(argv=argv,**birth))
   while child.poll()is None:
    resource_guard(start,DEADLINES[mode]);need((folder/'stdout').stat().st_size<=MIB and (folder/'stderr').stat().st_size<=MIB,'bounded controller child output')
    event(dict(state='RUNNING',ordinal=i,phase=mode,child=birth,available_bytes=available()));time.sleep(5)
   code=child.wait();out.flush();os.fsync(out.fileno());err.flush();os.fsync(err.fileno())
  need((folder/'stdout').stat().st_size<=MIB and (folder/'stderr').stat().st_size<=MIB,'final child output bound')
  doc=dict(complete=code==0,exit_code=code,child_reaped=True,argv=argv,**birth,ended_ns=time.time_ns(),stdout=pin(folder/'stdout'),stderr=pin(folder/'stderr'));save(folder/'terminal.json',doc);need(code==0,'dispatcher phase failed');gone(birth)
  text=(folder/'stdout').read_text().strip();result=json.loads(text);need(result['complete']and result.get('ordinal')==i and result.get('mode')==mode,'dispatcher result differs/stop before stage');save(folder/'result.json',result);return result
 except BaseException as exc:failure=repr(exc);raise
 finally:
  if child is not None and child.poll()is None:
   try:os.killpg(child.pid,signal.SIGKILL)
   except ProcessLookupError:pass
   child.wait()
  if failure is not None:save(folder/'failure.json',dict(complete=False,error=failure,exit_code=None if child is None else child.returncode,scope='Failed/unknown ordinal remains non-resumable automatically. Owned child group terminated; no original or partial transaction file removed. Descendant uncertainty requires root inspection.'))

def accounting(original,group):
 a=original.account(group);extra=allocated(OUT)+allocated(P);a['campaign_controller_allocated_bytes']=extra;a['conservative_net_allocated_change_bytes']-=extra
 a['scope']+=' Additional campaign preparation and all campaign output are charged. Includes completed cohort000 as explicitly separate bootstrap history; actual f_bavail is authoritative.'
 a['benchmark_runtime_ready']=False;return a

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--mode',choices=['run','status'],required=True);ap.add_argument('--inventory-sha256');ap.add_argument('--cohort-zero-terminal');ap.add_argument('--cohort-zero-terminal-sha256');ap.add_argument('--cohort-zero-child-terminal');ap.add_argument('--cohort-zero-child-terminal-sha256');a=ap.parse_args()
 if a.mode=='status':print(json.dumps(js(OUT/'status.json')));return 0
 need(os.geteuid()==0 and set(os.sched_getaffinity(0))==CPUS,'root and exact helper CPUs required');need(a.inventory_sha256 and pin(P/'inventory.json')['sha256']==a.inventory_sha256,'campaign inventory pin')
 for r in js(P/'inventory.json')['rows']:check(P/r['path'],r)
 original=original_modules();group=js(OLD/'group-plan.json');need(len(group['rows'])==96 and group['actual_available_stop_target_bytes']==STOP,'fixed campaign/target');zero=zero_audit_binding(group,a)
 mkdir(OUT);fd=os.open(OUT/'campaign.lock',os.O_CREAT|os.O_RDWR|os.O_NOFOLLOW,0o600);fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB);started=time.monotonic()
 def interrupted(signum,frame):raise InterruptedError('controller signal '+str(signum))
 signal.signal(signal.SIGTERM,interrupted);signal.signal(signal.SIGINT,interrupted)
 try:
  binding=dict(inventory_sha256=a.inventory_sha256,code_pins_sha256=CODE,group_sha256=GROUP,cohort_zero=zero,cohort_zero_terminal=dict(path=a.cohort_zero_terminal,sha256=a.cohort_zero_terminal_sha256),cohort_zero_child_terminal=dict(path=a.cohort_zero_child_terminal,sha256=a.cohort_zero_child_terminal_sha256))
  if absent(OUT/'binding.json'):save(OUT/'binding.json',binding)
  else:need(js(OUT/'binding.json')==binding,'campaign resume binding changed')
  done,next_i=resume_prefix(OUT)
  for r in done:need(completed(group,r['ordinal'],r['verified_sha256'])==r['acceptance'],'completed prefix changed/restored');check(OUT/f"{r['ordinal']:03d}"/'release.json',r['release_pin'])
  for i in range(next_i,96):
   resource_guard(started,CAMPAIGN_SECONDS);status=accounting(original,group);need(not status['incomplete_or_restored_cohorts'],'unknown incomplete/restored group state')
   if stop_reason(status['actual_available_bytes'],i)=='actual_available_target':
    event(dict(state='COMPLETE',reason='actual_available_target',next_ordinal=i,completed_ordinals=len(done),target_reached=True,available_bytes=status['actual_available_bytes']));save(OUT/('result-'+str(time.time_ns())+'.json'),dict(complete=True,reason='actual_available_target',completed_ordinals=len(done),target_reached=True,accounting=status));return 0
   row,s=selected(group,i);need(absent(Path(s['root']))and absent(EXEC/f'{i:03d}'),'next ordinal has existing transaction/handle; refuse automatic adoption')
   need(available()>=s['limits']['launch_available_bytes']+256*MIB+CAP,'fresh whole transaction plus both output allowances')
   folder=OUT/f'{i:03d}';mkdir(folder);event(dict(state='RUNNING',ordinal=i,phase='stage-verify',available_bytes=available()));invoke(i,'stage-verify')
   with dispatcher_read_lock():release=verify_release(group,i)
   save(folder/'release.json',dict(release,root_preauthorized_deterministic_release=True,observed_ns=time.time_ns()));release_pin=pin(folder/'release.json')
   event(dict(state='RUNNING',ordinal=i,phase='finish',verified_sha256=release['verified_sha256'],available_bytes=available()));invoke(i,'finish',release['verified_sha256'])
   acceptance=completed(group,i,release['verified_sha256']);status=accounting(original,group);r=dict(complete=True,ordinal=i,verified_sha256=release['verified_sha256'],acceptance=acceptance,release_pin=release_pin,accounting=status)
   save(folder/'complete.json',r);done.append(r);event(dict(state='COHORT_COMPLETE',ordinal=i,completed_ordinals=len(done),available_bytes=status['actual_available_bytes']))
  status=accounting(original,group);doc=dict(complete=True,reason='finite_exhaustion',completed_ordinals=len(done),target_reached=status['actual_available_bytes']>=STOP,accounting=status);save(OUT/('result-'+str(time.time_ns())+'.json'),doc);event(dict(state='COMPLETE',reason='finite_exhaustion',completed_ordinals=len(done),target_reached=doc['target_reached'],available_bytes=status['actual_available_bytes']));return 0
 except BaseException as exc:
  doc=dict(complete=False,error=repr(exc),scope='No automatic rerun of failed/partial/unknown ordinal; all originals, patches, archives, invocations and first failures retained.');save(OUT/('failure-'+str(time.time_ns())+'.json'),doc)
  try:event(dict(state='FAILED',error=repr(exc),available_bytes=available()))
  except BaseException:pass
  return 1
 finally:os.close(fd)
if __name__=='__main__':sys.exit(main())
