import hashlib,json,os,signal,subprocess,time
from pathlib import Path
root=Path('/tmp/kv9-quorum-trace-source-root-first');script=root/'build-release.py'
cmd=['taskset','-c','6-15,22-31','env','PYTHONDONTWRITEBYTECODE=1','PYTHONOPTIMIZE=0','CARGO_TARGET_DIR=/home/dongxu/kv9/target','CARGO_BUILD_JOBS=4','CARGO_NET_OFFLINE=true','CARGO_TERM_VERBOSE=true','python3',str(script),'--output','/tmp/kv9-quorum-trace-release-first','--release']
def available():
 s=os.statvfs('/home/dongxu/kv9');return s.f_bavail*s.f_frsize
start=available();floor=80*1024**3;ceiling=16*1024**3
if start-ceiling<floor:raise RuntimeError('release reservation cannot fit')
r=dict(complete=False,argv=cmd,script_sha256=hashlib.sha256(script.read_bytes()).hexdigest(),started_ns=time.time_ns(),initial_available_bytes=start,minimum_available_bytes=floor,additional_consumption_ceiling_bytes=ceiling,accounting='Conservative decrease in available filesystem bytes, including unrelated host writers; 5-second samples',samples=[])
p=None
try:
 with (root/'release-first.log').open('x') as log:
  p=subprocess.Popen(cmd,stdout=log,stderr=subprocess.STDOUT,start_new_session=True);r['pid']=p.pid
  (root/'release-invocation.json').write_text(json.dumps(r,indent=2)+'\n')
  while True:
   free=available();r['samples'].append(dict(unix_ns=time.time_ns(),available_bytes=free))
   if free<floor or start-free>ceiling:raise RuntimeError('diagnostic release build disk guard exceeded')
   try:r['exit_code']=p.wait(timeout=5);break
   except subprocess.TimeoutExpired:pass
  if r['exit_code']!=0:raise RuntimeError('diagnostic release build failed')
 r['complete']=True
except BaseException as e:
 r['failure']=repr(e)
 if p is not None and p.poll() is None:
  os.killpg(p.pid,signal.SIGTERM)
  try:p.wait(timeout=10)
  except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
 raise
finally:
 r['ended_ns']=time.time_ns();(root/'release-result.json').write_text(json.dumps(r,indent=2)+'\n')
print('PASS: diagnostic release build completed within its disk reservation')
