"""Single finite metadata archive; no payload input or mutation of originals."""
import gzip,hashlib,io,json,os,stat,tarfile,time
from pathlib import Path
O=Path(__file__).parent;start=time.monotonic();baseline=os.statvfs(O).f_bavail*os.statvfs(O).f_frsize
assert os.geteuid()==0 and set(os.sched_getaffinity(0))==set(range(6,16))|set(range(22,32))
def save(n,v):
 with (O/n).open('x')as f:json.dump(v,f,indent=2,sort_keys=True);f.write('\n');f.flush();os.fsync(f.fileno())
def digest(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def guard():
 free=os.statvfs(O).f_bavail*os.statvfs(O).f_frsize
 assert time.monotonic()-start<120 and free>=8*1024**3 and baseline-free<=64*1024**2
 return free
selected=json.loads((O/'source-selection.json').read_text())['original_members'];members=dict(selected)
for n in ['README.md','summary.json','source-selection.json','prepare.py','prepare.first.py','prepare-first-failure.json','prepare-tool-terminal.json','package.py','verify.py']:
 p=O/n;members['report/'+n]=dict(path=str(p),bytes=p.stat().st_size,sha256=digest(p))
assert len(members)<1500 and sum(v['bytes']for v in members.values())<32*1024**2
save('archive-manifest.json',dict(schema=1,members=members,source_selection_sha256=digest(O/'source-selection.json'),scope='Finite exact metadata; excluded payloads remain original local authorities.'))
with (O/'metadata.tar.gz').open('xb')as raw:
 with gzip.GzipFile(filename='',fileobj=raw,mode='wb',compresslevel=6,mtime=0)as compressed:
  with tarfile.open(fileobj=compressed,mode='w',format=tarfile.PAX_FORMAT)as archive:
   for name,row in sorted(members.items()):
    guard();p=Path(row['path']);s=p.lstat();assert p.resolve()==p and stat.S_ISREG(s.st_mode) and s.st_size==row['bytes']<=4*1024**2
    data=p.read_bytes();now=p.lstat();assert all(getattr(s,k)==getattr(now,k)for k in ['st_dev','st_ino','st_size','st_mode','st_mtime_ns','st_ctime_ns']) and hashlib.sha256(data).hexdigest()==row['sha256']
    item=tarfile.TarInfo(name);item.size=len(data);item.mode=0o644;item.mtime=0;archive.addfile(item,io.BytesIO(data));assert raw.tell()<=8*1024**2
 raw.flush();os.fsync(raw.fileno())
assert (O/'metadata.tar.gz').stat().st_size<=8*1024**2
save('package-result.json',dict(complete=True,archive=dict(path=str(O/'metadata.tar.gz'),bytes=(O/'metadata.tar.gz').stat().st_size,sha256=digest(O/'metadata.tar.gz')),manifest_sha256=digest(O/'archive-manifest.json'),member_count=len(members),file_bytes=sum(x['bytes']for x in members.values()),available_before=baseline,available_after=guard(),elapsed_seconds=time.monotonic()-start,original_roots_modified=False,payload_decode_or_reaudit=False))
print((O/'package-result.json').read_text())
