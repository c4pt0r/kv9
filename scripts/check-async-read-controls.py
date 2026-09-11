#!/usr/bin/env python3
"""Require compiled semantic failures for isolated asynchronous read mutations.

Every control runs one exact baseline test, its semantic mutant, and the restored
source. All copied inputs and attempt logs remain under a fresh output directory.
The Cargo target is private by default. An explicitly selected compiler cache
can be reused when the caller owns it exclusively for the entire control run.
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
ASYNC = 'crates/raft/src/async_read.rs'
DRIVER = 'crates/raft/src/driver.rs'
PEER = 'crates/raft/src/rawnode.rs'
TESTS = 'async_read::tests::'
CONFIRMATION = TESTS + 'exact_first_confirmation_and_apply_coverage_are_both_required'
COVERAGE_FAILURE = 'confirmation was downgraded or apply coverage bypassed'
WAKE = '''            if self.async_reads.snapshot().queued != 0 && self.peer.read_admission_can_progress() {
                self.peer.work_signal.notify();
            }'''
CASES = [
    ('ignored-context-sequence', ASYNC, [(
        'let Some(group) = state.active.get_mut(&context) else {',
        '''let Some(group) = state.active.iter_mut().find_map(|(key, group)| {
            (key[..16] == context[..16]).then_some(group)
        }) else {''')], CONFIRMATION,
     'assertion failed: !queue.confirm(&context(9), 1)'),
    ('overwritten-first-confirmation', ASYNC, [(
        'if group.confirmed.is_none() {', 'if true {')],
     CONFIRMATION, COVERAGE_FAILURE),
    ('omitted-apply-coverage', ASYNC, [(
        '.filter(|index| applied.is_some_and(|at| at >= *index))',
        '.filter(|_index| applied.is_some())')],
     CONFIRMATION, COVERAGE_FAILURE),
    ('submitted-cancelled-request', ASYNC, [(
        'if request.abandoned() {', 'if false && request.abandoned() {')],
     TESTS + 'cancellation_never_submits_and_capacity_is_bounded_until_owner_cleanup',
     'cancelled read reached Raft'),
    ('released-claimed-reservation', ASYNC, [(
        '            let admitted = read_index(context.to_vec());',
        '''            for request in &mut members {
                if let Some(owner) = request.owner.upgrade() {
                    let mut state = owner.state.lock().expect("async read queue poisoned");
                    state.reserved.remove(&request.context);
                    state.in_flight -= 1;
                }
                request.owner = Weak::new();
            }
            let admitted = read_index(context.to_vec());''')],
     TESTS + 'stop_reaches_unclaimed_suffix_while_one_callback_is_blocked',
     'claimed storage was released before callback completion'),
    ('omitted-election-progress-wake', DRIVER, [(WAKE, '')],
     'driver::tests::async_read_deferred_before_election_commit_progresses_without_a_tick',
     'election-ready admission waited for another tick'),
    ('deferred-admission-self-spin', ASYNC, [(
        'if retained && !deferred {', 'if retained || deferred {')],
     TESTS + 'deferred_admission_has_a_finite_turn_and_does_not_self_spin',
     'deferred admission spun without new Raft work'),
    ('late-member-after-quorum-start', ASYNC, [(
        '            if admitted == Some(true) {\n                state.admitted_groups',
        '            members.extend(state.queued.drain(..));\n            if admitted == Some(true) {\n                state.admitted_groups')],
     TESTS + 'sealed_members_share_one_confirmation_and_late_arrivals_require_another',
     'arrival joined a group after its quorum request started'),
    ('representative-dependent-confirmation', ASYNC, [(
        '        if group.confirmed.is_none() {',
        '''        if !group.members.iter().any(|request| request.context == context) {
            return false;
        }
        if group.confirmed.is_none() {''')],
     TESTS + 'canceled_representative_leaves_group_identity_for_its_live_members',
     'representative cancellation destroyed confirmation routing'),
    ('unbounded-group-inspection', ASYNC, [(
        'state.queued.len().min(TURN_REQUESTS)', 'state.queued.len().min(MAX_REQUESTS)')],
     TESTS + 'full_queue_emits_two_bounded_groups_and_retains_every_member',
     'sealed group exceeded its inspection or membership bound'),
    ('per-member-quorum-broadcast', ASYNC, [(
        '            let admitted = read_index(context.to_vec());',
        '''            let mut admitted = Ok(true);
            for request in &members {
                admitted = read_index(request.context.to_vec());
                if !matches!(admitted, Ok(true)) { break; }
            }''')],
     'driver::tests::sealed_read_groups_use_one_heartbeat_per_follower_and_fence_late_reads',
     'sealed group did not retain all submitted members'),
    ('bypassed-pending-read-credit', PEER, [(
        'if g.raw.raft.pending_read_count() >= MAX_PENDING_READ_INDEX {',
        'if false && g.raw.raft.pending_read_count() >= MAX_PENDING_READ_INDEX {')],
     'driver::read_credit_tests::cap_two_seals_late_members_until_distinct_confirmation',
     'read credit admitted a third unconfirmed context'),
    ('full-read-credit-owner-spin', PEER, [(
        '&& g.raw.raft.pending_read_count() < MAX_PENDING_READ_INDEX)',
        '&& g.raw.raft.pending_read_count() < usize::MAX)')],
     'driver::read_credit_tests::full_credit_owner_parks_and_confirmation_wakes_without_tick',
     'full read credit self-woke instead of parking the real owner'),
    ('synchronous-read-credit-bypass', PEER, [(
        'if g.raw.raft.pending_read_count() >= MAX_PENDING_READ_INDEX {',
        'if false && g.raw.raft.pending_read_count() >= MAX_PENDING_READ_INDEX {')],
     'driver::read_credit_tests::synchronous_read_shares_credit_and_requires_its_own_confirmation',
     'synchronous admission bypassed outstanding protocol credit'),
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
    marker = '#[cfg(test)]\nmod tests {'
    if original.count(marker) != 1 or source.split(marker, 1)[1] != original.split(marker, 1)[1]:
        raise RuntimeError(f'{label}: mutation altered tests')
    return source


def validate(log, exit_code, phase, test, failure, artifact):
    if not artifact:
        raise RuntimeError('expected one compiled kv9-raft test executable')
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
    parser.add_argument('--cargo-target', type=Path,
                        help='explicit compiler cache owned exclusively by this run; default: private output cache')
    parser.add_argument('--timeout-seconds', type=int, default=600)
    args = parser.parse_args()
    if args.timeout_seconds <= 0:
        parser.error('--timeout-seconds must be positive')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    tree = output / 'source'
    target = (args.cargo_target or output / 'cargo-target').resolve()
    if target.is_relative_to(tree) or tree.is_relative_to(target):
        parser.error('Cargo target and copied source must not overlap')
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
        originals = {name: (tree / name).read_text() for name in (ASYNC, DRIVER, PEER)}
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
                    command = ['cargo', 'test', '--locked', '-p', 'kv9-raft', '--lib',
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
                                and item['target']['name'] == 'kv9_raft'
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
                                raise RuntimeError('test executable escaped the selected Cargo target')
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
        print(f'PASS: {len(CASES)} isolated asynchronous read implementation controls', flush=True)
    except Exception as error:
        manifest['error'] = repr(error)
        save()
        raise


if __name__ == '__main__':
    main()
