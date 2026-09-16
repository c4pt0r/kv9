#!/usr/bin/env python3
"""Retain a fresh release allocation probe using the exact safe index source."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[2]
OUT = Path(sys.argv[1]).resolve()
OUT.mkdir(parents=True, exist_ok=False)
assert OUT.is_relative_to(Path('/mnt/data/kv9-work'))
assert not any(os.environ.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'))
os.environ['CARGO_TARGET_DIR'] = str(REPO / 'target')
os.environ['CARGO_BUILD_JOBS'] = '4'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
work = OUT / 'source'
work.mkdir()
for source, target in [('scripts/resident-radix/src/lib.rs', 'radix.rs'),
                       ('scripts/resident-radix/layout.rs', 'main.rs')]:
    shutil.copyfile(REPO / source, work / target)
# Reuse the existing request counter. Only the allocator delegate changes;
# request size/count semantics are independent of usable-size/RSS behavior.
original = (REPO / 'scripts/engine-interface-counting/src/counting.rs').read_text()
assert original.count('tikv_jemallocator::Jemalloc') == 4
(work / 'counting.rs').write_text(original.replace('tikv_jemallocator::Jemalloc', 'std::alloc::System'))
(work / 'Cargo.toml').write_text('''[package]
name = "kv9-radix-allocation-probe"
version = "0.0.0"
edition = "2021"
publish = false
[workspace]
[[bin]]
name = "kv9-radix-allocation-probe"
path = "main.rs"
[profile.release]
lto = "thin"
codegen-units = 1
''')
subprocess.run(['cargo', 'generate-lockfile', '--offline'], cwd=work, check=True, timeout=30)
sources = {str(p): sha(p) for p in sorted(work.iterdir()) if p.is_file()}
for p in [Path(__file__).resolve(), REPO / 'scripts/build_cache.py',
          REPO / 'scripts/engine-interface-counting/src/counting.rs']:
    sources[str(p)] = sha(p)
(OUT / 'sources.json').write_text(json.dumps(sources, indent=2) + '\n')
spec = importlib.util.spec_from_file_location('cache', REPO / 'scripts/build_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
result = {'complete': False, 'started_ns': time.time_ns(), 'sources': sources}
try:
    with cache.BuildCache(work, OUT, True, sources) as build:
        for name, command in [
            ('clippy', ['cargo', 'clippy', '--offline', '--locked', '--release', '--bin', 'kv9-radix-allocation-probe', '--', '-D', 'warnings']),
            ('cargo', ['cargo', 'build', '--offline', '--locked', '--release', '--message-format', 'json-render-diagnostics']),
        ]:
            with (OUT / (name + '.jsonl')).open('x') as stdout, (OUT / (name + '.stderr')).open('x') as stderr:
                build.run(command, stdout=stdout, stderr=stderr)
        build.check_artifacts(OUT / 'cargo.jsonl')
        elf = OUT / 'allocation-probe'
        shutil.copyfile(REPO / 'target/release/kv9-radix-allocation-probe', elf)
        elf.chmod(0o755)
        result['binary'] = {'sha256': sha(elf), 'bytes': elf.stat().st_size}
    with (OUT / 'observations.json').open('x') as stdout, (OUT / 'observations.stderr').open('x') as stderr:
        p = subprocess.run([str(elf)], stdout=stdout, stderr=stderr, check=True, timeout=120)
    observations = json.loads((OUT / 'observations.json').read_text())
    assert observations['complete'] and not observations['elapsed_time_recorded']
    assert len(observations['rows']) == 54
    assert observations['all_six_cases_return_to_requested_byte_baseline']
    assert (work / 'radix.rs').read_bytes() == (REPO / 'scripts/resident-radix/src/lib.rs').read_bytes()
    assert all(sha(Path(name)) == digest for name, digest in sources.items())
    result.update(complete=True, rows=54, allocator='System', elapsed_time_recorded=False)
    print(json.dumps({'complete': True, 'rows': 54, 'all_six_teardowns_return_to_baseline': True}))
except BaseException as error:
    result['failure'] = repr(error)
    raise
finally:
    result['ended_ns'] = time.time_ns()
    (OUT / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
