#!/usr/bin/env python3
"""Audited native16 A/B reporting only. No workload, codec or campaign audit."""
import argparse,hashlib,json,math,os,sys
from pathlib import Path
HERE=Path(__file__).resolve().parent
PREP=Path('/tmp/kv9-write-crc-slicing8-ab-preparation-first')
RUN=Path('/tmp/kv9-write-crc-slicing8-ab-timing-first/cohorts')
AUDIT=PREP/'results-first/audit.json'
ROLES=('old','new');CPUS=('client','voter-1','voter-2','voter-3')
CAP=64*1024**2
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

def latency(ps):
 if sum(p['calls']for p in ps):return histogram(ps)
 assert all(p['whole_call']['raw']['count']==p['whole_call']['raw']['sum_ns']==0 and not any(p['whole_call']['raw']['buckets'])for p in ps)
 return dict(count=0,sum_ns=0,mean_ns=None,p50=None,p95=None,p99=None)
def summed(dicts):
 return {k:sum(d.get(k,0)for d in dicts)for k in sorted({k for d in dicts for k in d})}
def percent(old,new):return (new/old-1)*100 if old is not None and new is not None and old>0 else None
def bucket_direction(old,new):
 if old is None or new is None:return 'unavailable'
 if old==new:return 'same_bucket'
 if new['upper_ns']<old['lower_ns']:return 'new_lower_interval'
 if new['lower_ns']>old['upper_ns']:return 'new_higher_interval'
 return 'overlapping_intervals'
def cpu_from_samples(samples,coverage,report):
 begin=report['measurement_start_unix_ns'];end=begin+report['cohort_elapsed_ns'];inside=[s for s in samples if begin<=s['unix_ns']<=end]
 assert len(inside)==coverage['samples']>=10 and set(inside[0]['processes'])==set(CPUS)
 result={}
 for role in CPUS:
  first=inside[0]['processes'][role];last=inside[-1]['processes'][role]
  assert first['pid']==last['pid']and first['start_ticks']==last['start_ticks']
  duration_ns=last['observed_monotonic_ns']-first['observed_monotonic_ns'];delta=last['user_ticks']+last['system_ticks']-first['user_ticks']-first['system_ticks']
  assert duration_ns>0 and delta>=0
  seconds=duration_ns/1e9;cores=delta/100/seconds
  assert math.isclose(cores,coverage['cpu'][role]['cpu_cores'],rel_tol=1e-12)and seconds==coverage['cpu'][role]['sample_seconds']
  result[role]=dict(pid=first['pid'],start_ticks=first['start_ticks'],user_system_ticks=delta,sample_duration_ns=duration_ns,cpu_seconds=delta/100,sample_seconds=seconds,cpu_cores=cores)
 return result

def aggregate(rows,populations):
 assert rows and len(rows)==len(populations)
 elapsed=sum(x['elapsed_ns']for x in rows);success=sum(x['successful_calls']for x in rows);items=sum(x['successful_items']for x in rows)
 outcomes=summed([x['outcomes']for x in rows]);completed=sum(x['completed_calls']for x in rows)
 result=dict(ordinals=[x['ordinal']for x in rows],repeats=[x['repeat']for x in rows],elapsed_ns=elapsed,successful_calls=success,successful_items=items,completed_calls=completed,completed_items=sum(x['completed_items']for x in rows),issued_calls=sum(x['issued_calls']for x in rows),data_attempts=sum(x['data_attempts']for x in rows),dropped_slots=sum(x['dropped_slots']for x in rows),completed_before_cutoff=sum(x['completed_before_cutoff']for x in rows),outcomes=outcomes,outcome_items=summed([x['outcome_items']for x in rows]),reasons=summed([x['reasons']for x in rows]),attempt_reasons=summed([x['attempt_reasons']for x in rows]),terminal_rpc_codes=[sum(x['terminal_rpc_codes'][i]for x in rows)for i in range(17)])
 result.update(calls_per_second=success*1e9/elapsed,items_per_second=items*1e9/elapsed,completed_calls_per_second=completed*1e9/elapsed,completed_items_per_second=result['completed_items']*1e9/elapsed,completed_after_cutoff=completed-result['completed_before_cutoff'],whole_call_latency=latency([p for ps in populations for p in ps]),successful_whole_call_latency=latency([ps[0]for ps in populations]),outcome_latency={name:latency([ps[i]for ps in populations])for i,name in enumerate(rows[0]['outcome_order'])})
 cpu={}
 for role in CPUS:
  ticks=sum(x['cpu'][role]['user_system_ticks']for x in rows);ns=sum(x['cpu'][role]['sample_duration_ns']for x in rows)
  cpu[role]=dict(user_system_ticks=ticks,sample_duration_ns=ns,cpu_seconds=ticks/100,sample_seconds=ns/1e9,cpu_cores=(ticks/100)/(ns/1e9))
 result.update(cpu=cpu,server_cpu_cores=sum(cpu[k]['cpu_cores']for k in CPUS if k!='client'),client_cpu_cores=cpu['client']['cpu_cores'],healthy_comparison_eligible=all(x['healthy_comparison_eligible']for x in rows))
 return result

