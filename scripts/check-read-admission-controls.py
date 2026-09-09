#!/usr/bin/env python3
"""Reject isolated read-admission faults using the real Raft peer and driver."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
RAW = 'crates/raft/src/rawnode.rs'
GRPC = 'crates/raft/src/grpc.rs'
DRIVER = 'crates/raft/src/driver.rs'


def digest(text):
    return hashlib.sha256(text.encode()).hexdigest()


def replace_once(text, before, after):
    if text.count(before) != 1 or before == after:
        raise RuntimeError('source control anchor is not unique')
    return text.replace(before, after)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    sources = {p: (ROOT / p).read_text() for p in (RAW, DRIVER, GRPC)}
    raw, driver = sources[RAW], sources[DRIVER]
    gate = raw[raw.index('    pub fn read_index('):raw.index('    /// Drain quorum-confirmed read states')]
    admission = driver[driver.index('        // A new leader can be observable'):driver.index('        // 3. Wait for the quorum confirmation')]
    early = 'driver::tests::read_barrier_survives_submission_before_current_term_commit'
    early_failure = 'read submitted before election barrier was lost after quorum recovered'
    cases = [
        ('ignore-current-term', RAW, replace_once(raw,
         'if !g.raw.raft.commit_to_current_term() {', 'if false && !g.raw.raft.commit_to_current_term() {'), early, early_failure),
        ('discard-deferred-request', DRIVER, replace_once(driver,
         'if submitted {', 'if submitted || !submitted {'), early, early_failure),
        ('ignore-deposed-leader', RAW, replace_once(raw, gate, replace_once(gate,
         'if g.raw.raft.state != StateRole::Leader {', 'if false && g.raw.raft.state != StateRole::Leader {')),
         'driver::tests::deferred_read_admission_refuses_a_deposed_leader', 'deferred admission ignored loss of leadership'),
        ('ignore-admission-deadline', DRIVER, replace_once(driver, admission, replace_once(admission,
         'if start.elapsed() > deadline {', 'if false && start.elapsed() > deadline {')),
         'driver::tests::deferred_read_admission_respects_the_original_deadline', 'deferred admission exceeded the original request budget'),
        ('skip-exact-quorum-confirmation', DRIVER, replace_once(driver,
         'if let Some(index) = hit {', 'if let Some(index) = hit.or(Some(self.peer.status_snapshot().committed)) {'),
         'driver::tests::an_isolated_leader_cannot_confirm_a_read_barrier', 'an isolated leader must not confirm a barrier'),
        ('skip-apply-coverage', DRIVER, replace_once(driver,
         '.is_some_and(|wm| wm.index >= confirmed)', '.is_some_and(|wm| wm.index >= confirmed || wm.index < confirmed)'),
         'driver::tests::a_read_barrier_waits_for_apply_not_just_confirmation', 'a barrier must not establish while apply lags the confirmed index'),
    ]
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')))
    manifest = dict(sources={p: digest(s) for p, s in sources.items()}, controls=[])
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    for p, s in sources.items():
        target = output / 'original' / p
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(s)
    with tempfile.TemporaryDirectory(prefix='kv9-read-admission-controls.') as directory:
        tree = Path(directory)
        for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates', 'src', 'proto']:
            source = ROOT / name
            if source.is_dir():
                shutil.copytree(source, tree / name)
            else:
                shutil.copy2(source, tree / name)
        for name, path, mutant, test, failure in cases:
            folder = output / name
            folder.mkdir()
            (folder / 'mutant.rs').write_text(mutant)
            case = dict(name=name, path=path, test=test, expected_failure=failure,
                        mutant_sha256=digest(mutant), runs=[])
            for phase, text in [('baseline', sources[path]), ('mutant', mutant), ('restored', sources[path])]:
                for p, s in sources.items():
                    (tree / p).write_text(text if p == path else s)
                expected_sources = {p: digest(text if p == path else s) for p, s in sources.items()}
                command = ['cargo', 'test', '--locked', '-p', 'kv9-raft', '--lib', '--features', 'testing', test, '--', '--exact']
                result = subprocess.run(command, cwd=tree, env=env, text=True, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT, timeout=180)
                (folder / f'{phase}.log').write_text(result.stdout)
                if 'running 1 test\n' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: expected exactly one compiled test')
                if phase == 'mutant':
                    if result.returncode != 101 or failure not in result.stdout or '0 passed; 1 failed;' not in result.stdout:
                        raise RuntimeError(f'{name}: mutant missed the intended assertion')
                elif result.returncode or '1 passed; 0 failed;' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: valid source was rejected')
                actual_sources = {p: digest((tree / p).read_text()) for p in sources}
                if actual_sources != expected_sources:
                    raise RuntimeError('isolated source changed during a control')
                case['runs'].append(dict(phase=phase, command=command, exit_code=result.returncode, sources=actual_sources))
            manifest['controls'].append(case)
            (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
            print(f'PASS: {name} baseline, intended failure and restored source', flush=True)
    if any((ROOT / p).read_text() != text for p, text in sources.items()):
        raise RuntimeError('source changed during controls')
    print('PASS: 6 isolated read admission source controls checked', flush=True)


if __name__ == '__main__':
    main()
