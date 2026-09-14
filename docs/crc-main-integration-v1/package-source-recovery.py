import gzip,hashlib,json,os,tarfile,time
from pathlib import Path
ROOT=Path(__file__).parent
OUT=Path('/tmp/kv9-crc-main-integration-evidence-worktree-20260914-first/docs/crc-main-integration-v1')
OUT.mkdir(parents=True)
roots=[Path(x) for x in [
 '/tmp/kv9-crc-main-integration-proof-20260914-first',
 '/tmp/kv9-crc-main-integration-source-root-20260914-first',
 '/tmp/kv9-crc-main-integration-source-20260914-first',
 '/tmp/kv9-crc-main-integration-release-root-20260914-first',
 '/tmp/kv9-crc-main-integration-release-20260914-first',
 '/tmp/kv9-crc-main-integration-recovery-20260914-first',
 '/tmp/kv9-crc-main-integration-recovery-audit-20260914-first']]
OMIT={'/tmp/kv9-crc-main-integration-release-20260914-first/kv9','/tmp/kv9-crc-main-integration-release-20260914-first/workload/kv9-batch-workload'}
files={};omitted=[]
sha=lambda p:hashlib.file_digest(p.open('rb'),'sha256').hexdigest()
for root in roots:
 for path in sorted(root.rglob('*')):
  assert not path.is_symlink(),path
  if not path.is_file():continue
  if str(path) in OMIT:
   omitted.append(dict(path=str(path),bytes=path.stat().st_size,sha256=sha(path)));continue
  assert path.stat().st_size<=8*1024**2,path
  key=root.name+'/'+str(path.relative_to(root));assert key not in files
  files[key]=dict(source=str(path),bytes=path.stat().st_size,sha256=sha(path))
prep=Path('/tmp/kv9-crc-main-recovery-preparation-20260914-first')
for name in ('commands.source-pinned.json','binding.final.json','run-process.py','process-runner.py','audit.py','helper-equivalence.json','source-first.log','source-invocation.json','source-result.json','actual-process-tool-terminal.json','actual-audit-tool-terminal.json'):
 path=prep/name;files[prep.name+'/'+name]=dict(source=str(path),bytes=path.stat().st_size,sha256=sha(path))
assert len(files)<=1000 and sum(v['bytes'] for v in files.values())<=128*1024**2
s=os.statvfs('/tmp');assert s.f_bavail*s.f_frsize>=97*1024**3
archive=ROOT/'source-recovery.tar.gz'
with archive.open('xb') as raw,gzip.GzipFile(filename='',fileobj=raw,mode='wb',mtime=0) as gz,tarfile.open(fileobj=gz,mode='w|') as tar:
 for name,row in sorted(files.items()):
  path=Path(row['source']);info=tarfile.TarInfo(name);info.size=row['bytes'];info.mode=0o644;info.mtime=0
  with path.open('rb') as f:tar.addfile(info,f)
  assert path.stat().st_size==row['bytes'] and sha(path)==row['sha256'],path
assert archive.stat().st_size<=32*1024**2
seen=set()
with tarfile.open(archive,'r:gz') as tar:
 for member in tar:
  assert member.name in files and member.name not in seen and member.isfile();row=files[member.name]
  assert member.size==row['bytes'];f=tar.extractfile(member);assert hashlib.file_digest(f,'sha256').hexdigest()==row['sha256'];seen.add(member.name)
assert seen==set(files)
parts=[]
with archive.open('rb') as src:
 while block:=src.read(2*1024**2):
  name=f'source-recovery.tar.gz.{len(parts)+1:03d}';(OUT/name).write_bytes(block);parts.append(dict(name=name,bytes=len(block),sha256=hashlib.sha256(block).hexdigest()))
index=dict(complete=True,source_revision='bd42e60f84657e22e36e34924a5c80a08eac623a',files=files,omitted_executables=omitted,parts=parts,decoded_bytes=sum(v['bytes'] for v in files.values()),compressed_bytes=archive.stat().st_size,compressed_sha256=sha(archive),scope='Original source/proof/build/recovery artifacts, with full proof and recovery WAL bytes. Executables referenced by original hashes. No Chaos or performance acceptance.')
(OUT/'source-recovery-inventory.json').write_text(json.dumps(index,indent=2,sort_keys=True)+'\n')
(ROOT/'result.json').write_text(json.dumps({k:v for k,v in index.items() if k not in ('files','omitted_executables','parts')},indent=2)+'\n')
print(json.dumps(dict(complete=True,files=len(files),decoded_bytes=index['decoded_bytes'],compressed_bytes=index['compressed_bytes'],parts=len(parts),inventory_sha256=sha(OUT/'source-recovery-inventory.json'))))
