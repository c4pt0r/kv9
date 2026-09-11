"""Bounded read-only CLI prefix; no measurement work, retries or dataset writes."""
if not __debug__:
    raise RuntimeError('active prefix requires PYTHONOPTIMIZE=0')
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

CONFIG = dict(duration_ms=1000, calls=128, concurrency=8, call_deadline_ms=1500,
              overall_deadline_ms=4000, output_limit_bytes=65536,
              command='raw-get', expected_stdout='found=false', retries=0)
CPUS = list(range(6, 16)) + list(range(22, 32))

def require(value, message):
    if not value:
        raise ValueError(message)

def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def save(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')

def identity(pid, executable):
    root = Path('/proc', str(pid))
    stat = (root / 'stat').read_text()
    fields = stat.rsplit(')', 1)[1].split()
    observed = (root / 'exe').stat()
    expected = Path(executable).stat()
    require((observed.st_dev, observed.st_ino, observed.st_size) ==
            (expected.st_dev, expected.st_ino, expected.st_size), 'prefix executable inode differs')
    return dict(pid=pid, start_ticks=int(fields[19]), user_ticks=int(fields[11]),
                system_ticks=int(fields[12]), boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
                executable_path=os.readlink(root / 'exe'), executable_device=observed.st_dev,
                executable_inode=observed.st_ino, executable_bytes=observed.st_size,
                cpu_affinity=sorted(os.sched_getaffinity(pid)), observed_monotonic_ns=time.monotonic_ns(),
                observed_unix_ns=time.time_ns())

def same_lifetime(before, after):
    return all(before[key] == after[key] for key in
               ['pid', 'start_ticks', 'boot_id', 'executable_device', 'executable_inode', 'executable_bytes'])

def command(binary, address, keyspace, key):
    require(key.startswith(b'__profile_prefix__:') and keyspace > 0, 'prefix keyspace/key differs')
    return [str(binary), 'client', 'raw-get', '--addr', address,
            '--keyspace', str(keyspace), '--key-hex', key.hex()]

def validate_summary(summary):
    require(summary['configuration'] == CONFIG, 'prefix bounds changed')
    require(summary['retries'] == 0, 'prefix replay policy changed')
    require(summary['dispatch_end_monotonic_ns'] - summary['start_monotonic_ns'] >= 1_000_000_000,
            'active prefix dispatch interval too short')
    require(summary['end_monotonic_ns'] - summary['start_monotonic_ns'] <= 4_000_000_000,
            'prefix overall deadline exceeded')
    calls = summary['calls']
    require(len(calls) == CONFIG['calls'] and [c['slot'] for c in calls] == list(range(CONFIG['calls'])),
            'prefix call accounting incomplete')
    require(summary['max_live'] <= CONFIG['concurrency'], 'prefix concurrency exceeded')
    for call in calls:
        require(call['command'][1:3] == ['client', 'raw-get'], 'prefix is not read-only RawGet')
        require(call['identity_captured'] and call['exited'] and call['absent'], 'prefix child not bound/exited')
        require(call['outcome'] == 'not_found' and call['exit_code'] == 0,
                'prefix did not complete the declared absent-key read')
        require(call['completed_monotonic_ns'] <= call['deadline_monotonic_ns'], 'prefix call exceeded deadline')
        require(call['stdout'] == 'found=false\n' and call['stderr'] == '', 'prefix reply differs')
    require(summary['voter_lifetimes_unchanged'] and summary['executable_unchanged'], 'prefix voter/binary changed')
    require(not summary['cleanup_errors'], 'prefix cleanup errors')

def run(fixture, directory, expected_binary_sha256, recorded_voters, perf_process):
    require(sorted(os.sched_getaffinity(0)) == CPUS, 'prefix harness affinity differs')
    directory.mkdir(exist_ok=False)
    binary = fixture.build / 'kv9'
    require(sha(binary) == expected_binary_sha256, 'prefix binary differs')
    config = json.loads((directory.parent / 'requested-config.json').read_text())
    key = b'__profile_prefix__:' + config['run_id'].encode()
    require(not key.startswith(config['run_id'].encode() + b':'), 'prefix key overlaps measured dataset')
    leader = fixture.leader()
    require(leader in fixture.nodes, 'prefix leader unavailable')
    voters_before = {str(n): identity(child.pid, binary) for n, child in fixture.nodes.items()}
    for node, voter in voters_before.items():
        expected = recorded_voters[node]
        require(voter['pid'] == expected['pid'] and voter['start_ticks'] == expected['start_ticks']
                and voter['boot_id'] == expected['boot_id'], 'prefix voter is not recorded lifetime')
    summary = dict(complete=False, configuration=CONFIG, retries=0, key_hex=key.hex(),
                   keyspace_id=config['client']['keyspace_id'], leader_id=leader,
                   binary_sha256=expected_binary_sha256, voters_before=voters_before,
                   calls=[], cleanup_errors=[], max_live=0, scope='Unmeasured unary CLI RawGet priming only; alters cache/CPU state, never benchmark or actual sample-coverage acceptance')
    save(directory / 'summary.json', summary)
    active = {}
    started = time.monotonic_ns()
    cutoff = started + CONFIG['duration_ms'] * 1_000_000
    overall = started + CONFIG['overall_deadline_ms'] * 1_000_000
    summary['start_monotonic_ns'] = started
    summary['start_unix_ns'] = time.time_ns()

    def finish(slot, outcome_override=None):
        child, row, stdout, stderr = active[slot]
        code = child.wait(timeout=0)
        stdout.close(); stderr.close()
        row.update(exit_code=code, exited=True, completed_monotonic_ns=time.monotonic_ns(),
                   completed_unix_ns=time.time_ns(), absent=not Path('/proc', str(child.pid)).exists())
        for suffix in ['stdout', 'stderr']:
            path = directory / f'call-{slot:03d}.{suffix}'
            row[suffix + '_bytes'] = path.stat().st_size
            row[suffix + '_sha256'] = sha(path)
            # Retain bounded text; raw files preserve any oversize failure.
            row[suffix] = path.read_bytes()[:CONFIG['output_limit_bytes'] + 1].decode('utf-8', 'replace')
        row['outcome'] = outcome_override or ('not_found' if code == 0 and row['stdout'] == 'found=false\n' and row['stderr'] == '' else 'read_failure')
        del active[slot]
        save(directory / 'summary.json', summary)

    try:
        while active or len(summary['calls']) < CONFIG['calls'] or time.monotonic_ns() < cutoff:
            now = time.monotonic_ns()
            require(now < overall, 'prefix overall deadline exceeded')
            require(perf_process.poll() is None, 'profiler exited during active prefix')
            for slot in list(active):
                child, row, stdout, stderr = active[slot]
                require(stdout.tell() <= CONFIG['output_limit_bytes'] and stderr.tell() <= CONFIG['output_limit_bytes'],
                        'prefix output bound exceeded')
                if child.poll() is not None:
                    finish(slot)
                    require(row['outcome'] == 'not_found', 'prefix read failure retained')
                    require(row['completed_monotonic_ns'] <= row['deadline_monotonic_ns'], 'prefix read deadline exceeded')
                elif now >= row['deadline_monotonic_ns']:
                    row['outcome'] = 'deadline'
                    raise ValueError('prefix read deadline exceeded')
            slot = len(summary['calls'])
            if slot < CONFIG['calls'] and len(active) < CONFIG['concurrency']:
                scheduled = started + slot * CONFIG['duration_ms'] * 1_000_000 // CONFIG['calls']
                if now >= scheduled:
                    require(now < cutoff, 'prefix dispatch ceiling reached with unissued slots')
                    argv = command(binary, fixture.addresses[leader], summary['keyspace_id'], key)
                    stdout = (directory / f'call-{slot:03d}.stdout').open('xb')
                    stderr = (directory / f'call-{slot:03d}.stderr').open('xb')
                    row = dict(slot=slot, command=argv, scheduled_monotonic_ns=scheduled,
                               started_monotonic_ns=now, deadline_monotonic_ns=now + CONFIG['call_deadline_ms'] * 1_000_000,
                               identity_captured=False, exited=False, absent=False, outcome='pending')
                    summary['calls'].append(row)
                    try:
                        child = subprocess.Popen(argv, env=fixture.env, stdout=stdout, stderr=stderr)
                    except BaseException:
                        stdout.close(); stderr.close(); row['outcome'] = 'spawn_failure'; raise
                    active[slot] = (child, row, stdout, stderr)
                    row['pid'] = child.pid
                    # Popen returns after exec; bind this exact retained binary inode.
                    row['identity'] = identity(child.pid, binary)
                    require(row['identity']['cpu_affinity'] == CPUS, 'prefix client affinity differs')
                    row['identity_captured'] = True
                    summary['max_live'] = max(summary['max_live'], len(active))
                    save(directory / 'summary.json', summary)
            if time.monotonic_ns() >= cutoff:
                summary.setdefault('dispatch_end_monotonic_ns', time.monotonic_ns())
            time.sleep(.002)
        summary.setdefault('dispatch_end_monotonic_ns', time.monotonic_ns())
        summary['voters_after'] = {str(n): identity(child.pid, binary) for n, child in fixture.nodes.items()}
        summary['voter_lifetimes_unchanged'] = all(same_lifetime(voters_before[n], summary['voters_after'][n]) for n in voters_before)
        summary['executable_unchanged'] = sha(binary) == expected_binary_sha256
        summary['end_monotonic_ns'] = time.monotonic_ns()
        summary['end_unix_ns'] = time.time_ns()
        validate_summary(summary)
        summary['complete'] = True
    except BaseException as error:
        summary['failure'] = repr(error)
        raise
    finally:
        # Every launched CLI is tracked before identity inspection can fail.
        for slot in list(active):
            child, row, stdout, stderr = active[slot]
            try:
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=.25)
                    except subprocess.TimeoutExpired:
                        child.kill(); child.wait(timeout=1)
                finish(slot, row['outcome'] if row['outcome'] != 'pending' else 'cancelled')
            except BaseException as error:
                summary['cleanup_errors'].append(dict(slot=slot, pid=child.pid, error=repr(error)))
                stdout.close(); stderr.close()
        summary['cleanup_completed_monotonic_ns'] = time.monotonic_ns()
        summary['cleanup_completed_unix_ns'] = time.time_ns()
        if summary['cleanup_errors']:
            summary['complete'] = False
        save(directory / 'summary.json', summary)
    require(summary['complete'], 'active prefix incomplete')
    return summary
