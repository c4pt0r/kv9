"""Independent readback of atomic native histories, 21 exact fault windows and final drains."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse,collections,datetime,hashlib,importlib.util,json,re,sys,traceback
from pathlib import Path
from native_protocol import require,prefix,completed_batches,fault_check,state_writer,process_identity,config_contract
from native_capture import parse_capture,PROBE
ROOT=Path(__file__).resolve().parent
SCRIPTS=ROOT.parent
sys.path.insert(0,str(SCRIPTS))
from batch_workload_report import validate
from history.checker import load,check,verify_witness,coverage

def module(name,path):
 s=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(s);s.loader.exec_module(m);return m
point=module('native_point_contract',SCRIPTS/'check-persistent-chaos.py')
drain=module('native_drain_contract',ROOT/'native-final-drain.py')

def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def obj(p):return json.loads(Path(p).read_text())
def iso_ns(path):
 text=Path(path).read_text().strip();m=re.fullmatch(r'(.*T\d\d:\d\d:\d\d)(?:[.,](\d{1,9}))?([+-]\d\d:\d\d|Z)',text);require(m is not None,'physical observation timestamp malformed')
 dt=datetime.datetime.fromisoformat(m[1]+m[3].replace('Z','+00:00'));return int(dt.timestamp())*10**9+int((m[2]or'').ljust(9,'0'))
def commands(folder):
 rows=obj(folder/'commands.json');last=0
 for row in rows:
  require(row['exit_code']==0 and 0<row['started_unix_ns']<=row['ended_unix_ns'] and row['started_unix_ns']>=last,'observation command failed/reordered');last=row['ended_unix_ns'];require((folder/row['stdout']).is_file() and (folder/row['stderr']).is_file(),'raw command output missing')
 return rows

def window(folder,plan,config,full,report):
 r=obj(folder/'record.json');cmds=commands(folder);phase=r['phase'];require(r['accepted']is True and r['timeout_seconds']==25 and 0<r['ended_unix_ns']-r['started_unix_ns']<=26*10**9,'native window failed or exceeded fixed bound')
 require(folder.name==phase and r['namespace'].startswith('kv9-chaos-') and r['namespace']not in plan['preserve_namespaces'],'native window namespace differs')
 require(r['started_unix_ns']<=cmds[0]['started_unix_ns']<=cmds[-1]['ended_unix_ns']<=r['ended_unix_ns'],'native command envelope differs')
 def text(i):return (folder/cmds[i]['stdout']).read_text()
 previous=None;identity=None;parsed=[]
 for capture in r['captures']:
  a,b,c=[capture[k] for k in ('before_command','capture_command','after_command')];require(a+1==b and b+1==c,'native capture commands not consecutive')
  require(cmds[b]['command']==['kubectl','--kubeconfig',plan['kubeconfig'],'--request-timeout=8s','exec','-n',r['namespace'],'kv9-native-batch-client','--','/bin/bash','-c',PROBE,'native-probe',str(config['history_bytes']+1)],'native capture command/source differs')
  one,data=parse_capture(text(b),json.loads(text(a)),json.loads(text(c)),plan,config,phase)
  require(all(one[k]==capture[k]for k in one),'native capture derived identity/sequence differs');require(capture['started_unix_ns']==cmds[a]['started_unix_ns'] and capture['ended_unix_ns']==cmds[c]['ended_unix_ns'],'native capture timestamps differ')
  require(full['complete_text'].startswith(data['complete_text']),'native live prefix is not exact final history prefix')
  if previous is not None:require(data['complete_text'].startswith(previous),'native live prefix regressed')
  previous=data['complete_text'];parsed.append(data)
  if identity is None:identity=state_writer(one)
  require(identity==state_writer(one),'native process lifetime changed in window')
 require(len(parsed)>=3 and r['initial_seq']==parsed[0]['last_seq'],'native seed sequence differs')
 barrier=next((x['last_seq']for x in parsed[1:]if x['last_seq']>r['initial_seq']),None);require(barrier is not None and r['barrier_seq']==barrier,'native serial prefix barrier missing')
 choices=completed_batches(parsed[-1],phase,barrier);require(set(r['completed'])=={'batch_get','batch_put'},'native window lacks both batch operations')
 start,end=r['window_start_unix_ns'],r['window_end_unix_ns'];require(start<end,'native fault time envelope empty')
 if phase!='baseline':
  victim=obj(folder/'victim.json') if (folder/'victim.json').exists()else None
  f=[]
  for key in ('first_fault_command','last_fault_command'):
   i=r[key];require(cmds[i]['command']==['kubectl','--kubeconfig',plan['kubeconfig'],'--request-timeout=8s','get','podchaos,networkchaos,iochaos','-n',r['namespace'],'-o','json'],'native fault query differs');f.append(fault_check(phase,json.loads(text(i)),victim))
  require(f[0]==f[1]==r['fault'] and f[0]['namespace']==r['namespace'],'native exact fault identity changed')
  require(start==cmds[r['first_fault_command']]['ended_unix_ns'] and end==cmds[r['last_fault_command']]['started_unix_ns'],'native fault time cut differs')
 bounds={}
 for kind,pair in r['completed'].items():
  require(pair in choices[kind],'native selected batch lacks complete successful post-barrier evidence')
  call,done=pair['invocation'],pair['completion'];require(full['calls'][call['id']]==call and full['returns'][call['id']]==done,'native selected batch differs from final history')
  wall=lambda x:report['wall_anchor_unix_ns']+x['monotonic_ns']-report['wall_anchor_monotonic_ns']
  lo,hi=wall(call),wall(done);require(start<lo<=hi<end,'native whole batch call lies outside actual fault envelope');bounds[kind]=dict(id=call['id'],invoked_unix_ns=lo,returned_unix_ns=hi,items=len(call['args']['keys' if kind=='batch_get' else'pairs']))
 return dict(phase=phase,namespace=r['namespace'],writer=identity,started_unix_ns=start,ended_unix_ns=end,batches=bounds,fault=r.get('fault'))

def final_drain(folder,plan):
 r=obj(folder/'record.json');cmds=commands(folder);require(r['started_unix_ns']<=cmds[0]['started_unix_ns']<=cmds[-1]['ended_unix_ns']<=r['ended_unix_ns'],'final drain command envelope differs');require(r['accepted']is True and r['timeout_seconds']==20 and r['ended_unix_ns']-r['started_unix_ns']<=21*10**9,'final drain failed or exceeded bound');ids={};last={};advances={};lastrows={}
 for n,sample in enumerate(r['samples']):
  first=obj(folder/cmds[sample['before_command']]['stdout'])['items'];lastpods=obj(folder/cmds[sample['after_command']]['stdout'])['items'];before={p['metadata']['name']:p for p in first};after={p['metadata']['name']:p for p in lastpods};require(not any(x in before or x in after for x in ('kv9-native-batch-client','kv9-persistent-client')),'collector still present at final drain');require(set(sample['rows'])==set('1234'),'final drain coverage incomplete')
  for node,row in sample['rows'].items():
   pod=before[row['pod']];require(pod['metadata']['namespace']==r['namespace'] and pod['metadata']['labels']['kv9-node']==node and pod['metadata']['labels']['app']=='kv9','final drain node identity differs');require(process_identity(pod)==process_identity(after[row['pod']]),'final drain container identity changed')
   require(pod['status']['phase']=='Running' and all('running'in x['state'] and x['imageID']in plan['accepted_cri_image_ids']for x in pod['status']['containerStatuses']),'final drain image/process not running')
   require(sample['before_command']<row['probe_command']<sample['after_command'],'final drain sample command order differs');cmd=cmds[row['probe_command']];require(cmd['command'][-6:]==['--','/bin/bash','-c',drain.probe.PROBE,'native-drain','/data/status','/usr/local/bin/kv9'][-6:],'final drain probe source differs');parsed=drain.probe.parse((folder/cmd['stdout']).read_text(),plan['production_binary']['sha256'],'/usr/local/bin/kv9');drain.fields.validate(parsed,pod,plan['source_default_limits']);require(all(parsed[k]==row[k]for k in parsed),'final drain raw status differs');require(row['pod_uid']==pod['metadata']['uid'],'final drain Pod UID differs')
   ident=state_writer(row);a=int(parsed['state']['metrics_export_successes']);b=int(parsed['state_after']['metrics_export_successes'])
   if n==0:ids[node]=ident;last[node]=b;advances[node]=0
   else:
    require(ident==ids[node] and last[node]<=a<=b,'final drain publication/writer regressed')
    if a>last[node]:advances[node]+=1;last[node]=b
   lastrows[node]=parsed
  if n>0:require(sample['advances']==advances,'final drain advance counts differ')
 require(all(v>=2 for v in advances.values()) and all(drain.drained(x[k])for x in lastrows.values()for k in ('state','state_after')),'final drain lacks two fresh empty Serving publications')
 return dict(accepted=True,writers=ids,advances=advances,started_unix_ns=r['started_unix_ns'],ended_unix_ns=r['ended_unix_ns'])

def audit(raw,plan):
 for filename,expected in plan['overlay_inputs'].items():require(sha(filename)==expected,'overlay helper/source changed')
 for filename,expected in plan['source'].items():require(sha(Path(plan['worktree'])/filename)==expected,'frozen source changed')
 config=obj(raw/'native-config.json');config_contract(config);report=obj(raw/'native-run/report.json');require(config==report['configuration'] and config['rpc_transport']=='tonic_stream','native config/transport differs')
 require(report['complete']is True and report['failure']is None and report['stop']['reason']=='stop_file','native full history ended at a cap/failure')
 require((raw/'native-exit.txt').read_text().strip()=='0','native workload failed')
 require(report['build']==plan['native_client_build'],'native source/default-feature identity differs')
 checked=validate(raw/'native-run',raw/'native-build',plan['revision'],60)
 history=load(raw/'native-run/history.jsonl');require(verify_witness(history,checked['history']['witness']),'atomic native history witness does not replay');full=prefix((raw/'native-run/history.jsonl').read_text(),config);require(len(full['calls'])==len(full['returns']) and full['partial_tail_bytes']==0,'native full history truncated')
 windows=[window(raw/'native-windows'/phase,plan,config,full,report) for phase in ['baseline',*point.WINDOWS]];require(set(p.name for p in (raw/'native-windows').iterdir())==set(['baseline',*point.WINDOWS]),'native windows missing/extra');require(len({tuple(w['writer'])for w in windows})==1,'native client restarted across matrix');require(report['process_id']==windows[0]['writer'][1] and report['process_id']==obj(raw/'native-run/ready.json')['process_id'],'native final report PID differs')
 for w in windows[1:]:
  phase=w['phase'];lower=upper=None
  if phase.startswith('store-loss-voter'):
   lower=iso_ns(raw/phase/'rejected-2/at.txt');upper=iso_ns(raw/phase/'rejected-during-history/at.txt')
  if phase.startswith('endpoint-migration'):
   side=phase.removeprefix('endpoint-migration-');lower=iso_ns(raw/'endpoint-migration'/f'{side}-before-history-probed-at.txt');upper=iso_ns(raw/'endpoint-migration'/f'{side}-after-history-probed-at.txt')
  if lower is not None:require(lower<w['started_unix_ns']<w['ended_unix_ns']<upper,'native work lies outside retained original physical-refusal/endpoint bracket');w['physical_effect_bracket']=dict(started_unix_ns=lower,ended_unix_ns=upper)
 # Original validators remain separately executed, retaining their full results.
 for name in ('formation-audit.json','history-checker.json','persistent-history-checker.json','store-loss-audit.json','store-replacement-audit.json','endpoint-migration-audit.json'):
  require((raw/name).is_file(),'original acceptance output missing: '+name)
  original=obj(raw/name);require(original.get('accepted')is True or original.get('verdict')in ('valid','accepted'),'original acceptance output did not pass: '+name)
 point_record=obj(raw/'persistent-history-checker.json');require(point_record['build']==plan['correctness_client_build'] and len(point_record['windows'])==21,'original point identity/windows differ')
 exited=dict(line.split('=',1)for line in (raw/'native-process-exit.txt').read_text().splitlines());require(exited==dict(process_pid=str(windows[0]['writer'][1]),process_boot_id=windows[0]['writer'][3],process_absent='true'),'native process exit identity differs')
 initial=obj(raw/'native-client-pod.json');final=obj(raw/'native-client-final-pod.json');require(process_identity(initial)==process_identity(final) and initial['metadata']['uid']==windows[0]['writer'][0],'native collector Pod/container replaced');require(not any(p['metadata']['name']=='kv9-native-batch-client'for p in obj(raw/'native-after-collector-pods.json')['items']),'native collector not removed')
 drained=final_drain(raw/'native-final-drain',plan);require(drained['started_unix_ns']>report['wall_anchor_unix_ns']+report['elapsed_ns']-report['wall_anchor_monotonic_ns'],'final drain precedes native completion')
 outcomes=collections.Counter(x['outcome']for x in full['returns'].values());kinds=collections.Counter(x['op']for x in full['calls'].values());batch_items={kind:sum(len(c['args']['keys'if kind=='batch_get'else'pairs'])for c in full['calls'].values()if c['op']==kind)for kind in ('batch_get','batch_put')}
 return dict(accepted=True,revision=plan['revision'],scope='Original 21 voter/storage/endpoint Chaos windows plus independent normal-port streaming native atomic batch history. Dedicated client-link and quorum-loss phases are separate; no throughput or latency claim.',config_sha256=sha(raw/'native-config.json'),history_sha256=sha(raw/'native-run/history.jsonl'),report_sha256=sha(raw/'native-run/report.json'),native_build=report['build'],calls=len(full['calls']),outcomes=dict(outcomes),operations=dict(kinds),batch_items=batch_items,checked=checked,windows=windows,final_drain=drained)

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--plan',type=Path,required=True);ap.add_argument('--artifact',type=Path,required=True);ap.add_argument('--output',type=Path);a=ap.parse_args();output=a.output or a.artifact/'native-window-audit.json';require(not output.exists(),'native audit output already exists')
 try:result=audit(a.artifact,obj(a.plan))
 except BaseException as e:output.write_text(json.dumps(dict(accepted=False,failure=str(e),traceback=traceback.format_exc()),indent=2)+'\n');raise
 output.write_text(json.dumps(result,indent=2)+'\n');print('PASS: independent native atomic history and all 21 actual fault windows, with final fresh drains')
if __name__=='__main__':main()
