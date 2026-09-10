#!/usr/bin/env python3
"""Run isolated original-Ready durability controls against the real Rust storage."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
STORAGE = 'crates/raft/src/storage.rs'
SYNC = """            if !entries.is_empty() || hs.is_some() {
                operation = "ready sync";
                Self::sync_records(&self.io_metrics, file)?;
            }"""
PUBLICATION = """            // One memory write lock publishes the complete successful Ready.
            // No entry or vote/commit update is exposed before the common sync.
            operation = "ready memory publication";
            let mut memory = self.mem.wl();
            if !entries.is_empty() {
                memory
                    .append(entries)
                    .map_err(|err| Error::Raft(err.to_string()))?;
            }
            if let Some(hs) = hs {
                memory.set_hardstate(hs.clone());
            }"""
ENTRIES = """            operation = "append";
            Self::write_entries_unsynced(&self.io_metrics, file, entries)?;"""
HARDSTATE = """            if let Some(hs) = hs {
                operation = "hardstate";
                let bytes = hs
                    .write_to_bytes()
                    .map_err(|err| Error::Raft(format!("hardstate encode: {err}")))?;
                Self::write_record_unsynced(&self.io_metrics, file, REC_HARD_STATE, &bytes)?;
            }"""
MATRIX = 'storage::persistence_model::every_ready_group_failure_preserves_a_valid_recovery_prefix_and_commit'
CASES = [
    ('omitted-ready-sync', SYNC, '',
     'storage::persistence_model::original_ready_ack_survives_loss_of_unsynced_bytes',
     'acknowledged Ready lost an entry'),
    ('memory-before-ready-sync', SYNC + '\n' + PUBLICATION,
     PUBLICATION + '\n' + SYNC, MATRIX,
     'failed original Ready published memory entries'),
    ('ignored-ready-sync-error', SYNC,
     SYNC.replace('Self::sync_records(&self.io_metrics, file)?;',
                  'let _ = Self::sync_records(&self.io_metrics, file);'), MATRIX,
     'failed original Ready was acknowledged'),
    ('hardstate-before-ready-entries', ENTRIES + '\n' + HARDSTATE,
     HARDSTATE + '\n' + ENTRIES, MATRIX,
     'new HardState survived without its complete preceding log'),
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

    with tempfile.TemporaryDirectory(prefix='kv9-ready-controls.') as temp:
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
    print(f'PASS: {len(CASES)} isolated original-Ready implementation controls')


if __name__ == '__main__':
    main()
