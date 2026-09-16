#!/usr/bin/env python3
import hashlib,json,math,struct,tomllib
from pathlib import Path
R=Path(__file__).resolve().parent;used={}
def data(p):
 b=p.read_bytes();used[str(p)]=dict(bytes=len(b),sha256=hashlib.sha256(b).hexdigest());return b
def read(p):return json.loads(data(p))
def pct(s,p):return sorted(s)[math.ceil(len(s)*p/100)-1]
plan=read(R/'timing-plan.json');build=read(R/'benchmark-build-corrected/result.json');cache=read(R/'benchmark-build-corrected/cache-safety.json');qualification=read(R/'qualification-corrected/result.json')
assert build['complete'] and cache['complete'] and cache['invalidation_complete'] and qualification['complete'] and qualification['tests_passed']==6
assert hashlib.sha256(json.dumps(build['sources'],sort_keys=True,separators=(',',':')).encode()).hexdigest()==cache['source_identity_sha256']
for path,digest in build['sources'].items():assert hashlib.sha256(Path(path).read_bytes()).hexdigest()==digest
for name in ('src/lib.rs','src/tests.rs'):
 p=str(Path(plan['source'])/name);assert qualification['sources'][p]==build['sources'][p]
assert '6 passed; 0 failed' in data(R/'qualification-corrected/tests.stdout').decode()
results={}
for mode in ('timing','allocations'):
 terminal=read(R/'runs-first'/f'{mode}-terminal.json');assert terminal['complete'] and terminal['exit_code']==0
 pin=build['artifacts'][mode];assert terminal['argv'][3]==pin['path'] and terminal['binary_sha256']==pin['sha256']==hashlib.sha256(Path(pin['path']).read_bytes()).hexdigest()
 x=read(R/'runs-first'/f'{mode}.json');assert x['complete'] and x['counting_build']==(mode=='allocations') and len(x['rows'])==56
 assert x['input_plan_sha256']==used[str(R/'timing-plan.json')]['sha256'];assert terminal['result_sha256']==used[str(R/'runs-first'/f'{mode}.json')]['sha256']
 assert x['correctness_live_prefixes']==1272 and x['correctness_old_views']==636
 for row in x['rows']:
  expected_passes=12 if mode=='timing' else 1;assert row['passes']==expected_passes
  m=row['metrics'];assert m['windows']==expected_passes*(106 if row['operation']=='write' else 512)
  if row['operation']=='write':assert row['mutations']==expected_passes*100096
  else:assert row['queries']==512 and row['all_query_outputs_checked']
  if mode=='timing':
   s=m['samples_ns'];assert len(s)==m['windows'] and all(type(v)is int and v>0 for v in s)
   assert m['sum_ns']==sum(s) and m['mean_ns']==sum(s)/len(s)
   for percentile in (50,95,99):assert m[f'p{percentile}_ns']==pct(s,percentile)
  else:assert 'samples_ns'not in m and 'mean_ns'not in m
 results[mode]=x
assert results['timing']['workloads']==results['allocations']['workloads']
b=data(R/'groups.bin');assert hashlib.sha256(b).hexdigest()==results['timing']['group_file_sha256']==results['allocations']['group_file_sha256']=='d978ba9e49131f6277f975ecd333c33854845cc03bd14ee57a22fff915a08350'
position=8;groups=[];previous_end=14221
assert b[:8]==b'KV9GRP01'
while position<len(b):
 previous,last,term,length=struct.unpack_from('<QQQI',b,position);position+=28;end=position+length;assert previous==previous_end and last>previous and term==1;previous_end=last
 count=struct.unpack_from('<I',b,position)[0];position+=4;group=[]
 for _ in range(count):
  tag,cf,keylen=struct.unpack_from('<BBI',b,position);position+=6;assert tag==0 and cf==0
  key=b[position:position+keylen];position+=keylen;vallen=struct.unpack_from('<I',b,position)[0];position+=4;value=b[position:position+vallen];position+=vallen;assert position<=end;group.append((key,value))
 assert position==end;groups.append(group)
assert len(groups)==106 and sum(map(len,groups))==100096 and previous_end==15785

def digest(rows):
 h=hashlib.sha256();h.update(struct.pack('<Q',len(rows)))
 for key,value in rows:h.update(struct.pack('<Q',len(key)));h.update(key);h.update(struct.pack('<Q',len(value)));h.update(value)
 return h.hexdigest()
models={}
for name in ('overwrite','initial_fill','unique_insert'):
 initial={};final={};ordinal=0
 if name=='overwrite':
  for group in groups:
   for key,value in group:initial[key]=bytes([value[0]^255])+value[1:]
 for group in groups:
  for key,value in group:
   ordinal+=1
   if name=='unique_insert':key+=struct.pack('>Q',ordinal);assert key not in final
   final[key]=value
 rows=sorted(final.items());m=dict(workload=name,groups=106,initial_keys=len(initial),final_keys=len(final),final_state_sha256=digest(rows));assert m in results['timing']['workloads'];models[name]=rows
