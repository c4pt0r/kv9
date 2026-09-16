#!/usr/bin/env python3
"""Retain a fresh local prototype build, source identity and model-test results."""
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
CRATE = Path(__file__).resolve().parent
OUT = Path(sys.argv[1]).resolve()
OUT.mkdir(parents=True, exist_ok=False)
assert OUT.is_relative_to(Path('/mnt/data/kv9-work'))
assert not any(os.environ.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'))
os.environ['CARGO_TARGET_DIR'] = str(REPO / 'target')
os.environ['CARGO_BUILD_JOBS'] = '4'
sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
source_files = [p for p in CRATE.rglob('*') if p.is_file() and '__pycache__' not in p.parts]
source_files.append(REPO / 'scripts/build_cache.py')
sources = {str(p.relative_to(REPO)): sha(p) for p in sorted(source_files)}
for name in sources:
    target = OUT / 'source' / name
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(REPO / name, target)
(OUT / 'sources.json').write_text(json.dumps(sources, indent=2) + '\n')
(OUT / 'rustc.txt').write_bytes(subprocess.check_output(['rustc', '-vV']))
sysroot = Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip())
arc_source = sysroot / 'lib/rustlib/src/rust/library/alloc/src/sync.rs'
if arc_source.exists():
    shutil.copyfile(arc_source, OUT / 'rust-arc-sync.rs')

spec = importlib.util.spec_from_file_location('cache', REPO / 'scripts/build_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
result = {'complete': False, 'started_ns': time.time_ns(), 'sources': sources,
          'scope': 'Isolated radix prototype model tests; no formal proof, engine acceptance or timing.'}
try:
    with cache.BuildCache(CRATE, OUT, False, sources) as build:
        for name, command in [
            ('clippy', ['cargo', 'clippy', '--offline', '--locked', '--all-targets', '--', '-D', 'warnings']),
            ('cargo', ['cargo', 'test', '--offline', '--locked', '--all-targets', '--no-run', '--message-format', 'json-render-diagnostics']),
        ]:
            with (OUT / (name + '.jsonl')).open('x') as stdout, (OUT / (name + '.stderr')).open('x') as stderr:
                build.run(command, stdout=stdout, stderr=stderr)
        build.check_artifacts(OUT / 'cargo.jsonl')
        units = [json.loads(line) for line in (OUT / 'cargo.jsonl').read_text().splitlines()]
        units = [u for u in units if u.get('reason') == 'compiler-artifact' and u.get('executable')]
        assert len(units) == 1
        elf = OUT / 'radix-model-tests'
        shutil.copyfile(units[0]['executable'], elf)
        elf.chmod(0o755)
        result['binary'] = {'sha256': sha(elf), 'bytes': elf.stat().st_size}
    command = [str(elf), '--test-threads=1', '--nocapture']
    result['test_command'] = command
    with (OUT / 'tests.stdout').open('x') as stdout, (OUT / 'tests.stderr').open('x') as stderr:
        completed = subprocess.run(command, stdout=stdout, stderr=stderr, cwd=CRATE, timeout=900)
    result['test_exit_code'] = completed.returncode
    assert completed.returncode == 0
    lines = [line for line in (OUT / 'tests.stdout').read_text().splitlines() if line.startswith('test result:')]
    assert len(lines) == 1
    counts = re.search(r'(\d+) passed; (\d+) failed; (\d+) ignored', lines[0])
    assert counts and int(counts[1]) == 11 and int(counts[2]) == 0 and int(counts[3]) == 0
    result['tests'] = {'passed': int(counts[1]), 'failed': int(counts[2]), 'ignored': int(counts[3])}
    assert all(sha(REPO / name) == digest for name, digest in sources.items())
    result['complete'] = True
    print(json.dumps(result['tests']), flush=True)
except BaseException as error:
    result['failure'] = repr(error)
    raise
finally:
    result['ended_ns'] = time.time_ns()
    (OUT / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
