import gzip,hashlib,json,os,tarfile,time
from pathlib import Path
ROOT=Path(__file__).parent
import argparse
ap=argparse.ArgumentParser();ap.add_argument('--output',type=Path,required=True);args=ap.parse_args()
OUT=args.output.resolve()
OUT.mkdir(parents=True)
selection_path=ROOT/'metadata-selection.json'
selection_bytes=selection_path.read_bytes()
assert hashlib.sha256(selection_bytes).hexdigest()=='274ff63cdafe4daffa3f4561182ab7f212a6551b4f8ae4367797ed17be39571a'
selected=json.loads(selection_bytes);assert selected['complete'] is True
files=selected['files'];omitted=selected['omitted_executables']
sha=lambda p:hashlib.file_digest(p.open('rb'),'sha256').hexdigest()
for name,row in files.items():
 path=Path(row['source']);assert path.is_file() and not path.is_symlink()
 assert path.stat().st_size==row['bytes'] and sha(path)==row['sha256'],path
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
index=dict(complete=True,source_revision='e9249f2cbd069dcdc44312be826a68494cf694db',files=files,omitted_executables=omitted,parts=parts,decoded_bytes=sum(v['bytes'] for v in files.values()),compressed_bytes=archive.stat().st_size,compressed_sha256=sha(archive),scope=selected['scope'])
(OUT/'source-recovery-inventory.json').write_text(json.dumps(index,indent=2,sort_keys=True)+'\n')
(ROOT/'result.json').write_text(json.dumps({k:v for k,v in index.items() if k not in ('files','omitted_executables','parts')},indent=2)+'\n')
print(json.dumps(dict(complete=True,files=len(files),decoded_bytes=index['decoded_bytes'],compressed_bytes=index['compressed_bytes'],parts=len(parts),inventory_sha256=sha(OUT/'source-recovery-inventory.json'))))
