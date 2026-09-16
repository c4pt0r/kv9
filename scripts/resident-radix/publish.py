#!/usr/bin/env python3
"""Audit retained qualification and package evidence; no GitHub/CI side effects."""
import ast
import gzip
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

REPO = Path(__file__).resolve().parents[2]
BASE = Path('/mnt/data/kv9-work')
DEST = REPO / 'docs/persistent-radix-prototype-v1'
DEST.mkdir(exist_ok=False)
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
read = lambda p: json.loads(p.read_text())
roots = {
    'initial-clippy': BASE / 'radix-prototype-20260916-first',
    'initial-models': BASE / 'radix-prototype-20260916-v2',
    'initial-engine': BASE / 'radix-engine-qualification-20260916-first',
    'coverage-gap-control': BASE / 'radix-controls-20260916-first',
    'qualified-models': BASE / 'radix-prototype-20260916-v3',
    'qualified-engine': BASE / 'radix-engine-qualification-20260916-v2',
    'allocation-lifetime': BASE / 'radix-layout-20260916-first',
    'qualified-controls': BASE / 'radix-controls-20260916-v2',
}
qualified = {name: read(roots[name] / 'result.json') for name in
             ['qualified-models', 'qualified-engine', 'allocation-lifetime', 'qualified-controls']}
assert all(result['complete'] for result in qualified.values())
checked_hashes = 0
for name in ['qualified-models', 'qualified-engine', 'allocation-lifetime']:
    for path, digest in read(roots[name] / 'sources.json').items():
        actual = Path(path) if Path(path).is_absolute() else REPO / path
        assert sha(actual) == digest, path
        checked_hashes += 1
    assert read(roots[name] / 'cache-safety.json')['complete']
assert qualified['qualified-models']['tests'] == {'passed': 11, 'failed': 0, 'ignored': 0}
engine_results = qualified['qualified-engine']['results']
assert sum(row['passed'] for row in engine_results) == 213
assert sum(row['ignored'] for row in engine_results) == 25
index = REPO / 'scripts/resident-radix/src/lib.rs'
tests = REPO / 'scripts/resident-radix/src/tests.rs'
engine = roots['qualified-engine'] / 'candidate'
assert sha(engine / 'crates/engine/src/radix.rs') == sha(index)
assert sha(engine / 'crates/engine/src/radix/tests.rs') == sha(tests)
assert sha(roots['allocation-lifetime'] / 'source/radix.rs') == sha(index)
original_mem = (REPO / 'crates/engine/src/mem.rs').read_text()
expected_mem = original_mem.replace('use rpds::RedBlackTreeMapSync;', 'use crate::radix::RadixMap;').replace(
    'type CfMap = RedBlackTreeMapSync<Vec<u8>, Vec<u8>>;', 'type CfMap = RadixMap;')
assert (engine / 'crates/engine/src/mem.rs').read_text() == expected_mem
expected_lib = (REPO / 'crates/engine/src/lib.rs').read_text().replace('pub mod mem;', 'pub mod mem;\npub mod radix;')
assert (engine / 'crates/engine/src/lib.rs').read_text() == expected_lib
unchanged_files = 0
for crate in ['common', 'engine']:
    for path in (REPO / 'crates' / crate).rglob('*'):
        if not path.is_file():
            continue
        relative = path.relative_to(REPO)
        if str(relative) in ['crates/engine/src/mem.rs', 'crates/engine/src/lib.rs']:
            continue
        assert sha(engine / relative) == sha(path), str(relative)
        unchanged_files += 1
assert sha(engine / 'crates/engine/tests/radix_engine_model.rs') == sha(REPO / 'scripts/inline-key/model.rs')
assert sha(engine / 'crates/engine/tests/radix_payload_model.rs') == sha(REPO / 'scripts/entry-buffer/model.rs')
controls = qualified['qualified-controls']
assert controls['source_sha256'] == sha(index) and controls['tests_sha256'] == sha(tests)
assert len(controls['controls']) == 6
assert all(row['compiled'] and row['rejected'] for row in controls['controls'])
assert [row['exit_code'] for row in controls['controls']] == [101] * 5 + [-6]
observations = read(roots['allocation-lifetime'] / 'observations.json')
assert observations['complete'] and observations['all_six_cases_return_to_requested_byte_baseline']
assert len(observations['rows']) == 54 and observations['elapsed_time_recorded'] is False
for row in observations['rows']:
    if row['phase'] in ['snapshot', 'second_snapshot', 'absent_delete', 'drop_first_snapshot']:
        assert row['counts'] == [0] * 6 and row['net_requested_bytes'] == 0
