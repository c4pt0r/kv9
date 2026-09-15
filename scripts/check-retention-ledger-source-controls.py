#!/usr/bin/env python3
"""Compile isolated ledger faults and require the exact guarding assertion."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'crates/meta/src/retention.rs'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.artifacts, args.output = args.artifacts.resolve(), args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    dependencies = {}
    for line in args.artifacts.read_text().splitlines():
        artifact = json.loads(line)
        if artifact.get('reason') == 'compiler-artifact':
            rlibs = [Path(p) for p in artifact['filenames'] if p.endswith('.rlib')]
            if rlibs:
                dependencies[artifact['target']['name']] = rlibs[0]
    required = ('kv9_common', 'kv9_region', 'kv9_raft', 'kv9_engine')
    for dep in required:
        if dep not in dependencies:
            raise RuntimeError(f'missing Cargo artifact: {dep}')
    deps = dependencies['kv9_common'].parent
    source = SOURCE.read_text()
    cases = {
        'immutable-binding': (
            'if old.binding.descriptor != binding.descriptor\n'
            '                    || old.binding.resources != binding.resources',
            'if false',
            'exact_retry_does_not_rebind_identity_or_advance_ledger_revision',
            'assert!(plan(&engine, &root, &LedgerRequest::Acquire(wrong)).is_err());'),
        'unpublished-successor': (
            '                || target.phase != PinPhase::Published\n', '',
            'complete_transfer_keeps_source_until_published_destination_and_fences_generations',
            '        assert!(plan(\n            &engine,\n            &root,\n'
            '            &LedgerRequest::QuiesceAfterTransfer {\n'
            '                from: a.token(),\n                to: b.token()\n'
            '            }\n        )\n        .is_err());'),
        'partial-successor': (
            '                || source.binding.resources != target.binding.resources\n', '',
            'transfer_requires_the_whole_subject_and_cannot_cycle_quiesced_owners',
            '        assert!(plan(\n            &engine,\n            &root,\n'
            '            &LedgerRequest::QuiesceAfterTransfer {\n'
            '                from: a.token(),\n                to: b.token()\n'
            '            }\n        )\n        .is_err());'),
        'skip-late-resource': (
            '        for identity in &owner.binding.resources {\n',
            '        for identity in owner.binding.resources.iter().take(1) {\n',
            'failed_late_resource_validation_exposes_no_partial_batch',
            'assert!(plan(&engine, &root, &LedgerRequest::Acquire(binding(&root, 1))).is_err());'),
    }
    results = {}
    baseline_binary = None
    for name in ['baseline', *cases]:
        work = args.output / name
        work.mkdir()
        changed = source
        selected = expected_line = None
        if name != 'baseline':
            before, after, test, assertion = cases[name]
            if source.count(before) != 1:
                raise RuntimeError(f'{name}: mutation is no longer unique')
            changed = source.replace(before, after)
            selected = f'retention::tests::{test}'
            # Find the assertion inside this selected test, not another similar test.
            test_start = changed.index(f'fn {test}()')
            position = changed.index(assertion, test_start)
            expected_line = changed[:position].count('\n') + 1
        shutil.copytree(ROOT / 'crates/meta/src', work / 'src')
        (work / 'src/retention.rs').write_text(changed)
        command = ['rustc', '--edition=2021', '--crate-name', 'kv9_meta',
                   '--test', 'src/lib.rs', '-C', 'debuginfo=0',
                   '-L', f'dependency={deps}', '-o', str(work / 'tests')]
        for dep in required:
            command += ['--extern', f'{dep}={dependencies[dep]}']
        record = {'source_sha256': sha(work / 'src/retention.rs'),
                  'compile_command': command, 'selected_test': selected,
                  'expected_panic_line': expected_line}
        with (work / 'compile.log').open('w') as log:
            compiled = subprocess.run(command, cwd=work, stdout=log,
                                      stderr=subprocess.STDOUT, timeout=120)
        record['compile_exit_code'] = compiled.returncode
        if compiled.returncode:
            (work / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
            raise RuntimeError(f'{name}: compilation failure is not a control result')
        binary = work / 'tests'
        record['test_binary_sha256'] = sha(binary)
        if name == 'baseline':
            baseline_binary = binary
            runs = [('baseline', binary, ['retention::tests::', '--nocapture'], 9, 0)]
        else:
            runs = [('same-filter-baseline', baseline_binary, ['--exact', selected, '--nocapture'], 1, 0),
                    ('fault', binary, ['--exact', selected, '--nocapture'], 0, 1)]
        record['runs'] = []
        for label, executable, filters, passed, failed in runs:
            cmd = [str(executable), *filters, '--test-threads=1']
            with (work / f'{label}.log').open('w') as log:
                tested = subprocess.run(cmd, cwd=work, env=dict(os.environ, TMPDIR='/tmp'),
                                        stdout=log, stderr=subprocess.STDOUT, timeout=60)
            output = (work / f'{label}.log').read_text()
            expected_exit = 101 if failed else 0
            accepted = tested.returncode == expected_exit and re.search(
                rf'test result: (?:ok|FAILED)\. {passed} passed; {failed} failed; 0 ignored; 0 measured; \d+ filtered out;', output)
            if failed:
                accepted = (accepted and 'assertion failed:' in output
                            and f'panicked at src/retention.rs:{expected_line}:' in output
                            and f'test {selected} ...' in output)
            record['runs'].append({'label': label, 'command': cmd,
                                   'exit_code': tested.returncode, 'accepted': bool(accepted)})
            (work / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
            if not accepted:
                raise RuntimeError(f'{name}/{label}: exact outcome/assertion was not observed')
        results[name] = record
        print(f'PASS: {name}', flush=True)
    summary = {'status': 'PASS', 'source_sha256': sha(SOURCE),
               'runner_sha256': sha(Path(__file__)), 'artifacts_sha256': sha(args.artifacts),
               'baseline_tests': 9, 'compiled_faults': 4, 'results': results,
               'dependencies': {name: {'path': str(dependencies[name]), 'sha256': sha(dependencies[name])}
                                for name in required},
               'rustc': subprocess.check_output(['rustc', '-vV'], text=True),
               'scope': 'Four isolated metadata planner faults; no mutation of the frozen server or runtime/Chaos replay.'}
    (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')


if __name__ == '__main__':
    main()
