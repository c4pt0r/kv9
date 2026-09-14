"""Select finite completed correctness metadata; no archive/WAL/ELF reads."""
import hashlib,json,stat
from pathlib import Path
H=Path(__file__).resolve().parent
REV='e9249f2cbd069dcdc44312be826a68494cf694db'
P=Path('/tmp/kv9-frame-crc-chaos-preparation-20260914-first')
R=Path('/tmp/kv9-frame-crc-chaos-root-20260914-first')
roots=[Path('/tmp')/n for n in [
 'kv9-frame-crc-proof-20260914-first','kv9-frame-crc-slicing-proof-20260914-first',
 'kv9-frame-crc-source-root-20260914-first','kv9-frame-crc-source-20260914-first',
 'kv9-frame-crc-release-root-20260914-first','kv9-frame-crc-release-20260914-first',
 'kv9-frame-crc-recovery-root-20260914-first','kv9-frame-crc-recovery-20260914-first',
 'kv9-frame-crc-recovery-audit-20260914-first','kv9-frame-crc-auxiliary-root-20260914-first',
 'kv9-frame-crc-image-root-20260914-first','kv9-frame-crc-finalize-root-20260914-first',
 'kv9-frame-crc-chaos-root-20260914-first']]+[P/'independent',P/'cleanup']
suffixes={'.json','.jsonl','.stdout','.stderr','.log','.py','.smt2','.lean','.rs','.in','.md'}
files={};excluded=[]
def add(path):
 s=path.lstat()
 if not stat.S_ISREG(s.st_mode) or (path.suffix not in suffixes and path.name not in ['stdout','stderr','image.Dockerfile']) or path.name.startswith('cache-') or any(x in path.parts for x in ['.git','target','__pycache__']):
  excluded.append(dict(path=str(path),bytes=s.st_size,reason='Non-metadata, compiled proof output, runtime WAL/data, or non-regular entry; retained locally and/or inside original Chaos archive.'));return
 assert s.st_size<=8*1024**2,path
 data=path.read_bytes();key=path.as_posix().removeprefix('/')
 assert key not in files,path
 files[key]=dict(source=str(path),bytes=len(data),sha256=hashlib.sha256(data).hexdigest())
for root in roots:
 assert root.is_dir() and not root.is_symlink(),root
 for p in sorted(root.rglob('*')):
  if not p.is_dir():add(p)
for name in ['plan.json','ready-plan.json','timestamp-repair.json','input-bindings.json','recovery-binding.final.json',
 'release-binding.final.json','build-binding-first.json','check-build-binding.py','bind-builds.py','commands.json',
 'root-runtime-release.json','runtime-release-consumed.json','archive.py','image.Dockerfile',
 'archive-first/inventory.json','archive-first/result.json',
 'root-runners/run-runtime.py','root-runners/run-post.py']:
 p=P/name
 if p.exists():add(p)
 else:raise FileNotFoundError(p)
Q=Path('/tmp/kv9-frame-crc-qualification-preparation-20260914-first')
for name in ['check-source.py','run-release.py','run-recovery.py','process-runner.py','audit.py','commands.source-pinned.json','binding.release-final.json','inventory.final.json']:
 add(Q/name)
for p in [R/'terminal.json',R/'post-terminal.json',R/'result.json',R/'post-result.json',P/'independent/audit.json',P/'archive-first/result.json',P/'cleanup/summary.json',P/'cleanup/all-server-lifetimes-exited.json']:
 d=json.loads(p.read_text());assert d.get('complete') is True,p
assert json.loads((P/'independent/audit.json').read_text())['revision']==REV
assert json.loads((R/'post-result.json').read_text())['complete'] is True
assert len(files)<=1000 and sum(r['bytes'] for r in files.values())<=128*1024**2
archive=json.loads((P/'archive-first/result.json').read_text())
assert archive['all_members_read_back'] and archive['all_inputs_rehashed'] and archive['independent_retained_evidence_accepted']
assert Path(archive['archive']).stat().st_size==archive['bytes']
def save(n,d):(H/n).write_text(json.dumps(d,indent=2,sort_keys=True)+'\n')
save('metadata-selection.json',dict(complete=True,revision=REV,files=files,omitted_executables=excluded,
 decoded_bytes=sum(r['bytes'] for r in files.values()),scope='Original source/proof/release/recovery/Chaos metadata and complete ordinary histories; compiled proof files and ordinary recovery WAL payloads stay locally bound. Full raw Chaos evidence is separately preserved by its original accepted archive.'))
save('original-chaos-archive.json',dict(complete=True,scope='Original accepted archive identity copied from its completed readback; payload not read by this metadata preparation.',
 source=archive['archive'],bytes=archive['bytes'],sha256=archive['sha256'],
 inventory=archive['inventory'],inventory_sha256=archive['inventory_sha256'],members=archive['members'],files=archive['files'],file_bytes=archive['file_bytes'],
 all_members_read_back=archive['all_members_read_back'],all_inputs_rehashed=archive['all_inputs_rehashed']))
print(json.dumps(dict(complete=True,files=len(files),decoded_bytes=sum(r['bytes'] for r in files.values()),excluded=len(excluded),original_archive_bytes=archive['bytes'])))
