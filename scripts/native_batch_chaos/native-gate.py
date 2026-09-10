"""Additive bounded native BatchGet/BatchPut history-prefix gate; no invented client clocks."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse,json,time,traceback
from pathlib import Path
from native_capture import Commands
from native_protocol import require,fault_check,completed_batches,state_writer,config_contract

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--plan',type=Path,required=True);ap.add_argument('--artifact',type=Path,required=True);ap.add_argument('--namespace',required=True);ap.add_argument('--phase',required=True);a=ap.parse_args()
 p=json.loads(a.plan.read_text());config=json.loads((a.artifact/'native-config.json').read_text());out=a.artifact/'native-windows'/a.phase;out.parent.mkdir(exist_ok=True)
 config_contract(config);c=Commands(out,p,25);r=dict(accepted=False,phase=a.phase,namespace=a.namespace,timeout_seconds=25,started_unix_ns=time.time_ns(),captures=[])
 require(a.namespace.startswith('kv9-chaos-') and a.namespace not in p['preserve_namespaces'],'not owned matrix namespace')
 victimpath=a.artifact/f'{a.phase}-persistent-victim.json';victim=json.loads(victimpath.read_text()) if victimpath.exists() else None
 if victim is not None:(out/'victim.json').write_text(json.dumps(victim,indent=2)+'\n')
 def fault():
  row,text=c.run('get','podchaos,networkchaos,iochaos','-n',a.namespace,'-o','json');f=fault_check(a.phase,json.loads(text),victim);require(f['namespace']==a.namespace,'fault namespace differs');return c.rows.index(row),row,f
 try:
  if a.phase!='baseline':
   index,row,f=fault();r.update(fault=f,first_fault_command=index,window_start_unix_ns=row['ended_unix_ns'])
  else:r['window_start_unix_ns']=time.time_ns()
  seed,parsed=c.capture(a.namespace,config,a.phase);r['captures'].append(seed);r['initial_seq']=parsed['last_seq'];identity=state_writer(seed);barrier=None
  while time.monotonic()<c.deadline:
   current,parsed=c.capture(a.namespace,config,a.phase);r['captures'].append(current);require(state_writer(current)==identity,'native client writer changed across window')
   require(parsed['last_seq']>=r['initial_seq'],'native prefix shrank')
   if barrier is None:
    if parsed['last_seq']>r['initial_seq']:barrier=parsed['last_seq'];r['barrier_seq']=barrier
   else:
    found=completed_batches(parsed,a.phase,barrier)
    if all(found.values()):
     r['completed']={kind:rows[0] for kind,rows in found.items()}
     if a.phase!='baseline':
      index,row,f=fault();require(f==r['fault'],'fault identity/action/selector changed');r.update(last_fault_command=index,window_end_unix_ns=row['started_unix_ns'])
     else:r['window_end_unix_ns']=time.time_ns()
     r['accepted']=True;break
   time.sleep(min(.2,max(0,c.deadline-time.monotonic())))
  require(r['accepted'],'native window lacks new fully completed BatchGet and BatchPut before fixed deadline')
 except BaseException as e:r['failure']=str(e);r['traceback']=traceback.format_exc();raise
 finally:
  r['ended_unix_ns']=time.time_ns();(out/'record.json').write_text(json.dumps(r,indent=2)+'\n')
 print('PASS: additive native complete BatchGet and BatchPut retained during '+a.phase)
if __name__=='__main__':main()
