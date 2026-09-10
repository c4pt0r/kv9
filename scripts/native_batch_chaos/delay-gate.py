"""Owned bounded observation gate; extends only an already-active delay window."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
from pathlib import Path
import argparse,hashlib,importlib.util,json,subprocess,time
PROBE_PATH=Path(__file__).parent/'process-probe.py'
spec=importlib.util.spec_from_file_location('owned_probe3',PROBE_PATH);probe=importlib.util.module_from_spec(spec);spec.loader.exec_module(probe)

def active_fault(document,namespace):
 found=[x for x in document['items'] if x['kind']=='NetworkChaos' and x['metadata']['name']=='delay-follower']
 assert len(found)==1,'delay fault missing or ambiguous';f=found[0]
 assert f['metadata']['namespace']==namespace and f['spec']['selector']['namespaces']==[namespace],'wrong namespace'
 assert not f['metadata'].get('deletionTimestamp'),'delay fault is healing'
 assert any(c['type']=='AllInjected' and c['status']=='True' for c in f.get('status',{}).get('conditions',[])),'delay fault not injected'
 assert not any(c['type']=='AllRecovered' and c['status']=='True' for c in f.get('status',{}).get('conditions',[])),'delay fault recovered'
 return f['metadata']['uid']

def completed_get(events,threshold):
 calls={x['id']:x for x in events if x.get('type')=='invoke'}
 return [dict(invocation=calls[x['id']],completion=x) for x in events if x.get('type')=='return' and x.get('outcome')=='ok' and x['id'] in calls and calls[x['id']].get('op')=='get' and calls[x['id']].get('phase')=='delay' and calls[x['id']]['monotonic_ns']>=threshold and x['monotonic_ns']>=calls[x['id']]['monotonic_ns']]

def matching_growth(before,after):
 found=[]
 for node,a in before.items():
  if node not in after:continue
  b=after[node]
  if (a['pod_uid'],a['process_pid'],a['process_start_ticks'],a['process_boot_id'])!=(b['pod_uid'],b['process_pid'],b['process_start_ticks'],b['process_boot_id']):continue
  assert a['status_writer_identity_matched'] and b['status_writer_identity_matched']
  aa=int(a['state_after']['public_raw_get_completed_inline']);bb=int(b['state']['public_raw_get_completed_inline'])
  assert bb>=aa,'inline counter decreased inside an attested lifetime'
  if bb>aa:found.append(dict(node=node,inline_before=aa,inline_after=bb,pod_uid=a['pod_uid'],pid=a['process_pid'],start_ticks=a['process_start_ticks'],boot_id=a['process_boot_id']))
 return found

def writer_identity(row):
 return row['pod_uid'],row['process_pid'],row['process_start_ticks'],row['process_boot_id']

class ServerFreshness:
 def __init__(self,seed):
  self.identity={node:writer_identity(row) for node,row in seed.items()}
  self.last={node:int(row['state_after']['metrics_export_successes']) for node,row in seed.items()}
  self.advances={node:0 for node in seed}
 def observe(self,rows):
  fresh={}
  for node,row in rows.items():
   if node not in self.identity or writer_identity(row)!=self.identity[node]:continue
   before=int(row['state']['metrics_export_successes']);after=int(row['state_after']['metrics_export_successes'])
   assert 0<=self.last[node]<=before<=after,'server export counter decreased in one writer lifetime'
   if before>self.last[node]:
    self.advances[node]+=1;self.last[node]=after
   if self.advances[node]>=2:fresh[node]=row
  return fresh

def wait_qualifying(check,deadline,interval=.2):
 while time.monotonic()<deadline:
  result=check()
  if result is not None:return result
  time.sleep(min(interval,max(0,deadline-time.monotonic())))
 raise TimeoutError('bounded delay observation gate expired without qualifying evidence')

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--plan',required=True);ap.add_argument('--artifact',required=True);ap.add_argument('--namespace',required=True);ap.add_argument('--timeout-seconds',type=float,default=20);args=ap.parse_args();assert 0<args.timeout_seconds<=20
 plan=json.loads(Path(args.plan).read_text());namespace=args.namespace;assert namespace not in plan['preserve_namespaces'] and namespace.startswith('kv9-chaos-');raw=Path(args.artifact);out=raw/'owned-delay-gate';out.mkdir(exist_ok=False);deadline=time.monotonic()+args.timeout_seconds;commands=[];record=dict(accepted=False,timeout_seconds=args.timeout_seconds,namespace=namespace,plan=args.plan,probe_sha256=hashlib.sha256(PROBE_PATH.read_bytes()).hexdigest(),started_unix_ns=time.time_ns(),commands=commands)
 def run(*argv):
  remaining=deadline-time.monotonic();assert remaining>0,'gate deadline expired'
  started=time.time_ns();p=subprocess.run(['kubectl','--kubeconfig',plan['kubeconfig'],*argv],capture_output=True,text=True,timeout=min(4,remaining));row=dict(command=['kubectl','--kubeconfig',plan['kubeconfig'],*argv],started_unix_ns=started,ended_unix_ns=time.time_ns(),exit_code=p.returncode,stdout=p.stdout,stderr=p.stderr);commands.append(row);assert p.returncode==0,('owned probe command failed',argv,p.stderr);return row
 def fault():
  assert (raw/'history.phase').read_text().strip()=='delay','delay phase ended'
  return active_fault(json.loads(run('get','networkchaos','-n',namespace,'-o','json')['stdout']),namespace)
 def sample():
  first=json.loads(run('get','pods','-n',namespace,'-l','app=kv9','-o','json')['stdout']);states={};pods={}
  for pod in first['items']:
   node=pod['metadata']['labels'].get('kv9-node')
   if node not in ['1','2','3','4'] or pod['status'].get('phase')!='Running':continue
   if not all('running' in x.get('state',{}) for x in pod['status'].get('containerStatuses',[])):continue
   row=run('exec','-n',namespace,pod['metadata']['name'],'--','/bin/bash','-c',probe.PROBE,'owned-probe','/data/status','/usr/local/bin/kv9')
   parsed=probe.parse(row['stdout'],plan['production_binary']['sha256'],'/usr/local/bin/kv9');parsed.update(pod_uid=pod['metadata']['uid'],pod=pod['metadata']['name'],capture_started_unix_ns=row['started_unix_ns'],capture_ended_unix_ns=row['ended_unix_ns']);states[node]=parsed;pods[node]=pod
  last=json.loads(run('get','pods','-n',namespace,'-l','app=kv9','-o','json')['stdout']);later={x['metadata']['name']:x for x in last['items']}
  def identity(p):return p['metadata']['uid'],sorted((x['name'],x.get('containerID'),x.get('imageID'),x.get('restartCount')) for x in p['status'].get('containerStatuses',[]))
  for node,pod in pods.items():
   other=later.get(pod['metadata']['name']);assert other and identity(pod)==identity(other) and all('running' in x.get('state',{}) for x in other['status'].get('containerStatuses',[])),'container identity changed during gate sample'
  assert states,'no boot/start-attested running process available'
  return states
 def progress():
  row=run('exec','-n',namespace,'kv9-persistent-client','--','/bin/bash','-c','test ! -e /tmp/workload.exit && cat /tmp/workload/progress.json');j=json.loads(row['stdout']);assert j['stage']=='measure','correctness client not measuring';return j
 try:
  uid=fault();seed=sample();assert fault()==uid;record['freshness_seed']=seed;record['fault_uid']=uid;tracker=ServerFreshness(seed);freshness_samples=[]
  # NodeRuntime has one production write_status loop. It synchronously exports
  # metrics, increments successes after successful publication, samples the
  # public counters, then atomically publishes status. Two observed advances
  # exclude an already-in-progress pre-fault status publication.
  def fresh_server():
   assert fault()==uid;current=sample();assert fault()==uid
   fresh=tracker.observe(current);freshness_samples.append(dict(rows=current,advances=dict(tracker.advances)))
   return fresh or None
  a=wait_qualifying(fresh_server,deadline);record['server_freshness_samples']=freshness_samples;record['before']=a
  # A precedes the first progress read. Require TWO serial export advances:
  # one export could already have sampled its clock before A but still be
  # publishing. The next export samples its clock after that publication.
  # The exact client runner has one synchronous atomic_json export loop.
  first=progress();record['progress_first']=first;advances=[]
  for _ in range(2):
   def fresh_progress():
    assert fault()==uid;current=progress()
    return current if current['monotonic_ns']>first['monotonic_ns'] else None
   first=wait_qualifying(fresh_progress,deadline);advances.append(first)
  record['progress_advances']=advances;threshold=advances[-1]['monotonic_ns']
  def qualify():
   assert fault()==uid
   history=run('exec','-n',namespace,'kv9-persistent-client','--','/bin/bash','-c','test ! -e /tmp/workload.exit && cat /tmp/workload/history.jsonl');events=[]
   for line in history['stdout'].splitlines(keepends=True):
    if line.endswith('\n'):events.append(json.loads(line))
   gets=completed_get(events,threshold)
   if not gets:return None
   b=sample();assert fault()==uid;growth=matching_growth(a,b)
   if not growth:return None
   return dict(after=b,contained_successful_gets=gets,inline_growth=growth,history_capture_ended_unix_ns=history['ended_unix_ns'])
  accepted=wait_qualifying(qualify,deadline);record.update(accepted,accepted=True,ended_unix_ns=time.time_ns(),scope='Owned fixed-bound delay extension. Active exact fault brackets boot/start-attested status captures; the first qualifying capture follows two serial server export-success advances within that same writer lifetime. Two serial live progress advances and program order place complete successful persistent GETs after the first capture and before the second; final complete-history wall anchors must independently confirm containment. Inline activity is envelope-scoped, not assigned to a named GET. Original workloads/checkers and counter monotonicity remain unchanged.')
 finally:
  record.setdefault('ended_unix_ns',time.time_ns());(out/'record.json').write_text(json.dumps(record,indent=2)+'\n')
 assert record['accepted'];print('PASS: bounded owned delay observation gate captured exact-lifetime inline growth and contained persistent GET')
if __name__=='__main__':main()
