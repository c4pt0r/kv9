#!/usr/bin/env python3
"""Compile isolated checkpoint publication faults against recorded Cargo artifacts."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import os
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'crates/raft/src/state_machine/checkpoint_recovery.rs'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.artifacts, args.output = args.artifacts.resolve(), args.output.resolve()
    if args.output.exists():
        raise RuntimeError('output must be a new directory')
    args.output.mkdir(parents=True)
    dependencies = {}
    build_output = None
    for line in args.artifacts.read_text().splitlines():
        artifact = json.loads(line)
        if artifact.get('reason') == 'build-script-executed' and '#kv9-raft@' in artifact['package_id']:
            build_output = artifact['out_dir']
        if artifact.get('reason') != 'compiler-artifact':
            continue
        rlibs = [Path(p) for p in artifact['filenames'] if p.endswith('.rlib')]
        if rlibs:
            dependencies[artifact['target']['name']] = rlibs[0]
    for name in ('kv9_raft', 'kv9_common', 'raft', 'protobuf'):
        if name not in dependencies:
            raise RuntimeError(f'missing Cargo artifact: {name}')
    args.deps = dependencies['kv9_raft'].parent
    if args.deps.name != 'deps':
        args.deps /= 'deps'
    if build_output is None:
        raise RuntimeError('missing recorded Raft build-script output')
    source = SOURCE.read_text()
    cases = {
        'baseline': (None, None, None),
        'wrong-scope': (
            'if manifest.scope.cluster != self.cluster || manifest.scope.region != self.region {',
            'if false {', 'wrong_scope_or_cut_is_refused_before_object_restore'),
        'missing-atomic-pair': (
            'if pair_writes.len() != 1\n                || !matches!(pair_writes[0], Mutation::Put { value, .. } if value == &pair)',
            'if false', 'winning_batch_requires_its_atomic_pair_and_exact_publication_term'),
        'replace-first-winner': (
            'self.publication.get_or_insert((at, generation));',
            'self.publication = Some((at, generation));',
            'retry_or_later_republication_keeps_the_first_actual_winner_position'),
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
        shutil.copytree(ROOT / 'crates/raft/src', work / 'src')
        (work / 'src/state_machine/checkpoint_recovery.rs').write_text(changed)
        compile_command = ['rustc', '--edition=2021', '--crate-name', 'kv9_raft',
                           '--test', 'src/lib.rs', '-L', f'dependency={args.deps}',
                           '-o', str(work / 'tests')]
        for dep, path in dependencies.items():
            compile_command += ['--extern', f'{dep}={path}']
        record = {'source_sha256': sha(work / 'src/state_machine/checkpoint_recovery.rs'), 'compile_command': compile_command, 'build_output': build_output,
                  'source_tree': {str(p.relative_to(work)): sha(p) for p in sorted((work / 'src').rglob('*')) if p.is_file()}}
        with (work / 'compile.log').open('w') as log:
            compiled = subprocess.run(compile_command, cwd=work, env=dict(os.environ, OUT_DIR=build_output), stdout=log, stderr=subprocess.STDOUT, timeout=60)
        record['compile_exit_code'] = compiled.returncode
        if compiled.returncode:
            (work / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
            raise RuntimeError(f'{name}: compilation failure cannot qualify a source fault')
        test_command = [str(work / 'tests')]
        if selected:
            test_command += ['--exact', f'state_machine::checkpoint_recovery::tests::{selected}', '--nocapture']
        else:
            test_command += ['state_machine::checkpoint_recovery::tests::']
        record['test_command'] = test_command
        with (work / 'test.log').open('w') as log:
            tested = subprocess.run(test_command, cwd=work, stdout=log, stderr=subprocess.STDOUT, timeout=60)
        record['test_exit_code'] = tested.returncode
        text = (work / 'test.log').read_text()
        if selected:
            accepted = (tested.returncode == 101 and 'panicked at' in text
                        and f'test state_machine::checkpoint_recovery::tests::{selected} ... FAILED' in text
                        and re.search(r'test result: FAILED\. 0 passed; 1 failed; 0 ignored; 0 measured; \d+ filtered out;', text))
        else:
            accepted = (tested.returncode == 0 and re.search(
                r'test result: ok\. 5 passed; 0 failed; 1 ignored; 0 measured; \d+ filtered out;', text))
        record['status'] = 'accepted' if accepted else 'rejected'
        (work / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
        if not accepted:
            raise RuntimeError(f'{name}: missing exact test outcome')
        results[name] = record
        print(f'PASS: {name}: compiled and selected expected result observed', flush=True)
    result = {'status': 'PASS', 'source_sha256': sha(SOURCE), 'runner_sha256': sha(Path(__file__)),
              'artifacts_sha256': sha(args.artifacts),
              'storage_sha256': sha(ROOT / 'crates/raft/src/storage.rs'),
              'engine_persist_sha256': sha(ROOT / 'crates/engine/src/persist.rs'),
              'rustc': subprocess.check_output(['rustc', '-vV'], text=True),
              'dependencies': {name: {'path': str(path), 'sha256': sha(path)} for name, path in dependencies.items()},
              'baseline_tests': 5, 'compiled_faults': 3, 'results': results,
              'scope': 'Isolated compiled publication source faults; five baseline tests overlap the Raft suite. Real MinIO and process/Chaos acceptance are separate evidence.'}
    (args.output / 'summary.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
