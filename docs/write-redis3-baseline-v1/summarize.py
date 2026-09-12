import argparse,hashlib,json,math,os
from pathlib import Path
ap=argparse.ArgumentParser();ap.add_argument('--output',type=Path,required=True);ap.add_argument('--root',type=Path,default=Path('/'));a=ap.parse_args()
AUDIT='/tmp/kv9-write-redis3-audit-repair-first/results-first/audit.json'
def path(p):return a.root/str(p).lstrip('/')
def read(p):return json.loads(path(p).read_text())
def sha(p):return hashlib.sha256(path(p).read_bytes()).hexdigest()
aud=read(AUDIT);inventory=read(str(Path(AUDIT).with_name('input-inventory.json')))
assert all(aud[k] is True for k in ('complete','matched_diagnostic_accepted','all_success_single_attempt','healthy_comparison_eligible'))
assert aud['parent_confirmed_session']==42336 and aud['parent_confirmed_exit_code']==0 and len(aud['cases'])==24
used={}
def bound(p):
 p=str(p);b=path(p).read_bytes();v=dict(bytes=len(b),sha256=hashlib.sha256(b).hexdigest());assert inventory[p]==v,p;used[p]=v;return json.loads(b)
def histogram(ps):
 buckets=[0]*3776
 for p in ps:
  assert len(p['whole_call']['raw']['buckets'])<=3776
  for i,n in enumerate(p['whole_call']['raw']['buckets']):buckets[i]+=n
 count=sum(p['calls'] for p in ps);total=sum(p['whole_call']['raw']['sum_ns'] for p in ps)
 assert count==sum(buckets)==sum(p['whole_call']['raw']['count'] for p in ps)>0
 out=dict(count=count,sum_ns=total,mean_ns=total/count)
 for percent in (50,95,99):
  rank=(count*percent+99)//100;seen=0
  for i,n in enumerate(buckets):
   seen+=n
   if seen>=rank:
    shift=(i-64)//64 if i>=64 else 0;lower=((64+(i-64)%64)<<shift) if i>=64 else i
    out['p'+str(percent)]=dict(lower_ns=lower,upper_ns=lower+(1<<shift)-1);break
 return out
rows=[];groups={};ticks=os.sysconf('SC_CLK_TCK');assert ticks==100
for c in aud['cases']:
 d=Path(c['directory']);r=bound(d/'run/report.json');s=bound(d/'resource-samples.json');coverage=bound(d/'resource-coverage.json')
 assert sha(d/'run/report.json')==c['report_sha256']
 ps=r['metrics']['measurement']['statistics'][1]['populations'];assert histogram(ps)==c['whole_call_latency']
 assert sum(p['calls'] for p in ps[1:])==0
 start=r['measurement_start_unix_ns'];end=start+r['cohort_elapsed_ns'];inside=[x for x in s if start<=x['unix_ns']<=end]
 assert len(inside)==coverage['samples']
 cpu={}
 for role in inside[0]['processes']:
  first=inside[0]['processes'][role];last=inside[-1]['processes'][role]
  assert first['pid']==last['pid'] and first['start_ticks']==last['start_ticks']
  seconds=(last['observed_monotonic_ns']-first['observed_monotonic_ns'])/1e9
  delta=last['user_ticks']+last['system_ticks']-first['user_ticks']-first['system_ticks'];assert delta>=0 and seconds>0
  cores=delta/ticks/seconds
  assert math.isclose(cores,coverage['cpu'][role]['cpu_cores'],rel_tol=1e-12)
  assert seconds==coverage['cpu'][role]['sample_seconds']
  cpu[role]=dict(cpu_seconds=delta/ticks,sample_seconds=seconds,cpu_cores=cores)
 target='kv9' if c['arm'].startswith('kv9-') else 'redis-wait1' if c['arm'].startswith('redis-wait1-') else 'redis-wait2'
 workload='point' if c['batch_size']==1 else 'batch64'
 row=dict(ordinal=c['ordinal'],repeat=c['repeat'],target=target,workload=workload,concurrency=c['concurrency'],successful_calls=c['calls'],successful_items=c['input_items'],elapsed_ns=c['cohort_elapsed_ns'],calls_per_second=c['successful_calls_per_second'],items_per_second=c['successful_items_per_second'],whole_call_latency=c['whole_call_latency'],cpu=cpu,outcomes=c['outcomes'],dropped_slots=c['dropped_slots'],data_attempts=c['attempts'],source_report=str(d/'run/report.json'),source_report_sha256=c['report_sha256'])
 rows.append(row);groups.setdefault((workload,c['concurrency'],target),[]).append((row,ps[0]))
