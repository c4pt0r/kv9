"""Additional exact root development archive scan; runtime packet is unchanged."""
from pathlib import Path
import json,hashlib,gzip,tarfile
P=Path(__file__).resolve().parent;D=Path('/home/dongxu/kv9/docs/checkpoint-owners-v1/development.tar.gz')
expected='bd3464ce63ddca6c826c06a9f8a6b6aacd6dcd9cae58a3532268e5e8d302a123'
assert hashlib.sha256(D.read_bytes()).hexdigest()==expected
sources=[Path('/mnt/data/kv9-work/checkpoint-owner-chaos-20260915-first/credentials.json'),Path('/tmp/kv9-c04-minio-tmpfs-fixture-20260915-first/attempt-second/credentials.env')]
secrets=set()
for p in sources:
 if p.suffix=='.json':values=json.loads(p.read_text()).values()
 else:values=[l.split('=',1)[1].strip().strip('\'"')for l in p.read_text().splitlines()if '='in l and l.split('=',1)[0]in('MINIO_ROOT_USER','MINIO_ROOT_PASSWORD')]
 for v in values:assert isinstance(v,str)and len(v)>=12;secrets.add(v.encode())
findings=[];seen=set();total=0
with gzip.open(D,'rb')as gz:
 with tarfile.open(fileobj=gz,mode='r|')as tar:
  for m in tar:
   assert m.isfile()and m.name not in seen and m.size<=128*1024**2;seen.add(m.name);b=tar.extractfile(m).read();assert len(b)==m.size;total+=len(b)
   if any(s in b for s in secrets):findings.append({'member':m.name})
 assert not gz.read().strip(b'\0')
assert len(seen)==1534 and total==16971713
runtime=json.loads((P/'known-secret-scan-result.json').read_text());assert runtime['complete']and runtime['known_secret_count']==len(secrets)
result={'complete':not findings,'archive':{'path':str(D),'bytes':D.stat().st_size,'sha256':expected},'development_members':len(seen),'development_decoded_bytes':total,'gzip_full_eof':True,'known_secret_count':len(secrets),'known_secret_matches':findings,'runtime_scan_result_sha256':hashlib.sha256((P/'known-secret-scan-result.json').read_bytes()).hexdigest(),'secret_values_and_hashes_retained':False,'scope':'Known current Chaos and reused host-MinIO credentials checked against the exact complete root development archive and separately verified runtime packet.'}
with(P/'development-secret-scan-result.json').open('x')as f:json.dump(result,f,indent=2,sort_keys=True);f.write('\n')
print(json.dumps(result));assert not findings,'known fixture secret matched; member names only retained'
