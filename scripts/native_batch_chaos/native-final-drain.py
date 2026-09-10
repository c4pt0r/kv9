"""Capture two fresh status publications per final replica before fixture cleanup."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse,importlib.util,json,time,traceback
from pathlib import Path
from native_capture import Commands
from native_protocol import require,process_identity,state_writer
ROOT=Path(__file__).parent

def module(name,path):
 s=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(s);s.loader.exec_module(m);return m
probe=module('drain_probe',ROOT/'process-probe.py');fields=module('drain_fields',ROOT/'observer-fields.py')
ZERO=('public_rpc_in_flight','public_rpc_queued','public_rpc_running','public_rpc_encoded_bytes','raft_async_apply_queued','raft_async_apply_in_flight','raft_async_read_queued','raft_async_read_active','raft_async_read_in_flight','raft_async_read_active_groups')
def drained(state):
 return state.get('bootstrap_state')=='Serving' and state.get('fatal')=='' and all(state.get(k)=='0' for k in ZERO) and state.get('raft_async_apply_stopped')==state.get('raft_async_read_stopped')=='false'
def main():
 ap=argparse.ArgumentParser();ap.add_argument('--plan',type=Path,required=True);ap.add_argument('--artifact',type=Path,required=True);ap.add_argument('--namespace',required=True);a=ap.parse_args();p=json.loads(a.plan.read_text());c=Commands(a.artifact/'native-final-drain',p,20)
 r=dict(accepted=False,started_unix_ns=time.time_ns(),timeout_seconds=20,namespace=a.namespace,samples=[])
 def sample():
  first,text=c.run('get','pods','-n',a.namespace,'-o','json');pods=json.loads(text)['items'];require(not any(x['metadata']['name'] in ('kv9-native-batch-client','kv9-persistent-client') for x in pods),'collector still present at final drain');rows={};selected={}
  for pod in pods:
   node=pod['metadata'].get('labels',{}).get('kv9-node')
   if node not in ('1','2','3','4') or pod['metadata']['labels'].get('app')!='kv9':continue
   require(node not in rows and pod['status']['phase']=='Running' and all('running'in x['state'] for x in pod['status']['containerStatuses']),'final replica absent/ambiguous/not running')
   require(all(x['imageID']in p['accepted_cri_image_ids'] for x in pod['status']['containerStatuses']),'final replica image differs')
   cmd,text=c.run('exec','-n',a.namespace,pod['metadata']['name'],'--','/bin/bash','-c',probe.PROBE,'native-drain','/data/status','/usr/local/bin/kv9');row=probe.parse(text,p['production_binary']['sha256'],'/usr/local/bin/kv9');fields.validate(row,pod,p['source_default_limits']);row.update(pod_uid=pod['metadata']['uid'],pod=pod['metadata']['name'],probe_command=c.rows.index(cmd));rows[node]=row;selected[node]=pod
  require(set(rows)==set('1234'),'final drain lacks all four registered replicas')
  last,text=c.run('get','pods','-n',a.namespace,'-o','json');after={x['metadata']['name']:x for x in json.loads(text)['items']}
  for node,pod in selected.items():require(process_identity(pod)==process_identity(after[pod['metadata']['name']]),'final replica container lifetime changed during sample')
  return dict(before_command=c.rows.index(first),after_command=c.rows.index(last),rows=rows)
 try:
  base=sample();r['samples'].append(base);ids={k:state_writer(v)for k,v in base['rows'].items()};last={k:int(v['state_after']['metrics_export_successes'])for k,v in base['rows'].items()};advances={k:0 for k in ids}
  while time.monotonic()<c.deadline:
   current=sample();r['samples'].append(current)
   for node,row in current['rows'].items():
    require(state_writer(row)==ids[node],'final drain writer lifetime changed');before=int(row['state']['metrics_export_successes']);after=int(row['state_after']['metrics_export_successes']);require(last[node]<=before<=after,'final drain publication counter decreased')
    if before>last[node]:advances[node]+=1;last[node]=after
   current['advances']=dict(advances)
   if all(v>=2 for v in advances.values()) and all(drained(row[key])for row in current['rows'].values()for key in ('state','state_after')):r['accepted']=True;break
   time.sleep(min(.2,max(0,c.deadline-time.monotonic())))
  require(r['accepted'],'final replicas did not publish two fresh drained statuses within 20 seconds')
 except BaseException as e:r['failure']=str(e);r['traceback']=traceback.format_exc();raise
 finally:r['ended_unix_ns']=time.time_ns();(c.out/'record.json').write_text(json.dumps(r,indent=2)+'\n')
 print('PASS: all four final replica lifetimes published two fresh drained statuses before cleanup')
if __name__=='__main__':main()
