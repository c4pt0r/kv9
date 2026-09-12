#!/usr/bin/env python3
"""Compile actual lease read-view, admission and service fault controls in an isolated workspace copy.

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
CONTROLS = [('skip-final-lease-validation', 'crates/raft/src/driver/lease_read.rs', '        if !self\n            .peer\n            .try_finish_lease_read(read, through.index)\n            .map_err(ReadIndexError::Failed)?\n        {', '        if false {', 'driver::lease_read::tests::expiry_after_snapshot_capture_discards_local_authority_and_queues_fallback', 'read succeeded before its required authority/application'), ('cache-old-commit-frontier', 'crates/raft/src/lease.rs', '            committed: progress.committed,', '            committed: 1,', 'driver::lease_read::tests::fresh_commit_frontier_cannot_authorize_an_unapplied_view', 'cached renewal commit ignored a newer committed write'), ('bypass-apply-coverage', 'crates/raft/src/lease.rs', '        if view_index < ticket.committed {', '        if false {', 'driver::lease_read::tests::fresh_commit_frontier_cannot_authorize_an_unapplied_view', 'cached renewal commit ignored a newer committed write'), ('foreign-read-installation', 'crates/raft/src/rawnode/lease_gate.rs', '        if !Arc::ptr_eq(&read.owner, &self.owner) {', '        if false {', 'driver::lease_read::tests::a_read_ticket_from_another_peer_installation_cannot_authorize_a_view', 'foreign peer installation authorized a lease view'), ('unbounded-local-admission', 'crates/raft/src/async_read.rs', '\n        if state.in_flight >= MAX_REQUESTS {', '\n        if false {', 'driver::lease_read::tests::a_valid_lease_cannot_bypass_the_existing_read_admission_limit', 'valid lease bypassed the shared read admission limit'), ('reset-fallback-deadline', 'crates/raft/src/async_read.rs', '    pub(crate) fn submit(mut self) -> std::result::Result<ReadTicket, ReadIndexError> {\n        let (sender, receiver) = oneshot::channel();', '    pub(crate) fn submit(mut self) -> std::result::Result<ReadTicket, ReadIndexError> {\n        self.deadline = Instant::now() + self.deadline.duration_since(self.started);\n        let (sender, receiver) = oneshot::channel();', 'async_read::tests::local_fallback_keeps_one_slot_original_context_and_absolute_deadline', 'local fallback reset the original request budget'), ('replace-deferred-get-view', 'crates/server/src/runtime.rs', '                self.ensure_serving()?;\n                self.prepared_get_from_view(view, &ctx, &key)', '                self.ensure_serving()?;\n                let view = self.node.meta_raft.store.engine().try_resident_snapshot().expect("mutation snapshot");\n                self.prepared_get_from_view(view, &ctx, &key)', 'runtime::tests::lease_read_tests::deferred_lease_get_checks_metadata_and_values_on_its_retained_view', 'deferred lease read lost its retained metadata authorization'), ('replace-deferred-batch-view', 'crates/server/src/runtime.rs', '                self.ensure_serving()?;\n                self.prepared_batch_get_from_view(view, &ctx, &keys)', '                self.ensure_serving()?;\n                let view = self.node.meta_raft.store.engine().try_resident_snapshot().expect("mutation snapshot");\n                self.prepared_batch_get_from_view(view, &ctx, &keys)', 'runtime::tests::lease_read_tests::deferred_lease_batch_checks_metadata_and_values_on_its_retained_view', 'deferred lease read lost its retained metadata authorization'), ('replace-large-batch-view', 'crates/server/src/runtime.rs', '            let read = LeaderRead::new(view.as_ref(), true, None)?;\n            RawExecutor.batch_get(&read, ctx.keyspace, &keys)', '            let view = self.node.meta_raft.store.engine().try_resident_snapshot().expect("mutation snapshot");\n            let read = LeaderRead::new(view.as_ref(), true, None)?;\n            RawExecutor.batch_get(&read, ctx.keyspace, &keys)', 'runtime::tests::lease_read_tests::large_lease_batch_defers_copy_without_changing_the_authorized_view', 'deferred lease read replaced its authorized metadata/data view')]


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
    spec = importlib.util.spec_from_file_location('lease_read_cache', ROOT / 'scripts/build_cache.py')
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

    def compile_tests(cache, directory, integration=False, package="kv9-raft"):
        argv = ['cargo', 'test', '--locked', '-p', package]
        argv += ['--test', 'lease_restart'] if integration else ['--lib']
        argv += ['--no-run', '--message-format=json-render-diagnostics']
        output = command(cache, argv, directory, 'compile')
        cache.check_artifacts(output)
        artifacts = [json.loads(line) for line in output.read_text().splitlines()]
        target = 'lease_restart' if integration else package.replace('-', '_')
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

    def test(cache, binary, directory, name, expected=0, assertion=None):
        stdout = command(cache, [str(binary), name, '--exact', '--nocapture'], directory,
                         name.split('::')[-1], expected)
        text = stdout.read_text()
        marker = f'test {name} ... ' + ('ok' if expected == 0 else 'FAILED')
        if marker not in text or ('1 passed; 0 failed' if expected == 0 else '0 passed; 1 failed') not in text:
            raise RuntimeError('exact semantic test result missing')
        if assertion is not None and assertion not in stdout.with_suffix('.stderr').read_text():
            raise RuntimeError('mutant did not fail at its declared semantic assertion')

    try:
        with module.BuildCache(source, out, False, inputs) as cache:
            directory = out / 'feature-disabled'
            binary = compile_tests(cache, directory, True)
            test(cache, binary, directory, 'feature_disabled_library_refuses_a_durably_marked_lease_voter')
            for package in ['kv9-raft', 'kv9-server']:
                directory = out / ('baseline-' + package)
                binary = compile_tests(cache, directory, package=package)
                for name in sorted({c[4] for c in CONTROLS if ('kv9-server' if c[4].startswith('runtime::') else 'kv9-raft') == package}):
                    test(cache, binary, directory, name)
            for label, name, old, new, target, assertion in CONTROLS:
                text = originals[name]
                if text.count(old) != 1:
                    raise RuntimeError('ambiguous mutation: ' + label)
                mutated = text.replace(old, new)
                (source / name).write_text(mutated)
                directory = out / label
                directory.mkdir()
                (directory / 'mutated-source.rs').write_text(mutated)
                row = {'name': label, 'path': name, 'test': target,
                       'assertion': assertion, 'original_sha256': digest(text.encode()), 'mutant_sha256': digest(mutated.encode())}
                result['controls'].append(row)
                try:
                    binary = compile_tests(cache, directory, package='kv9-server' if target.startswith('runtime::') else 'kv9-raft')
                    test(cache, binary, directory, target, 101, assertion)
                    row['expected_failure_observed'] = True
                finally:
                    (source / name).write_text(text)
                save(out / 'result.json', result)
                print('PASS control:', label, flush=True)
            for package in ['kv9-raft', 'kv9-server']:
                directory = out / ('restored-' + package)
                binary = compile_tests(cache, directory, package=package)
                for name in sorted({c[4] for c in CONTROLS if ('kv9-server' if c[4].startswith('runtime::') else 'kv9-raft') == package}):
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
    print('PASS: feature-disabled recovery and nine actual-source lease read-view controls', flush=True)


if __name__ == '__main__':
    main()