assert read(roots['coverage-gap-control'] / 'result.json')['complete'] is False
assert '1 passed; 0 failed' in (roots['coverage-gap-control'] / 'discard-unique-leaf-overwrite/test.stdout').read_text()
assert read(roots['initial-clippy'] / 'result.json')['complete'] is False
for name, expected in {
    'scripts/checkpoint-publication-chaos.py': 'ebef796c6e19d4f3d391aa69b973e22df501f2e9af13f64dfcc7f34fd564d343',
    'scripts/checkpoint-owned-crash.py': '3e9d4babe9f263b2360f676cdcdd92985f4722710394580659890b7c6abe617a',
}.items():
    assert sha(REPO / name) == expected
for path in (REPO / 'scripts/resident-radix').glob('*.py'):
    ast.parse(path.read_text())
assert subprocess.check_output(['git', 'diff', '--name-only', '--', 'crates'], cwd=REPO) == b''
audit = {'complete': True, 'source_hashes_checked': checked_hashes,
         'unchanged_common_engine_files_checked': unchanged_files,
         'index_sha256': sha(index), 'tests_sha256': sha(tests),
         'isolated_mem_change': 'Only map import and alias; isolated module/test additions verified.',
         'protected_drafts_unchanged': True, 'production_crates_unchanged': True,
         'new_formal_proof': False, 'elapsed_time_benchmark': False,
         'new_chaos_or_minio_acceptance': False}
(DEST / 'source-audit.json').write_text(json.dumps(audit, indent=2) + '\n')
summary = {'status': 'Prototype/model/lifetime qualified; complete algorithm proof and performance gates open.',
           'source_base': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip(),
           'roots': {name: str(path) for name, path in roots.items()},
           'standalone_tests': 11, 'engine_tests_including_standalone': 213, 'existing_ignored_engine_tests': 25,
           'mutation_prefixes': 6770, 'bound_pairs': 818, 'mixed_iterator_patterns': 5,
           'rejecting_rust_controls': 6, 'allocation_rows': 54, 'allocation_cases': 6,
           'depth_small_stack': {'levels': 4096, 'stack_bytes': 65536},
           'concurrent_release': {'threads': 8, 'levels': 2048, 'stack_bytes': 65536},
           'layout_bytes': {'map': 16, 'node': 48, 'branch': 96, 'edge': 16, 'entry': 48, 'arc': 8},
           'formal_refinement_complete': False, 'new_database_qps': False, 'production_promoted': False,
           'next': 'Source-bound finite-map proof of actual split/collapse/routing/cursor/frame/snapshot algorithms, then predeclared matched timing.'}
(DEST / 'qualification.json').write_text(json.dumps(summary, indent=2) + '\n')
(DEST / 'allocation-observations.json').write_text(json.dumps(observations, indent=2) + '\n')
members = []
inputs = []
for prefix, root in roots.items():
    for path in sorted(root.rglob('*')):
        if not path.is_file() or '__pycache__' in path.parts:
            continue
        with path.open('rb') as file:
            magic = file.read(4)
        if magic == b'\x7fELF':
            continue
        inputs.append((prefix + '/' + str(path.relative_to(root)), path))
for path in sorted((REPO / 'scripts/resident-radix').rglob('*')):
    if path.is_file() and '__pycache__' not in path.parts:
        inputs.append(('repo/' + str(path.relative_to(REPO)), path))
inputs.append(('repo/scripts/build_cache.py', REPO / 'scripts/build_cache.py'))
archive = DEST / 'evidence.tar.gz'
with archive.open('xb') as raw, gzip.GzipFile(fileobj=raw, mode='wb', mtime=0) as zipped, tarfile.open(fileobj=zipped, mode='w') as tar:
    for name, path in sorted(inputs):
        data = path.read_bytes()
        info = tarfile.TarInfo(name)
        info.size = len(data)
        info.mode = 0o644
        tar.addfile(info, io.BytesIO(data))
        members.append({'path': name, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
with tarfile.open(archive) as tar:
    assert tar.getnames() == [row['path'] for row in members]
    for row in members:
        data = tar.extractfile(row['path']).read()
        assert len(data) == row['bytes'] and hashlib.sha256(data).hexdigest() == row['sha256']
(DEST / 'members.json').write_text(json.dumps(members, indent=2) + '\n')
manifest = {'archive': 'evidence.tar.gz', 'sha256': sha(archive), 'bytes': archive.stat().st_size,
            'members': len(members), 'expanded_bytes': sum(row['bytes'] for row in members),
            'all_members_read_back': True, 'excluded': ['ELFs', 'Cargo target', 'Python bytecode']}
(DEST / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps({'audit': audit, 'packet': manifest}))
