"""Bounded streaming gzip/tar publication, derived from the accepted full21 publisher."""
import gzip, hashlib, json, os, re, shutil, stat, tarfile, time
from pathlib import Path, PurePosixPath
H=Path(__file__).resolve().parent
OUT=H/'package-first'
M=1024**2
LIMITS=dict(member_bytes=16*M,decoded_bytes=64*M,compressed_bytes=32*M,root_allocated_bytes=64*M,part_bytes=2*M,seconds=1200,host_floor_bytes=8*1024**3)
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def read(p):return json.loads(Path(p).read_text())
def safe(n):
 p=PurePosixPath(n);return bool(n)and not p.is_absolute()and str(p)==n and all(x not in ('.','..')for x in p.parts)and '\\'not in n
def identity(s):return(s.st_dev,s.st_ino,s.st_size,s.st_mtime_ns,s.st_ctime_ns,s.st_mode)
def available():
 s=os.statvfs(H);return s.f_bavail*s.f_frsize
def allocation():return sum(p.lstat().st_blocks*512 for p in [H,*H.rglob('*')])
assert __debug__
binding=read(H/'package-bindings.json')
assert binding['source_revision']=='e2e23cca5e70a9ea0cc241877b3b35b5b6433d27'and binding['limits']==LIMITS
assert sha(H/'selection.json')==binding['selection_sha256']
for n,v in binding['preparation_files'].items():
 assert safe(n)and (H/n).stat().st_size==v['bytes']and sha(H/n)==v['sha256'],n
for p,v in binding['terminal_inputs'].items():
 assert Path(p).stat().st_size==v['bytes']and sha(p)==v['sha256'],p
selected=read(H/'selection.json')['members']
for name,pin in binding['preparation_files'].items():selected.append(dict(original_path=str(H/name),archive_name='publication/'+name,**pin))
assert len({x['archive_name']for x in selected})==len(selected)
assert len({x['original_path']for x in selected})==len(selected)
assert sum(x['bytes']for x in selected)<=LIMITS['decoded_bytes']
started=time.time_ns();initial=available()
assert initial>=LIMITS['host_floor_bytes']+LIMITS['root_allocated_bytes']
assert not OUT.exists()and not(H/'package-result.json').exists()
OUT.mkdir()
files={};parts=[];samples=[];whole=hashlib.sha256();stored=0;current=None;part_hash=None;part_size=0
def guard():
 free=available();used=allocation()
 assert free>=max(LIMITS['host_floor_bytes'],initial-LIMITS['root_allocated_bytes'])
 assert used<=LIMITS['root_allocated_bytes']and time.time_ns()-started<1200*10**9
 samples.append(dict(observed_ns=time.time_ns(),available_bytes=free,root_allocated_bytes=used,completed_members=len(files)))
class Parts:
 def write(self,data):
  global current,part_hash,part_size,stored
  total=len(data);whole.update(data)
  while data:
   if current is None:
    assert len(parts)<16
    current=(OUT/f'evidence.tar.gz.{len(parts)+1:03d}').open('xb');part_hash=hashlib.sha256();part_size=0
   n=min(len(data),LIMITS['part_bytes']-part_size);piece,data=data[:n],data[n:]
   assert stored+n<=LIMITS['compressed_bytes']
   current.write(piece);part_hash.update(piece);part_size+=n;stored+=n
   if part_size==LIMITS['part_bytes']:self.finish()
  return total
 def flush(self):
  if current:current.flush()
 def finish(self):
  global current
  if current:
   current.flush();os.fsync(current.fileno());parts.append(dict(name=Path(current.name).name,bytes=part_size,sha256=part_hash.hexdigest()));current.close();current=None;guard()
secret=re.compile(rb'(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{50,}|AKIA[0-9A-Z]{16}|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----)')
class CheckedReader:
 def __init__(self,handle):self.handle=handle;self.digest=hashlib.sha256();self.count=0;self.tail=b''
 def read(self,size=-1):
  b=self.handle.read(size)
  if self.count==0:assert not b.startswith((b'\x7fELF',b'\x1f\x8b',b'\x28\xb5\x2f\xfd'))
  assert not secret.search(self.tail+b),'credential pattern in member'
  self.tail=(self.tail+b)[-512:];self.digest.update(b);self.count+=len(b);return b
result=dict(complete=False,started_ns=started,initial_available_bytes=initial,limits=LIMITS,source_revision=binding['source_revision'])
try:
 sink=Parts()
 with gzip.GzipFile(filename='',fileobj=sink,mode='wb',compresslevel=6,mtime=0)as gz,tarfile.open(fileobj=gz,mode='w|')as archive:
  for row in selected:
   p=Path(row['original_path']);name=row['archive_name']
   assert safe(name)and name not in files and 0<=row['bytes']<=LIMITS['member_bytes']
   before=p.lstat();assert stat.S_ISREG(before.st_mode)and before.st_size==row['bytes']
   with os.fdopen(os.open(p,os.O_RDONLY|os.O_NOFOLLOW),'rb')as f:
    assert identity(os.fstat(f.fileno()))==identity(before)
    reader=CheckedReader(f);entry=tarfile.TarInfo(name);entry.size=row['bytes'];entry.mode=0o644;entry.mtime=0
    archive.addfile(entry,reader)
    assert reader.count==row['bytes']and not f.read(1)and reader.digest.hexdigest()==row['sha256'],str(p)
    assert identity(os.fstat(f.fileno()))==identity(before)
   assert identity(p.lstat())==identity(before)
   files[name]=dict(bytes=row['bytes'],sha256=row['sha256'],original_path=str(p))
   if len(files)%25==0:guard()
 sink.finish()
 for n in ['verify.py','summary.json','README.md','package.py']:shutil.copyfile(H/n,OUT/n)
 top={n:dict(bytes=(OUT/n).stat().st_size,sha256=sha(OUT/n))for n in ['verify.py','summary.json','README.md','package.py']}
 index=dict(version=1,source_revision=binding['source_revision'],scope='Portable exact original-byte readback and history counts; original semantic acceptance is separately retained.',
  selection_sha256=binding['selection_sha256'],files=files,parts=parts,compressed_bytes=stored,compressed_sha256=whole.hexdigest(),
  decoded_member_count=len(files),decoded_member_bytes=sum(v['bytes']for v in files.values()),top_level=top,limits=LIMITS)
 with(OUT/'inventory.json').open('x')as f:json.dump(index,f,indent=2);f.write('\n');f.flush();os.fsync(f.fileno())
 guard();result.update(complete=True,members=len(files),decoded_bytes=index['decoded_member_bytes'],compressed_bytes=stored,
  compressed_sha256=whole.hexdigest(),parts=len(parts),inventory_sha256=sha(OUT/'inventory.json'))
except BaseException as error:result['failure']=repr(error);raise
finally:
 if current:current.close()
 result.update(ended_ns=time.time_ns(),final_available_bytes=available(),root_allocated_bytes=allocation())
 with(H/'package-result.json').open('x')as f:json.dump(result,f,indent=2);f.write('\n')
 with(H/'package-resource-samples.json').open('x')as f:json.dump(samples,f,indent=2);f.write('\n')
print(json.dumps(result))
