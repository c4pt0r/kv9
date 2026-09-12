#!/usr/bin/env python3
"""Compile actual Raft adapter fault controls in an isolated workspace copy.

Run locally under the source-build disk/CPU reservation. Uses the shared retained
build lock; never mutates the caller's checkout or dispatches hosted CI.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
RAW = 'crates/raft/src/rawnode.rs'
POLICY = 'crates/raft/src/lease_policy.rs'
STORAGE = 'crates/raft/src/storage.rs'
PREFIX = 'rawnode::lease_gate::tests::'
QUARANTINE = PREFIX + 'quarantine_blocks_campaign_ticks_timeout_and_forced_vote_until_exact_boundary'
CONTROLS = [
    ('implicit-prevote', RAW, '                    | MsgRequestPreVoteResponse\n', '',
     PREFIX + 'permitted_prevote_response_rechecks_clock_before_implicit_self_vote'),
    ('tick-self-vote', RAW,
     'if g.raw.raft.state != StateRole::Leader && g.lease_may_vote().is_err() {',
     'if false && g.raw.raft.state != StateRole::Leader && g.lease_may_vote().is_err() {', QUARANTINE),
    ('explicit-campaign', RAW, '        g.lease_may_vote()?;\n', '', QUARANTINE),
    ('forced-vote', RAW, '                    | MsgRequestVote\n', '', QUARANTINE),
    ('disable-on-restart', RAW, 'if storage.recovered_lease_epoch().is_some() {', 'if false {',
     PREFIX + 'restart_increments_epoch_restarts_quarantine_and_cannot_disable_or_shorten_policy'),
    ('epoch-before-sync', STORAGE,
     'Self::write_record(&self.io_metrics, file, REC_LEASE_EPOCH, &next.encode())?;',
     'Self::write_record_unsynced(&self.io_metrics, file, REC_LEASE_EPOCH, &next.encode())?;',
     'storage::persistence_model::lease_epochs_survive_crash_and_are_strictly_increasing'),
    ('changed-policy', POLICY, 'Some(old) if &old.policy == policy =>',
     'Some(old) if old.policy.configuration == policy.configuration =>',
     'lease_policy::tests::every_policy_field_is_immutable_and_incarnation_cannot_wrap'),
    ('snapshot-membership', RAW, 'msg.get_msg_type() == MsgSnapshot || ', '',
     PREFIX + 'snapshot_and_membership_cannot_change_fixed_lease_configuration'),
]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def save(path, data):
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')


def main():
    os.environ.update(CARGO_TARGET_DIR=str(ROOT / 'target'), CARGO_BUILD_JOBS='4',
                      CARGO_NET_OFFLINE='true')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    if out.is_relative_to(ROOT):
        raise RuntimeError('control output must be outside the checkout')
    out.mkdir()
    source = out / 'source'
    source.mkdir()
    names = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others',
        '--exclude-standard'], cwd=ROOT).decode().split('\0')
    names = sorted({n for n in names if n and (n.startswith(('crates/', 'proto/', '.cargo/'))
        or n in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain', 'rust-toolchain.toml', 'rustfmt.toml'))})
    inputs = {}
    total = 0
    for name in names:
        original = ROOT / name
        if original.is_symlink() or not original.is_file():
            raise RuntimeError('unexpected workspace source type: ' + name)
        data = original.read_bytes()
        total += len(data)
        if len(data) > 2 * 1024**2 or total > 64 * 1024**2 or len(names) > 10_000:
            raise RuntimeError('workspace source capsule exceeds bound')
        target = source / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        inputs[name] = digest(data)
    save(out / 'source-inputs.json', inputs)
    save(out / 'controls.json', CONTROLS)
    originals = {name: (source / name).read_text() for name in (RAW, POLICY, STORAGE)}
    spec = importlib.util.spec_from_file_location('lease_vote_cache', ROOT / 'scripts/build_cache.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    result = {'complete': False, 'source_bytes': total, 'source_files': len(inputs),
              'commands': [], 'controls': []}

    def command(cache, argv, directory, label, expected=0):
        directory.mkdir(exist_ok=True)
        row = {'argv': argv, 'started_ns': time.time_ns(), 'expected_exit_code': expected}
        result['commands'].append(row)
        save(out / 'result.json', result)
        with (directory / (label + '.stdout')).open('x') as stdout, \
             (directory / (label + '.stderr')).open('x') as stderr:
            cache.require_lock()
            completed = subprocess.run(argv, cwd=source, env=cache.env, stdout=stdout,
                stderr=stderr, timeout=1200, pass_fds=(cache.lock.fileno(),))
        row.update(exit_code=completed.returncode, ended_ns=time.time_ns())
        save(out / 'result.json', result)
        if completed.returncode != expected:
            raise RuntimeError(f'unexpected exit: {directory.name}/{label}: {completed.returncode}')
        return directory / (label + '.stdout')

    def compile_tests(cache, directory, integration=False):
        argv = ['cargo', 'test', '--locked', '-p', 'kv9-raft']
        argv += ['--test', 'lease_restart'] if integration else ['--lib']
        argv += ['--no-run', '--message-format=json-render-diagnostics']
        output = command(cache, argv, directory, 'compile')
        cache.check_artifacts(output)
        artifacts = [json.loads(line) for line in output.read_text().splitlines()]
        target = 'lease_restart' if integration else 'kv9_raft'
        selected = [r for r in artifacts if r.get('reason') == 'compiler-artifact'
            and r['target']['name'] == target and r.get('executable')]
        if len(selected) != 1 or selected[0]['fresh']:
            raise RuntimeError('expected a freshly compiled selected test executable')
        binary = Path(selected[0]['executable'])
        if binary.stat().st_size > 512 * 1024**2:
            raise RuntimeError('test executable exceeds bound')
        with binary.open('rb') as stream:
            binary_sha = hashlib.file_digest(stream, 'sha256').hexdigest()
        save(directory / 'executable.json', {'path': str(binary), 'sha256': binary_sha,
             'retained': False, 'source_variant': directory.name})
        return binary

    def test(cache, binary, directory, name, expected=0):
        stdout = command(cache, [str(binary), name, '--exact', '--nocapture'], directory,
                         name.split('::')[-1], expected)
        text = stdout.read_text()
        marker = f'test {name} ... ' + ('ok' if expected == 0 else 'FAILED')
        if marker not in text or ('1 passed; 0 failed' if expected == 0 else '0 passed; 1 failed') not in text:
            raise RuntimeError('exact semantic test result missing')

    try:
        with module.BuildCache(source, out, False, inputs) as cache:
            directory = out / 'feature-disabled'
            binary = compile_tests(cache, directory, True)
            test(cache, binary, directory, 'feature_disabled_library_refuses_a_durably_marked_lease_voter')
            directory = out / 'baseline'
            binary = compile_tests(cache, directory)
            for name in sorted({c[4] for c in CONTROLS}):
                test(cache, binary, directory, name)
            for label, name, old, new, target in CONTROLS:
                text = originals[name]
                if text.count(old) != 1:
                    raise RuntimeError('ambiguous mutation: ' + label)
                mutated = text.replace(old, new)
                (source / name).write_text(mutated)
                directory = out / label
                directory.mkdir()
                (directory / 'mutated-source.rs').write_text(mutated)
                row = {'name': label, 'path': name, 'test': target,
                       'original_sha256': digest(text.encode()), 'mutant_sha256': digest(mutated.encode())}
                result['controls'].append(row)
                try:
                    binary = compile_tests(cache, directory)
                    test(cache, binary, directory, target, 101)
                    row['expected_failure_observed'] = True
                finally:
                    (source / name).write_text(text)
                save(out / 'result.json', result)
                print('PASS control:', label, flush=True)
            directory = out / 'restored'
            binary = compile_tests(cache, directory)
            for name in sorted({c[4] for c in CONTROLS}):
                test(cache, binary, directory, name)
            for name, sha in inputs.items():
                if digest((source / name).read_bytes()) != sha or digest((ROOT / name).read_bytes()) != sha:
                    raise RuntimeError('source restoration or caller source identity changed: ' + name)
            result['complete'] = True
    except BaseException as error:
        result['failure'] = repr(error)
        raise
    finally:
        for name, text in originals.items():
            (source / name).write_text(text)
        save(out / 'result.json', result)
    print('PASS: feature-disabled recovery and eight actual-source vote binding controls', flush=True)


if __name__ == '__main__':
    main()
