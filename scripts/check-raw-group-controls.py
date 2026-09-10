#!/usr/bin/env python3
"""Run isolated Raw-group durability controls against the real Rust storage."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
GROUP = 'crates/raft/src/state_machine/raw_group.rs'
SM = 'crates/raft/src/state_machine.rs'
TESTS = 'state_machine::raw_group::tests::'
WRITE = '        self.engine.write_applied(batch, at)?;'
WATERMARK = '        self.applied = LogIndex(at.index);'
CASES = [
    ('ignored-group-write-error', GROUP, WRITE,
     '        let _ = self.engine.write_applied(batch, at);',
     TESTS + 'failed_group_returns_no_receipts_even_if_durable_effect_occurred',
     'failed Raw group returned receipts'),
    ('watermark-before-write', GROUP, WRITE + '\n' + WATERMARK,
     WATERMARK + '\n' + WRITE,
     TESTS + 'failed_group_returns_no_receipts_even_if_durable_effect_occurred',
     'failed group advanced live apply watermark'),
    ('system-key-in-raw-group', GROUP,
     'key.mode == KeyMode::Raw && key.keyspace != KeyspaceId::SYSTEM',
     'key.keyspace != KeyspaceId(999_999)',
     TESTS + 'real_driver_system_mutation_inside_fenced_write_remains_an_epoch_barrier',
     'physical System mutation was hidden from the next epoch verdict'),
    ('stale-verdict-accepted', GROUP,
     'if adjudicator.is_fresh(fence)? {',
     'if adjudicator.is_fresh(fence)? || true {',
     TESTS + 'stale_verdict_is_retained_and_later_read_error_publishes_nothing',
     'stale Raw group member lost its verdict'),
    ('mutation-order-reversed', GROUP,
     '            batch.append(next);',
     '            let mut next = next;\n            next.append(batch);\n            batch = next;',
     TESTS + 'ordered_group_preserves_overwrites_deletes_and_exact_tail',
     'Raw group changed mutation order'),
    ('arbitrary-adjudicator-opted-in', SM,
     'fn independent_of_raw_writes(&self) -> bool {\n        false\n    }',
     'fn independent_of_raw_writes(&self) -> bool {\n        true\n    }',
     TESTS + 'grouping_refuses_hidden_metadata_and_non_opted_in_adjudicators',
     'arbitrary adjudicator opted into grouped apply'),
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

    with tempfile.TemporaryDirectory(prefix='kv9-raw-group-controls.') as temp:
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
        for name, relative, before, after, test, failure in CASES:
            target = tree / relative
            original = target.read_text()
            if original.count(before) != 1:
                raise RuntimeError(f'{name}: source anchor is not unique')
            mutant = original.replace(before, after)
            folder = output / name
            folder.mkdir()
            (folder / 'original.rs').write_text(original)
            (folder / 'mutant.rs').write_text(mutant)
            case = dict(name=name, source=relative, test=test,
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
    print(f'PASS: {len(CASES)} isolated Raw-group implementation controls')


if __name__ == '__main__':
    main()
