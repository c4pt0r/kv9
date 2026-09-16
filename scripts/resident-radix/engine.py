#!/usr/bin/env python3
"""Qualify radix in an isolated engine copy; never edit production crates."""
import difflib
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[2]
OUT = Path(sys.argv[1]).resolve()
OUT.mkdir(parents=True, exist_ok=False)
assert OUT.is_relative_to(Path('/mnt/data/kv9-work'))
assert not any(os.environ.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CFLAGS'))
os.environ['CARGO_TARGET_DIR'] = str(REPO / 'target')
os.environ['CARGO_BUILD_JOBS'] = '4'
sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
work = OUT / 'candidate'
work.mkdir()
manifest = (REPO / 'Cargo.toml').read_text().split('\n[package]\n')[0]
start = manifest.index('members = [')
end = manifest.index('\n]', start) + 2
manifest = manifest[:start] + 'members = ["crates/common", "crates/engine"]' + manifest[end:]
(work / 'Cargo.toml').write_text(manifest)
shutil.copyfile(REPO / 'Cargo.lock', work / 'Cargo.lock')
for name in ('common', 'engine'):
    shutil.copytree(REPO / 'crates' / name, work / 'crates' / name)
engine = work / 'crates/engine'
mem = engine / 'src/mem.rs'
original = mem.read_text()
updated = original
for before, after in [
    ('use rpds::RedBlackTreeMapSync;', 'use crate::radix::RadixMap;'),
    ('type CfMap = RedBlackTreeMapSync<Vec<u8>, Vec<u8>>;', 'type CfMap = RadixMap;'),
]:
    assert updated.count(before) == 1
    updated = updated.replace(before, after)
mem.write_text(updated)
(OUT / 'mem.patch').write_text(''.join(difflib.unified_diff(
    original.splitlines(True), updated.splitlines(True), fromfile='original/mem.rs', tofile='candidate/mem.rs')))
lib = engine / 'src/lib.rs'
assert lib.read_text().count('pub mod mem;') == 1
lib.write_text(lib.read_text().replace('pub mod mem;', 'pub mod mem;\npub mod radix;'))
shutil.copyfile(REPO / 'scripts/resident-radix/src/lib.rs', engine / 'src/radix.rs')
(engine / 'src/radix').mkdir()
shutil.copyfile(REPO / 'scripts/resident-radix/src/tests.rs', engine / 'src/radix/tests.rs')
for source, target in [('scripts/inline-key/model.rs', 'radix_engine_model.rs'),
                       ('scripts/entry-buffer/model.rs', 'radix_payload_model.rs')]:
    shutil.copyfile(REPO / source, engine / 'tests' / target)
with (OUT / 'metadata.json').open('x') as stdout, (OUT / 'metadata.stderr').open('x') as stderr:
    subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1'], cwd=work,
                   stdout=stdout, stderr=stderr, check=True, timeout=120)
sources = {str(p): sha(p) for p in sorted(work.rglob('*')) if p.is_file()}
for p in [Path(__file__).resolve(), REPO / 'scripts/build_cache.py']:
    sources[str(p)] = sha(p)
(OUT / 'sources.json').write_text(json.dumps(sources, indent=2) + '\n')
spec = importlib.util.spec_from_file_location('cache', REPO / 'scripts/build_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
result = {'complete': False, 'started_ns': time.time_ns(), 'results': [], 'binaries': [],
          'scope': 'Isolated common/engine suites and independent models. No new external MinIO, Chaos Mesh, formal proof or timing.'}
try:
    with cache.BuildCache(work, OUT, False, sources) as build:
        for name, command in [
            ('clippy', ['cargo', 'clippy', '--offline', '--locked', '--workspace', '--all-targets', '--', '-D', 'warnings']),
            ('cargo', ['cargo', 'test', '--offline', '--locked', '--workspace', '--all-targets', '--no-run', '--message-format', 'json-render-diagnostics']),
        ]:
            with (OUT / (name + '.jsonl')).open('x') as stdout, (OUT / (name + '.stderr')).open('x') as stderr:
                build.run(command, stdout=stdout, stderr=stderr)
        build.check_artifacts(OUT / 'cargo.jsonl')
        for unit in map(json.loads, (OUT / 'cargo.jsonl').read_text().splitlines()):
            if unit.get('reason') != 'compiler-artifact' or not unit.get('executable') or unit['package_id'] not in build.packages:
                continue
            elf = OUT / Path(unit['executable']).name
            shutil.copyfile(unit['executable'], elf)
            elf.chmod(0o755)
            result['binaries'].append({'path': str(elf), 'sha256': sha(elf), 'bytes': elf.stat().st_size})
    assert result['binaries']
    for binary in result['binaries']:
        elf = Path(binary['path'])
        with (OUT / (elf.name + '.stdout')).open('x') as stdout, (OUT / (elf.name + '.stderr')).open('x') as stderr:
            process = subprocess.run([str(elf), '--test-threads=4'], cwd=work,
                                     stdout=stdout, stderr=stderr, timeout=900)
        row = {'binary': elf.name, 'exit_code': process.returncode}
        result['results'].append(row)
        assert process.returncode == 0
        lines = [line for line in (OUT / (elf.name + '.stdout')).read_text().splitlines() if line.startswith('test result:')]
        assert len(lines) == 1
        counts = re.search(r'(\d+) passed; (\d+) failed; (\d+) ignored', lines[0])
        assert counts and int(counts[2]) == 0
        row.update(passed=int(counts[1]), failed=int(counts[2]), ignored=int(counts[3]))
    assert all(sha(Path(name)) == digest for name, digest in sources.items())
    result['complete'] = True
    print(json.dumps({'complete': True, 'passed': sum(r['passed'] for r in result['results']),
                      'ignored': sum(r['ignored'] for r in result['results'])}), flush=True)
except BaseException as error:
    result['failure'] = repr(error)
    raise
finally:
    result['ended_ns'] = time.time_ns()
    (OUT / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
