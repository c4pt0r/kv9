"""Same-artifact unary/tarpc/unary bracket with paired Redis and volatile tmpfs."""
from pathlib import Path
import hashlib, importlib.util, json, os, shutil, subprocess, sys, time
ROOT=Path('/tmp/kv9-rpc-framework-experiments')
HELPERS=Path('/tmp/kv9-redis-comparison-baseline/scripts')
sys.path.insert(0,str(ROOT/'scripts'))
from benchmark import save,read,host,process_sample

def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
cmp=module('comparison',HELPERS/'redis-comparison.py')
tmpfs=module('tmpfs',HELPERS/'tmpfs-redis-diagnostic.py')
rpc=module('rpc_fixture',ROOT/'scripts/rpc-experiment-e2e.py')
BUILD=Path('/tmp/kv9-rpc-framework-release-b49a2f6')
REDIS=Path('/tmp/kv9-redis-reference-build-v2/kv9-redis-reference')
OUT=Path('/tmp/kv9-rpc-framework-comparison-b49a2f6-first');OUT.mkdir(exist_ok=False)
REV='b49a2f6abe6e91b986a633e8a36b19ffccea9a0b'
placement=dict(client_cpus=[0,1],server_cpus=[2,3,4,5],separated=True,exclusive_host=False)
manifest=read(BUILD/'build.json');client_manifest=read(BUILD/'workload/build.json')
assert manifest['revision']==REV and not manifest['dirty'] and manifest['profile']=='release'
assert client_manifest['revision']==REV and not client_manifest['dirty'] and client_manifest['profile']=='release'
assert manifest['features']==['rpc-experiment']
assert cmp.sha(BUILD/'kv9')==manifest['binary_sha256']
assert cmp.sha(BUILD/'workload/kv9-workload')==client_manifest['binary_sha256']
assert cmp.sha(BUILD/'workload/build.json')==manifest['workload_build_sha256']
assert manifest['source_tree_sha256']==client_manifest['source_tree_sha256']
assert cmp.sha(REDIS)=='57947d5a94c739dd58ca98e2c9a85c40814040826f13f99e85cd0e2c7fb2f636'
assert sorted(os.sched_getaffinity(0))==list(range(32))
for name,digest in manifest['sources'].items():assert cmp.sha(ROOT/name)==digest,name
assert not subprocess.check_output(['git','status','--porcelain'],cwd=ROOT)
shutil.copy2(__file__,OUT/'driver.py')
save(OUT/'driver-inputs.json',{str(p):cmp.sha(p) for p in [Path(__file__),HELPERS/'redis-comparison.py',HELPERS/'tmpfs-redis-diagnostic.py',ROOT/'scripts/rpc-experiment-e2e.py',ROOT/'scripts/benchmark.py',ROOT/'scripts/workload_report.py']})
host(OUT,True)
protocol=dict(version=1,diagnostic_only=True,volatile=True,power_loss_durability=False,revision=REV,
 keys=64,value_bytes=128,key_bytes=23,concurrency=[64],repetitions=2,measure_ms=1500,warmup_operations=32,
 kv9_max_operations=1000000,redis_max_operations=5000000,kv9_durability=tmpfs.VOLATILE,
 redis_durability='Standalone Redis memory; save disabled, AOF disabled, no replicas',
 comparable_durability=False,placement=placement,pipeline_depth=1,
 server_sha256=manifest['binary_sha256'],client_sha256=client_manifest['binary_sha256'],
 redis_client_sha256=cmp.sha(REDIS),redis_server_sha256=cmp.sha(shutil.which('redis-server')),
 redis_version=subprocess.check_output(['redis-server','--version'],text=True).strip(),
 order=['tonic-before','tarpc','tonic-after'],mixes=['read','write','mixed'],
 window_reason='1500ms in all arms keeps the fixed 1M KV9 operation cap above the Redis-class target; short new diagnostic, not the older 3s protocol',
 scope='Repeated same-artifact RPC framework diagnostic on one shared host with volatile tmpfs; no durable/cross-host/Chaos or production acceptance. New experimental client used in BOTH KV9 arms; no old client relabeling.',
 retry_rule='Existing KV9 client retries typed NotLeader only; unknown writes remain terminal. Redis has no retries.')
