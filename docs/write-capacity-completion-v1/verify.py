"""Independent bounded gzip EOF and exact tar-member SHA readback, without extraction."""
import gzip,hashlib,io,json,os,tarfile,time
from pathlib import Path
O=Path(__file__).parent;start=time.monotonic()
assert os.geteuid()==0 and set(os.sched_getaffinity(0))==set(range(6,16))|set(range(22,32))
def h(p):
 with p.open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
def guard():assert time.monotonic()-start<120 and os.statvfs(O).f_bavail*os.statvfs(O).f_frsize>=8*1024**3
result=json.loads((O/'package-result.json').read_text());manifest=json.loads((O/'archive-manifest.json').read_text());expected=manifest['members'];arc=O/'metadata.tar.gz'
assert result['complete'] and arc.stat().st_size==result['archive']['bytes']<=8*1024**2 and h(arc)==result['archive']['sha256'] and h(O/'archive-manifest.json')==result['manifest_sha256']
class Counted:
 def __init__(self,stream):self.stream=stream;self.count=0
 def read(self,n=-1):
  guard();data=self.stream.read(n);self.count+=len(data);assert self.count<=40*1024**2;return data
seen={}
with gzip.open(arc,'rb')as gz:
 counted=Counted(gz)
 with tarfile.open(fileobj=counted,mode='r|')as archive:
  for item in archive:
   guard();assert item.isfile() and item.name in expected and item.name not in seen and '..'not in Path(item.name).parts and not Path(item.name).is_absolute()
   row=expected[item.name];assert item.size==row['bytes']<=4*1024**2
   stream=archive.extractfile(item);digest=hashlib.sha256();length=0
   while b:=stream.read(65536):guard();length+=len(b);digest.update(b)
   assert length==item.size and digest.hexdigest()==row['sha256'];seen[item.name]=dict(bytes=length,sha256=digest.hexdigest())
 # tarfile streams may buffer trailing tar padding; reading the remaining gzip
 # stream through EOF additionally validates its trailer CRC and complete length.
 while b:=counted.read(65536):assert not any(b),'nonzero trailing tar data'
assert set(seen)==set(expected) and len(seen)==result['member_count'] and sum(x['bytes']for x in seen.values())==result['file_bytes']
record=dict(complete=True,archive_sha256=h(arc),manifest_sha256=h(O/'archive-manifest.json'),package_result_sha256=h(O/'package-result.json'),members=len(seen),file_bytes=sum(x['bytes']for x in seen.values()),decoded_tar_bytes=counted.count,full_gzip_eof=True,exact_members_and_sha256=True,extracted_files=0,original_payloads_read=False,scope='Portable metadata integrity only; original runtime/readback acceptance remains historical.',elapsed_seconds=time.monotonic()-start)
with (O/'readback.json').open('x')as f:json.dump(record,f,indent=2,sort_keys=True);f.write('\n')
print(json.dumps(record))
