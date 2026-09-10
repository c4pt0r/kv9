#!/usr/bin/env python3
"""Require compiled semantic failures for isolated receipt FIFO mutations.

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
DRIVER = 'crates/raft/src/driver.rs'
TEST = 'driver::tests::receipt_fifo_preserves_order_verdicts_and_allocation_across_wraparound'
CASES = [
    ('evicting-newest-receipt', DRIVER, [(
        '        let _ = applied.pop_front();',
        '        let _ = applied.pop_back();')],
     TEST, 'receipt FIFO changed the retained chronological suffix'),
    ('inserting-at-wrong-end', DRIVER, [(
        '    applied.push_back(RingEntry {',
        '    applied.push_front(RingEntry {')],
     TEST, 'receipt FIFO changed the retained chronological suffix'),
    ('growing-full-allocation-before-eviction', DRIVER, [(
        """    if applied.len() == APPLIED_RING {
        let _ = applied.pop_front();
    }
    applied.push_back(RingEntry {
        index,
        term,
        outcome,
    });""",
        """    applied.push_back(RingEntry {
        index,
        term,
        outcome,
    });
    if applied.len() > APPLIED_RING {
        let _ = applied.pop_front();
    }""")],
     TEST, 'full receipt FIFO grew its allocation'),
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
        originals = {name: (tree / name).read_text() for name in (DRIVER,)}
        frozen_root = {name: digest(ROOT / name) for name in originals}
        if any(digest(tree / name) != frozen_root[name] for name in originals):
            raise RuntimeError('reviewed core changed during source snapshot')
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
            raise RuntimeError('reviewed core changed during controls; retained snapshot cannot attest current source')
        manifest['accepted'] = True
        save()
        print(f'PASS: {len(CASES)} isolated receipt FIFO implementation controls', flush=True)
    except Exception as error:
        manifest['error'] = repr(error)
        save()
        raise


if __name__ == '__main__':
    main()
