#!/usr/bin/env python3
"""Read back two lifecycle fixtures; retain all non-CPU correctness predicates."""
import hashlib,importlib.util,json,os,re,sys
from pathlib import Path
if not __debug__:raise RuntimeError('readback requires PYTHONOPTIMIZE=0')
sys.dont_write_bytecode=True
ROOT=Path(__file__).parent
def read(p):return json.loads(Path(p).read_text())
def sha(p):
 with Path(p).open('rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()
def check(ok,msg):
 if not ok:raise ValueError(msg)
def module(name,p):
 s=importlib.util.spec_from_file_location(name,p);m=importlib.util.module_from_spec(s);s.loader.exec_module(m);return m
runner=module('owned_profile_runner',ROOT/'profile.py')
driver=module('matched_readback',runner.DRIVER);resp=module('dataset_readback',runner.RESP)
check(runner.preflight(driver)==read(ROOT/'build-bindings.json'),'source/build changed')
protocol=read(ROOT/'protocol.json')
check(all(sha(p)==h for p,h in protocol['helper_sources'].items()),'executed helper changed')
sys.path.insert(0,str(runner.SOURCE/'scripts'))
import batch_benchmark_report as validator
result={'complete':False,'cpu_profiling':False,'profiles':[]}
for api in protocol['profiles']:
 d=ROOT/api;r=read(d/'run/report.json');c=read(d/'requested-config.json');cleanup=read(d/'cleanup.json');ids={x['pid']:x['identity']for x in cleanup['children']}
 validated=validator.validate(d/'run',runner.CLIENT_BUILD,d/'requested-config.json',protocol['client_revision'],require_timing=False)
 check(validated['accepted']and r['complete']and r['stop_reason']=='duration'and c['read_api']==api and c['measure_ms']==5000,'strict report identity differs')
 check(read(d/'fixture-result.json')['complete']and not cleanup.get('errors')and len(ids)==4,'fixture failed or cleanup reported errors')
 check(all(x['exit_code']==(0 if x['pid']==r['process_id']else -9)and x['absent']and not Path('/proc',str(x['pid'])).exists()for x in cleanup['children']),'fixture lifecycle differs')
 for pid,ident in ids.items():
  expected=protocol['client_sha256']if pid==r['process_id']else protocol['server_sha256']
  check(ident['executable_sha256']==expected and ident['cpu_affinity']==([0,1]if pid==r['process_id']else [2,3,4,5]),'source/placement differs')
 before=read(d/'listener-before.json');after=read(d/'listener-after.json')
 for node,first in before.items():
  last=after[node];check(all(first[k]==last[k]for k in ('pid','advertised_endpoint','listener_inodes'))and first['experimental_environment_absent']and last['experimental_environment_absent'],'listener changed')
 for label in ('before','post-client','post-readback'):
  drain=read(d/(label+'-fresh-drain.json'));check(0<=drain['completed_unix_ns']-drain['started_unix_ns']<=20_000_000_000,'drain stale')
  for node,base in drain['baseline'].items():
   first=drain['first_advances'][node]['status'];last=drain['final'][node]
   check(int(base['metrics_export_successes'])<int(first['metrics_export_successes'])<int(last['metrics_export_successes']),'exports not newer')
   for state in (base,first,last):
    ident=ids[int(state['pid'])];check(int(state['process_start_ticks'])==ident['start_ticks']and state['process_boot_id']==ident['boot_id'],'status writer changed')
   for state in (first,last):
    check(state['bootstrap_state']=='Serving'and state['fatal']==''and all(state[k]=='0'for k in driver.ZERO)and state['raft_async_apply_stopped']==state['raft_async_read_stopped']=='false'and state['applied_term']==state['driver_applied_term']and state['applied_index']==state['driver_applied_index'],'qualifying drain was busy')
  check(len({(v['applied_term'],v['applied_index'])for v in drain['final'].values()})==1,'drain positions differ')
 dataset={}
 for page in sorted(d.glob('readback-*.txt')):
  lines=page.read_text().splitlines();check(lines[-1]==f'count={len(lines)-1}','scan count differs')
  for line in lines[:-1]:
   fields=dict(part.split('=',1)for part in line.split());key=bytes.fromhex(fields['key_hex']);check(key not in dataset,'scan duplicates');dataset[key]=bytes.fromhex(fields['value_hex'])
 checked=driver.readonly_dataset(c,dataset,resp)
 start=r['measurement_start_unix_ns'];end=start+r['cohort_elapsed_ns'];samples=[x for x in read(d/'resource-samples.json')if start<=x['unix_ns']<=end]
 check(len(samples)==read(d/'resource-coverage.json')['samples']>=10,'resource coverage differs')
 for sample in samples:
  check({v['pid']for v in sample['processes'].values()}==set(ids),'sample processes differ')
  for name,v in sample['processes'].items():
   ident=ids[v['pid']];mask=[0,1]if name=='client'else [2,3,4,5];t=v['thread_placement']
   check(v['start_ticks']==ident['start_ticks']and v['cpu_affinity']==t['process']==t['expected']==mask and t['threads']and all(x==mask for x in t['threads'].values()),'thread sample placement differs')
 metrics=r['metrics']['measurement'];calls=r['measured_completed'];attempts=sum(sum(op['attempt_reasons'])for op in metrics['statistics'])
 reasons={n:sum(op['reasons'][i]for op in metrics['statistics'])for i,n in enumerate(metrics['reasons'])}
 check(calls==attempts==r['measured_issued']and r['dropped_slots']==0,'attempt/accounting differs')
 retention=read(d/'fixture/tmpfs-retention.json')
 check(retention['complete']and retention['scratch_removed']and not Path(retention['original_scratch']).exists(),'tmpfs cleanup incomplete')
 retained=Path(retention['retained_data'])
 check(retained==d/'fixture/data'and retained.is_dir()and not retained.is_symlink(),'retained data directory differs')
 check({str(p.relative_to(retained))for p in retained.rglob('*')if p.is_file()}==set(retention['files']),'retained file inventory differs')
 for name,item in retention['files'].items():
  path=retained/name;check(path.stat().st_size==item['bytes']and sha(path)==item['sha256'],'retained data differs')
 result['profiles'].append(dict(api=api,complete=True,owned_fixture_lifetimes_exited=4,ordinary_listeners_bound=3,fresh_two_export_drains=3,full_nonce_zero_dataset=checked,measured_resource_samples=len(samples),measured_calls=calls,sdk_attempts=attempts,reasons=reasons,retained_files=len(retention['files']),retained_bytes=sum(v['bytes']for v in retention['files'].values())))
result['complete']=True
(ROOT/'readback-summary.json').write_text(json.dumps(result,indent=2)+'\n')
print('PASS: two original profile fixtures, source bindings, accounting, full nonce-zero readback, resources and fresh drains')
