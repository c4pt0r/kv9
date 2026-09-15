"""Finite current default-owner metadata and full protocol-payload selection."""
from pathlib import Path
import hashlib,json,stat
P=Path(__file__).resolve().parent
C=Path('/mnt/data/kv9-work/checkpoint-owner-chaos-20260915-first')
PREP=Path('/mnt/data/kv9-work/checkpoint-owner-chaos-preparation-20260915-first')
AUDIT=Path('/mnt/data/kv9-work/checkpoint-owner-chaos-audit-20260915-first')
rows={};excluded=[]
def pin(p):
 p=Path(p);s=p.lstat();assert stat.S_ISREG(s.st_mode)and not p.is_symlink()
 with p.open('rb')as f:h=hashlib.file_digest(f,'sha256').hexdigest()
 return {'path':str(p),'bytes':s.st_size,'sha256':h}
def add(path,member,expected=None):
 p=Path(path);r=pin(p);assert r['bytes']<=128*1024**2
 if expected:assert all(r[k]==expected[k]for k in('bytes','sha256'))
 assert member not in rows and not member.startswith('/')and '..'not in Path(member).parts
 rows[member]={'member':member,**r}
def tree(root,prefix):
 for p in sorted(root.rglob('*')):
  if p.is_file():add(p,prefix+'/'+str(p.relative_to(root)))
assert json.loads((C/'result.json').read_text())['complete']
assert json.loads((PREP/'launch-first/child-terminal.json').read_text())['complete']
assert json.loads((AUDIT/'owner-result.json').read_text())['complete']
assert json.loads((AUDIT/'raw-result.json').read_text())['status']=='PASS'
public=json.loads((C/'public-evidence-manifest.json').read_text());assert public['complete']
for r in public['files']:add(r['path'],'chaos/'+r['relative_path'],r)
add(C/'public-evidence-manifest.json','chaos/public-evidence-manifest.json')
payload=json.loads((C/'owner-protocol-payloads.json').read_text());assert payload['runtime_complete']
explicit=set()
for row in payload['files']:
 name=Path(row['path']).name;assert name.startswith('selected-sst-')or name in('n1-stopped-data.tar','n2-stopped-data.tar','n3-stopped-data.tar')
 explicit.add(name);add(row['path'],'chaos/'+name,row)
assert {n for n in explicit if n.endswith('.tar')}=={'n1-stopped-data.tar','n2-stopped-data.tar','n3-stopped-data.tar'}
for row in public['excluded']:
 rel=str(Path(row['path']).relative_to(C))
 if rel in explicit:continue
 reason='Secret credential values: excluded, not hashed for publication'if rel=='credentials.json'else'Exact duplicate command stdout of a separately included named protocol payload'if rel.startswith('commands/')else'Executable/image/CA input bytes excluded; nonsecret identity remains pinned'
 excluded.append({'path':row['path'],'reason':reason})
tree(PREP,'preparation');tree(AUDIT,'audits')
for mode in('default','testing'):
 p=Path('/mnt/data/kv9-work/checkpoint-owner-'+mode+'-inputs-20260915-first/inputs.json');add(p,'runtime-inputs/'+mode+'.json')
for n in('images.json','runtime-ca-lineage.json'):
 add(Path('/tmp/kv9-c04-checkpoint-publication-chaos-20260915-fourth')/n,'retained-ca-authority/'+n)
for n in('comparison.json','tool-terminal.json','inventory.json','README.md'):
 add(Path('/mnt/data/kv9-work/performance-input-recovery-20260915-first/rebuild-first')/n,'fixed-client-mismatch/'+n)
for n in('README.md','prepare.py','pack.py','verify.py','scan-known-secrets.py','mechanism-lineage.json'):
 add(P/n,'reporting/'+n)
excluded.extend([
 {'path':'/mnt/data/kv9-work/chaos-tools-20260915-first/kubeconfig','reason':'Private cluster credentials, never included'},
 {'path':'Default and testing kv9 executable files, Kind tool, CA bundle and image layers','reason':'Exact source/build/tool/image identities retained; executable and credential/CA material omitted'},
 {'path':'Historical performance payloads and old unrelated Chaos campaigns','reason':'Not part of this finite automatic-owner acceptance packet'}])
assert len(rows)<=4096 and sum(r['bytes']for r in rows.values())<=512*1024**2
selection={'complete':True,'schema_version':1,'files':[rows[k]for k in sorted(rows)],'member_count':len(rows),'original_bytes':sum(r['bytes']for r in rows.values()),'limits':{'members':4096,'member_bytes':128*1024**2,'aggregate_original_bytes':512*1024**2,'part_bytes':2*1024**2},'exclusions':excluded}
with(P/'selection.json').open('x')as f:json.dump(selection,f,indent=2,sort_keys=True);f.write('\n')
print(json.dumps({'complete':True,'selection':pin(P/'selection.json'),'members':len(rows),'original_bytes':selection['original_bytes']}))
