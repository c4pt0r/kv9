import os, subprocess, json, time
from pathlib import Path
out=Path(__file__).resolve().parent
root=Path('/home/dongxu/kv9')
env=dict(os.environ,CARGO_TARGET_DIR='/tmp/kv9-c04-configuration-development-20260915-first/target',CARGO_BUILD_JOBS='4')
for name,args in [
 ('workspace-default',['cargo','test','--offline','--locked','--workspace']),
 ('workspace-feature',['cargo','test','--offline','--locked','--workspace','--features','wal-payload-preallocation']),
 ('clippy-default',['cargo','clippy','--offline','--locked','--workspace','--all-targets','--','-D','warnings']),
 ('clippy-feature',['cargo','clippy','--offline','--locked','--workspace','--all-targets','--features','wal-payload-preallocation','--','-D','warnings']),
]:
 record={'argv':args,'cwd':str(root),'env_override':{k:env[k] for k in ['CARGO_TARGET_DIR','CARGO_BUILD_JOBS']},'started_ns':time.time_ns()}
 (out/(name+'-command.json')).write_text(json.dumps(record,indent=2)+'\n')
 with (out/(name+'.stdout')).open('w') as stdout,(out/(name+'.stderr')).open('w') as stderr:r=subprocess.run(args,cwd=root,env=env,stdout=stdout,stderr=stderr)
 record.update(exit_code=r.returncode,finished_ns=time.time_ns());(out/(name+'-result.json')).write_text(json.dumps(record,indent=2)+'\n')
 print(json.dumps({'name':name,'exit_code':r.returncode}),flush=True)
 if r.returncode:
  print((out/(name+'.stdout')).read_text()[-7000:]);print((out/(name+'.stderr')).read_text()[-7000:]);raise SystemExit(r.returncode)