pooled=[]
for (w,c,t),rr in groups.items():
 assert len(rr)==2 and {x[0]['repeat'] for x in rr}=={0,1}
 entries=[x[0] for x in rr];seconds=sum(x['elapsed_ns'] for x in entries)/1e9;calls=sum(x['successful_calls'] for x in entries);items=sum(x['successful_items'] for x in entries)
 cpu={role:sum(x['cpu'][role]['cpu_seconds'] for x in entries)/sum(x['cpu'][role]['sample_seconds'] for x in entries) for role in entries[0]['cpu']}
 pooled.append(dict(workload=w,concurrency=c,target=t,successful_calls=calls,successful_items=items,elapsed_seconds=seconds,calls_per_second=calls/seconds,items_per_second=items/seconds,whole_call_latency=histogram([x[1] for x in rr]),cpu_cores_by_role=cpu,server_cpu_cores=sum(v for k,v in cpu.items() if k!='client'),client_cpu_cores=cpu['client'],calls_per_second_min=min(x['calls_per_second'] for x in entries),calls_per_second_max=max(x['calls_per_second'] for x in entries),ordinals=[x['ordinal'] for x in entries]))
order={'kv9':0,'redis-wait1':1,'redis-wait2':2};pooled.sort(key=lambda x:(x['workload']!='point',x['concurrency'],order[x['target']]))
result=dict(version=1,complete=True,scope=aud['scope'],audit_path=AUDIT,audit_sha256=sha(AUDIT),timed_cohorts=24,smoke_cohorts=12,measured_successful_calls=sum(x['successful_calls'] for x in rows),measured_successful_items=sum(x['successful_items'] for x in rows),measured_unknown_writes=0,measured_errors=0,measured_dropped_slots=0,all_measured_data_calls_single_attempt=True,rate_aggregation='Sum successful calls/items divided by sum cohort elapsed time; two opposite target orders',latency_aggregation='Merged successful whole-call raw histograms; percentiles are bucket bounds, not averaged quantiles',cpu_scope='Recomputed from first/last in-window proc process user+system ticks and each process monotonic sample duration; 100 ticks/s on the same captured boot. Three server rates summed; endpoint gaps <=150 ms, no substage attribution.',cpu_ticks_per_second=ticks,pooled=pooled,repetitions=rows,input_bindings=used)
a.output.mkdir(exist_ok=False);(a.output/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
lines=['| Workload | c | Target | Calls/s | Items/s | Mean us | p50 us | p95 us | p99 us | Server CPU | Client CPU |','| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |']
def q(x,k):return f"{x[k]['lower_ns']/1000:.3f}–{x[k]['upper_ns']/1000:.3f}"
for p in pooled:
 h=p['whole_call_latency'];lines.append(f"| {p['workload']} | {p['concurrency']} | {p['target']} | {p['calls_per_second']:,.3f} | {p['items_per_second']:,.3f} | {h['mean_ns']/1000:.3f} | {q(h,'p50')} | {q(h,'p95')} | {q(h,'p99')} | {p['server_cpu_cores']:.3f} | {p['client_cpu_cores']:.3f} |")
(a.output/'table.md').write_text('\n'.join(lines)+'\n');print('\n'.join(lines));print('SUCCESSFUL CALLS',result['measured_successful_calls'],'ITEMS',result['measured_successful_items'])
