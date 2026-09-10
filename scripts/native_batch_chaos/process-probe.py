"""Owned probe: status writer boot/start identity must match the live process."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import re
PROBE = r'''set -euo pipefail
status_path=$1
binary_path=$2
before=$(cat "$status_path")
pid=$(printf '%s\n' "$before" | sed -n 's/^pid=//p')
[[ "$pid" =~ ^[1-9][0-9]*$ ]]
printf '\n@@STATUS_BEFORE@@\n%s\n' "$before"
printf '@@BOOT_BEFORE@@\n'; cat /proc/sys/kernel/random/boot_id
printf '@@STAT_BEFORE@@\n'; cat "/proc/$pid/stat"
printf '@@HASH_BEFORE@@\n'; sha256sum "$binary_path" "/proc/$pid/exe"
printf '@@CPU@@\n'; sed -n '/Cpus_allowed_list/p' "/proc/$pid/status"
printf '@@WRAPPERS@@\n'
for f in /tmp/kv9-io.pid /tmp/kv9-loss.pid /tmp/kv9-loss.last-pid; do
 if [ -f "$f" ]; then printf '%s=' "$f"; cat "$f"; printf '\n'; fi
done
printf '@@HASH_AFTER@@\n'; sha256sum "/proc/$pid/exe"
printf '@@STAT_AFTER@@\n'; cat "/proc/$pid/stat"
printf '@@BOOT_AFTER@@\n'; cat /proc/sys/kernel/random/boot_id
printf '@@STATUS_AFTER@@\n'; cat "$status_path"
printf '@@END@@\n'
'''
def parse(stdout, expected, binary_path):
 parts=re.split(r'^@@([A-Z_]+)@@\n',stdout,flags=re.M);sections=dict(zip(parts[1::2],parts[2::2]))
 def state(name):return dict(line.split('=',1) for line in sections[name].splitlines() if '=' in line)
 before,after=state('STATUS_BEFORE'),state('STATUS_AFTER');pid=before['pid']
 assert pid.isdigit() and int(pid)>0 and after['pid']==pid,'status PID changed'
 def stat(name):
  text=sections[name].strip();assert text.split(' ',1)[0]==pid,'stat PID mismatch';fields=text.rsplit(')',1)[1].split();assert fields[0] not in ['Z','X','x'],'process not live';return int(fields[19])
 start=stat('STAT_BEFORE');assert stat('STAT_AFTER')==start,'process start ticks changed'
 boot=sections['BOOT_BEFORE'].strip();assert re.fullmatch(r'[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}',boot),'malformed actual boot ID';assert sections['BOOT_AFTER'].strip()==boot,'actual boot ID changed'
 for when,status in [('before',before),('after',after)]:
  claimed=status.get('process_start_ticks','');assert claimed.isdigit() and int(claimed)==start,('status writer start identity mismatch',when,claimed,start)
  assert status.get('process_boot_id')==boot,('status writer boot identity mismatch',when,status.get('process_boot_id'),boot)
 def hashes(name):return dict((line.split(None,1)[1].strip(),line.split(None,1)[0]) for line in sections[name].splitlines() if line.strip())
 h1,h2=hashes('HASH_BEFORE'),hashes('HASH_AFTER');assert h1[binary_path]==expected,'static executable mismatch';assert h1[f'/proc/{pid}/exe']==expected and h2[f'/proc/{pid}/exe']==expected,'executing executable mismatch'
 cpu=sections['CPU'].strip().split(':',1)[1].strip();assert cpu,'CPU mask missing'
 return dict(identity_matched=True,process_pid=int(pid),process_start_ticks=start,process_boot_id=boot,status_writer_identity_matched=True,cpu_allowed=cpu,state=before,state_after=after,wrapper_pidfiles=sections['WRAPPERS'].strip(),executable_sha256_before=h1[f'/proc/{pid}/exe'],executable_sha256_after=h2[f'/proc/{pid}/exe'],status_pid_before=int(pid),status_pid_after=int(after['pid']))
