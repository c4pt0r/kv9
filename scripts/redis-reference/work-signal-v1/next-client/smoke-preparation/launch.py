"""One retained launch of the reviewed finite smoke; no automatic retries."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

p = Path(__file__).resolve().parent
digest = lambda f: hashlib.sha256(f.read_bytes()).hexdigest()
assert digest(p/'run.py') == '45e8e02ffd9c9c3229ba81ed0cc200476755b73c570dcd15d381cc64e6532c51'
assert digest(p/'protocol.json') == '97d350afe95484a4228c9f44563e7ee31b85a7cffa90579fece66165006cb9d1'
assert digest(p/'command.json') == 'd8beddb8fab322c82a3305ada36141d4693a1e2def8392c5c5d1a709a6012595'
command = json.loads((p/'command.json').read_text())
assert all(os.environ.get(k) == v for k, v in command['environment'].items())
with (p/'runtime-first.log').open('x') as log:
    child = subprocess.Popen(command['argv'], stdout=log, stderr=log)
    row = dict(command=command, pid=child.pid, launcher_pid=os.getpid(),
               launched_unix_ns=time.time_ns(), driver_sha256=digest(p/'run.py'))
    with (p/'launch-first.json').open('x') as f:
        json.dump(row, f, indent=2); f.write('\n')
    print('LIVE smoke PID '+str(child.pid), flush=True)
    code = child.wait()
    row.update(exit_code=code, terminal_unix_ns=time.time_ns(), log_sha256=digest(p/'runtime-first.log'))
    with (p/'terminal-first.json').open('x') as f:
        json.dump(row, f, indent=2); f.write('\n')
    print('TERMINAL smoke exit '+str(code), flush=True)
    raise SystemExit(code)
