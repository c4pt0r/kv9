#!/usr/bin/env python3
"""Reject isolated store-lifecycle source faults in real runtime tests."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
RUNTIME = 'crates/server/src/runtime.rs'
COMMON = 'crates/common/src/store_lifecycle.rs'
STORAGE = 'crates/raft/src/storage.rs'


def sha(text):
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
    sources = {p: (ROOT / p).read_text() for p in (RUNTIME, COMMON, STORAGE)}
    runtime, common, storage = sources[RUNTIME], sources[COMMON], sources[STORAGE]
    identity = 'store_lifecycle::tests::preparation_is_independent_and_activation_is_monotonic_across_restarts'
    recovery = 'storage::tests::activated_store_recovery_never_creates_or_reinitializes_a_log'
    owner_start = runtime.index('        let driver_thread = if authorized {')
    owner_end = runtime.index('        Ok(Self {', owner_start)
    owner = runtime[owner_start:owner_end]
    cases = [
        ('copy-root-identity', COMMON, replace_once(common,
         'if record.node_id != identity.node_id || record.incarnation != identity.store_incarnation {',
         'if false {'), 'kv9-common', identity, "another disk copied the root's incarnation"),
        ('share-store-owner', COMMON, replace_once(common, 'lock.try_lock()',
         'Ok::<(), std::fs::TryLockError>(())'), 'kv9-common', identity, 'a second owner acquired the same store'),
        ('create-missing-active-log', STORAGE, replace_once(storage,
         'fs.open_existing_append(&path)', 'fs.open_append(&path)'), 'kv9-raft', recovery, 'recovery recreated a missing log'),
        ('initialize-empty-active-log', STORAGE, replace_once(storage,
         'if !initialize && !saw_any {', 'if false && !initialize && !saw_any {'), 'kv9-raft', recovery,
         'activated store reinitialized an empty or torn log'),
        ('start-before-activation', RUNTIME, replace_once(runtime,
         'store_guard.activate(&store_identity)?;', '/* invalid control: activation skipped */'), 'kv9-server',
         'runtime::tests::root_voter_rejects_another_disk_and_missing_activated_log_before_starting',
         'Raft owner started before durable store activation'),
        ('nonpristine-formation-fence', RUNTIME, replace_once(runtime,
         'if local_identity.is_some() && !marker_initialized {',
         'if recover_only && !joining { bootstrap.mark_data_dir_initialized(); }\n'
         '        if local_identity.is_some() && !marker_initialized {'), 'kv9-server',
         'runtime::tests::original_stores_resume_formation_after_each_pre_catalog_crash_cut',
         'original root formation resumes after a pre-catalog crash'),
        ('plan-before-current-term-apply', RUNTIME, replace_once(runtime,
         '.is_none_or(|at| at.term != status.term)', '.is_some_and(|_at| false)'), 'kv9-server',
         'runtime::fence_firing_tests::initialization_waits_for_apply_before_planning_a_seed',
         'initialization must not plan another seed behind unapplied committed metadata'),
        ('detach-owner-on-startup-error', RUNTIME,
         replace_once(replace_once(runtime, owner, ''),
                      '        let status_path = data_dir.join("status");',
                      owner + '        let status_path = data_dir.join("status");'),
         'kv9-server', 'runtime::tests::failed_listener_bind_releases_every_store_owner_before_unlocking',
         'failed startup left a detached Raft owner using an unlocked store'),
        ('reuse-failed-publication', COMMON, replace_once(common, 'self.failed = true;', 'self.failed = false;'),
         'kv9-common', 'store_lifecycle::tests::publication_errors_poison_authority_and_reopen_stabilizes_visible_state',
         'failed guard still authorized a store'),
    ]
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')))
    manifest = dict(sources={p: sha(s) for p, s in sources.items()}, controls=[])
    original = output / 'original'
    for p, s in sources.items():
        (original / p).parent.mkdir(parents=True, exist_ok=True)
        (original / p).write_text(s)
    with tempfile.TemporaryDirectory(prefix='kv9-store-controls.') as temp:
        tree = Path(temp)
        for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates', 'src', 'proto']:
            source = ROOT / name
            if source.is_dir():
                shutil.copytree(source, tree / name)
            else:
                shutil.copy2(source, tree / name)
        for name, path, mutant, package, test, failure in cases:
            folder = output / name
            folder.mkdir()
            (folder / 'mutant.rs').write_text(mutant)
            case = dict(name=name, path=path, test=test, expected_failure=failure, mutant_sha256=sha(mutant), runs=[])
            for phase, text in [('baseline', sources[path]), ('mutant', mutant), ('restored', sources[path])]:
                for p, s in sources.items():
                    (tree / p).write_text(text if p == path else s)
                command = ['cargo', 'test', '--locked', '-p', package, '--lib', test, '--', '--exact']
                result = subprocess.run(command, cwd=tree, env=env, text=True, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT, timeout=180)
                (folder / f'{phase}.log').write_text(result.stdout)
                if 'running 1 test\n' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: expected exactly one compiled test')
                if phase == 'mutant':
                    if result.returncode != 101 or failure not in result.stdout or '1 failed;' not in result.stdout:
                        raise RuntimeError(f'{name}: mutant missed the intended assertion')
                elif result.returncode or '1 passed; 0 failed;' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: valid source was rejected')
                case['runs'].append(dict(phase=phase, command=command, exit_code=result.returncode,
                    sources={p: sha((tree / p).read_text()) for p in sources}))
            manifest['controls'].append(case)
            (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
            print(f'PASS: {name} baseline, intended failure and restored source', flush=True)
    print('PASS: 9 isolated store-lifecycle source controls checked', flush=True)


if __name__ == '__main__':
    main()