def comparison(old,new,scope,workload,c,repeat=None):
 h0=old['whole_call_latency'];h1=new['whole_call_latency']
 return dict(scope=scope,workload=workload,concurrency=c,repeat=repeat,old_ordinals=old.get('ordinals',[old.get('ordinal')]),new_ordinals=new.get('ordinals',[new.get('ordinal')]),successful_calls_per_second_change_percent=percent(old['calls_per_second'],new['calls_per_second']),successful_items_per_second_change_percent=percent(old['items_per_second'],new['items_per_second']),completed_calls_per_second_change_percent=percent(old['completed_calls_per_second'],new['completed_calls_per_second']),mean_latency_change_percent=percent(h0['mean_ns'],h1['mean_ns']),server_cpu_change_percent=percent(old['server_cpu_cores'],new['server_cpu_cores']),client_cpu_change_percent=percent(old['client_cpu_cores'],new['client_cpu_cores']),quantile_directions={q:bucket_direction(h0[q],h1[q])for q in ('p50','p95','p99')},old_whole_call_latency=h0,new_whole_call_latency=h1,healthy_comparison_eligible=old['healthy_comparison_eligible']and new['healthy_comparison_eligible'],direction_scope='Positive rate change means higher throughput; negative mean change means lower latency. CPU changes are observed sampled rates. Quantiles remain bucket intervals; no percentile averaging or significance claim.')

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--output',type=Path,required=True);ap.add_argument('--audit-sha256',required=True);ap.add_argument('--input-inventory-sha256',required=True);ap.add_argument('--timing-session',type=int,required=True);ap.add_argument('--audit-terminal',type=Path,required=True);ap.add_argument('--audit-terminal-sha256',required=True);args=ap.parse_args()
 assert __debug__ and os.sysconf('SC_CLK_TCK')==100 and args.timing_session>0
 assert args.output.is_absolute()and args.output.parent==HERE;args.output.mkdir(exist_ok=False)
 used={};result=dict(complete=False,performance_promotion=False)
 def raw(p):
  p=Path(p);assert p.is_file()and not p.is_symlink()and p.stat().st_size<=CAP
  data=p.read_bytes();b=dict(bytes=len(data),sha256=hashlib.sha256(data).hexdigest());assert str(p)not in used or used[str(p)]==b;used[str(p)]=b;return data
 def read(p):return json.loads(raw(p))
 try:
  pins=read(HERE/'preparation-pins.json')
  for p,b in pins['bindings'].items():raw(p);assert used[p]==b
  protocol=read(PREP/'protocol.json');aud=read(AUDIT);index_path=AUDIT.with_name('input-inventory.json');inventory=read(index_path)
  assert used[str(AUDIT)]['sha256']==args.audit_sha256 and used[str(index_path)]['sha256']==args.input_inventory_sha256
  term=read(args.audit_terminal);assert used[str(args.audit_terminal)]['sha256']==args.audit_terminal_sha256
  assert term['complete']is True and term['exit_code']==0 and term['terminal_receipt']
  if term['session_id']is None:
   assert term['execution_kind']=='direct_tool_terminal'and term['tool_result']['exit_code']==0 and term['tool_result']['chunk_id']==term['terminal_receipt']
  else:assert type(term['session_id'])is int and term['session_id']>0
  assert term['audit_path']==str(AUDIT)and term['audit_sha256']==args.audit_sha256 and term['input_inventory_path']==str(index_path)and term['input_inventory_sha256']==args.input_inventory_sha256
  assert aud['complete']is True and aud['matched_diagnostic_accepted']is True and aud['parent_confirmed_session']==args.timing_session and aud['parent_confirmed_exit_code']==0
  assert aud['protocol_id']==protocol['protocol_id']=='kv9-write-crc-slicing8-ab-c1-c64-v1'and aud['successful_arms']==aud['expected_successful_arms']==16 and len(aud['cases'])==16
  assert aud['owned_lifetimes_exited']==64 and aud['qualifying_drains']==48 and aud['voter_writer_listener_bindings']==48 and aud['performance_promotion']is False
  assert len(aud['compressed_retention']['cohorts'])==16 and aud['compressed_retention']['independently_decoded_all_bytes']is True
  def bound(p):
   v=read(p);assert inventory[str(p)]==used[str(p)],str(p);return v
  matrix=bound(RUN/'matrix.json');smoke=bound(Path('/tmp/kv9-write-crc-slicing8-ab-smoke-first/matrix.json'))
  assert len(smoke['attempts'])==8 and smoke['complete']is True and smoke['smoke']is True
  assert matrix['cohort_inventory']==protocol['timed_inventory']and matrix['complete']is True and matrix['smoke']is False
  assert matrix['role_bindings']==smoke['role_bindings']and set(matrix['role_bindings'])=={'old','new','client'}
  for role,pin in protocol['server_pins'].items():
   role_binding=matrix['role_bindings'][role];assert role_binding['binary_sha256']==pin['binary_sha256']and role_binding['source']['revision']==pin['revision']
  assert matrix['role_bindings']['client']['binary_sha256']==protocol['client_pins']['client']['binary_sha256']and matrix['role_bindings']['client']['source']['revision']==protocol['client_revision']
  rows=[];groups={};raw_pops={}
  for c,d in zip(aud['cases'],protocol['timed_inventory']):
   ordinal=len(rows);role=d['server_role'];w='point'if d['shared']['batch_size']==1 else 'batch64';con=d['shared']['workers'];repeat=d['repeat']
   assert c['ordinal']==ordinal and c['repeat']==repeat and c['concurrency']==con and c['arm']==d['arm']and c['batch_size']==d['shared']['batch_size']and c['write_api']==d['write_api']and c['read_percent']==0
   directory=RUN/f'{ordinal:03d}-{d["arm"]}-p{repeat}{con:04d}';assert c['directory']==str(directory)
   report=bound(directory/'run/report.json');cfg=bound(directory/'requested-config.json');samples=bound(directory/'resource-samples.json');coverage=bound(directory/'resource-coverage.json')
   assert used[str(directory/'run/report.json')]['sha256']==c['report_sha256']and all(cfg[k]==v for k,v in d['shared'].items())
   m=report['metrics']['measurement'];assert m['operations'][1]==('put'if w=='point'else 'batch_put')and m['outcomes'][0]=='success'
   assert sum(p['calls']for p in m['statistics'][0]['populations'])==0
   ps=m['statistics'][1]['populations'];assert latency(ps)==c['whole_call_latency']and latency([ps[0]])==c['successful_whole_call_latency']
   outcomes={n:p['calls']for n,p in zip(m['outcomes'],ps)};items={n:p['input_items']for n,p in zip(m['outcomes'],ps)}
   assert outcomes==c['outcomes']and sum(outcomes.values())==c['calls']==report['measured_completed']and sum(items.values())==c['input_items']
   assert c['cohort_elapsed_ns']==report['cohort_elapsed_ns']and c['issued']==report['measured_issued']and c['dropped_slots']==report['dropped_slots']and c['attempts']==sum(sum(op['attempt_reasons'])for op in m['statistics'])
   cpu=cpu_from_samples(samples,coverage,report)
   row=dict(ordinal=ordinal,repeat=repeat,order='forward'if repeat==0 else 'reverse',target=role,workload=w,operation=m['operations'][1],concurrency=con,batch_size=c['batch_size'],elapsed_ns=c['cohort_elapsed_ns'],successful_calls=outcomes['success'],successful_items=items['success'],completed_calls=c['calls'],completed_items=c['input_items'],issued_calls=c['issued'],data_attempts=c['attempts'],dropped_slots=c['dropped_slots'],completed_before_cutoff=c['completed_before_cutoff'],outcome_order=m['outcomes'],outcomes=outcomes,outcome_items=items,reasons=c['reasons'],attempt_reasons=c['attempt_reasons'],terminal_rpc_codes=c['terminal_rpc_codes'],cpu=cpu,healthy_comparison_eligible=c['all_success_single_attempt']and c['dropped_slots']==0 and outcomes['success']==c['issued']==c['calls']==c['attempts'] and all(n=='success'or value==0 for n,value in outcomes.items()),source_report=str(directory/'run/report.json'),source_report_sha256=c['report_sha256'],server_memory=c['server_memory'])
   single=aggregate([row],[ps]);row.update({k:single[k]for k in ('calls_per_second','items_per_second','completed_calls_per_second','completed_items_per_second','completed_after_cutoff','whole_call_latency','successful_whole_call_latency','outcome_latency','server_cpu_cores','client_cpu_cores')})
   assert math.isclose(row['calls_per_second'],c['successful_calls_per_second'],rel_tol=1e-12)and math.isclose(row['items_per_second'],c['successful_items_per_second'],rel_tol=1e-12)
   rows.append(row);groups.setdefault((w,con,role),[]).append(row);raw_pops[ordinal]=ps
  pooled=[];pairs=[]
  for (w,con,role),rs in groups.items():
   assert len(rs)==2 and {x['repeat']for x in rs}=={0,1}
   pooled.append(dict(workload=w,concurrency=con,target=role,operation=rs[0]['operation'],batch_size=rs[0]['batch_size'],**aggregate(rs,[raw_pops[x['ordinal']]for x in rs])))
  assert len(pooled)==8
  for w in ('point','batch64'):
   for con in (1,64):
    for repeat in (0,1):
     old,new=[next(r for r in rows if (r['workload'],r['concurrency'],r['repeat'],r['target'])==(w,con,repeat,t))for t in ROLES]
     pairs.append(comparison(old,new,'per_repeat',w,con,repeat))
    old,new=[next(r for r in pooled if(r['workload'],r['concurrency'],r['target'])==(w,con,t))for t in ROLES];pairs.append(comparison(old,new,'pooled',w,con))
  pooled.sort(key=lambda x:(x['workload']!='point',x['concurrency'],ROLES.index(x['target'])))
  healthy=all(r['healthy_comparison_eligible']for r in rows);assert aud['all_success_single_attempt']==all(c['all_success_single_attempt']for c in aud['cases'])
  result.update(version=1,complete=True,performance_promotion=False,healthy_comparison_eligible=healthy,scope=aud['scope'],timed_cohorts=16,smoke_cohorts=8,audit_path=str(AUDIT),audit_sha256=args.audit_sha256,audit_input_inventory_sha256=args.input_inventory_sha256,actual_timing_session=args.timing_session,audit_terminal=term,source_roles=matrix['role_bindings'],frozen_source_pins=dict(servers=protocol['server_pins'],client=protocol['client_pins']),durability=protocol['durability'],measured_successful_calls=sum(r['successful_calls']for r in rows),measured_successful_items=sum(r['successful_items']for r in rows),measured_completed_calls=sum(r['completed_calls']for r in rows),measured_issued_calls=sum(r['issued_calls']for r in rows),measured_data_attempts=sum(r['data_attempts']for r in rows),measured_dropped_slots=sum(r['dropped_slots']for r in rows),outcomes=summed([r['outcomes']for r in rows]),outcome_items=summed([r['outcome_items']for r in rows]),pooled=pooled,repetitions=rows,comparisons=pairs,rate_aggregation='Sum successful calls/items divided by sum cohort elapsed time; completed rates retained separately.',latency_aggregation='Merged integer raw histogram counts/sums, count-weighted means; p50/p95/p99 are bucket bounds, never averaged percentiles.',cpu_scope='Per-process first/last observations from samples tagged inside measurement, weighted by each process observed interval; three voter rates summed plus client separately. Endpoint gaps <=150ms. Sampling boundaries are nonidentical across processes and do not prove exclusive measurement-window CPU or per-request service time.',decision_scope='Valid accounting is separate from healthy comparison. Errors/unknowns/drops remain visible; no unconditional speedup/promotion or significance conclusion.')
  for path,b in list(used.items()):raw(path);assert used[path]==b
  write_outputs(args.output,result,used)
 except BaseException as e:
  result.update(complete=False,failure=repr(e));(args.output/'failure.json').write_text(json.dumps(result,indent=2)+'\n');raise
 print(json.dumps({k:result[k]for k in ('complete','healthy_comparison_eligible','timed_cohorts','measured_successful_calls','measured_successful_items','performance_promotion')}))

