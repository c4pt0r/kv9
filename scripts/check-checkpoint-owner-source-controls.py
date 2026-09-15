#!/usr/bin/env python3
"""Compile isolated journal/scheduling faults and require the guarding assertion.

Uses recorded Cargo dependencies, a fresh output directory, and NVMe fixtures.
It does not mutate the working source or any frozen server executable.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.artifacts, args.output = args.artifacts.resolve(), args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    artifacts = {}
    for line in args.artifacts.read_text().splitlines():
        row = json.loads(line)
        if row.get('reason') == 'compiler-artifact':
            rlibs = [Path(p) for p in row['filenames'] if p.endswith('.rlib')]
            if rlibs:
                artifacts[row['target']['name']] = rlibs[0]
    dependencies = {name: artifacts[artifact] for name, artifact in {
        'kv9_common': 'kv9_common', 'thiserror': 'thiserror', 'rpds': 'rpds',
        's3_client': 'object_store', 'futures_util': 'futures_util',
        'tokio': 'tokio', 'serde': 'serde', 'serde_json': 'serde_json',
        'sha2': 'sha2'}.items()}
    deps = artifacts['kv9_common'].parent
    if deps.name != 'deps':
        deps /= 'deps'
    env = dict(os.environ, TMPDIR='/tmp')
    sources = {str(p.relative_to(ROOT / 'crates/engine')): sha(p)
               for p in sorted((ROOT / 'crates/engine/src').rglob('*.rs'))}
    cases = {
        'replace-unsettled-plan': (
            'flush_journal.rs', 'pub fn stage_planned(',
            'return Err(Error::Engine(\n'
            '                "pending flush must settle before staging another".into(),\n'
            '            ));', 'return self.publish(&bytes);',
            'flush_journal::tests::durable_plan_keeps_exact_bytes_before_io_and_upgrades_only_after_verification',
            'assert!(journal.stage_planned(&plan, 9).is_err());'),
        'unchecked-persisted-object': (
            'flush_journal.rs', 'fn decode(',
            '                checked_sst_bytes(file, object)?;',
            '                let _ = file;',
            'flush_journal::tests::planned_journal_refuses_all_truncations_and_valid_checksum_bad_payloads',
            '            assert!(\n                journal.load().is_err(),'),
        'checkpoint-feedback': (
            'mem.rs', 'impl ReplicatedEngine for MemEngine',
            'key.starts_with(b"\\0kv9\\0retention_v1\\0")', 'false',
            'mem::tests::checkpoint_bookkeeping_does_not_schedule_itself_but_is_frozen',
            '        assert_eq!(engine.data_revision(), 0);'),
        'exclude-user-column': (
            'mem.rs', 'impl ReplicatedEngine for MemEngine',
            '*cf != ColumnFamily::Default\n                || ', '',
            'mem::tests::checkpoint_bookkeeping_does_not_schedule_itself_but_is_frozen',
            '        assert_eq!(\n            engine.data_revision(),\n            1,'),
    }
    results = {}
    baseline = None
    for name in ['baseline', *cases]:
        work = args.output / name
        work.mkdir()
        shutil.copytree(ROOT / 'crates/engine/src', work / 'src')
        selected = panic_line = source_file = None
        if name in cases:
            source_file, anchor, before, after, selected, assertion = cases[name]
            p = work / 'src' / source_file
            source = p.read_text()
            start = source.index(anchor)
            # The plan's conflict branch shares text with stage(PreparedFlush).
            # Restrict this mutation to the one intended function suffix.
            assert source[start:].count(before) == 1, f'{name}: non-unique mutation'
            changed = source[:start] + source[start:].replace(before, after, 1)
            test_start = changed.index('fn ' + selected.split('::')[-1] + '(')
            pos = changed.index(assertion, test_start)
            panic_line = changed[:pos].count('\n') + 1
            p.write_text(changed)
        command = ['rustc', '--edition=2021', '--crate-name', 'kv9_engine',
                   '--test', 'src/lib.rs', '-C', 'debuginfo=0',
                   '-L', f'dependency={deps}', '-o', str(work / 'tests')]
        for dep, path in dependencies.items():
            command += ['--extern', f'{dep}={path}']
        record = {'compile_command': command, 'selected_test': selected,
                  'expected_panic_line': panic_line,
                  'sources': {str(p.relative_to(work)): sha(p)
                              for p in sorted((work / 'src').rglob('*.rs'))}, 'runs': []}
        with (work / 'compile.log').open('w') as log:
            compiled = subprocess.run(command, cwd=work, env=env,
                                      stdout=log, stderr=subprocess.STDOUT, timeout=120)
        record['compile_exit_code'] = compiled.returncode
        save(work / 'result.json', record)
        if compiled.returncode:
            raise RuntimeError(f'{name}: compilation failure is not an accepted fault')
        binary = work / 'tests'
        record['binary_sha256'] = sha(binary)
        if name == 'baseline':
            baseline = binary
            selections = sorted({case[4] for case in cases.values()})
            runs = [(f'baseline-{n}', binary, test, False) for n, test in enumerate(selections)]
        else:
            runs = [('same-filter-baseline', baseline, selected, False),
                    ('fault', binary, selected, True)]
        for label, executable, test, fault in runs:
            cmd = [str(executable), '--exact', test, '--nocapture', '--test-threads=1']
            with (work / (label + '.log')).open('w') as log:
                run = subprocess.run(cmd, cwd=work, env=env,
                                     stdout=log, stderr=subprocess.STDOUT, timeout=60)
            output = (work / (label + '.log')).read_text()
            passed, failed = (0, 1) if fault else (1, 0)
            accepted = bool(run.returncode == (101 if fault else 0) and re.search(
                rf'test result: (?:ok|FAILED)\. {passed} passed; {failed} failed; 0 ignored; 0 measured; \d+ filtered out;', output))
            if fault:
                expected_message = {
                    'replace-unsettled-plan': 'assertion failed: journal.stage_planned(&plan, 9).is_err()',
                    'unchecked-persisted-object': 'checksum-valid malformed plan 0',
                    'checkpoint-feedback': 'assertion `left == right` failed',
                    'exclude-user-column': 'reserved-looking user-column bytes must still trigger checkpoints',
                }[name]
                accepted = (accepted and expected_message in output
                            and f'panicked at src/{source_file}:{panic_line}:' in output
                            and f'test {test} ...' in output)
            record['runs'].append(dict(label=label, command=cmd, exit_code=run.returncode,
                                       accepted=accepted))
            save(work / 'result.json', record)
            if not accepted:
                raise RuntimeError(f'{name}/{label}: exact test/assertion missing')
        results[name] = record
        print('PASS:', name, flush=True)
    save(args.output / 'summary.json', dict(status='PASS', baseline_tests=3,
        compiled_faults=4, results=results, source_pins=sources,
        runner_sha256=sha(Path(__file__)), artifacts_sha256=sha(args.artifacts),
        rustc=subprocess.check_output(['rustc', '-vV'], text=True),
        dependencies={name: dict(path=str(path), sha256=sha(path))
                      for name, path in dependencies.items()},
        scope='Four isolated engine faults with exact baseline/fault test and panic location. No live worker, Raft or object-store recovery claim.'))


if __name__ == '__main__':
    main()
