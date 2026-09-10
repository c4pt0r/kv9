"""Synthetic schema/predicate controls, never actual current-candidate Chaos evidence."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse,copy,importlib.util,json,sys,traceback
from pathlib import Path
ROOT=Path(__file__).resolve().parent;sys.path.insert(0,str(ROOT))
from native_protocol import prefix,completed_batches,fault_check,require
from native_capture import parse_capture,PROBE
s=importlib.util.spec_from_file_location('native_check',ROOT/'check-native-windows.py');m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--output',type=Path,required=True);ap.add_argument('--artifact',type=Path,required=True);ap.add_argument('--plan',type=Path,required=True);options=ap.parse_args();OUT=options.output;OUT.mkdir(exist_ok=False)
plan=json.loads(options.plan.read_text());phase='delay';namespace='kv9-chaos-synthetic-preflight'
config=json.loads((options.artifact/'native-config.json').read_text());config['rpc_transport']='tonic_stream'
header=dict(type='header',version=2,range_chunk_size=1024,generator='kv9-native-batch-workload',configuration=config,initial=dict(keyspaces=[dict(name=config['keyspace_name'],id=config['client']['keyspace_id'])],kv=[]))
events=[]
for i,kind in enumerate(['batch_get','batch_put','batch_get','batch_put','batch_get','batch_put']):
 args={'keyspace':config['client']['keyspace_id'],('keys'if kind=='batch_get'else'pairs'):['61']if kind=='batch_get'else[['61','62']]}
 events.extend([dict(type='invoke',id=i,seq=2*i,monotonic_ns=(2*i+1)*100_000_000,op=kind,phase=phase,args=args),dict(type='return',id=i,seq=2*i+1,monotonic_ns=(2*i+2)*100_000_000,outcome='ok')])
fulltext=''.join(json.dumps(x)+'\n'for x in [header,*events]);full=prefix(fulltext,config);report=dict(wall_anchor_unix_ns=10**18,wall_anchor_monotonic_ns=0)
old=options.artifact;faults=json.loads((old/'delay-persistent-faults.json').read_text());fault=next(x for x in faults['items']if x['kind']=='NetworkChaos'and x['metadata']['name']=='delay-follower');faults={'items':[fault]};fault['metadata']['namespace']=namespace;fault['spec']['selector']['namespaces']=[namespace]
pod=json.loads((old/'persistent-client-pod.json').read_text());pod['metadata'].update(name='kv9-native-batch-client',namespace=namespace,labels={'app':'kv9-native-batch-client'});pod['spec']['containers'][0]['image']=plan['image'];pod['status']['containerStatuses'][0]['imageID']=plan['accepted_cri_image_ids'][0]
pid=12345;start=98765;boot='01234567-89ab-cdef-0123-456789abcdef';statfields=['S']+['0']*18+[str(start)];stat=f'{pid} (native) '+' '.join(statfields)+'\n';sha=plan['native_client_build']['binary_sha256']
def capture(history):
 parts={'READY':json.dumps(dict(version=1,process_id=pid,monotonic_ns=1))+'\n','BOOT_BEFORE':boot+'\n','STAT_BEFORE':stat,'HASH_BEFORE':f'{sha}  /usr/local/bin/kv9-batch-workload\n{sha}  /proc/{pid}/exe\n','CPU':'Cpus_allowed_list:\t6-31\n','CONFIG':json.dumps(config)+'\n','BUILD':json.dumps(plan['native_client_build'])+'\n','PHASE':phase+'\n','HISTORY':history+'\n','HASH_AFTER':f'{sha}  /proc/{pid}/exe\n','STAT_AFTER':stat,'BOOT_AFTER':boot+'\n','END':''}
 return ''.join('@@'+k+'@@\n'+v for k,v in parts.items())
folder=OUT/'delay';folder.mkdir();commands=[];clock=10**18
# Synthetic clock and process samples are explicitly parser fixtures.
def command(argv,stdout):
 global clock
 n=len(commands);row=dict(command=['kubectl','--kubeconfig',plan['kubeconfig'],'--request-timeout=8s',*argv],started_unix_ns=clock,ended_unix_ns=clock+1,stdout=f'command-{n:04}.stdout',stderr=f'command-{n:04}.stderr',exit_code=0);clock+=2;commands.append(row);(folder/row['stdout']).write_text(stdout);(folder/row['stderr']).write_text('');return n
first=command(['get','podchaos,networkchaos,iochaos','-n',namespace,'-o','json'],json.dumps(faults));captures=[]
for size in (2,4,12):
 text=''.join(json.dumps(x)+'\n'for x in [header,*events[:size]]);a=command(['get','pod','-n',namespace,'kv9-native-batch-client','-o','json'],json.dumps(pod));b=command(['exec','-n',namespace,'kv9-native-batch-client','--','/bin/bash','-c',PROBE,'native-probe',str(config['history_bytes']+1)],capture(text));c=command(['get','pod','-n',namespace,'kv9-native-batch-client','-o','json'],json.dumps(pod));row,_=parse_capture(capture(text),pod,pod,plan,config,phase);row.update(before_command=a,capture_command=b,after_command=c,started_unix_ns=commands[a]['started_unix_ns'],ended_unix_ns=commands[c]['ended_unix_ns']);captures.append(row)
clock=10**18+2*10**9;last=command(['get','podchaos,networkchaos,iochaos','-n',namespace,'-o','json'],json.dumps(faults));found=completed_batches(full,phase,3)
record=dict(accepted=True,phase=phase,namespace=namespace,timeout_seconds=25,started_unix_ns=10**18-1,ended_unix_ns=clock+1,captures=captures,initial_seq=1,barrier_seq=3,completed={k:v[0]for k,v in found.items()},window_start_unix_ns=commands[first]['ended_unix_ns'],window_end_unix_ns=commands[last]['started_unix_ns'],first_fault_command=first,last_fault_command=last,fault=fault_check(phase,faults))
(folder/'commands.json').write_text(json.dumps(commands,indent=2)+'\n');(folder/'record.json').write_text(json.dumps(record,indent=2)+'\n')
results=[]
active_control=None
retained=OUT/'mutants';retained.mkdir()
def triple(name,baseline,mutant,expected):
 global active_control
 active_control=name
 baseline()
 try:mutant()
 except (ValueError,KeyError,AssertionError)as e:
  require(expected in str(e),'unexpected rejection '+name+': '+str(e));results.append(dict(name=name,baseline='accepted',mutant='rejected',reason=str(e),restored='accepted'))
 else:raise ValueError('invalid native evidence accepted: '+name)
 baseline()
base=lambda:m.window(folder,plan,config,full,report)
def record_mutate(fn):
 original=(folder/'record.json').read_bytes();r=copy.deepcopy(record);fn(r);data=json.dumps(r);(retained/(active_control+'.record.json')).write_text(data);(folder/'record.json').write_text(data)
 try:return base()
 finally:(folder/'record.json').write_bytes(original)
def raw_mutate(index,fn):
 path=folder/commands[index]['stdout'];original=path.read_text();data=fn(original);(retained/(active_control+'.stdout')).write_text(data);path.write_text(data)
 try:return base()
 finally:path.write_text(original)
try:
 base()
 triple('missing-batch-get',base,lambda:record_mutate(lambda r:r['completed'].pop('batch_get')),'lacks both')
 triple('missing-batch-put',base,lambda:record_mutate(lambda r:r['completed'].pop('batch_put')),'lacks both')
 triple('stale-prefix-barrier',base,lambda:record_mutate(lambda r:r.update(barrier_seq=-1)),'barrier')
 triple('old-invocation',base,lambda:record_mutate(lambda r:r['completed'].update(batch_get=dict(invocation=events[0],completion=events[1]))),'post-barrier')
 triple('unknown-labeled-success',base,lambda:record_mutate(lambda r:r['completed']['batch_put']['completion'].update(outcome='unknown')),'post-barrier')
 shifted={**report,'wall_anchor_unix_ns':10**18-10**10};(retained/'call-outside-fault.report.json').write_text(json.dumps(shifted));triple('call-outside-fault',base,lambda:m.window(folder,plan,config,full,shifted),'outside actual fault')
 def fault_change(fn):
  def change(s):f=json.loads(s);fn(f['items'][0]);return json.dumps(f)
  return raw_mutate(last,change)
 triple('fault-target-changed',base,lambda:fault_change(lambda f:f['spec'].update(direction='from')),'identity changed')
 triple('fault-uid-replaced',base,lambda:fault_change(lambda f:f['metadata'].update(uid='replaced')),'identity changed')
 triple('fault-uninjected',base,lambda:fault_change(lambda f:f['status'].update(conditions=[dict(type='AllInjected',status='False')])),'injected Chaos')
 triple('fault-recovered',base,lambda:fault_change(lambda f:f['status']['conditions'].append(dict(type='AllRecovered',status='True'))),'already recovered')
 triple('fault-wrong-member',base,lambda:fault_change(lambda f:f['spec']['selector']['labelSelectors'].update({'kv9-node':'9'})),'intended database')
 triple('fault-wrong-namespace',base,lambda:fault_change(lambda f:f['metadata'].update(namespace='other')),'owned database namespace')
 idx=captures[-1]['capture_command']
 triple('native-executable-mismatch',base,lambda:raw_mutate(idx,lambda s:s.replace(sha,'0'*64)),'binary differs')
 triple('native-boot-mismatch',base,lambda:raw_mutate(idx,lambda s:s.replace('@@BOOT_AFTER@@\n'+boot,'@@BOOT_AFTER@@\n11111111-1111-1111-1111-111111111111')),'boot changed')
 triple('native-start-mismatch',base,lambda:raw_mutate(idx,lambda s:s.replace('@@STAT_AFTER@@\n'+stat,'@@STAT_AFTER@@\n'+stat.replace(str(start),str(start+1)))),'start changed')
 triple('native-config-mismatch',base,lambda:raw_mutate(idx,lambda s:s.replace('"max_attempts": 6','"max_attempts": 7',1)),'configuration/build differs')
 triple('native-phase-mismatch',base,lambda:raw_mutate(idx,lambda s:s.replace('@@PHASE@@\ndelay','@@PHASE@@\nhealing')),'phase mirror')
 triple('native-partial-prefix-relabel',base,lambda:raw_mutate(idx,lambda s:s.replace(json.dumps(events[-1])+'\n\n@@HASH_AFTER@@',json.dumps(events[-1])[:-2]+'\n@@HASH_AFTER@@')),'sequence differs')
 p=prefix(fulltext+'{"incomplete":',config);require(p['complete_text']==fulltext and p['partial_tail_bytes']>0,'partial final JSONL handling invalid')
 # Reapply unchanged fault predicates to all 21 historical resources under their
 # historical source identities; this is compatibility validation, not a rerun.
 historical=[]
 for phase in m.point.WINDOWS:
  fp=old/f'{phase}-persistent-faults.json';vp=old/f'{phase}-persistent-victim.json';f=fault_check(phase,json.loads(fp.read_text()),json.loads(vp.read_text())if vp.exists()else None);historical.append(f)
 result=dict(accepted=True,scope=__doc__,synthetic_controls=results,historical_predicate_compatibility=historical,actual_current_matrix_launched=False)
except BaseException as e:result=dict(accepted=False,scope=__doc__,completed_controls=results,failure=str(e),traceback=traceback.format_exc());raise
finally:(OUT/'summary.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(dict(accepted=True,synthetic_controls=len(results),historical_faults=len(historical))))
