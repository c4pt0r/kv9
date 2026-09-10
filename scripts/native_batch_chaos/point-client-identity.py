"""Read-only live identity/configuration/connection capture for this exact persistent client."""

if not __debug__:
    raise SystemExit("FAIL: Python optimization disables required checks; use PYTHONOPTIMIZE=0 without -O or -OO.")
import argparse
import json
from pathlib import Path
import subprocess
import time

ap=argparse.ArgumentParser(description=__doc__)
ap.add_argument('--plan',type=Path,required=True)
ap.add_argument('--artifact',type=Path,required=True)
ap.add_argument('--namespace',required=True)
args=ap.parse_args()
ROOT=args.artifact
p=json.loads(args.plan.read_text())
raw=ROOT
kube=['kubectl','--kubeconfig',p['kubeconfig'],'--request-timeout=10s']
assert args.namespace.startswith('kv9-chaos-') and args.namespace not in p['preserve_namespaces']
# The prebuilt verifier already rechecked every loaded image before launch.
mapping=next(iter(p['cri_mappings'].values()))
assert mapping['status']['id']==p['prepared_image']['docker_image_id']
attempts = []
commands = []
deadline = time.monotonic() + 300
script = r'''set -euo pipefail
found=0
for exe in /proc/[0-9]*/exe; do
 path=$(readlink "$exe" 2>/dev/null || true)
 if [ "$path" = /usr/local/bin/kv9-workload ]; then
  found=$((found+1)); pid=${exe#/proc/}; pid=${pid%/exe}
  printf '@@BOOT_BEFORE@@\n';cat /proc/sys/kernel/random/boot_id
  printf '@@STAT_BEFORE@@\n';cat "/proc/$pid/stat"
  printf '@@HASH@@\n';sha256sum /usr/local/bin/kv9-workload "$exe"
  printf '@@COMMAND@@\n';tr '\0' '\n' < "/proc/$pid/cmdline"
  printf '@@CPU@@\n';sed -n '/Cpus_allowed_list/p' "/proc/$pid/status"
  printf '@@SOCKET_FDS@@\n'
  for fd in /proc/$pid/fd/*; do readlink "$fd" 2>/dev/null || true; done
  printf '@@TCP@@\n';cat "/proc/$pid/net/tcp"
  printf '@@STAT_AFTER@@\n';cat "/proc/$pid/stat"
  printf '@@BOOT_AFTER@@\n';cat /proc/sys/kernel/random/boot_id
 fi
done
[ "$found" = 1 ]
printf '@@PROGRESS@@\n';cat /tmp/workload/progress.json
printf '@@CONFIG@@\n';cat /tmp/workload/config.json
printf '@@BUILD@@\n';cat /tmp/workload/build.json
'''


def run(args):
    start = time.time_ns()
    done = subprocess.run(kube + args, text=True, capture_output=True, timeout=12)
    record = dict(command=kube + args, started_unix_ns=start, ended_unix_ns=time.time_ns(),
                  exit_code=done.returncode, stdout=done.stdout, stderr=done.stderr)
    commands.append(record)
    assert done.returncode == 0, done.stderr
    return done.stdout


while time.monotonic() < deadline:
    try:
        ns = args.namespace
        assert ns not in p['preserve_namespaces']
        before = json.loads(run(['get', 'pod', '-n', ns, 'kv9-persistent-client', '-o', 'json']))
        output = run(['exec', '-n', ns, 'kv9-persistent-client', '--', '/bin/bash', '-c', script])
        after = json.loads(run(['get', 'pod', '-n', ns, 'kv9-persistent-client', '-o', 'json']))
        def identity(pod):
            return pod['metadata']['uid'], [(r['name'], r.get('containerID'), r.get('imageID'), r.get('restartCount'))
                                           for r in pod['status']['containerStatuses']]
        assert identity(before) == identity(after)
        assert all(r['imageID'] in mapping['status']['repoDigests']
                   and r['image'] in mapping['status']['repoTags']
                   for r in before['status']['containerStatuses'])
        def section(name):
            return output.split('@@' + name + '@@\n')[1].split('@@')[0].strip()
        a, b = section('STAT_BEFORE'), section('STAT_AFTER')
        pid = int(a.split(' ', 1)[0])
        ticks = int(a.rsplit(')', 1)[1].split()[19])
        assert int(b.split(' ', 1)[0]) == pid and int(b.rsplit(')', 1)[1].split()[19]) == ticks
        assert section('BOOT_BEFORE') == section('BOOT_AFTER')
        expected = p['correctness_client_build']['binary_sha256']
        assert expected + f'  /proc/{pid}/exe' in output and expected + '  /usr/local/bin/kv9-workload' in output
        progress = json.loads(section('PROGRESS'))
        config = json.loads(section('CONFIG'))
        build = json.loads(section('BUILD'))
        assert progress['stage'] == 'measure' and config['rpc_transport'] == 'tonic_stream'
        assert config == json.loads((raw / 'persistent-config.json').read_text())
        assert build == p['correctness_client_build']
        inodes = {line[8:-1] for line in section('SOCKET_FDS').splitlines() if line.startswith('socket:[')}
        connected = [line.split() for line in section('TCP').splitlines()[1:]
                     if line.split()[9] in inodes and line.split()[3] == '01']
        assert connected and all(int(row[2].split(':')[1], 16) == 20160 for row in connected)
        result = dict(complete=True, namespace=ns, pod_uid=before['metadata']['uid'], process_pid=pid,
            process_start_ticks=ticks, boot_id=section('BOOT_BEFORE'), executing_sha256=expected,
            actual_cpu_mask=section('CPU').split(':', 1)[1].strip(), configuration=config,
            connections=connected, cri_image_mapping=mapping, before=before, after=after, commands=commands, attempts=attempts,
            scope='One contemporaneous measurement-stage exact default-stream client identity/config/ordinary-port socket capture. Read-only; not a performance observation or batch claim.')
        (ROOT / 'point-live-client-identity.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({key: result[key] for key in ('complete', 'namespace', 'process_pid', 'executing_sha256', 'actual_cpu_mask')}))
        break
    except (OSError, ValueError, KeyError, IndexError, AssertionError, subprocess.TimeoutExpired) as error:
        attempts.append(dict(at_unix_ns=time.time_ns(), reason=repr(error)))
        (ROOT / 'point-live-client-attempts.json').write_text(json.dumps(dict(complete=False, attempts=attempts, commands=commands), indent=2) + '\n')
        time.sleep(2)
else:
    raise TimeoutError('no exact live default-stream client identity capture within the bounded window')
