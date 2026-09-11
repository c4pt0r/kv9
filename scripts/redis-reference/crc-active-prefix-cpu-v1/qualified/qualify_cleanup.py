#!/usr/bin/env python3
"""Proposed real dummy-process qualification; no KV9, perf, RPC, sockets or cluster."""
if not __debug__:
    raise RuntimeError('qualification requires assertions')
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
EXPECTED = 'ed1a5ff2f804e9d9cc85c8a1f7fea8c42b579618d0260e3fe1da8c0ea2d8ccbe'
CPUS = set(range(6, 16)) | set(range(22, 32))
DUMMY = '''import os, signal, time
if os.environ['PREFIX_DUMMY_MODE'] == 'timeout':
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    print('dummy_ready_term_ignored', flush=True)
time.sleep(60)
'''

def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def save(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')

def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    assert output.is_relative_to(HERE) and not output.exists()
    assert os.sched_getaffinity(0) == CPUS
    helper_path = HERE / 'active_prefix.py'
    assert sha(helper_path) == EXPECTED
    spec = importlib.util.spec_from_file_location('prefix_cleanup_target', helper_path)
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    original_identity = helper.identity
    original_popen = subprocess.Popen
    source_before = {str(HERE / name): sha(HERE / name) for name in
                     ['active_prefix.py', 'profile.py', 'analyze.py', 'test_prefix_contract.py']}
    output.mkdir()
    build = output / 'dummy-build'
    build.mkdir()
    python = Path(sys.executable).resolve()
    (build / 'kv9').symlink_to(python)
    (output / 'client').write_text(DUMMY)
    # The frozen helper's argv becomes: exact Python executable, script "client",
    # then inert raw-get arguments. This script never opens a socket.
    result = dict(accepted=False, source_before=source_before, cases=[], cleanup=[],
                  scope='Expected failure-path qualification of frozen prefix helper using real Python dummy children; no RPC/profile/measurement acceptance',
                  python_executable=str(python), python_sha256=sha(python),
                  dummy_source_sha256=sha(output / 'client'), helper_sha256=EXPECTED,
                  started_unix_ns=time.time_ns(), pid=os.getpid())
    logs, voters, tracked_children = [], {}, []
    old_cwd = Path.cwd()
    save(output / 'summary.json', result)

    class TrackedPopen(original_popen):
        def __init__(self, *pos, **kwargs):
            super().__init__(*pos, **kwargs)
            tracked_children.append(self)

    try:
        os.chdir(output)
        for node in [1, 2, 3]:
            log = (output / f'dummy-voter-{node}.log').open('xb')
            logs.append(log)
            voters[node] = original_popen([str(python), '-c', 'import time; time.sleep(60)'], stdout=log, stderr=log)
        recorded = {str(node): original_identity(child.pid, build / 'kv9') for node, child in voters.items()}
        result['dummy_voters'] = recorded
        save(output / 'summary.json', result)
        for mode in ['post_spawn_exception', 'timeout']:
            case_dir = output / mode
            case_dir.mkdir()
            save(case_dir / 'requested-config.json', {'run_id': 'dummy1', 'client': {'keyspace_id': 100}})
            fixture = SimpleNamespace(build=build, nodes=voters, addresses={n: 'unused-no-network' for n in voters},
                                      env=dict(os.environ, PREFIX_DUMMY_MODE=mode), leader=lambda: 1)
            voter_pids = {child.pid for child in voters.values()}
            if mode == 'post_spawn_exception':
                def fail_after_spawn(pid, executable):
                    if pid not in voter_pids:
                        raise RuntimeError('injected post-spawn identity failure')
                    return original_identity(pid, executable)
                helper.identity = fail_after_spawn
            else:
                helper.identity = original_identity
            first_child = len(tracked_children)
            expected_error = None
            # Only record handles. All process/signal/wait implementations remain
            # the real Popen methods called by the original frozen helper.
            subprocess.Popen = TrackedPopen
            try:
                helper.run(fixture, case_dir / 'active-prefix', sha(python), recorded,
                           SimpleNamespace(poll=lambda: None))
            except Exception as error:
                expected_error = repr(error)
            finally:
                subprocess.Popen = original_popen
                helper.identity = original_identity
            assert expected_error is not None, 'negative qualification unexpectedly launched measurement-ready prefix'
            summary_path = case_dir / 'active-prefix/summary.json'
            summary = json.loads(summary_path.read_text())
            assert summary['complete'] is False and not summary['cleanup_errors']
            assert summary['calls'] and len(summary['calls']) <= helper.CONFIG['concurrency']
            assert all(row['exited'] and row['absent'] and row['exit_code'] is not None for row in summary['calls'])
            assert all(child.poll() is not None and not Path('/proc', str(child.pid)).exists()
                       for child in tracked_children[first_child:]), 'helper left an owned dummy child alive'
            if mode == 'post_spawn_exception':
                assert 'injected post-spawn identity failure' in expected_error
                assert len(summary['calls']) == 1 and summary['calls'][0]['identity_captured'] is False
            else:
                assert 'prefix read deadline exceeded' in expected_error
                assert any(row['outcome'] == 'deadline' for row in summary['calls'])
                assert any(row['exit_code'] == -9 and 'dummy_ready_term_ignored' in row['stdout']
                           for row in summary['calls']), 'actual TERM-resistant KILL path was not reached'
            for node, child in voters.items():
                assert child.poll() is None and helper.same_lifetime(recorded[str(node)], original_identity(child.pid, build / 'kv9'))
            result['cases'].append(dict(name=mode, expected_error=expected_error, accepted=True,
                                        original_prefix_complete=False, calls=len(summary['calls']),
                                        pids=[row['pid'] for row in summary['calls']], summary_sha256=sha(summary_path)))
            save(output / 'summary.json', result)
        result['accepted'] = True
    except BaseException as error:
        result['failure'] = repr(error)
        raise
    finally:
        subprocess.Popen = original_popen
        helper.identity = original_identity
        # Qualification-owned fallback prevents a defective helper from leaking
        # children. Helper success was checked BEFORE this fallback cleanup.
        for child in [*tracked_children, *voters.values()]:
            record = dict(pid=child.pid, fallback_intervened=child.poll() is None)
            try:
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=.25)
                    except subprocess.TimeoutExpired:
                        child.kill(); child.wait(timeout=1)
                record.update(exit_code=child.wait(), absent=not Path('/proc', str(child.pid)).exists())
                assert record['absent']
            except BaseException as error:
                record['error'] = repr(error)
                result['accepted'] = False
            result['cleanup'].append(record)
        for log in logs:
            log.close()
        os.chdir(old_cwd)
        result['source_after'] = {name: sha(name) for name in source_before}
        result['source_unchanged'] = result['source_after'] == source_before
        result['accepted'] = result['accepted'] and result['source_unchanged']
        result['ended_unix_ns'] = time.time_ns()
        save(output / 'summary.json', result)
    assert result['accepted']
    print('PASS: two real dummy-process cleanup cases; no KV9/perf/network runtime')

if __name__ == '__main__':
    main()