def write_outputs(out,result,used):
 def save(name,value):
  data=(json.dumps(value,indent=2,sort_keys=True)+'\n').encode();assert len(data)<=CAP
  with(out/name).open('xb')as f:f.write(data)
 save('summary.json',result);save('input-hashes.json',used)
 def q(h,key):
  v=h[key];return 'unavailable'if v is None else f'{v["lower_ns"]/1000:.3f}–{v["upper_ns"]/1000:.3f}'
 def table(rows,repeat):
  lines=['| '+('Order | 'if repeat else '')+'API | c | Role | Success calls/s | Success items/s | Mean us | p50 us bounds | p95 us bounds | p99 us bounds | Voter1 CPU | Voter2 CPU | Voter3 CPU | Server CPU sum | Client CPU | Outcomes |', '| '+('--- | 'if repeat else '')+'--- | ---: | --- | ---: | ---: | ---: | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | --- |']
  for row in rows:
   h=row['whole_call_latency'];mean='unavailable'if h['mean_ns']is None else f'{h["mean_ns"]/1000:.3f}'
   lines.append('| '+(row['order']+' | 'if repeat else '')+f'{row["operation"]} | {row["concurrency"]} | {row["target"]} | {row["calls_per_second"]:,.3f} | {row["items_per_second"]:,.3f} | {mean} | {q(h,"p50")} | {q(h,"p95")} | {q(h,"p99")} | {row["cpu"]["voter-1"]["cpu_cores"]:.3f} | {row["cpu"]["voter-2"]["cpu_cores"]:.3f} | {row["cpu"]["voter-3"]["cpu_cores"]:.3f} | {row["server_cpu_cores"]:.3f} | {row["client_cpu_cores"]:.3f} | '+json.dumps(row['outcomes'],sort_keys=True)+' |')
  return '\n'.join(lines)+'\n'
 note='Whole-call latency includes every completed outcome. Quantiles are histogram bucket bounds. CPU is sampled process usage, not per-request service time. Complete per-voter CPU, attempts, cutoff counts, outcomes and directional old/new comparisons are retained in summary.json.\n\n'
 (out/'README.md').write_text('Native CRC slicing-by-8 A/B screen\n\nHealthy comparison eligible: '+str(result['healthy_comparison_eligible']).lower()+'. No automatic promotion.\n\n'+note+table(result['pooled'],False))
 (out/'PER-REPEAT.md').write_text(note+table(result['repetitions'],True))
 (out/'COMPARISONS.md').write_text('Positive rate change means higher throughput; negative mean change means lower latency. Percentiles retain interval directions.\n\n| Scope | API | c | Repeat | Calls/s change % | Mean change % | p99 direction | Healthy |\n| --- | --- | ---: | --- | ---: | ---: | --- | --- |\n'+'\n'.join('| '+f'{r["scope"]} | {r["workload"]} | {r["concurrency"]} | {r["repeat"]} | {r["successful_calls_per_second_change_percent"]} | {r["mean_latency_change_percent"]} | {r["quantile_directions"]["p99"]} | {r["healthy_comparison_eligible"]}'+' |'for r in result['comparisons'])+'\n')
if __name__=='__main__':main()
