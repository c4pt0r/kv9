#!/usr/bin/env python3
"""Recompute sample statistics and oracle states from retained bytes independently."""
import hashlib,json,math,struct
from pathlib import Path
R=Path(__file__).resolve().parent;used={}
def data(path):
 b=path.read_bytes();used[str(path)]=dict(bytes=len(b),sha256=hashlib.sha256(b).hexdigest());return b
def read(path):return json.loads(data(path))
def percentile(s,p):return sorted(s)[math.ceil(len(s)*p/100)-1]
plan=read(R/'plan.json');build=read(R/'build-corrected/result.json');cache=read(R/'build-corrected/cache-safety.json')
assert build['complete'] and cache['complete'] and cache['invalidation_complete']
identity=json.dumps(dict(source=build['source_pins'],bench=build['bench_sources']),sort_keys=True,separators=(',',':')).encode();assert hashlib.sha256(identity).hexdigest()==cache['source_identity_sha256']
for path,digest in {**build['source_pins'],**build['bench_sources']}.items():assert hashlib.sha256(Path(path).read_bytes()).hexdigest()==digest
assert '2 passed; 0 failed' in data(R/'build-corrected/semantics-test.stdout').decode()
term=read(R/'runs-first/timing-terminal.json');assert term['complete'] and term['exit_code']==0
pin=build['artifacts']['timing'];assert hashlib.sha256(Path(pin['path']).read_bytes()).hexdigest()==pin['sha256']==term['binary_sha256'] and term['argv'][5]==pin['path']
x=read(R/'runs-first/timing.json');assert used[str(R/'runs-first/timing.json')]['sha256']==term['result_sha256'];assert x['complete']
assert x['input_plan_sha256']==used[str(R/'plan.json')]['sha256']
assert x['groups']==106 and x['commands']==1564 and x['mutations']==100096 and x['prefix_entries_replayed']==14221
assert x['all_selected_original_fences_fresh'] and x['all_original_group_payloads_byte_identical'] and x['raft']['final_commit']==33991 and x['raft']['replaced_suffix_entries']==0
original=read(Path('/mnt/data/kv9-work/raw-lowering-direct-20260916-first/runs-first/timing.json'));assert x['joins']==original['joins']
b=data(R/'groups.bin');assert hashlib.sha256(b).hexdigest()==x['group_file_sha256']==original['group_file_sha256'];assert b[:8]==b'KV9GRP01'
position=8;groups=[];previous_end=14221
while position<len(b):
 previous,last,term,length=struct.unpack_from('<QQQI',b,position);position+=28;end=position+length
 assert previous==previous_end and last>previous and term==1 and end<=len(b);previous_end=last
 selected=[j for j in x['joins'] if previous<j['index']<=last];assert len(selected)==last-previous<=128 and sum(j['command_bytes']for j in selected)<=1024**2
 count=struct.unpack_from('<I',b,position)[0];position+=4;mutations=[]
 for _ in range(count):
  tag,cf,keylen=struct.unpack_from('<BBI',b,position);position+=6;assert tag==0 and cf==0
  key=b[position:position+keylen];position+=keylen;vallen=struct.unpack_from('<I',b,position)[0];position+=4;value=b[position:position+vallen];position+=vallen
  assert key and key[:1]==b'r' and len(key)>=4 and value and position<=end;mutations.append((key,value))
 assert position==end;groups.append(mutations)
assert len(groups)==106 and previous_end==15785 and sum(map(len,groups))==100096

def digest(model):
 h=hashlib.sha256()
 for cf,entries in enumerate(model):
  h.update(bytes([cf]));h.update(struct.pack('<Q',len(entries)))
  for key,value in sorted(entries.items()):
   h.update(struct.pack('<Q',len(key)));h.update(key);h.update(struct.pack('<Q',len(value)));h.update(value)
 return h.hexdigest()