for name in ('overwrite','unique_insert'):
 rows=models[name];keys={k for k,v in rows}
 for operation in ('get_hit','get_miss','predecessor','scan16'):
  probes=[]
  for i in range(512):
   key=rows[(i*len(rows)//512+71)%len(rows)][0]
   if operation=='get_miss' or (operation=='predecessor' and i%2==1):
    key+=bytes([255])
    while key in keys:key+=bytes([255])
   probes.append(key)
  state=71;mask=(1<<64)-1
  for i in range(len(probes)-1,0,-1):
   state^=(state<<13)&mask;state^=state>>7;state^=(state<<17)&mask;other=state%(i+1);probes[i],probes[other]=probes[other],probes[i]
  expected=digest([(key,b'')for key in probes])
  for mode in results:
   selected=[r for r in results[mode]['rows']if r['workload']==name and r['operation']==operation];assert len(selected)==4 and all(r['query_sha256']==expected for r in selected)
reports=[]
case_keys=[('write',name,pinned)for name in ('overwrite','initial_fill','unique_insert')for pinned in (False,True)]+[(op,name,False)for name in ('overwrite','unique_insert')for op in ('get_hit','get_miss','predecessor','scan16')]
for operation,name,pinned in case_keys:
 select=lambda mode:[r for r in results[mode]['rows']if (r['operation'],r['workload'],r['pinned'])==(operation,name,pinned)]
 rows=select('timing');counts=select('allocations');assert len(rows)==len(counts)==4
 assert [r['backend']for r in rows]==[r['backend']for r in counts]==plan['orders'];assert [r['order']for r in rows]==list(range(4))
 def pool(backend):
  s=[n for r in rows if r['backend']==backend for n in r['metrics']['samples_ns']];return dict(samples=len(s),mean_ns=sum(s)/len(s),p99_ns=pct(s,99))
 baseline,packed=pool('rpds'),pool('packed');pairs=[]
 for a,b in ((0,1),(3,2)):
  old,new=rows[a]['metrics'],rows[b]['metrics'];pairs.append(dict(baseline_order=a,packed_order=b,mean_change_percent=(new['mean_ns']/old['mean_ns']-1)*100,baseline_mean_ns=old['mean_ns'],packed_mean_ns=new['mean_ns'],baseline_p99_ns=old['p99_ns'],packed_p99_ns=new['p99_ns']))
 allocation={}
 for backend in ('rpds','packed'):
  matches=[r for r in counts if r['backend']==backend];assert matches[0]['metrics']==matches[1]['metrics'];allocation[backend]=matches[0]['metrics']
 memory={}
 if operation=='write':
  for backend in ('rpds','packed'):
   matches=[r for r in counts if r['backend']==backend];assert matches[0]['requested_live_bytes']==matches[1]['requested_live_bytes'] and matches[0]['requested_live_bytes']['released_after_drop'];memory[backend]=matches[0]['requested_live_bytes']
  for r in rows+counts:assert r['final_state_sha256']==digest(models[name]) and r['final_keys']==len(models[name])
 reports.append(dict(operation=operation,workload=name,pinned=pinned,baseline=baseline,packed=packed,mean_change_percent=(packed['mean_ns']/baseline['mean_ns']-1)*100,p99_change_percent=(packed['p99_ns']/baseline['p99_ns']-1)*100,pairs=pairs,allocation_counts_per_one_pass=allocation,requested_live_bytes=memory))
write=[r for r in reports if r['operation']=='write'];reads=[r for r in reports if r['operation']!='write']
material=all(r['mean_change_percent']<=-10 for r in write if r['workload'] in ('overwrite','unique_insert'))
orders=all(p['mean_change_percent']<0 for r in write for p in r['pairs'])
read_gate=all(r['mean_change_percent']<=2 and r['p99_change_percent']<=2 for r in reads)
result=dict(complete=True,production_changed=False,new_database_qps=False,independent_final_states_and_query_identity=True,cases=reports,tests_passed=6,predeclared_gate=dict(material_overwrite_and_unique_both_snapshot_modes=material,all_write_order_means_better=orders,reads_within_two_percent=read_gate,advance=material and orders and read_gate),scope=results['timing']['scope'])
(R/'analysis.json').write_text(json.dumps(result,indent=2)+'\n');(R/'analysis-inputs.json').write_text(json.dumps(used,indent=2)+'\n')
print(json.dumps(dict(complete=True,predeclared_gate=result['predeclared_gate'],cases=[{k:r[k]for k in ('operation','workload','pinned','mean_change_percent','p99_change_percent','baseline','packed','requested_live_bytes')}for r in reports]),indent=2))
