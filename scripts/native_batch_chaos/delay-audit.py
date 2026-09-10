"""Independent retained-evidence audit of fresh fault-window observation ordering."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
from pathlib import Path
import json,hashlib,importlib.util
import argparse
ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--plan',required=True);ap.add_argument('--artifact',required=True);args=ap.parse_args()
P=Path(args.plan);p=json.loads(P.read_text());r=Path(args.artifact);record=json.loads((r/'owned-delay-gate/record.json').read_text());assert record['accepted'] and record['timeout_seconds']==20
assert record['ended_unix_ns'] - record['started_unix_ns'] < 20_000_000_000
spec=importlib.util.spec_from_file_location('probe3',str(Path(__file__).parent/'process-probe.py'));probe=importlib.util.module_from_spec(spec);spec.loader.exec_module(probe)
commands=record['commands'];probes={(x['started_unix_ns'],x['ended_unix_ns']):x for x in commands if '@@STATUS_BEFORE@@' in x['stdout']};expected=p['production_binary']['sha256']
def identity(row):return row['pod_uid'],row['process_pid'],row['process_start_ticks'],row['process_boot_id']
def verify(row):
 raw=probes[(row['capture_started_unix_ns'],row['capture_ended_unix_ns'])];assert raw['exit_code']==0
 q=probe.parse(raw['stdout'],expected,'/usr/local/bin/kv9');assert q['status_writer_identity_matched']
 for name,value in q.items():assert row[name]==value,(name,row.get(name),value)
 return q
for group in [record['freshness_seed'],record['before'],record['after']]+[x['rows'] for x in record['server_freshness_samples']]:
 for row in group.values():verify(row)
for cmd in commands:
 assert cmd['exit_code']==0
 assert cmd['command'][0:3]==['kubectl','--kubeconfig',p['kubeconfig']]
faults=[]
for cmd in commands:
 if cmd['command'][3:5]!=['get','networkchaos']:continue
 document=json.loads(cmd['stdout']);found=[x for x in document['items'] if x['kind']=='NetworkChaos' and x['metadata']['name']=='delay-follower'];assert len(found)==1
 f=found[0];assert f['metadata']['uid']==record['fault_uid'] and f['metadata']['namespace']==record['namespace'];assert f['spec']['selector']['namespaces']==[record['namespace']];assert not f['metadata'].get('deletionTimestamp');conditions=f.get('status',{}).get('conditions',[]);assert any(x['type']=='AllInjected' and x['status']=='True' for x in conditions);assert not any(x['type']=='AllRecovered' and x['status']=='True' for x in conditions);faults.append(cmd)
assert len(faults)>=4
freshness=[]
for node,a in record['before'].items():
 seed=record['freshness_seed'][node];assert identity(seed)==identity(a);last=int(seed['state_after']['metrics_export_successes']);advances=[]
 for sample in record['server_freshness_samples']:
  row=sample['rows'].get(node)
  if not row or identity(row)!=identity(a):continue
  lower=int(row['state']['metrics_export_successes']);upper=int(row['state_after']['metrics_export_successes']);assert lower>=last and upper>=lower
  if lower>last:advances.append(dict(before=lower,after=upper,capture_ended_unix_ns=row['capture_ended_unix_ns']));last=upper
 assert len(advances)>=2,(node,advances)
 assert advances[-1]['capture_ended_unix_ns']<=a['capture_ended_unix_ns'];freshness.append(dict(node=node,identity=identity(a),advances=advances))
progress=[record['progress_first'],*record['progress_advances']];assert len(progress)==3 and all(x['stage']=='measure' for x in progress);assert progress[0]['monotonic_ns']<progress[1]['monotonic_ns']<progress[2]['monotonic_ns']
report=json.loads((r/'persistent-run/report.json').read_text());events=[json.loads(line) for line in (r/'persistent-run/history.jsonl').read_text().splitlines()];indexed={(x['id'],x['type']):x for x in events if x.get('type') in ['invoke','return']};left=max(x['capture_ended_unix_ns'] for x in record['before'].values());right=min(x['capture_started_unix_ns'] for x in record['after'].values());assert left<record['history_capture_ended_unix_ns']<=right
gets=[]
for pair in record['contained_successful_gets']:
 a,b=pair['invocation'],pair['completion'];assert indexed[(a['id'],'invoke')]==a and indexed[(b['id'],'return')]==b;assert a['op']=='get' and a['phase']=='delay' and b['outcome']=='ok';assert progress[-1]['monotonic_ns']<=a['monotonic_ns']<=b['monotonic_ns'];start=report['wall_anchor_unix_ns']+a['monotonic_ns']-report['wall_anchor_monotonic_ns'];end=report['wall_anchor_unix_ns']+b['monotonic_ns']-report['wall_anchor_monotonic_ns'];assert left<=start<=end<=right,(left,start,end,right);gets.append(dict(id=a['id'],start_unix_ns=start,end_unix_ns=end))
assert gets
positive=[]
for node,a in record['before'].items():
 b=record['after'].get(node)
 if not b or identity(a)!=identity(b):continue
 delta=int(b['state']['public_raw_get_completed_inline'])-int(a['state_after']['public_raw_get_completed_inline']);assert delta>=0
 if delta>0:positive.append(dict(node=node,inline_delta=delta,identity=identity(a)))
assert positive
for rel,h in p['source'].items():assert hashlib.sha256((Path(p['worktree'])/rel).read_bytes()).hexdigest()==h
result=dict(complete=True,revision=p['revision'],fault_uid=record['fault_uid'],fresh_server_boundaries=freshness,anchored_contained_successful_gets=gets,positive_inline_growth=positive,envelope_unix_ns=[left,right],fault_observations=len(faults),gate_wall_seconds=(record['ended_unix_ns']-record['started_unix_ns'])/1e9,source_hashes=p['source'],scope='Independent raw-probe reparse, exact writer identity, active-fault boundaries, two serial server freshness advances and two client progress advances, plus final complete-history wall-anchor containment. Positive inline activity remains envelope-scoped; no per-response routing attribution or criterion waiver.')
(r/'owned-delay-gate/independent-audit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(dict(complete=True,gets=len(gets),positive_nodes=[x['node'] for x in positive],gate_wall_seconds=result['gate_wall_seconds'])))
