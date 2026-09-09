#!/usr/bin/env python3
"""Run isolated registration source controls with baseline/mutant/restored evidence."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
REL = Path('crates/server/src/runtime.rs')


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    original = (ROOT / REL).read_text()
    cases = [
        ('frozen-first-seed', 'self.next = if start + 1 == ordered.len() {\n                0\n            } else {\n                start + 1\n            };', 'self.next = start;',
         'repeated_registration_passes_reach_a_healthy_seed_after_a_blackhole',
         'a blackholed first seed consumed every retry window'),
        ('delayed-hint', 'queue.insert(i, (id, addr));', 'queue.push((id, addr));',
         'a_novel_leader_hint_precedes_a_blackholed_remaining_seed',
         'a pending blackhole prevented following an already received leader hint'),
        ('discovery-sized-budget', 'const REGISTRATION_PASS_TIMEOUT: Duration = Duration::from_secs(5);',
         'const REGISTRATION_PASS_TIMEOUT: Duration = Duration::from_millis(50);',
         'registration_budget_allows_durable_work_beyond_a_discovery_probe',
         'a discovery-probe timeout cannot budget several durable consensus steps'),
    ]
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR', str((ROOT / 'target').resolve())))
    manifest = dict(source=str(REL), source_sha256=sha(original), controls=[])
    (output / 'original.rs').write_text(original)
    with tempfile.TemporaryDirectory(prefix='kv9-registration-controls.') as temp:
        tree = Path(temp)
        for name in ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'crates', 'src', 'proto']:
            source = ROOT / name
            if source.is_dir():
                shutil.copytree(source, tree / name)
            else:
                shutil.copy2(source, tree / name)
        target = tree / REL
        for name, before, after, test, failure in cases:
            if original.count(before) != 1:
                raise RuntimeError(f'{name}: source anchor is not unique')
            mutant = original.replace(before, after)
            folder = output / name
            folder.mkdir()
            (folder / 'mutant.rs').write_text(mutant)
            case = dict(name=name, test=test, expected_failure=failure, mutant_sha256=sha(mutant), runs=[])
            for phase, source in [('baseline', original), ('mutant', mutant), ('restored', original)]:
                target.write_text(source)
                command = ['cargo', 'test', '--locked', '-p', 'kv9-server', '--lib',
                           'runtime::tests::' + test, '--', '--exact']
                result = subprocess.run(command, cwd=tree, env=env, text=True,
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=180)
                (folder / f'{phase}.log').write_text(result.stdout)
                if 'running 1 test\n' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: expected exactly one selected test')
                if phase == 'mutant':
                    if result.returncode == 0 or failure not in result.stdout or '1 failed;' not in result.stdout:
                        raise RuntimeError(f'{name}: mutant missed the intended assertion')
                elif result.returncode or '1 passed; 0 failed;' not in result.stdout:
                    raise RuntimeError(f'{name}/{phase}: valid source was rejected')
                case['runs'].append(dict(phase=phase, exit_code=result.returncode, source_sha256=sha(source)))
            manifest['controls'].append(case)
            print(f'PASS: {name} baseline, intended failure and restored source')
    (output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print('PASS: 3 isolated registration source controls checked')


if __name__ == '__main__':
    main()
