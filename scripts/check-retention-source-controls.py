#!/usr/bin/env python3
"""Compile isolated pin-source faults against a retained local common build."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'crates/common/src/retention.rs'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--deps', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.deps, args.output = args.deps.resolve(), args.output.resolve()
    if args.output.exists():
        raise RuntimeError('output must be a new directory')
    args.output.mkdir(parents=True)
    dependencies = {}
    for name in ('kv9_common', 'sha2', 'thiserror'):
        choices = list(args.deps.glob(f'lib{name}-*.rlib'))
        if len(choices) != 1:
            raise RuntimeError(f'exactly one retained {name} rlib is required')
        dependencies[name] = choices[0]
    source = SOURCE.read_text()
    cases = {
        'baseline': (None, None, None),
        'reuse-retired': (
            '        if self.retired {\n            return Err(PinError::Retired);\n        }\n', '',
            'publication_retirement_and_retries_preserve_protection'),
        'stale-generation': (
            '        if slot.generation != token.generation {\n            return Err(PinError::Generation);\n        }\n', '',
            'stale_owner_messages_cannot_release_or_revive_another_generation'),
        'premature-release': (
            '                self.acquire(to)?.map(|phase| (to, phase))',
            '                self.owners.remove(&from.owner);\n                self.acquire(to)?.map(|phase| (to, phase))',
            'sharing_keeps_both_owners_and_failed_handoff_keeps_the_source'),
        'retire-pinned': (
            '                if self\n                    .owners\n                    .values()\n                    .any(|slot| slot.phase != PinPhase::Released)\n                {\n                    return Err(PinError::Pinned);\n                }\n', '',
            'publication_retirement_and_retries_preserve_protection'),
    }
    results = {}
    for name, (before, after, selected) in cases.items():
        work = args.output / name
        work.mkdir()
        changed = source
        if before is not None:
            if source.count(before) != 1:
                raise RuntimeError(f'{name}: mutation is no longer unique')
            changed = source.replace(before, after)
        (work / 'retention.rs').write_text(changed)
        (work / 'wrapper.rs').write_text('pub use kv9_common::RootDigest;\nmod retention;\n')
        compile_command = ['rustc', '--edition=2021', '--test', 'wrapper.rs',
                           '-L', f'dependency={args.deps}', '-o', str(work / 'tests')]
        for dep, path in dependencies.items():
            compile_command += ['--extern', f'{dep}={path}']
        record = {'source_sha256': sha(work / 'retention.rs'), 'compile_command': compile_command}
        with (work / 'compile.log').open('w') as log:
            compiled = subprocess.run(compile_command, cwd=work, stdout=log, stderr=subprocess.STDOUT, timeout=60)
        record['compile_exit_code'] = compiled.returncode
        if compiled.returncode:
            (work / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
            raise RuntimeError(f'{name}: compilation failure cannot qualify a source fault')
        test_command = [str(work / 'tests')]
        if selected:
            test_command += ['--exact', f'retention::tests::{selected}', '--nocapture']
        record['test_command'] = test_command
        with (work / 'test.log').open('w') as log:
            tested = subprocess.run(test_command, cwd=work, stdout=log, stderr=subprocess.STDOUT, timeout=60)
        record['test_exit_code'] = tested.returncode
        text = (work / 'test.log').read_text()
        if selected:
            accepted = (tested.returncode == 101 and 'panicked at' in text
                        and f'test retention::tests::{selected} ... FAILED' in text
                        and re.search(r'test result: FAILED\. 0 passed; 1 failed; 0 ignored; 0 measured; 6 filtered out;', text))
        else:
            accepted = (tested.returncode == 0 and re.search(
                r'test result: ok\. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;', text))
        record['status'] = 'accepted' if accepted else 'rejected'
        (work / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
        if not accepted:
            raise RuntimeError(f'{name}: missing exact test outcome')
        results[name] = record
        print(f'PASS: {name}: compiled and selected expected result observed', flush=True)
    result = {'status': 'PASS', 'source_sha256': sha(SOURCE), 'runner_sha256': sha(Path(__file__)),
              'rustc': subprocess.check_output(['rustc', '-vV'], text=True),
              'dependencies': {name: {'path': str(path), 'sha256': sha(path)} for name, path in dependencies.items()},
              'baseline_tests': 7, 'compiled_faults': 4, 'results': results,
              'scope': 'Isolated actual source faults, not a server build or physical deletion test.'}
    (args.output / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
