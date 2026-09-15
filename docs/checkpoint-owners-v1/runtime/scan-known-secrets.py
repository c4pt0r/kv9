"""Only actual known fixture secrets, never their bytes or hashes in output."""
from pathlib import Path
import json,hashlib
P=Path(__file__).resolve().parent
sources=[Path('/mnt/data/kv9-work/checkpoint-owner-chaos-20260915-first/credentials.json'),Path('/tmp/kv9-c04-minio-tmpfs-fixture-20260915-first/attempt-second/credentials.env')]
secrets=set()
for p in sources:
 if p.suffix=='.json':values=json.loads(p.read_text()).values()
 else:values=[l.split('=',1)[1].strip().strip('\'"')for l in p.read_text().splitlines()if '='in l and l.split('=',1)[0]in('MINIO_ROOT_USER','MINIO_ROOT_PASSWORD')]
 for value in values:
  assert isinstance(value,str)and len(value)>=12;secrets.add(value.encode())
selection=json.loads((P/'selection.json').read_text());readback=json.loads((P/'readback-result.json').read_text());assert readback['complete']and readback['all_original_bytes_compared']
findings=[];count=0;total=0
for row in selection['files']:
 data=Path(row['path']).read_bytes();assert len(data)==row['bytes']and hashlib.sha256(data).hexdigest()==row['sha256'];count+=1;total+=len(data)
 if any(s in data for s in secrets):findings.append({'member':row['member']})
result={'complete':not findings,'known_secret_matches':findings,'known_secret_count':len(secrets),'credential_sources':[str(p)for p in sources],'secret_values_and_hashes_retained':False,'selection_sha256':hashlib.sha256((P/'selection.json').read_bytes()).hexdigest(),'selected_members_scanned':count,'original_bytes_scanned':total,'archive_byte_equality_authority':'readback-result.json','scope':'Only current actual Chaos and reused host-MinIO fixture secrets; no arbitrary unknown-secret claim.'}
with(P/'known-secret-scan-result.json').open('x')as f:json.dump(result,f,indent=2,sort_keys=True);f.write('\n')
print(json.dumps(result));assert not findings,'known fixture secret matched; only member paths retained'
