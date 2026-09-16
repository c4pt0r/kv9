#!/usr/bin/env python3
import hashlib,json,math
from pathlib import Path
R=Path(__file__).resolve().parent;D=R.with_name('raw-lowering-direct-20260916-first')
used={}
def read(p):
 b=p.read_bytes();used[str(p)]=dict(bytes=len(b),sha256=hashlib.sha256(b).hexdigest());return json.loads(b)
def percentile(samples,p):return sorted(samples)[math.ceil(len(samples)*p/100)-1]
def check(root,orders):
 plan=read(root/'plan.json');build=read(root/'build-first/result.json');assert build['complete']
 cache=read(root/'build-first/cache-safety.json');assert cache['complete'] and cache['invalidation_complete']
 expected_source=json.dumps(dict(source=build['source_pins'],bench=build['bench_sources']),sort_keys=True,separators=(',',':')).encode();assert hashlib.sha256(expected_source).hexdigest()==cache['source_identity_sha256']
 for path,digest in build['source_pins'].items():assert hashlib.sha256(Path(path).read_bytes()).hexdigest()==digest
 for path,digest in build['bench_sources'].items():assert hashlib.sha256(Path(path).read_bytes()).hexdigest()==digest
 cases={}
 for mode in ('timing','allocations'):
  term=read(root/'runs-first'/f'{mode}-terminal.json');assert term['complete'] and term['exit_code']==0
  pin=build['artifacts'][mode];assert hashlib.sha256(Path(pin['path']).read_bytes()).hexdigest()==pin['sha256'] and term['argv'][5]==pin['path']
  x=read(root/'runs-first'/f'{mode}.json');assert x['complete'] and x['counting_build']==(mode=='allocations')
  assert x['groups']==106 and x['commands']==1564 and x['mutations']==100096 and x['prefix_entries_replayed']==14221
  assert x['all_selected_fences_fresh'] and x['all_group_payloads_byte_identical'] and x['raft']['final_commit']==33991 and x['raft']['replaced_suffix_entries']==0
  assert [j['index']for j in x['joins']]==list(range(14222,15786)) and {j['term']for j in x['joins']}=={1}
  assert x['input_plan_sha256']==used[str(root/'plan.json')]['sha256'] and [v['component']for v in x['rows']]==orders
  for row in x['rows']:
   assert row['groups']==1272 and row['commands']==18768 and row['mutations']==1201152 and row['passes']==12
   if mode=='timing':
    t=row['timing'];s=t['samples_ns'];assert len(s)==row['groups'] and all(type(v)is int and v>0 for v in s)
    assert t['sum_ns']==sum(s) and t['mean_group_ns']==sum(s)/len(s)
    for p in (50,95,99):assert t[f'p{p}_group_ns']==percentile(s,p)
   else: assert 'timing'not in row
  cases[mode]=x
 assert cases['timing']['joins']==cases['allocations']['joins']
 return cases
base=check(R,['decode','lower','fence','fence','lower','decode']);direct=check(D,['lower','flat_lower','flat_lower','lower'])
assert base['timing']['joins']==direct['timing']['joins']
source_groups=read(R/'groups.json')['rows'];joins=base['timing']['joins']
for group in source_groups:
 selected=[j for j in joins if group['previous']<j['index']<=group['index']]
 assert len(selected)==group['index']-group['previous']<=128
 assert sum(x['command_bytes']for x in selected)<=1024**2
assert '1 passed; 0 failed' in (D/'build-first/semantics-test.stdout').read_text()
def pooled(rows,component):
 rows=[r for r in rows if r['component']==component];s=[n for r in rows for n in r['timing']['samples_ns']]
 return dict(groups=len(s),mean_group_ns=sum(s)/len(s),sum_ns=sum(s),p99_group_ns=percentile(s,99))
components={c:pooled(base['timing']['rows'],c)for c in ('decode','lower','fence')}
old=pooled(direct['timing']['rows'],'lower');new=pooled(direct['timing']['rows'],'flat_lower')
pairs=[]
for a,b in ((0,1),(3,2)):
 x,y=direct['timing']['rows'][a]['timing'],direct['timing']['rows'][b]['timing']
 pairs.append(dict(baseline_order=a,candidate_order=b,old_mean_ns=x['mean_group_ns'],new_mean_ns=y['mean_group_ns'],mean_change_percent=(y['mean_group_ns']/x['mean_group_ns']-1)*100,old_p99_ns=x['p99_group_ns'],new_p99_ns=y['p99_group_ns']))
counts={c:next(r['allocation_counts']for r in direct['allocations']['rows']if r['component']==c)for c in ('lower','flat_lower')}
for c in counts:assert all(r['allocation_counts']==counts[c]for r in direct['allocations']['rows']if r['component']==c)
result=dict(complete=True,production_runtime_changed=False,new_database_qps=False,selected_groups=106,selected_commands=1564,selected_mutations=100096,groups_byte_matched=True,components=components,direct_comparison=dict(old=old,new=new,mean_change_percent=(new['mean_group_ns']/old['mean_group_ns']-1)*100,absolute_mean_saved_ns=old['mean_group_ns']-new['mean_group_ns'],pairs=pairs,allocation_counts_per_12_pass_row=counts,realloc_change_percent=(counts['flat_lower']['realloc_calls']/counts['lower']['realloc_calls']-1)*100),scope='Independent recomputation of original component samples and matching counts/source/group joins. No independent re-execution of every Raft/engine parser or complete apply/endpoint measurement. Different component means must not be subtracted from earlier runtime group timings.')
(R/'analysis.json').write_text(json.dumps(result,indent=2)+'\n');(R/'analysis-inputs.json').write_text(json.dumps(used,indent=2)+'\n');print(json.dumps(result,indent=2))
