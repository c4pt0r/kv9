#!/usr/bin/env python3
"""Exercise success, command failure, deadline escalation and log-retention failures."""
import argparse
import json
from pathlib import Path
import subprocess
import time

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--output',type=Path,required=True)
a=p.parse_args();a.output.mkdir(parents=True,exist_ok=False)
wrapper=Path(__file__).resolve().parent/'run-bounded-log.sh'
cases=[('success','2s','printf "out\\n"; printf "err\\n" >&2',0),
       ('command-failure','2s','printf "before failure\\n"; exit 7',7),
       ('deadline','0.2s','printf "before deadline\\n"; sleep 30',124),
       ('kill-escalation','0.2s','trap "" TERM; printf "before kill\\n"; sleep 30',137)]
results=[]
for name,limit,command,expected in cases:
    log=a.output/(name+'.log');start=time.monotonic()
    r=subprocess.run(['bash',str(wrapper),str(log),limit,'0.2s','bash','-c',command],capture_output=True,text=True,timeout=5)
    elapsed=time.monotonic()-start
    if r.returncode!=expected or elapsed>=5: raise ValueError(name+': command status/deadline was not preserved')
    text=log.read_text()
    if not text.endswith(f'COMMAND_EXIT={expected} LOG_EXIT=0\n'): raise ValueError(name+': missing final status')
    if name=='success' and not text.startswith('out\nerr\n'): raise ValueError('stdout/stderr were not retained')
    if name!='success' and 'before ' not in text: raise ValueError('progress before failure was lost')
    if 'before ' not in r.stdout and name!='success': raise ValueError('progress was not streamed')
    results.append(dict(name=name,exit_code=r.returncode,elapsed_seconds=elapsed))
log=a.output/'missing-parent/log'
r=subprocess.run(['bash',str(wrapper),str(log),'2s','0.2s','true'],capture_output=True,text=True,timeout=5)
if r.returncode!=1: raise ValueError('logger failure was accepted')
results.append(dict(name='logger-failure',exit_code=r.returncode))
log=a.output/'existing.log';log.write_text('original evidence\n')
r=subprocess.run(['bash',str(wrapper),str(log),'2s','0.2s','bash','-c','touch "$1"','existing-log-control',str(a.output/'unexpected-command')],capture_output=True,text=True,timeout=5)
if r.returncode!=64 or log.read_text()!='original evidence\n' or (a.output/'unexpected-command').exists():
    raise ValueError('existing evidence was overwritten or refused command ran')
results.append(dict(name='existing-log',exit_code=r.returncode))
(a.output/'results.json').write_text(json.dumps(results,indent=2)+'\n')
print('PASS: 6 bounded logging controls verified',flush=True)
