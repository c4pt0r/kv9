#!/usr/bin/env python3
"""Private in-memory known-secret comparison; never publishes secrets or their hashes."""
import gzip,hashlib,json,tarfile,time,traceback
from pathlib import Path
P=Path(__file__).resolve().parent
credential_sources=[Path('/mnt/data/kv9-work/ledger-chaos-20260915-first/credentials.json'),Path('/tmp/kv9-c04-minio-tmpfs-fixture-20260915-first/attempt-second/credentials.env')]
secrets=set()
for src in credential_sources:
 if src.suffix=='.json':vals=json.loads(src.read_text()).values()
 else:
  vals=[]
  for line in src.read_text().splitlines():
   if line and not line.startswith('#') and '=' in line and line.split('=',1)[0] in ('MINIO_ROOT_USER','MINIO_ROOT_PASSWORD'):
    vals.append(line.split('=',1)[1].strip().strip("'\""))
 for val in vals:
  assert isinstance(val,str) and len(val)>=12
  secrets.add(val.encode())
findings=[];checked=0;total=0
selection=json.loads((P/'selection.json').read_text())
for row in selection['files']:
 data=Path(row['path']).read_bytes();checked+=1;total+=len(data)
 if any(secret in data for secret in secrets):findings.append({'packet':'runtime','member':row['member']})
dev=Path('/home/dongxu/kv9/docs/retention-ledger-v1/development.tar.gz')
digest=hashlib.sha256(dev.read_bytes()).hexdigest();assert digest=='0d369ff3ce97abec1e14f8154fcfdf2ddcb15559e52c2f9faf24747e8cc029ac'
dev_count=0;dev_bytes=0
with gzip.open(dev,'rb') as gz:
 with tarfile.open(fileobj=gz,mode='r|') as tar:
  for m in tar:
   assert m.isfile()
   data=tar.extractfile(m).read();dev_count+=1;dev_bytes+=len(data)
   if any(secret in data for secret in secrets):findings.append({'packet':'development','member':m.name})
 assert not gz.read().strip(b'\0')
result={'complete':not findings,'known_secret_matches':findings,'known_secret_count':len(secrets),'credential_sources':[str(p)for p in credential_sources],'secret_values_and_hashes_retained':False,'runtime_selection_sha256':hashlib.sha256((P/'selection.json').read_bytes()).hexdigest(),'runtime_members_scanned':checked,'runtime_original_bytes_scanned':total,'runtime_archive_full_original_byte_equality_authority':'readback-result.json','development_archive':{'path':str(dev),'sha256':digest,'members_scanned':dev_count,'bytes_scanned':dev_bytes},'scope':'Known actual current Chaos and reused host-MinIO fixture credentials only. Not a claim that arbitrary unknown secrets can be detected.'}
with (P/'known-secret-scan-result.json').open('x') as f:json.dump(result,f,indent=2,sort_keys=True);f.write('\n')
print(json.dumps(result));assert not findings,'known fixture secret match; paths retained without values'
