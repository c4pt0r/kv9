"""Independent complete gzip EOF and exact metadata member byte readback, no extraction."""
from pathlib import Path
import gzip,hashlib,json,tarfile
P=Path(__file__).resolve().parent;MIB=1024**2
class Count:
 def __init__(self,f):self.f=f;self.n=0
 def read(self,n=-1):
  b=self.f.read(n);self.n+=len(b);assert self.n<=40*MIB;return b
m=json.loads((P/'packet-manifest.json').read_text());expected={r['member']:r for r in m['rows']};seen=set();total=0
assert len(expected)==m['members']
with gzip.open(P/'metadata.tar.gz','rb')as gz:
 stream=Count(gz)
 with tarfile.open(fileobj=stream,mode='r|')as tar:
  for entry in tar:
   assert entry.isfile()and entry.name in expected and entry.name not in seen and entry.size<=4*MIB
   r=expected[entry.name];assert entry.size==r['bytes'];f=tar.extractfile(entry);h=hashlib.sha256();n=0
   while b:=f.read(65536):h.update(b);n+=len(b)
   assert n==r['bytes']and h.hexdigest()==r['sha256'];total+=n;seen.add(entry.name)
 while stream.read(65536):pass
assert seen==set(expected)and total==m['file_bytes']
result=dict(complete=True,archive_sha256=hashlib.sha256((P/'metadata.tar.gz').read_bytes()).hexdigest(),manifest_sha256=hashlib.sha256((P/'packet-manifest.json').read_bytes()).hexdigest(),members=len(seen),file_bytes=total,decoded_gzip_bytes=stream.n,gzip_eof=True,all_members_exact=True,no_extraction=True)
with (P/'readback.json').open('x')as f:json.dump(result,f,indent=2,sort_keys=True);f.write('\n')
print(json.dumps(result,sort_keys=True))
