#!/usr/bin/env python3
"""Run isolated indexed read-receipt controls against the real Rust container."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
STORAGE = 'crates/raft/src/read_receipts.rs'
LOOKUP = '        self.indices.get(context)?.front().copied()'
EXACT = 'read_receipts::tests::exact_context_lookup_is_nonconsuming_and_fifo_eviction_is_not_lru'
CASES = [
    ('latest-duplicate-receipt', LOOKUP, LOOKUP.replace('.front()', '.back()'),
     'read_receipts::tests::duplicate_context_keeps_the_first_retained_confirmation',
     'duplicate context replaced its first retained receipt'),
    ('omitted-receipt-eviction', '        if self.order.len() == self.capacity {',
     '        if false {', EXACT, 'evicted read context remained confirmed'),
    ('inexact-context-confirmation', LOOKUP,
     '        self.indices.values().next()?.front().copied()', EXACT,
     'a different exact context acquired confirmation'),
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

    with tempfile.TemporaryDirectory(prefix='kv9-read-receipt-controls.') as temp:
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
    print(f'PASS: {len(CASES)} isolated read-receipt implementation controls')


if __name__ == '__main__':
    main()
