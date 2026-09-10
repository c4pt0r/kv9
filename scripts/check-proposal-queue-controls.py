#!/usr/bin/env python3
"""Run isolated proposal admission and certainty controls against the real Rust queue."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
STORAGE = 'crates/raft/src/proposal_queue.rs'
PREFIX = 'proposal_queue::tests::'
CASES = [
    ('cancelled-work-submitted', 'if !matches!(*state, Submission::Queued) {',
     'if false && !matches!(*state, Submission::Queued) {',
     PREFIX + 'cancelled_and_expired_requests_never_reach_submission',
     'cancelled or expired proposal reached Raft'),
    ('claimed-timeout-falsely-refused',
     'Submission::Claimed => Err(Error::ProposalUnconfirmed),',
     'Submission::Claimed => Err(refused(ProposalRefusal::Expired)),',
     PREFIX + 'claimed_timeout_retains_capacity_until_real_submission_returns',
     'claimed timeout was falsely definite'),
    ('early-handoff-reservation-release',
     '            if claimed {\n                let result = submit(',
     '            if claimed {\n                let queue = pending.reservation.queue.clone();\n'
     '                let bytes = pending.reservation.bytes;\n'
     '                pending.reservation.queue = Weak::new();\n'
     '                drop(Reservation { queue, bytes });\n'
     '                let result = submit(',
     PREFIX + 'claimed_ticket_drop_does_not_release_in_progress_reservation',
     'claimed reservation was released before actual submission returned'),
    ('unbounded-cancelled-inspection', 'for _ in 0..TURN_REQUESTS {\n            if bytes',
     'for _ in 0..(TURN_REQUESTS + 1) {\n            if bytes',
     PREFIX + 'cancelled_entries_consume_turn_budget_and_preserve_the_fifo_suffix',
     'cancelled work bypassed the turn inspection bound'),
    ('ignored-turn-byte-target', 'if bytes >= TURN_BYTES {',
     'if false && bytes >= TURN_BYTES {',
     PREFIX + 'byte_target_allows_one_legal_overshoot_and_keeps_the_suffix',
     'proposal turn byte target did not retain its suffix'),
    ('ignored-request-capacity', 'if state.in_flight >= self.0.max_requests {',
     'if false && state.in_flight >= self.0.max_requests {',
     PREFIX + 'count_bytes_and_oversize_refusals_do_not_change_the_ledger',
     'request-count admission bound was bypassed'),
    ('ignored-byte-capacity', 'if data.len() > self.0.max_bytes - state.bytes {',
     'if false && data.len() > self.0.max_bytes - state.bytes {',
     PREFIX + 'count_bytes_and_oversize_refusals_do_not_change_the_ledger',
     'encoded-byte admission bound was bypassed'),
    ('stop-reopens-admission', 'state.stopped = true;', 'state.stopped = false;',
     PREFIX + 'stop_and_destruction_refuse_only_unclaimed_work',
     'stopped queue admitted new work'),
]

def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    output = parser.parse_args().output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get(
        'CARGO_TARGET_DIR', str((ROOT / 'target').resolve())))
    manifest = dict(version=1, accepted=False, controls=[],
                    revision=subprocess.check_output(
                        ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                    source_status=subprocess.check_output(
                        ['git', 'status', '--porcelain'], cwd=ROOT, text=True),
                    runner_sha256=digest(Path(__file__).read_bytes()))

    def save():
        (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

    with tempfile.TemporaryDirectory(prefix='kv9-proposal-controls.') as temp:
        tree = Path(temp)
        for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates', 'src', 'proto']:
            source = ROOT / name
            if source.is_dir():
                shutil.copytree(source, tree / name)
            else:
                shutil.copy2(source, tree / name)
        manifest['source_files'] = {
            str(path.relative_to(tree)): digest(path.read_bytes())
            for path in sorted(tree.rglob('*')) if path.is_file()}
        save()
        target = tree / STORAGE
        original = target.read_text()
        for name, before, after, test, failure in CASES:
            if original.count(before) != 1:
                raise RuntimeError(f'{name}: source anchor is not unique')
            mutant = original.replace(before, after)
            folder = output / name
            folder.mkdir()
            (folder / 'original.rs').write_text(original)
            (folder / 'mutant.rs').write_text(mutant)
            case = dict(name=name, source=STORAGE, test=test,
                        expected_failure=failure, runs=[])
            manifest['controls'].append(case)
            save()
            for phase, source in [('baseline', original), ('mutant', mutant), ('restored', original)]:
                target.write_text(source)
                command = ['cargo', 'test', '--locked', '-p', 'kv9-raft', '--lib',
                           test, '--', '--exact']
                result = subprocess.run(command, cwd=tree, env=env, text=True,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                        timeout=180)
                log = result.stdout
                (folder / f'{phase}.log').write_text(log)
                case['runs'].append(dict(phase=phase, exit_code=result.returncode,
                                         source_sha256=digest(source.encode()),
                                         log_sha256=digest(log.encode()), command=command))
                save()
                if 'running 1 test\n' not in log:
                    raise RuntimeError(f'{name}/{phase}: expected exactly one selected test')
                if phase == 'mutant':
                    if result.returncode == 0 or failure not in log or '1 failed;' not in log:
                        raise RuntimeError(f'{name}: mutant missed the intended assertion')
                elif result.returncode or '1 passed; 0 failed;' not in log:
                    raise RuntimeError(f'{name}/{phase}: valid source was rejected')
            print(f'PASS: {name} baseline, intended failure and restored source', flush=True)
    manifest['accepted'] = True
    save()
    print(f'PASS: {len(CASES)} isolated proposal-queue implementation controls')


if __name__ == '__main__':
    main()
