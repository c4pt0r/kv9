"""Reparse retained process evidence and check async-apply lifecycle bounds/drain."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
from pathlib import Path
from collections import defaultdict
import importlib.util,json,hashlib,time
import argparse
ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--plan',required=True);ap.add_argument('--artifact',required=True);args=ap.parse_args()
P=Path(args.plan);p=json.loads(P.read_text());r=Path(args.artifact);live=json.loads((r/'async-write-live-observer.json').read_text());assert live['complete']
def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
probe=module('owned_probe',str(Path(__file__).parent/'process-probe.py'));fields=module('owned_fields',str(Path(__file__).parent/'observer-fields.py'))
for path,expected in p['overlay_inputs'].items():assert hashlib.sha256(Path(path).read_bytes()).hexdigest()==expected,path
lifetimes=defaultdict(list);rejections=[];accepted=0
for number,batch in enumerate(live['observations']):
 pods={x['metadata']['uid']:x for x in json.loads(batch['pods']['stdout'])['items']} if batch['pods']['exit_code']==0 else {}
 for row in batch['rows']:
  if not row.get('identity_matched'):
   rejections.append({'observation':number,'pod':row['pod'],'exit_code':row['exit_code'],'reason':row.get('identity_rejection')});continue
  assert row.get('status_contract_valid') is True,('observer preserved a status-contract failure',number,row['pod'])
  parsed=probe.parse(row['stdout'],p['production_binary']['sha256'],'/usr/local/bin/kv9');assert parsed['identity_matched']
  assert parsed['status_writer_identity_matched'] and parsed['process_boot_id']==row['process_boot_id'];assert parsed['process_pid']==row['process_pid'] and parsed['process_start_ticks']==row['process_start_ticks']
  validated=fields.validate(parsed,pods[row['pod_uid']],p['source_default_limits']);assert validated['async_apply_observations']==row['async_apply_observations']
  key=(row['node'],row['pod_uid'],row['process_pid'],row['process_start_ticks']);accepted+=1
  for state_name,when in [('state','before'),('state_after','after')]:
   state=parsed[state_name];requests=int(state['public_rpc_in_flight']);limit=int(state['public_rpc_limit_requests']);encoded=int(state['public_rpc_encoded_bytes']);byte_limit=int(state['public_rpc_limit_encoded_bytes']);assert 0<=requests<=limit and 0<=encoded<=byte_limit
   assert 0<=int(state['public_rpc_running'])<=requests and 0<=int(state['public_rpc_queued'])<=requests
   lifetimes[key].append(dict(observation=number,when=when,phase=row['phase_before'],**validated['async_apply_observations'][when]))
assert accepted>0 and {'1','2','3'}<={x[0] for x in lifetimes}
summary=[]
for key,rows in lifetimes.items():
 peaks=[x['peak'] for x in rows];assert peaks==sorted(peaks),('peak decreased across same lifetime',key)
 summary.append(dict(node=key[0],pod_uid=key[1],pid=key[2],start_ticks=key[3],observations=len(rows),max_in_flight=max(x['in_flight'] for x in rows),max_queued=max(x['queued'] for x in rows),max_peak=max(peaks),observed_stopped=any(x['stopped']=='true' for x in rows),phases=sorted({x['phase'] for x in rows})))
assert any(x['max_peak']>0 for x in summary),'no positive async-apply high-water evidence'
inline=json.loads((r/'async-write-inline-audit.json').read_text());assert inline['complete'];final=inline['final_drained_scene_states'];assert len(final)>=4
for row in final:fields.final_drained(row['state'])
result=dict(complete=True,revision=p['revision'],accepted_samples=accepted,rejected_samples=len(rejections),rejections=rejections,lifetimes=summary,final_drained_states=final,source_limit=p['source_default_limits']['raft_async_apply_limit'],scope='Independent reparse of the exact running executable/status evidence. Count/queue/high-water values and stopped flags are process-lifetime sampled observations; peak is not a cumulative admission count. Intermediate stopped states during intentional fatal I/O are retained. All exact matched samples satisfy bounds, peak never decreases within an attested lifetime, and final healed replicas are stopped=false and fully drained. This does not attribute waits to individual responses.')
(r/'async-apply-audit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:result[k] for k in ['complete','accepted_samples','rejected_samples','source_limit']}))
