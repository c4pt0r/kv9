import gzip,hashlib,io,json,os,re,shutil,stat,tarfile,time
from pathlib import Path,PurePosixPath
ROOT=Path(__file__).parent;P=Path('/tmp/kv9-write-redis3-publication-preparation-first');OUT=Path('/tmp/kv9-write-redis3-baseline-publication-first/docs/write-redis3-baseline-v1');G=1024**3
read=lambda p:json.loads(Path(p).read_text())
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def avail():s=os.statvfs('/tmp');return s.f_bavail*s.f_frsize
def safe(n):p=PurePosixPath(n);return bool(n) and not p.is_absolute() and str(p)==n and all(x not in ('.','..') for x in p.parts) and '\\' not in n
assert __debug__
assert sha(P/'inventory.json')=='f1be9ec2bb0b85dafbfb1b3ed63a823b9f680a8e1574ad7a6789cbfeb491ad45'
assert sha(P/'proposed-members.json')=='0bc7bf58f5c4055bf183723875f2fa01fa5774110ff837cad11899247b6b694a'
for name,b in read(P/'inventory.json')['files'].items():assert (P/name).stat().st_size==b['bytes'] and sha(P/name)==b['sha256'],name
selection=read(P/'proposed-members.json')['files'];assert len(selection)==2303 and sum(x['bytes'] for x in selection)==354258232
started=time.time_ns();initial=avail();assert initial>=96*G and initial-1*G>=96*G
assert not list(OUT.glob('evidence.tar.gz.*')) and not (OUT/'inventory.json').exists()
parts=[];whole=hashlib.sha256();stored=0;current=None;current_size=0;current_hash=None;samples=[]
class Parts:
 def write(self,data):
  global current,current_size,current_hash,stored
  total=len(data);whole.update(data)
  while data:
   if current is None:
    assert len(parts)<256;name=f'evidence.tar.gz.{len(parts)+1:03d}';current=(OUT/name).open('xb');current_size=0;current_hash=hashlib.sha256()
   count=min(len(data),2*1024**2-current_size);piece=data[:count];data=data[count:];current.write(piece);current_hash.update(piece);current_size+=count;stored+=count;assert stored<=512*1024**2
   if current_size==2*1024**2:self.finish()
  return total
 def flush(self):
  if current:current.flush()
 def finish(self):
  global current
  if current:
   current.flush();os.fsync(current.fileno());parts.append(dict(name=Path(current.name).name,bytes=current_size,sha256=current_hash.hexdigest()));current.close();current=None
stream=Parts();files={};secret=re.compile(rb'(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{50,}|AKIA[0-9A-Z]{16}|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----)')
result=dict(complete=False,started_ns=started,initial_available_bytes=initial,selection_sha256=sha(P/'proposed-members.json'),maximum_added_bytes=G,original_benchmark_guards_unchanged=True)
try:
 with gzip.GzipFile(filename='',fileobj=stream,mode='wb',compresslevel=6,mtime=0) as gz,tarfile.open(fileobj=gz,mode='w|') as tar:
  for n,row in enumerate(selection):
   p=Path(row['source']);member=row['member'];assert safe(member) and member not in files and row['bytes']<=64*1024**2
   assert not any(x in p.parts for x in ('retention-objects','target','.git','primary-data','replica-a-data','replica-b-data','__pycache__'))
   assert not p.name.endswith(('.zst','.tar.gz','.kubeconfig')) and p.suffix not in ('.pem','.key')
   fd=os.open(p,os.O_RDONLY|os.O_NOFOLLOW)
   with os.fdopen(fd,'rb') as f:
    s=os.fstat(f.fileno());assert stat.S_ISREG(s.st_mode) and (s.st_dev,s.st_ino,s.st_size,s.st_mtime_ns,stat.S_IMODE(s.st_mode))==(row['device'],row['inode'],row['bytes'],row['mtime_ns'],row['mode']),str(p)
    data=f.read(row['bytes']+1);assert len(data)==row['bytes'];assert not data.startswith(b'\x7fELF') and not secret.search(data),str(p)
    h=hashlib.sha256(data).hexdigest();assert 'expected_sha256' not in row or row['expected_sha256']==h,str(p)
    after=os.fstat(f.fileno());assert (after.st_size,after.st_mtime_ns)==(s.st_size,s.st_mtime_ns)
   entry=tarfile.TarInfo(member);entry.size=len(data);entry.mode=0o644;entry.mtime=0;tar.addfile(entry,io.BytesIO(data));files[member]=dict(bytes=len(data),sha256=h,source=str(p))
   if n%25==0:
    free=avail();samples.append(dict(observed_ns=time.time_ns(),available_bytes=free,completed_members=n+1));assert free>=max(96*G,initial-G);assert time.time_ns()-started<1200*10**9
 stream.finish()
 # Publication helpers are bounded and do not open omitted payloads.
 shutil.copy2('/home/dongxu/kv9/docs/coalesced-owner-performance-v1/verify.py',OUT/'verify.py')
 shutil.copy2(P/'omitted-payload-references.json',OUT/'omitted-payload-references.json')
 top={name:dict(bytes=(OUT/name).stat().st_size,sha256=sha(OUT/name)) for name in ('summary.json','summarize.py','verify.py','omitted-payload-references.json')}
 inventory=dict(version=1,scope='Reporting byte identity only; original local WAL objects and executables are excluded and remain catalog-bound',selection_inventory_sha256=sha(P/'inventory.json'),top_level=top,parts=parts,compressed_bytes=stored,compressed_sha256=whole.hexdigest(),decoded_member_count=len(files),decoded_member_bytes=sum(x['bytes'] for x in files.values()),files=files,limits=dict(member_bytes=64*1024**2,compressed_bytes=512*1024**2,parts=256,added_bytes=G),omitted_payloads='omitted-payload-references.json')
 (OUT/'inventory.json').write_text(json.dumps(inventory,indent=2)+'\n')
 result.update(complete=True,members=len(files),decoded_bytes=inventory['decoded_member_bytes'],compressed_bytes=stored,compressed_sha256=whole.hexdigest(),parts=len(parts),inventory_sha256=sha(OUT/'inventory.json'),credentials_pattern_scan_passed=True)
except BaseException as e:result['failure']=repr(e);raise
finally:
 result['ended_ns']=time.time_ns();result['final_available_bytes']=avail();(ROOT/'result.json').write_text(json.dumps(result,indent=2)+'\n');(ROOT/'resource-samples.json').write_text(json.dumps(samples,indent=2)+'\n')
print(json.dumps(result))
