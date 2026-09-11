#!/usr/bin/env python3
"""Temporarily constrain three owned test containers, run, and restore."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

BACKGROUND = list(range(6, 16)) + list(range(22, 32))
CPUSET = '6-15,22-31'
OWNED = {
    'kv9-chaos-ci-p0-20260908-control-plane': 'ae9b27a77d14f295e35e112b31f5dce62ed6fd2d86a90e7b2a69b52798e17582',
    'kv9-chaos-control-plane': 'd31317fe14d1d3344caa91ed45b4246e5e949d99b25cec5e28d9c6695fa43e8c',
    'kv9-minio-dev': '4b05c35453b87a409e7dd8a4dc935c782e282eb42cf14dd8c1ea6771efc0e042',
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def command(argv):
    return subprocess.check_output(list(map(str, argv)), text=True, timeout=30)


def identity(name):
    x = json.loads(command(['docker', 'inspect', name]))[0]
    require(x['Id'] == OWNED[name] and x['State']['Running'], 'owned container identity differs')
    pid = x['State']['Pid']
    stat = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    group = Path(f'/proc/{pid}/cgroup').read_text().strip()
    require(group.startswith('0::') and '\n' not in group, 'requires unified cgroup v2')
    root = Path('/sys/fs/cgroup') / group[3:].lstrip('/')
    effective = (root / 'cpuset.cpus.effective').read_text().strip()
    return dict(name=name, id=x['Id'], pid=pid, start_ticks=int(stat[19]),
                started=x['State']['StartedAt'], configured_cpus=x['HostConfig']['CpusetCpus'],
                cgroup=str(root), effective_cpus=effective)


def same_process(before, after):
    return all(before[k] == after[k] for k in ('name', 'id', 'pid', 'start_ticks', 'started', 'cgroup'))


def descendants(snapshot, constrained=False):
    tasks = []
    vanished = []
    for group in Path(snapshot['cgroup']).rglob('cgroup.threads'):
        try:
            tids = group.read_text().split()
        except FileNotFoundError:
            continue
        for tid in tids:
            try:
                cpus = sorted(os.sched_getaffinity(int(tid)))
                if constrained:
                    require(set(cpus).issubset(BACKGROUND), f'container thread {tid} escapes background CPUs')
                tasks.append(dict(tid=int(tid), cpus=cpus))
            except ProcessLookupError:
                vanished.append(int(tid))
    require(tasks, 'no container threads observed')
    return dict(tasks=tasks, vanished_during_read=vanished)


def cluster_inventory():
    result = {}
    for name in OWNED:
        if 'control-plane' not in name:
            continue
        base = ['docker', 'exec', name, 'kubectl', '--kubeconfig=/etc/kubernetes/admin.conf', 'get']
        namespaces = json.loads(command(base + ['namespaces', '-o', 'json']))['items']
        faults = json.loads(command(base + ['networkchaos,podchaos,iochaos,stresschaos,timechaos', '-A', '-o', 'json']))['items']
        pods = json.loads(command(base + ['pods', '-A', '-o', 'json']))['items']
        # Preserve the historical one-shot kill targeting an already absent pod.
        # No historical resource is deleted, paused, or re-labeled as recovered.
        for fault in faults:
            metadata = fault['metadata']
            require(metadata['uid'] == 'c3ededa7-0746-4864-bd95-4425ceb3d663'
                    and fault['kind'] == 'PodChaos' and fault['spec']['action'] == 'pod-kill',
                    'unexpected fault resource exists')
            selected = fault['spec']['selector']['pods']
            require(not any(p['metadata']['name'] in selected.get(p['metadata']['namespace'], []) for p in pods),
                    'historical killed pod is present')
        running = [p for p in pods if p['metadata']['namespace'].startswith('kv9-') and p.get('status', {}).get('phase') == 'Running']
        for pod in running:
            require(all(c.get('command') == ['sleep', '3600'] and not c.get('args') for c in pod['spec']['containers']),
                    'a historical project workload is running')
        result[name] = dict(namespaces={n['metadata']['name']: n['metadata']['uid'] for n in namespaces},
                            faults=faults, project_running_pods=running)
    return result


def host_observation():
    result = dict(unix_ns=time.time_ns(), proc_stat=Path('/proc/stat').read_text(),
                  loadavg=Path('/proc/loadavg').read_text(),
                  pressure={p.name: p.read_text() for p in Path('/proc/pressure').iterdir()},
                  processes=command(['ps', '-eo', 'pid,ppid,pcpu,comm', '--sort=-pcpu']))
    return result


def stop_group(child, outcome):
    """Give the driver's finally time to retain data, then stop owned stragglers."""
    errors = []
    if child.poll() is None:
        try:
            child.terminate()
            child.wait(timeout=60)
        except BaseException as error:
            errors.append(repr(error))
    try:
        os.killpg(child.pid, 0)
    except ProcessLookupError:
        pass
    else:
        outcome['forced_group_cleanup'] = True
        try:
            os.killpg(child.pid, signal.SIGTERM)
            limit = time.monotonic() + 5
            while time.monotonic() < limit:
                try:
                    os.killpg(child.pid, 0)
                except ProcessLookupError:
                    break
                time.sleep(.05)
            else:
                os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except BaseException as error:
            errors.append(repr(error))
    try:
        child.wait(timeout=10)
    except BaseException as error:
        errors.append(repr(error))
    outcome['child_cleanup_errors'] = errors


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--driver', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--driver-arguments-file', type=Path, required=True)
    args = parser.parse_args()
    argument_bytes = args.driver_arguments_file.read_bytes()
    driver_arguments = json.loads(argument_bytes)
    require(type(driver_arguments) is list and all(type(item) is str for item in driver_arguments), 'driver arguments must be a JSON string array')
    allowed = {'--client-source', '--client-build', '--expected-client-revision',
               '--old-server-source', '--old-server-build', '--new-server-source',
               '--new-server-build', '--redis-client-build', '--redis-client-source', '--storage',
               '--redis-server', '--resp-helper'}
    require(len(driver_arguments) % 2 == 0, 'driver arguments must be option/value pairs')
    flags = driver_arguments[::2]
    require(set(flags) <= allowed and len(flags) == len(set(flags)), 'driver options must be unique exact allowed names')
    require({'--client-source', '--client-build', '--expected-client-revision'} <= set(flags), 'shared client identity arguments missing')
    require(all(value and not value.startswith('-') for value in driver_arguments[1::2]), 'driver argument values must be explicit non-option strings')
    before = {name: identity(name) for name in OWNED}
    # Docker may silently ignore an empty cpuset restore. Reject that state
    # before changing any container; never rewrite a failed restoration as success.
    require(all(old['configured_cpus'] for old in before.values()), 'empty configured cpuset cannot be restored reliably; no container was changed')
    original_inventory = cluster_inventory()
    args.output.mkdir(parents=True, exist_ok=False)
    os.sched_setaffinity(0, BACKGROUND)
    save(args.output / 'before.json', dict(containers=before, clusters=original_inventory,
                                         host=host_observation()))
    changed = []
    child = None
    outcome = dict(complete=False, restoration_complete=False, exclusive_host=False)
    def interrupted(signum, frame):
        raise InterruptedError(f'received signal {signum}')
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        for name, old in before.items():
            require(same_process(old, identity(name)), 'container changed before update')
            # Register before the update so even a partial failure is restored.
            changed.append(name)
            command(['docker', 'update', '--cpuset-cpus', CPUSET, old['id']])
            now = identity(name)
            require(same_process(old, now) and now['configured_cpus'] == CPUSET and now['effective_cpus'] == CPUSET,
                    'container CPU restriction did not take effect')
            now['descendants'] = descendants(now, constrained=True)
            save(args.output / f'{name}-isolated.json', now)
        snapshot = dict(complete=True, measured_client_cpus=[0, 1], measured_server_cpus=[2, 3, 4, 5],
                        background_cpus=BACKGROUND, exclusive_host=False,
                        limitation='Unrelated host services and BuildKit remain unconstrained; shared-host diagnostic only.',
                        containers={name: identity(name) for name in OWNED},
                        before_path=str(args.output / 'before.json'),
                        historical_one_shot_fault_preserved=True,
                        driver_sha256=hashlib.sha256(args.driver.read_bytes()).hexdigest())
        save(args.output / 'isolation.json', snapshot)
        invocation = [sys.executable, str(args.driver), '--output', str(args.output / 'cohorts'),
                      '--isolation-snapshot', str(args.output / 'isolation.json'), *driver_arguments]
        save(args.output / 'invocation.json', dict(argv=invocation, driver_arguments_sha256=hashlib.sha256(argument_bytes).hexdigest(), wrapper_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()))
        child = subprocess.Popen(invocation, start_new_session=True)
        code = child.wait(timeout=3600)
        outcome['child_exit_code'] = code
        require(code == 0, 'paired driver failed; original artifacts retained')
        require(args.driver_arguments_file.read_bytes() == argument_bytes, 'driver arguments file changed during execution')
        outcome['complete'] = True
    except BaseException as error:
        outcome['error'] = repr(error)
    finally:
        restored = {}
        errors = []
        cleanup_signals = []
        def defer_signal(signum, frame):
            cleanup_signals.append(signum)
        signal.signal(signal.SIGTERM, defer_signal)
        signal.signal(signal.SIGINT, defer_signal)
        try:
            if child is not None:
                stop_group(child, outcome)
                if outcome.get('forced_group_cleanup') or outcome.get('child_cleanup_errors'):
                    outcome['complete'] = False
        except BaseException as error:
            errors.append(dict(scope='child_cleanup', error=repr(error)))
            outcome['complete'] = False
        finally:
            for name in reversed(changed):
                try:
                    old = before[name]
                    require(same_process(old, identity(name)), 'refusing to modify a replacement container')
                    command(['docker', 'update', '--cpuset-cpus', old['configured_cpus'], old['id']])
                    now = identity(name)
                    require(same_process(old, now) and now['configured_cpus'] == old['configured_cpus']
                            and now['effective_cpus'] == old['effective_cpus'], 'restored CPU settings differ')
                    restored[name] = now
                except BaseException as error:
                    errors.append(dict(name=name, error=repr(error)))
        try:
            final_inventory = cluster_inventory()
            require(all(final_inventory[n]['namespaces'] == original_inventory[n]['namespaces'] for n in original_inventory),
                    'historical namespace identities changed')
        except BaseException as error:
            final_inventory = None
            errors.append(dict(scope='cluster_inventory', error=repr(error)))
        save(args.output / 'after.json', dict(containers=restored, clusters=final_inventory,
                                            host=host_observation(), errors=errors))
        outcome['restoration_complete'] = not errors and len(restored) == len(changed)
        outcome['deferred_cleanup_signals'] = cleanup_signals
        save(args.output / 'summary.json', outcome)
    print(json.dumps(outcome), flush=True)
    return 0 if outcome['complete'] and outcome['restoration_complete'] else 1


if __name__ == '__main__':
    sys.exit(main())
