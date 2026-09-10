#!/usr/bin/env python3
"""Require compiled semantic failures for isolated point/batch resident read mutations.

Every control runs one exact baseline test, its semantic mutant, and the restored
source. All copied inputs and attempt logs remain under a fresh output directory.
The Cargo target is private to that directory, regardless of the inherited target.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
MEM = 'crates/engine/src/mem.rs'
RUNTIME = 'crates/server/src/runtime.rs'
GRPC = 'crates/server/src/grpc.rs'
BATCH_TESTS = 'runtime::tests::async_batch_read_tests::'
BATCH_BUDGET_TEST = BATCH_TESTS + 'oversized_resident_batch_moves_its_captured_authorized_view_into_the_job'
POINT_PREPARATION = '''        self.finish_prepared_read(established, move |backend, view| {
            backend.prepared_get_from_view(view, &ctx, &key)
        })'''
COMPLETED_COUNTER = '                self.admission.record_prepared_read(kind, true);'
CASES = [
    ('waiting-for-index-writer', MEM, [(
        'self.state.try_read()',
        'self.state.read().map_err(std::sync::TryLockError::Poisoned)')],
     'mem::resident_tests::resident_snapshot_never_waits_for_a_writer_and_owns_its_version',
     'resident snapshot waited for an index writer'),
    ('dispatching-completed-read', GRPC, [(
        COMPLETED_COUNTER,
        '''                let value = if matches!(kind, PreparedReadKind::Point) {
                    tokio::task::spawn_blocking(move || value).await.unwrap()
                } else {
                    value
                };
''' + COMPLETED_COUNTER)],
     'grpc::tests::admission::completed_public_get_does_not_wait_for_the_blocking_pool',
     'completed public GET was dispatched behind a blocked engine worker'),
    ('discarding-contended-read', RUNTIME, [(
        POINT_PREPARATION,
        '''        if self.try_ensure_serving().is_none() {
            return Ok(crate::api::RawReadJob::Completed(None));
        }
''' + POINT_PREPARATION)],
     'runtime::tests::a_contended_prepared_read_checks_the_epoch_when_its_engine_job_runs',
     'a contended lifecycle check must defer the engine job'),
    ('bypassing-resident-context-gate', RUNTIME, [(
        'let view = self.check_read_view(view, ctx, KeySpan::Point(key))?;',
        '// Deliberately omit the context gate for this compiled control.')],
     'runtime::tests::a_resident_prepared_read_finishes_on_one_authorized_version',
     'a fresh prepared read accepted the obsolete epoch'),
    ('bypassing-batch-value-budget', RUNTIME, [(
        '                MAX_RESIDENT_BATCH_READ_BYTES,',
        '                usize::MAX,')],
     BATCH_BUDGET_TEST,
     'large or repeated values must defer materialization before copying the full batch'),
    ('recapturing-batch-fallback-view', RUNTIME, [(
        '''            // No second barrier, snapshot, or current-epoch lookup: both the
            // authorization and the deferred copy refer to the captured view.
            let read = LeaderRead::new(view.as_ref(), true, None)?;''',
        '''            // Deliberately replace only the already-authorized snapshot.
            // No new barrier or context check is added by this control.
            drop(view);
            let view = self.node.meta_raft.store.engine().snapshot()?;
            let read = LeaderRead::new(view.as_ref(), true, None)?;''')],
     BATCH_BUDGET_TEST,
     'captured byte-budget fallback lost its old ordered view at slot'),
    # One defect (omitted batch context validation) at both materialization
    # branches. Covering both avoids a scheduling-dependent false green if
    # the fresh stale-epoch read happens to encounter lifecycle contention.
    ('bypassing-batch-context-gate', RUNTIME, [(
        '''        let view = self.check_read_view(
            view,
            ctx,
            KeySpan::Batch(keys.iter().map(|key| key.as_slice()).collect()),
        )?;
''',
        ''), (
        '''            let authorized = self.check_read_view(
                Box::new(view.as_ref()),
                &ctx,
                KeySpan::Batch(keys.iter().map(|key| key.as_slice()).collect()),
            )?;''',
        '''            let authorized: Box<dyn ReadView + '_> = Box::new(view.as_ref());''')],
     BATCH_TESTS + 'resident_batch_keeps_its_authorized_values_after_both_epoch_changes',
     'fresh batch accepted the old epoch'),
]


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def inventory(tree):
    return {str(path.relative_to(tree)): digest(path)
            for path in sorted(tree.rglob('*')) if path.is_file()}


def mutate(original, edits, label):
    source = original
    for before, after in edits:
        if source.count(before) != 1 or before == after:
            raise RuntimeError(f'{label}: source anchor is not unique')
        source = source.replace(before, after, 1)
    marker = '\n#[cfg(test)]\nmod '
    if marker not in original or source.split(marker, 1)[1] != original.split(marker, 1)[1]:
        raise RuntimeError(f'{label}: mutation altered tests')
    return source


def validate(log, exit_code, phase, test, failure, artifact):
    if not artifact:
        raise RuntimeError('expected one compiled selected-crate test executable')
    if len(re.findall(r'^running 1 test\r?$', log, re.MULTILINE)) != 1:
        raise RuntimeError('expected exactly one executed test')
    outcome = 'FAILED' if phase == 'mutant' else 'ok'
    named = rf'^test {re.escape(test)} \.\.\. {outcome}\r?$'
    if not re.search(named, log, re.MULTILINE):
        raise RuntimeError('exactly named test did not produce the required outcome')
    passed, failed = (0, 1) if phase == 'mutant' else (1, 0)
    summary = rf'^test result: {outcome}\. {passed} passed; {failed} failed; 0 ignored;'
    if not re.search(summary, log, re.MULTILINE):
        raise RuntimeError('test totals do not match one selected non-ignored test')
    if phase == 'mutant':
        if exit_code != 101 or failure not in log:
            raise RuntimeError('compiled mutant missed the intended semantic assertion')
    elif exit_code != 0:
        raise RuntimeError('valid source was rejected')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--timeout-seconds', type=int, default=600)
    args = parser.parse_args()
    if args.timeout_seconds <= 0:
        parser.error('--timeout-seconds must be positive')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    tree = output / 'source'
    target = output / 'cargo-target'
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_TERM_COLOR='never')
    manifest = dict(version=1, accepted=False, controls=[],
                    revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                    source_status=subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT, text=True),
                    runner_sha256=digest(__file__), cargo_target=str(target),
                    cpu_affinity=sorted(os.sched_getaffinity(0)) if hasattr(os, 'sched_getaffinity') else None,
                    scope='Compiled semantic implementation controls against retained copied inputs; not a proof, fault-matrix acceptance, or production benchmark.')

    def save():
        (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

    save()
    shutil.copy2(__file__, output / 'runner.py')
    try:
        tree.mkdir()
        for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates', 'src', 'proto']:
            source = ROOT / name
            if source.is_dir():
                shutil.copytree(source, tree / name)
            else:
                shutil.copy2(source, tree / name)
        manifest['source_files'] = inventory(tree)
        originals = {name: (tree / name).read_text() for name in (MEM, RUNTIME, GRPC)}
        # The new resident borrower and exact selected tests are dependencies
        # too: bind all retained build inputs, not only mutation target files.
        frozen_root = {name: digest(ROOT / name) for name in manifest['source_files']}
        if manifest['source_files'] != frozen_root:
            raise RuntimeError('reviewed build input changed during source snapshot')
        save()
        # Validate every anchor and unchanged test body before paying for a build.
        mutants = {name: mutate(originals[path], edits, name)
                   for name, path, edits, _, _ in CASES}
        for name, relative, _, test, failure in CASES:
            folder = output / name
            folder.mkdir()
            original, mutant = originals[relative], mutants[name]
            (folder / 'original.rs').write_text(original)
            (folder / 'mutant.rs').write_text(mutant)
            case = dict(name=name, source=relative, test=test,
                        expected_failure=failure, runs=[], accepted=False)
            manifest['controls'].append(case)
            save()
            try:
                for phase, source in [('baseline', original), ('mutant', mutant), ('restored', original)]:
                    (tree / relative).write_text(source)
                    expected = dict(manifest['source_files'])
                    expected[relative] = digest(tree / relative)
                    package = 'kv9-' + relative.split('/')[1]
                    command = ['cargo', 'test', '--locked', '-p', package, '--lib',
                               '--message-format=json-render-diagnostics', test, '--', '--exact']
                    log_path = folder / f'{phase}.log'
                    run = dict(phase=phase, command=command, started_unix_ns=time.time_ns(),
                               source_sha256=expected[relative], validation_passed=False)
                    case['runs'].append(run)
                    save()
                    with log_path.open('w') as stream:
                        try:
                            process = subprocess.Popen(command, cwd=tree, env=env, text=True,
                                                       stdout=stream, stderr=subprocess.STDOUT,
                                                       start_new_session=True)
                            run['process_id'] = process.pid
                            run['exit_code'] = process.wait(timeout=args.timeout_seconds)
                        except subprocess.TimeoutExpired:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait()
                            run.update(exit_code=None, timed_out=True, process_group_killed=True)
                    run.update(ended_unix_ns=time.time_ns(), log_sha256=digest(log_path))
                    log = log_path.read_text()
                    artifacts, compile_errors = [], []
                    for line in log.splitlines():
                        if not line.startswith('{'):
                            continue
                        try:
                            item = json.loads(line)
                        except json.JSONDecodeError:
                            continue
                        if item.get('reason') == 'compiler-message' and item['message']['level'] == 'error':
                            compile_errors.append(item['message']['message'])
                        if (item.get('reason') == 'compiler-artifact'
                                and item['target']['name'] == package.replace('-', '_')
                                and item['profile']['test'] and item.get('executable')):
                            artifacts.append(item)
                    run['compile_errors'] = compile_errors
                    try:
                        if inventory(tree) != expected:
                            raise RuntimeError('copied inputs changed outside the selected production mutation')
                        if compile_errors:
                            raise RuntimeError('compile error cannot count as a semantic failure')
                        artifact = artifacts[0] if len(artifacts) == 1 else None
                        if artifact:
                            executable = Path(artifact['executable']).resolve()
                            if not executable.is_relative_to(target):
                                raise RuntimeError('test executable escaped the private target')
                            run['executable'] = dict(path=str(executable), sha256=digest(executable),
                                                     bytes=executable.stat().st_size,
                                                     features=artifact['features'])
                        validate(log, run['exit_code'], phase, test, failure, artifact)
                        run['validation_passed'] = True
                    except Exception as error:
                        run['validation_error'] = str(error)
                    save()
                    # A bad baseline must not be explained away by a mutant.
                    if phase == 'baseline' and not run['validation_passed']:
                        raise RuntimeError(f'{name}/baseline: {run["validation_error"]}')
                if not all(run['validation_passed'] for run in case['runs']):
                    raise RuntimeError(f'{name}: a control phase failed validation; all attempted logs retained')
                case['accepted'] = True
                save()
                print(f'PASS: {name} baseline, intended failure and restored source', flush=True)
            finally:
                (tree / relative).write_text(original)
        if inventory(tree) != manifest['source_files']:
            raise RuntimeError('final source restoration differs from original snapshot')
        if any(digest(ROOT / path) != expected for path, expected in frozen_root.items()):
            raise RuntimeError('reviewed build input changed during controls; retained snapshot cannot attest current source')
        manifest['accepted'] = True
        save()
        print(f'PASS: {len(CASES)} isolated resident read implementation controls', flush=True)
    except Exception as error:
        manifest['error'] = repr(error)
        save()
        raise


if __name__ == '__main__':
    main()
