"""Read-only exact-runtime/inline-activity observations during the owned matrix."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
import argparse,json,subprocess,time,os,importlib.util
spec=importlib.util.spec_from_file_location("owned_probe",str(Path(__file__).parent/'process-probe.py'));probe=importlib.util.module_from_spec(spec);spec.loader.exec_module(probe)
field_spec=importlib.util.spec_from_file_location("owned_fields",str(Path(__file__).parent/'observer-fields.py'));fields=importlib.util.module_from_spec(field_spec);field_spec.loader.exec_module(fields)
ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--plan',type=Path,required=True);args=ap.parse_args();P=args.plan;plan=json.loads(P.read_text());raw=Path(plan['artifact']);output=raw/'async-write-live-observer.json';stop=raw/'async-write-observer.stop';namespace=None;observations=[];attempts=[];start=time.monotonic()
expected=plan['production_binary']['sha256'];before=plan['preserve_namespaces'];image=plan['image'];kube=plan['kubeconfig']
def command(argv):
 at=time.time_ns()
 try:
  p=subprocess.run(argv,text=True,capture_output=True,timeout=12);return dict(command=argv,started_unix_ns=at,ended_unix_ns=time.time_ns(),exit_code=p.returncode,stdout=p.stdout,stderr=p.stderr)
 except subprocess.TimeoutExpired as e:return dict(command=argv,started_unix_ns=at,ended_unix_ns=time.time_ns(),exit_code='timeout',stdout=e.stdout.decode() if isinstance(e.stdout,bytes) else e.stdout or '',stderr=e.stderr.decode() if isinstance(e.stderr,bytes) else e.stderr or '')
def k(*args):return command(['kubectl','--kubeconfig',kube,*args])
def phase():
 p=raw/'history.phase';return p.read_text().strip() if p.exists() else 'before-history'
def save(complete=False):output.write_text(json.dumps(dict(version=3,complete=complete,namespace=namespace,expected_server_sha256=expected,expected_image=image,observations=observations,attempts=attempts,scope='Read-only sampled status/executable/Chaos state. Inline deltas are per-lifecycle observation-envelope activity, not per-response attribution. Setup-only activity is insufficient. Every failed transient query remains visible; only successful identity-matched samples can support positive evidence. Async-apply occupancy/high-water values are lifecycle observations; stopped intermediate states are retained. Any status-contract violation tied to an exact process fails acceptance.'),indent=2)+'\n')
def sample_pod(pod):
 seen_phase=phase();name=pod['metadata']['name']
 r=k('exec','-n',namespace,name,'--','/bin/bash','-c',probe.PROBE,'owned-probe','/data/status','/usr/local/bin/kv9')
 r.update(node=pod['metadata']['labels'].get('kv9-node'),pod=name,pod_uid=pod['metadata']['uid'],container_statuses=pod['status'].get('containerStatuses',[]),phase_before=seen_phase)
 if r['exit_code']==0:
  try:
   parsed=probe.parse(r['stdout'],expected,'/usr/local/bin/kv9')
   assert 'public_raw_get_completed_inline' in parsed['state'] and 'raft_async_read_active_groups' in parsed['state'],'runtime metrics missing'
   r.update(parsed)
   try:r.update(fields.validate(parsed,pod,plan['source_default_limits']))
   except (AssertionError,KeyError,ValueError,IndexError) as e:r.update(status_contract_valid=False,status_contract_error=repr(e))
  except (AssertionError,KeyError,ValueError,IndexError) as e:r.update(identity_matched=False,identity_rejection=repr(e))
 r['phase_after']=phase();return r

while not stop.exists() and time.monotonic()-start<2400:
 if namespace is None:
  ns=k('get','namespaces','-o','json')
  if ns['exit_code']==0:
   for n in json.loads(ns['stdout'])['items']:
    name=n['metadata']['name']
    if name in before or not name.startswith('kv9-chaos-'):continue
    pods=k('get','pods','-n',name,'-l','app=kv9','-o','json')
    if pods['exit_code']==0 and any(any(c['image']==image for c in p['spec']['containers']) for p in json.loads(pods['stdout'])['items']):namespace=name;attempts.append(dict(kind='namespace_discovery',namespace_object=n,pods=pods));break
  if namespace is None:time.sleep(.5);continue
 first_phase=phase();at=time.time_ns();pods=k('get','pods','-n',namespace,'-l','app=kv9','-o','json');faults_before=k('get','podchaos,networkchaos,iochaos','-n',namespace,'-o','json')
 rows=[]
 if pods['exit_code']==0:
  chosen=[p for p in json.loads(pods['stdout'])['items'] if p['status'].get('phase')=='Running' and p['metadata']['labels'].get('kv9-node') in ['1','2','3','4'] and all(c['image']==image for c in p['spec']['containers'])]
  with ThreadPoolExecutor(max_workers=4) as pool:rows=list(pool.map(sample_pod,chosen))
 faults_after=k('get','podchaos,networkchaos,iochaos','-n',namespace,'-o','json')
 pods_after=k('get','pods','-n',namespace,'-l','app=kv9','-o','json')
 after_by_name={p['metadata']['name']:p for p in json.loads(pods_after['stdout'])['items']} if pods_after['exit_code']==0 else {}
 for row in rows:
  later=after_by_name.get(row['pod']);row['pod_after']=later
  def containers(states):return sorted((x['name'],x.get('containerID'),x.get('imageID'),x.get('restartCount')) for x in states)
  if row.get('identity_matched') and (later is None or later['metadata']['uid']!=row['pod_uid'] or containers(later['status'].get('containerStatuses',[]))!=containers(row['container_statuses']) or not all('running' in x.get('state',{}) for x in row['container_statuses']) or not all('running' in x.get('state',{}) for x in later['status'].get('containerStatuses',[]))):row.update(identity_matched=False,identity_rejection='Pod UID/container identity changed or container was not running across observation')
 observations.append(dict(started_unix_ns=at,ended_unix_ns=time.time_ns(),phase_before=first_phase,phase_after=phase(),pods=pods,pods_after=pods_after,rows=rows,faults_before=faults_before,faults_after=faults_after));save();time.sleep(.5)
save(complete=True)
assert namespace is not None and any(row.get('identity_matched') for o in observations for row in o['rows']), 'observer did not capture an executing exact async-write runtime'

assert not any(row.get('identity_matched') and row.get('status_contract_valid') is False for o in observations for row in o['rows']), 'exact executing process violated status contract; observations retained'
print(json.dumps(dict(complete=True,namespace=namespace,observations=len(observations),identity_matched_samples=sum(bool(row.get('identity_matched')) for o in observations for row in o['rows']),output=str(output))))