metadata={}
for name in plan['workloads']:
 initial=[{}, {}, {}];expected=[{}, {}, {}];ordinal=0;payloads=[]
 if name=='overwrite':
  for group in groups:
   for key,value in group:initial[0][key]=bytes([value[0]^255])+value[1:]
 expected=[v.copy()for v in initial]
 for group in groups:
  payload=bytearray(struct.pack('<I',len(group)))
  for key,value in group:
   ordinal+=1
   if name=='unique_insert':key=key+struct.pack('>Q',ordinal);assert key not in expected[0]
   expected[0][key]=value
   payload.extend(struct.pack('<BBI',0,0,len(key)));payload.extend(key);payload.extend(struct.pack('<I',len(value)));payload.extend(value)
  payloads.append(hashlib.sha256(payload).hexdigest())
 m=dict(name=name,groups=106,mutations=100096,initial_keys=sum(map(len,initial)),final_keys=sum(map(len,expected)),initial_state_sha256=digest(initial),final_state_sha256=digest(expected),group_payload_sha256=payloads)
 observed=next(m for m in x['workloads']if m['name']==name);assert observed==m,(name,'independent oracle/payload mismatch');metadata[name]=m
assert len(x['correctness'])==6 and len(x['rows'])==24 and x['warmup_rows']==12
reports=[];all_order_means_better=True;all_p99_nonregressing=True
for name in plan['workloads']:
 for pinned in plan['snapshots']:
  checks=[c for c in x['correctness']if c['workload']==name and c['pinned']==pinned];assert len(checks)==1
  c=checks[0];assert c['complete'] and c['checked_group_prefixes']==106 and c['checked_live_states']==212 and c['checked_old_snapshots']==(212 if pinned else 0)
  assert c['final_keys']==metadata[name]['final_keys'] and c['final_state_sha256']==metadata[name]['final_state_sha256']
  rows=[r for r in x['rows']if r['workload']==name and r['pinned']==pinned]
  assert [r['component']for r in rows]==plan['orders'] and [r['order']for r in rows]==list(range(4))
  for r in rows:
   assert r['passes']==12 and r['groups']==1272 and r['mutations']==1201152 and r['validated_final_states']==12 and r['final_state_sha256']==metadata[name]['final_state_sha256']
   t=r['timing'];s=t['samples_ns'];assert len(s)==1272 and all(type(v)is int and v>0 for v in s)
   assert t['sum_ns']==sum(s) and t['mean_group_ns']==sum(s)/len(s)
   for p in (50,95,99):assert t[f'p{p}_group_ns']==percentile(s,p)
  def pool(component):
   s=[v for r in rows if r['component']==component for v in r['timing']['samples_ns']]
   return dict(groups=len(s),mean_group_ns=sum(s)/len(s),p99_group_ns=percentile(s,99))
  old,new=pool('lower'),pool('flat_lower');pairs=[]
  for a,b in [(0,1),(3,2)]:
   left,right=rows[a]['timing'],rows[b]['timing'];change=(right['mean_group_ns']/left['mean_group_ns']-1)*100
   all_order_means_better &= change<0
   pairs.append(dict(baseline_order=a,direct_order=b,mean_change_percent=change,baseline_mean_ns=left['mean_group_ns'],direct_mean_ns=right['mean_group_ns'],baseline_p99_ns=left['p99_group_ns'],direct_p99_ns=right['p99_group_ns']))
  all_p99_nonregressing &= new['p99_group_ns']<=old['p99_group_ns']
  reports.append(dict(workload=name,pinned=pinned,baseline=old,direct=new,mean_change_percent=(new['mean_group_ns']/old['mean_group_ns']-1)*100,p99_change_percent=(new['p99_group_ns']/old['p99_group_ns']-1)*100,pairs=pairs))
material=all(r['mean_change_percent']<=-3 for r in reports if r['workload']=='overwrite')
result=dict(complete=True,production_changed=False,new_database_qps=False,cases=reports,independent_payload_and_final_state_models=True,source_files=build['source_files'],registry_packages=build['registry_packages'],timed_group_samples=sum(r['groups']for r in x['rows']),timed_mutations=sum(r['mutations']for r in x['rows']),validated_timed_final_states=288,checked_correctness_live_prefixes=1272,checked_old_snapshots=636,predeclared_gate=dict(material_overwrite_gain=material,all_order_means_better=all_order_means_better,all_pooled_p99_nonregressing=all_p99_nonregressing,advance=material and all_order_means_better and all_p99_nonregressing),scope='Offline lowering plus actual MemEngine::write_applied, including final batch destruction; no WAL/Raft/RPC/quorum/queue or full state-machine timing. Synthetic unique insert preserves original grouping and values, appends eight key bytes. Shared host, 12 passes per row; no database QPS or new core proof/Chaos claim.')
(R/'analysis.json').write_text(json.dumps(result,indent=2)+'\n');(R/'analysis-inputs.json').write_text(json.dumps(used,indent=2)+'\n');print(json.dumps(result,indent=2))
