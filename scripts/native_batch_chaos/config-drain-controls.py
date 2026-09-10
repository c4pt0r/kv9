"""Exact generated native configuration and final-drain predicate controls."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse,copy,hashlib,importlib.util,json,re,subprocess,sys,traceback
from pathlib import Path
R=Path(__file__).resolve().parent;sys.path.insert(0,str(R))
from native_protocol import config_contract,require
s=importlib.util.spec_from_file_location('drain',R/'native-final-drain.py');d=importlib.util.module_from_spec(s);s.loader.exec_module(d)
sys.path.insert(0,str(R.parent));from batch_workload_report import config_check
ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--output',type=Path,required=True);args=ap.parse_args();out=args.output;out.mkdir(exist_ok=False);script=(R/'fixture.sh').read_text();code=script.split("<<'PY'\n",1)[1].split('\nPY',1)[0]
r=subprocess.run(['python3','-','native-batch-1789070000-1234','102','10.96.1.1:20160','10.96.1.2:20160','10.96.1.3:20160'],input=code,text=True,capture_output=True);require(r.returncode==0,'shell configuration generator failed');config=json.loads(r.stdout);(out/'generated-config.json').write_text(r.stdout);config_contract(config);config_check(config)
state=dict(bootstrap_state='Serving',fatal='',raft_async_apply_stopped='false',raft_async_read_stopped='false',**{k:'0'for k in d.ZERO});require(d.drained(state),'baseline drain refused');rows=[]
for key,value in [('rpc_transport','tonic_unary'),('workers',5),('batch_size',1),('max_calls',7000),('history_bytes',134217728),('interval_ms',0),('run_id','unbound')]:
 mutant=copy.deepcopy(config);mutant[key]=value;(out/f'config-{key}.json').write_text(json.dumps(mutant))
 try:config_contract(mutant)
 except ValueError as e:rows.append(dict(control='config-'+key,rejected=str(e)))
 else:raise ValueError('changed native matrix config accepted')
 config_contract(config)
for key,value in [('max_attempts',7),('retry_backoff_ms',0),('max_in_flight',8)]:
 mutant=copy.deepcopy(config);mutant['client'][key]=value;(out/f'client-{key}.json').write_text(json.dumps(mutant))
 try:config_contract(mutant)
 except ValueError as e:rows.append(dict(control='client-'+key,rejected=str(e)))
 else:raise ValueError('changed native retries/limits accepted')
 config_contract(config)
for key,value in [('bootstrap_state','Pending'),('fatal','fatal'),('raft_async_apply_stopped','true'),('raft_async_read_stopped','true'),*[(k,'1')for k in d.ZERO]]:
 mutant={**state,key:value};(out/f'drain-{key}.json').write_text(json.dumps(mutant));require(not d.drained(mutant),'bad final drain accepted: '+key);require(d.drained(state),'restored final drain refused');rows.append(dict(control='drain-'+key,rejected=True))
(out/'summary.json').write_text(json.dumps(dict(accepted=True,scope=__doc__,controls=rows,configuration_generator_sha256=hashlib.sha256(code.encode()).hexdigest()),indent=2)+'\n');print(json.dumps(dict(accepted=True,controls=len(rows))))