save(OUT/'protocol.json',protocol)
class ExperimentFixture(rpc.RpcFixture,tmpfs.TmpfsFixture):
 def __init__(self,out,transport):
  super().__init__(out,BUILD);self.transport=transport;self.placement.update(placement)
 def launch(self,command,logfile,client=False):
  if client:
   values=list(map(str,command));path=Path(values[values.index('--config')+1]);c=read(path)
   c['rpc_transport']=self.transport
   if self.transport=='tarpc_tcp':
    c['client']['peers']=[dict(node_id=n,address=self.rpc_addresses[n]) for n in self.nodes]
   save(path,c)
  process=super().launch(command,logfile,client)
  identity=self.identities[process.pid]
  assert identity["cpu_affinity"]==self.placement["client_cpus" if client else "server_cpus"]
  assert identity["executable_sha256"]==(client_manifest["binary_sha256"] if client else manifest["binary_sha256"])
  return process
 def fresh_drain(self,directory):
  baseline={str(n):self.state(n) for n in self.nodes};first={};started=time.time_ns()
  assert all(baseline.values())
  zero=('public_rpc_in_flight','public_rpc_queued','public_rpc_running','public_rpc_encoded_bytes','raft_async_apply_queued','raft_async_apply_in_flight','raft_async_read_queued','raft_async_read_active','raft_async_read_in_flight','raft_async_read_active_groups')
  def check():
   states={str(n):self.state(n) for n in self.nodes}
   for n,state in states.items():
    assert state and all(state[k]==baseline[n][k] for k in ('pid','process_start_ticks','process_boot_id'))
    if int(state['metrics_export_successes'])>int(baseline[n]['metrics_export_successes']) and n not in first:first[n]=dict(status=state,observed_unix_ns=time.time_ns())
   if any(n not in first or int(s['metrics_export_successes'])<=int(first[n]['status']['metrics_export_successes']) for n,s in states.items()):return None
   return states if all(all(s.get(k)=='0' for k in zero) and s['raft_async_apply_stopped']==s['raft_async_read_stopped']=='false' for s in states.values()) else None
  final=self.wait('fresh post-client public/read/apply drain',check,15)
  save(directory/'fresh-drain.json',dict(started_unix_ns=started,baseline=baseline,first=first,final=final,completed_unix_ns=time.time_ns()))
result=dict(complete=False,attempts=[],fixtures=[])
save(OUT/'matrix.json',result)
try:
 for arm,transport in [('tonic-before','tonic_unary'),('tarpc','tarpc_tcp'),('tonic-after','tonic_unary')]:
  f=ExperimentFixture(OUT/arm,transport);fixture_record=dict(arm=arm,complete=False);result['fixtures'].append(fixture_record)
  try:
   f.start();f.trial('guard-before','mixed',4,0,protocol,guard=True)
   for repeat in range(2):
    for mi,mix in enumerate(protocol['mixes']):
     name=f'b{repeat:02}64{mi}';assert len(name)==6
     for target in (['kv9','redis-memory'] if repeat==0 else ['redis-memory','kv9']):
      d=f.out/target/name;d.mkdir(parents=True,exist_ok=False)
      entry=dict(arm=arm,transport=transport,target=target,mix=mix,repeat=repeat,directory=str(d),complete=False)
      result['attempts'].append(entry);save(OUT/'matrix.json',result)
      if target=='kv9':
       cmp.kv9_trial(f,d,mix,64,repeat,protocol);f.fresh_drain(d)
      else:cmp.redis_trial(d,REDIS,mix,64,repeat,protocol,placement)
      summary=cmp.summarize(d,target);save(d/'summary.json',summary)
      assert summary['successful']==summary['issued']==summary['completed'] and summary['failed']==0
      if target=='kv9':assert summary['attempts']==summary['issued']
      entry.update(complete=True,summary=summary);save(OUT/'matrix.json',result)
      print(f"PASS {arm} {target} {mix} r{repeat}: {summary['success_ops_per_second']:.1f}/s",flush=True)
   f.trial('guard-after','mixed',4,1,protocol,guard=True);f.fresh_drain(f.out)
   f.finish();fixture_record['workloads_complete']=True
  finally:
   f.close();fixture_record['children_exited']=all(p.poll() is not None for p in f.children)
   fixture_record['complete']=fixture_record.get('workloads_complete',False) and fixture_record['children_exited']
   save(OUT/'matrix.json',result)
 assert len(result['attempts'])==36 and all(e['complete'] for e in result['attempts'])
 assert all(f['complete'] for f in result['fixtures'])
 for name,digest in manifest['sources'].items():assert cmp.sha(ROOT/name)==digest,name
 assert cmp.sha(BUILD/'kv9')==manifest['binary_sha256'] and cmp.sha(BUILD/'workload/kv9-workload')==client_manifest['binary_sha256']
 result['complete']=True
except BaseException as error:
 result['failure']=repr(error);raise
finally:save(OUT/'matrix.json',result)
print('PASS: complete 36-cohort unary/tarpc/unary bracket with paired Redis; independent final audit still required',flush=True)
