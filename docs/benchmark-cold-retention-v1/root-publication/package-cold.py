import gzip,hashlib,io,json,os,tarfile
from pathlib import Path
root=Path('/tmp/kv9-benchmark-cold-publication-first')
out=Path('/tmp/kv9-release-thin-lto-evidence/docs/benchmark-cold-retention-v1')
raw=(root/'publication-inventory.json').read_bytes();assert hashlib.sha256(raw).hexdigest()=='20a62ee1197bf56eae39cf28b33b3c88cdd4ba53f8fd4220259c8083b2d98e32'
for name,expected in json.loads(raw).items():
 data=(root/name).read_bytes();assert len(data)==expected['bytes'] and hashlib.sha256(data).hexdigest()==expected['sha256'],name
assert hashlib.sha256((root/'READINESS.json').read_bytes()).hexdigest()=='a3cb3b41d563e83b3872cf7d4716e294558d89411829efa3738d66359e0ebc7f'
files={}
for p in sorted(root.rglob('*')):
 if p.is_file():
  assert not p.is_symlink();d=p.read_bytes();assert len(d)<64*1024*1024
  files[str(p.relative_to(root))]=(p,d)
archive=io.BytesIO()
with tarfile.open(fileobj=archive,mode='w') as t:
 for name,(_,data) in files.items():
  entry=tarfile.TarInfo(name);entry.size=len(data);entry.mode=0o644;entry.mtime=0;t.addfile(entry,io.BytesIO(data))
encoded=gzip.compress(archive.getvalue(),mtime=0)
v=os.statvfs('/tmp');assert v.f_bavail*v.f_frsize-len(encoded)-2*1024**2>96*1024**3,'Retain original disk floor'
out.mkdir()
index={'scope':'Exact metadata-only cold-retention publication. Includes prior failed metadata draft. No WAL/object/binary payload, payload revalidation, restore, backup or performance acceptance. See README for cold paths and capacity requirements.','files':{},'parts':[],'top_level':{}}
for name,(p,data) in files.items():index['files'][name]={'source':str(p),'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}
for start in range(0,len(encoded),2*1024**2):
 data=encoded[start:start+2*1024**2];name=f'evidence.tar.gz.{len(index["parts"])+1:03d}';(out/name).write_bytes(data);index['parts'].append({'name':name,'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()})
for name in ['README.md','restore-commands.json','outcome.json','READINESS.json']:
 data=(root/name).read_bytes();(out/name).write_bytes(data);index['top_level'][name]={'bytes':len(data),'sha256':hashlib.sha256(data).hexdigest()}
(out/'inventory.json').write_text(json.dumps(index,indent=2,sort_keys=True)+'\n')
(out/'verify.py').write_bytes(Path('/tmp/kv9-release-thin-lto-evidence/docs/release-thin-lto-performance-v1/verify.py').read_bytes())
print(json.dumps({'files':len(files),'decoded_bytes':sum(len(d) for _,d in files.values()),'compressed_bytes':len(encoded),'parts':len(index['parts']),'scope':'metadata only'},indent=2))
