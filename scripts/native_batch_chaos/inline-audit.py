"""Check per-lifecycle inline growth inside actual fault observation envelopes."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
from pathlib import Path
from collections import defaultdict
import json,hashlib,time
import argparse
ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--plan',required=True);ap.add_argument('--artifact',required=True);args=ap.parse_args()
R=Path(args.artifact)
def read(p):return json.loads(Path(p).read_text())
plan=read(args.plan);defaults=plan['source_default_limits'];
for item in defaults['sources']:assert hashlib.sha256(Path(item['path']).read_bytes()).hexdigest()==item['sha256']
live=read(R/'async-write-live-observer.json');assert live['complete'];report=read(R/'persistent-run/report.json');events=[json.loads(line) for line in (R/'persistent-run/history.jsonl').read_text().splitlines()];calls={x['id']:x for x in events if x.get('type')=='invoke'}
def wall(event):return report['wall_anchor_unix_ns']+event['monotonic_ns']-report['wall_anchor_monotonic_ns']
gets=[dict(id=x['id'],phase=calls[x['id']]['phase'],start=wall(calls[x['id']]),end=wall(x)) for x in events if x.get('type')=='return' and x['outcome']=='ok' and calls[x['id']]['op']=='get']
def target(phase):
 if phase.startswith('pod-failure'):return 'PodChaos','fail-leader'
 if phase.startswith('io-voter'):return 'IOChaos','raft-io-fault'
 if phase.startswith('store-loss-voter'):return 'PodChaos','store-loss-kill'
 if phase.startswith('endpoint-migration'):return 'PodChaos','endpoint-migration-kill'
 return {'partition':('NetworkChaos','isolate-leader'),'public-admission-overload':('NetworkChaos','isolate-leader'),'delay':('NetworkChaos','delay-follower'),'registration-seed-blackhole':('NetworkChaos','registration-seed-blackhole')}.get(phase)
def fault(sample,kind_name):
 if sample['exit_code']!=0 or kind_name is None:return None
 found=[x for x in json.loads(sample['stdout'])['items'] if (x['kind'],x['metadata']['name'])==kind_name and not x['metadata'].get('deletionTimestamp') and any(c['type']=='AllInjected' and c['status']=='True' for c in x.get('status',{}).get('conditions',[]))]
 if len(found)!=1:return None
 f=found[0];assert f['metadata']['namespace']==live['namespace'] and f['spec']['selector']['namespaces']==[live['namespace']]
 return f['metadata']['uid']
buckets=defaultdict(list);matched=[];errors=0
for i,obs in enumerate(live['observations']):
 phase=obs['phase_before'];kind_name=target(phase)
 if obs['phase_after']!=phase:continue
 uid=fault(obs['faults_before'],kind_name)
 if uid is None or fault(obs['faults_after'],kind_name)!=uid:continue
 for row in obs['rows']:
  if not row.get('identity_matched'):errors+=1;continue
  if row['phase_before']!=phase or row['phase_after']!=phase:continue
  pods=json.loads(obs['pods']['stdout'])['items'];pod=next(p for p in pods if p['metadata']['uid']==row['pod_uid'])
  env={v['name']:v.get('value') for c in pod['spec']['containers'] for v in c.get('env',[])}
  expected_requests=env.get('KV9_PUBLIC_MAX_REQUESTS',str(defaults['public_rpc_limit_requests']));expected_bytes=env.get('KV9_PUBLIC_MAX_ENCODED_BYTES',str(defaults['public_rpc_limit_encoded_bytes']))
  assert row['state']['pid']==row['state_after']['pid']==str(row['process_pid']) and row['state']['public_rpc_limit_requests']==expected_requests and row['state']['public_rpc_limit_encoded_bytes']==expected_bytes and row['state']['raft_async_read_limit']==str(defaults['raft_async_read_limit'])
  key=(phase,uid,row['node'],row['pod_uid'],row['process_pid'],row['process_start_ticks'])
  buckets[key].append(dict(observation=i,started=row['started_unix_ns'],ended=row['ended_unix_ns'],inline=int(row['state']['public_raw_get_completed_inline']),blocking=int(row['state']['public_raw_get_blocking_submitted']),state=row['state']))
for key,rows in buckets.items():
 if len(rows)<2:continue
 a,b=rows[0],rows[-1]
 if b['inline']<=a['inline'] or b['blocking']<a['blocking']:continue
 contained=[x for x in gets if x['phase']==key[0] and a['started']<=x['start'] and x['end']<=b['ended']]
 if not contained:continue
 matched.append(dict(phase=key[0],fault_uid=key[1],node=key[2],pod_uid=key[3],process_pid=key[4],process_start_ticks=key[5],first_observation=a['observation'],last_observation=b['observation'],envelope_unix_ns=[a['started'],b['ended']],inline_before=a['inline'],inline_after=b['inline'],inline_delta=b['inline']-a['inline'],blocking_delta=b['blocking']-a['blocking'],contained_successful_persistent_get_ids=[x['id'] for x in contained]))
phases=sorted({x['phase'] for x in matched})
for required in ['partition','delay','endpoint-migration-recovered']:assert required in phases,('required injected/recovered inline envelope missing',required,phases)
# All observed process/image identities stay exact; counter envelopes above never
# cross a Pod UID or namespace-PID start-tick boundary. Historical status files
# are sampled observations, not a claim of one inline count per named response.
identities={};masks=set();last_counters={};phase_identity_nodes=defaultdict(set)
for obs in live['observations']:
 for row in obs['rows']:
  if not row.get('identity_matched'):continue
  pid=row['process_pid'];expected=live['expected_server_sha256']
  assert row['status_pid_before']==row['status_pid_after']==pid
  assert row['state']['pid']==row['state_after']['pid']==str(pid)
  assert row['executable_sha256_before']==row['executable_sha256_after']==expected
  assert expected+f'  /proc/{pid}/exe' in row['stdout']
  def section(name):return row['stdout'].split('@@'+name+'@@\n',1)[1].split('@@',1)[0].strip()
  for name in ['STAT_BEFORE','STAT_AFTER']:
   stat=section(name);assert int(stat.split(' ',1)[0])==pid
   fields=stat.rsplit(')',1)[1].split();assert int(fields[19])==row['process_start_ticks'] and fields[0] not in ['Z','X','x']
  assert row['pod_after']['metadata']['uid']==row['pod_uid']
  assert row['pod_after']['metadata']['name']==row['pod']
  def cid(states):return sorted((x['name'],x.get('containerID'),x.get('imageID'),x.get('restartCount')) for x in states)
  assert cid(row['pod_after']['status']['containerStatuses'])==cid(row['container_statuses'])
  masks.add(row['cpu_allowed'])
  key=(row['node'],row['pod_uid'],row['process_pid'],row['process_start_ticks'])
  counters={name:int(row['state'][name]) for name in ['public_raw_get_completed_inline','public_raw_get_blocking_submitted','raft_async_read_admitted_members','raft_async_read_admitted_groups'] if name in row['state']}
  if key in last_counters:
   for name,value in counters.items():assert value>=last_counters[key][name],('non-monotonic runtime counter',key,name)
  last_counters[key]=counters
  if row['phase_before']==row['phase_after']:phase_identity_nodes[row['phase_before']].add(row['node'])
  identities[key]=dict(node=key[0],pod_uid=key[1],process_pid=key[2],process_start_ticks=key[3],container_statuses=row['container_statuses'],wrapper_pidfiles=row['wrapper_pidfiles'])
# The fixture's final scene is taken after stopping both clients and before
# namespace deletion. Require all four registered replicas' final ledgers.
assert any(x['process_pid']!=1 for x in identities.values()),'no wrapped runtime child observed'
scene_states=[]
for directory in sorted(R.glob('scene.*')):
 for path in directory.glob('*.status'):
  text=path.read_text();s=dict(line.split('=',1) for line in text.splitlines() if '=' in line)
  if 'public_raw_get_completed_inline' in s and s.get('bootstrap_state')=='Serving':scene_states.append(dict(path=str(path),state=s))
assert len(scene_states)>=4
for row in scene_states:
 s=row['state'];assert not s['fatal'] and s['raft_async_read_stopped']=='false'
 for key in ['public_rpc_in_flight','public_rpc_running','public_rpc_queued','public_rpc_encoded_bytes','raft_async_read_queued','raft_async_read_active','raft_async_read_active_groups','raft_async_read_in_flight']:assert s[key]=='0',(row['path'],key,s[key])
result=dict(complete=True,revision=plan['revision'],required_inline_phases=['partition','delay','endpoint-migration-recovered'],positive_phases=phases,envelopes=matched,observed_runtime_lifecycles=list(identities.values()),actual_cpu_masks=sorted(masks),matched_nodes_by_phase={p:sorted(nodes) for p,nodes in phase_identity_nodes.items()},counter_monotonicity_checked=True,final_drained_scene_states=scene_states,scope='Positive resident completed-inline growth inside sampled same-lifecycle envelopes with the exact owned Chaos resource AllInjected at both boundary observations, and independently checked persistent GET completions contained in that envelope. Counts include every public client and export/sampling timing; they are not per-response attribution or a new pre-deposition claim. Full 21-window acceptance and effects are independently checked separately.')
(R/'async-write-inline-audit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(dict(complete=True,phases=phases,envelopes=len(matched),lifecycles=len(identities),final_drained_snapshots=len(scene_states))))
