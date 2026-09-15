#!/usr/bin/env python3
"""Finite source selection for the completed ledger environment evidence only."""
import hashlib,json,os,stat,time
from pathlib import Path
OUT=Path(__file__).resolve().parent
CHAOS=Path('/mnt/data/kv9-work/ledger-chaos-20260915-first')
AUDIT=Path('/mnt/data/kv9-work/ledger-chaos-audit-20260915-first')
def pin(p):
 p=Path(p);s=p.lstat();assert stat.S_ISREG(s.st_mode) and not p.is_symlink()
 h=hashlib.sha256(p.read_bytes()).hexdigest()
 return {'path':str(p),'bytes':s.st_size,'sha256':h}
def save(name,x):
 with (OUT/name).open('x') as f:json.dump(x,f,indent=2,sort_keys=True);f.write('\n')
(OUT/'receipts').mkdir()
for name,receipt in [('raw','268089'),('ledger','6d7cff')]:
 save('receipts/'+name+'-audit-tool-terminal.json',{'complete':True,'execution_kind':'direct_tool_terminal','actual_terminal_receipt':receipt,'exit_code':0,'result':pin(AUDIT/(name+'-result.json')),'source':pin(AUDIT/('audit-'+name+'.py')),'recording_note':'Recorded after observed original tool completion; no audit was replayed to produce this receipt.'})
rows={};excluded=[]
def add(p,name,expected=None):
 p=Path(p);r=pin(p);assert r['bytes']<=128*1024**2
 if expected:
  assert r['bytes']==expected['bytes'] and r['sha256']==expected['sha256'],str(p)
 assert name not in rows and not name.startswith('/') and '..' not in Path(name).parts
 r['member']=name;rows[name]=r

def tree(root,prefix,skip=()):
 root=Path(root)
 for p in sorted(root.rglob('*')):
  if p.is_file():
   rel=str(p.relative_to(root))
   if rel in skip:
    excluded.append({'path':str(p),'reason':'Excluded payload or unrelated broad host inventory; metadata pins remain in original inventory.'});continue
   add(p,prefix+'/'+rel)

for root,prefix in [('/tmp/kv9-c04-ledger-storage-guard-20260915-first','guard'),('/tmp/kv9-c04-ledger-environment-capacity-20260915-first','environment/initial'),('/tmp/kv9-c04-ledger-environment-capacity-20260915-second','environment/reassessment')]:
 tree(root,prefix,('docker-readback.json',))
D=Path('/mnt/data/kv9-work/ledger-library-environment-diagnosis-20260915-first')
di=json.loads((D/'inventory.json').read_text())
for name,r in di['files'].items():
 if r.get('portable'):add(D/name,'discriminator/'+name,r)
 else:excluded.append({'path':str(D/name),'reason':r.get('reason'),'bytes':r['bytes'],'sha256':r['sha256']})
add(D/'inventory.json','discriminator/inventory.json')
for path,r in json.loads((D/'original-failure-input-pins.json').read_text()).items():
 p=Path(path);rel=p.relative_to('/mnt/data/kv9-work/ledger-development-acceptance-20260915-first/tmp')
 assert p.name in ('status','metrics.json');add(p,'discriminator/original-fixtures/'+str(rel),r)
for name in ('restoration.json','download.log','export.stderr','nodes.json','nodes.stderr'):
 add(Path('/mnt/data/kv9-work/chaos-tools-20260915-first')/name,'tools/'+name)
public=json.loads((CHAOS/'public-evidence-manifest.json').read_text());assert public['complete']
for r in public['files']:add(r['path'],'chaos/'+r['relative_path'],r)
for name in ('public-evidence-manifest.json','tool-terminal.json','selected-sst-000.bin','n1-stopped-data.tar','n2-stopped-data.tar','n3-stopped-data.tar'):
 add(CHAOS/name,'chaos/'+name)
explicit_payloads={'selected-sst-000.bin','n1-stopped-data.tar','n2-stopped-data.tar','n3-stopped-data.tar'}
for r in public['excluded']:
 rel=str(Path(r['path']).relative_to(CHAOS))
 if rel in explicit_payloads:continue
 reason='Secret credentials: neither read nor hashed for this packet' if rel=='credentials.json' else 'Exact duplicate stdout of separately included named payload' if rel.startswith('commands/') else 'Executable/image input omitted; runtime pins retained'
 excluded.append({'path':r['path'],'reason':reason})
tree('/mnt/data/kv9-work/ledger-chaos-launch-20260915-first','launch')
tree(AUDIT,'audits')
for p in sorted((OUT/'receipts').iterdir()):add(p,'reporting/receipts/'+p.name)
for name in ('images.json','runtime-ca-lineage.json'):
 add(Path('/tmp/kv9-c04-checkpoint-publication-chaos-20260915-fourth')/name,'retained-ca-authority/'+name)
excluded.extend([
 {'path':'/mnt/data/kv9-work/chaos-tools-20260915-first/kubeconfig','reason':'Private cluster credentials; never archived or hashed for publication'},
 {'path':'/mnt/data/kv9-work/chaos-tools-20260915-first/kind','reason':'Executable omitted; exact SHA and version in tools/restoration.json'},
 {'path':'/mnt/data/kv9-work/ledger-runtime-inputs-20260915-first/kv9','reason':'Executable omitted; exact SHA/source/binary binding in launch/preflight.json'},
 {'path':'Host CA certificate bundle and retained image layers','reason':'Material excluded; size/hash/source path and image lineage retained'},
 {'path':'Original large unit-fixture WAL/data files','reason':'Not selected; exact failing status/metrics and original logs retained. Only current Chaos stopped-node payloads included.'},
 {'path':'Prior ledger registered SST object aeb165c2ba15f38013c6563b58c4dd7170c2280c135a6435f4bd0222e5c4a909','reason':'Descriptor retained, but body was not separately captured before owned MinIO cleanup. Captured recovery SST is different; no body-preservation claim.'}
])
for name in ('prepare.py','pack.py','verify.py','README.md'):
 add(OUT/name,'reporting/'+name)
assert len(rows)<=4096 and sum(r['bytes']for r in rows.values())<=512*1024**2
save('selection.json',{'schema_version':1,'complete':True,'files':[rows[n]for n in sorted(rows)],'member_count':len(rows),'original_bytes':sum(r['bytes']for r in rows.values()),'limits':{'members':4096,'member_bytes':128*1024**2,'aggregate_original_bytes':512*1024**2,'part_bytes':2*1024**2},'exclusions':excluded,'scope':'Completed environment, guard, failed-library discriminator, actual ledger Chaos histories and complete retained stopped-node archives; no workload replay.'})
print(json.dumps({'complete':True,'selection':pin(OUT/'selection.json'),'members':len(rows),'original_bytes':sum(r['bytes']for r in rows.values())}))
