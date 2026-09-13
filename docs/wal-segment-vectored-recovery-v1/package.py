"""Preserve all small original release/recovery evidence with exact byte readback."""
import gzip,hashlib,io,json,os,re,stat,tarfile,time
from pathlib import Path,PurePosixPath
HERE=Path(__file__).parent
OUT=Path('/home/dongxu/kv9/docs/wal-segment-vectored-recovery-v1')
ROOTS={
 'release-root':Path('/tmp/kv9-write-segment-vectored-release-root-first'),
 'release':Path('/tmp/kv9-write-segment-vectored-release-first'),
 'preparation':Path('/tmp/kv9-write-segment-vectored-recovery-preparation-first'),
 'process':Path('/tmp/kv9-write-segment-vectored-process-first'),
 'audit':Path('/tmp/kv9-write-segment-vectored-recovery-audit-first')}
EXCLUDED={'release/kv9','release/workload/kv9-batch-workload'}
G=1024**3;M=1024**2
used={}
def sha(p):
 with p.open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
def read(p):return json.loads(p.read_text())
def save(p,d):
 with p.open('x')as f:f.write(json.dumps(d,indent=2,sort_keys=True)+'\n')
def free():s=os.statvfs(OUT.parent);return s.f_bavail*s.f_frsize
def safe(n):p=PurePosixPath(n);return bool(n) and not p.is_absolute() and str(p)==n and '..' not in p.parts and '\\' not in n
assert __debug__
release=read(ROOTS['release-root']/'readback-first/result.json');audit=read(ROOTS['audit']/'audit.json')
assert release['complete'] and audit['complete'] and audit['accepted'] and audit['owned_lifetimes_exited']
assert release['revision']==audit['revision']=='cfd9c927f8ecd33974100f696e6b08b227d25a41'
for phase,path in [('release',ROOTS['release-root']/'terminal.json'),('release-readback',ROOTS['release-root']/'readback-first/terminal.json'),('process',ROOTS['preparation']/'process-terminal.json'),('audit',ROOTS['preparation']/'audit-terminal.json')]:
 t=read(path);assert t['complete'] and t['exit_code']==0 and t['terminal_receipt'];assert sha(Path(t['result_path']))==t['result_sha256']
for root,name in [(ROOTS['release-root'],'source-result.json'),(ROOTS['preparation'],'source-result.json')]:
 r=read(root/name);assert r['complete'] and not Path('/proc',str(r['pid'])).exists()
known=read(ROOTS['release-root']/'readback-first/input-hashes.json')
originals=read(ROOTS['audit']/'original-inputs.json')
for key,root in [('build_files',ROOTS['release']),('run_files',ROOTS['process'])]:
 for name,b in originals[key].items():known[str(root/name)]=b
files=[];omitted=[]
for label,root in ROOTS.items():
 for p in sorted(root.rglob('*')):
  assert not p.is_symlink(),p
  if not p.is_file():continue
  member=label+'/'+str(p.relative_to(root));s=p.stat();assert stat.S_ISREG(s.st_mode)
  if member in EXCLUDED:
   assert sha(p)==known[str(p)]['sha256'];omitted.append(dict(source=str(p),**known[str(p)]));continue
  assert safe(member) and s.st_size<=8*M
  files.append((p,member,s.st_size,s.st_dev,s.st_ino,s.st_mtime_ns))
assert sum(x[2]for x in files)<=16*M and len(files)<=1024
OUT.mkdir(exist_ok=False);start=time.time_ns();initial=free();assert initial-G>=96*G
archive=OUT/'original-evidence.tar.gz';members={};secret=re.compile(rb'(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{50,}|AKIA[0-9A-Z]{16}|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----)')
with archive.open('xb')as raw,gzip.GzipFile(filename='',fileobj=raw,mode='wb',compresslevel=6,mtime=0)as gz,tarfile.open(fileobj=gz,mode='w|')as tar:
 for p,name,size,dev,ino,mtime in files:
  s=p.stat();assert (s.st_size,s.st_dev,s.st_ino,s.st_mtime_ns)==(size,dev,ino,mtime)
  data=p.read_bytes();h=hashlib.sha256(data).hexdigest();assert len(data)==size and not data.startswith(b'\x7fELF') and not secret.search(data)
  if str(p)in known:assert known[str(p)]['bytes']==size and known[str(p)]['sha256']==h
  item=tarfile.TarInfo(name);item.size=size;item.mtime=0;item.mode=0o644;tar.addfile(item,io.BytesIO(data));members[name]=dict(source=str(p),bytes=size,sha256=h)
  assert free()>=max(96*G,initial-G) and time.time_ns()-start<120*10**9
assert archive.stat().st_size<=2*M
seen=set()
with tarfile.open(archive,'r:gz')as tar:
 for item in tar:
  assert safe(item.name) and item.isfile() and item.name not in seen and item.name in members
  b=members[item.name];assert item.size==b['bytes']
  with tar.extractfile(item)as f:data=f.read(item.size+1)
  assert len(data)==b['bytes'] and hashlib.sha256(data).hexdigest()==b['sha256']==sha(Path(b['source']))
  seen.add(item.name)
assert seen==set(members)
archive_total=sum(p.stat().st_size for p in Path('/home/dongxu/kv9/docs').rglob('original-evidence.tar.gz'))
assert archive_total<=64*M
index=dict(complete=True,scope='Exact original release/recovery evidence bytes; full original small process/WAL/history inputs included, retained ELF payloads separately hash-bound',archive=dict(name=archive.name,bytes=archive.stat().st_size,sha256=sha(archive)),files=members,excluded_executables=omitted,decoded_member_bytes=sum(x['bytes']for x in members.values()),all_members_decoded_and_compared=True,main_archive_allowance_bytes=64*M,main_archive_bytes_after=archive_total,initial_available_bytes=initial,final_available_bytes=free(),started_ns=start,ended_ns=time.time_ns())
save(OUT/'inventory.json',index)
summary=dict(complete=True,source_revision=release['revision'],source_files=release['source_files'],source_tree_sha256=release['source_tree_sha256'],server_sha256=release['server_sha256'],client_sha256=release['workload_sha256'],release_manifest_sha256=release['manifest_sha256'],release=release['release_terminal'],release_readback=read(ROOTS['release-root']/'readback-first/terminal.json'),recovery=read(ROOTS['preparation']/'process-terminal.json'),audit=read(ROOTS['preparation']/'audit-terminal.json'),histories=[dict(transport=c['transport'],operations=c['operations'],outcomes=c['outcomes'],history_sha256=c['history_sha256'],fresh_drained_voters=c['fresh_drained_voters'])for c in audit['cases']],server_lifetimes=audit['server_lifetimes'],client_lifetimes=audit['client_lifetimes'],accepted_complete_operations=sum(c['operations']for c in audit['cases']),ok_operations=sum(c['outcomes']['ok']for c in audit['cases']),unknown_operations=sum(c['outcomes'].get('unknown',0)for c in audit['cases']),source_and_inputs_unchanged=audit['source_unchanged'] and audit['original_inputs_unchanged'],actual_chaos_accepted=False,performance_accepted=False,default_promoted=False)
save(OUT/'validation-summary.json',summary)
print(json.dumps(dict(complete=True,files=len(members),decoded_bytes=index['decoded_member_bytes'],compressed_bytes=archive.stat().st_size,archive_sha256=sha(archive),inventory_sha256=sha(OUT/'inventory.json'),main_archive_bytes_after=archive_total)))
