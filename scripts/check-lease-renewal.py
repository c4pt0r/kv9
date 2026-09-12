#!/usr/bin/env python3
"""Compile actual lease renewal and pump fault controls in an isolated workspace copy.

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
CONTROLS = [('ack-before-pump',
  'crates/raft/src/rawnode/lease_gate.rs',
  '                        self.confirmed = Some(renewal);',
  '                        let progress = Progress { configuration: self.epoch.policy.configuration, leader: '
  'raw.raft.leader_id, term: raw.raft.term, committed: raw.raft.raft_log.committed, committed_term: '
  'raw.raft.term };\n'
  '                        let _ = leader.publish(now, progress, renewal);\n'
  '                        self.confirmed = Some(renewal);',
  'rawnode::lease_gate::tests::renewal_certificate_requires_exact_grant_and_successful_owning_pump'),
 ('later-input-in-old-pump',
  'crates/raft/src/rawnode/lease_gate.rs',
  'if let Some(confirmed) = batch.confirmed {',
  'if let Some(confirmed) = batch.confirmed.or(self.confirmed.take()) {',
  'rawnode::lease_gate::tests::grant_after_capture_belongs_to_the_next_pump'),
 ('send-after-deadline',
  'crates/raft/src/rawnode/lease_gate.rs',
  'if leader.validate_renewal(now, progress, request).is_ok() {',
  'if true {',
  'rawnode::lease_gate::tests::delayed_request_grant_and_quorum_publication_cannot_extend_send_deadline'),
 ('foreign-pump-publication',
  'crates/raft/src/rawnode/lease_gate.rs',
  'if !Arc::ptr_eq(&batch.owner, &self.owner) || batch.sequence != self.captured {',
  'if false {',
  'rawnode::lease_gate::tests::pump_publication_rejects_foreign_peer_and_superseded_turn'),
 ('failed-driver-retains-authority',
  'crates/raft/src/driver.rs',
  '        } else {\n            self.drain.abort_lease();\n            self.async_applies.close();',
  '        } else {\n            self.async_applies.close();',
  'rawnode::lease_gate::tests::failed_driver_apply_never_publishes_staged_grant'),
 ('transfer-retains-authority',
  'crates/raft/src/rawnode.rs',
  '        if let Some(lease) = &mut g.lease {\n            lease.revoke_leader();\n        }',
  '        if let Some(_lease) = &mut g.lease {}',
  'rawnode::lease_gate::tests::transfer_and_clock_failure_revoke_renewals_without_erasing_voter_hold'),
 ('policy-mismatch',
  'crates/raft/src/rawnode/lease_wire.rs',
  '        || n(93) != policy.margin_ns\n',
  '',
  'rawnode::lease_wire::tests::envelope_rejects_wrong_identity_policy_and_raft_payload'),
 ('ordinary-echo-as-grant',
  'crates/raft/src/rawnode/lease_wire.rs',
  '        (2, MessageType::MsgHeartbeatResponse) => Kind::Grant,',
  '        (1 | 2, MessageType::MsgHeartbeatResponse) => Kind::Grant,',
  'rawnode::lease_wire::tests::ordinary_heartbeat_echo_and_read_context_never_become_grants')]


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
    originals = {name: (source / name).read_text() for name in sorted({control[1] for control in CONTROLS})}
    spec = importlib.util.spec_from_file_location('lease_renewal_cache', ROOT / 'scripts/build_cache.py')
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
    print('PASS: feature-disabled recovery and eight actual-source lease renewal controls', flush=True)


if __name__ == '__main__':
    main()
