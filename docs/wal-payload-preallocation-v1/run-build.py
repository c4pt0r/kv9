import os, subprocess, json, time
from pathlib import Path
out=Path(__file__).resolve().parent
env=dict(os.environ, CARGO_TARGET_DIR='/home/dongxu/kv9/target', CARGO_BUILD_JOBS='4')
command=['cargo','test','--release','--offline','-j4','-p','kv9-engine','--features','wal-payload-preallocation','--lib','--no-run','--message-format=json']
record={'argv':command,'cwd':str(out/'source'),'env_override':{k:env[k] for k in ['CARGO_TARGET_DIR','CARGO_BUILD_JOBS']},'started_ns':time.time_ns()}
(out/'build-command.json').write_text(json.dumps(record,indent=2)+'\n')
with (out/'build.stdout').open('w') as stdout,(out/'build.stderr').open('w') as stderr:
 result=subprocess.run(command,cwd=out/'source',env=env,stdout=stdout,stderr=stderr)
record.update(exit_code=result.returncode,finished_ns=time.time_ns())
(out/'build-result.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps(record));print((out/'build.stderr').read_text()[-5000:])
raise SystemExit(result.returncode)
