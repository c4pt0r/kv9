#!/usr/bin/env python3
"""Check the new dependency composition and unchanged engine in isolated copies."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

S = Path(sys.argv[1]).resolve()
R = Path(__file__).resolve().parents[2]
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
assert json.loads((S / 'proof-isolated/result.json').read_text())['accepted']
assert json.loads((S / 'build-summary.json').read_text())['complete']
os.environ['CARGO_TARGET_DIR'] = str(R / 'target')
os.environ['CARGO_BUILD_JOBS'] = '4'
assert not any(os.environ.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CFLAGS'))
spec = importlib.util.spec_from_file_location('cache', R / 'scripts/build_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
summary = dict(complete=False, started_ns=time.time_ns(), cases=[])
try:
    for label, flags in [('archery', ['--all-features']), ('rpds', ['--all-features']), ('engine', ['--workspace'])]:
        source = S / ('tests-' + label + '-source')
        out = S / ('tests-' + label)
        out.mkdir(exist_ok=False)
        row = dict(label=label, complete=False, flags=flags)
        summary['cases'].append(row)
        with (out / 'metadata.json').open('x') as stdout, (out / 'metadata.stderr').open('x') as stderr:
            subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1'], cwd=source, stdout=stdout, stderr=stderr, check=True, timeout=120)
        paths = [Path(__file__).resolve(), source / 'Cargo.toml', source / 'Cargo.lock', *sorted(source.rglob('*.rs')),
                 S / 'candidate-triomphe/Cargo.toml', *sorted((S / 'candidate-triomphe/src').rglob('*.rs'))]
        if label == 'engine':
            for name in ('engine', 'common'):
                paths.append(source / 'crates' / name / 'Cargo.toml')
                for p in (source / 'crates' / name).rglob('*'):
                    if p.is_file():
                        assert p.read_bytes() == (R / p.relative_to(source)).read_bytes(), p
        identity = {str(p): sha(p) for p in paths}
        (out / 'sources.json').write_text(json.dumps(identity, indent=2) + '\n')
        with cache.BuildCache(source, out, False, identity) as build:
            with (out / 'dependency-clean.stdout').open('x') as stdout, (out / 'dependency-clean.stderr').open('x') as stderr:
                build.run(['cargo', 'clean', '--offline', '--locked', '--profile', 'dev', '-p', 'triomphe', '-p', 'archery'], stdout=stdout, stderr=stderr)
            with (out / 'cargo.jsonl').open('x') as stdout, (out / 'cargo.stderr').open('x') as stderr:
                build.run(['cargo', 'test', '--offline', '--locked', *flags, '--lib', '--no-run', '--message-format', 'json-render-diagnostics'], stdout=stdout, stderr=stderr)
            build.check_artifacts(out / 'cargo.jsonl')
            units = [json.loads(line) for line in (out / 'cargo.jsonl').read_text().splitlines()]
            tri = [u for u in units if u.get('reason') == 'compiler-artifact' and u['target']['name'] == 'triomphe']
            assert len(tri) == 1 and not tri[0]['fresh'] and (S / 'candidate-triomphe').as_uri() in tri[0]['package_id']
            if label != 'archery':
                arc = [u for u in units if u.get('reason') == 'compiler-artifact' and u['target']['name'] == 'archery']
                assert len(arc) == 1 and not arc[0]['fresh'] and 'registry+' in arc[0]['package_id']
            selected = [u for u in units if u.get('reason') == 'compiler-artifact' and u.get('executable') and u['package_id'] in build.packages]
            assert len(selected) == (2 if label == 'engine' else 1)
            binaries = []
            for unit in selected:
                elf = out / (unit['target']['name'] + '-tests')
                shutil.copyfile(unit['executable'], elf)
                elf.chmod(0o755)
                binaries.append(dict(path=str(elf), sha256=sha(elf), bytes=elf.stat().st_size))
        row['binaries'] = binaries
        row['results'] = []
        for binary in binaries:
            name = Path(binary['path']).name
            with (out / (name + '.stdout')).open('x') as stdout, (out / (name + '.stderr')).open('x') as stderr:
                subprocess.run([binary['path'], '--test-threads=4'], cwd=source, stdout=stdout, stderr=stderr, check=True, timeout=900)
            result = [line for line in (out / (name + '.stdout')).read_text().splitlines() if line.startswith('test result:')]
            assert len(result) == 1 and '0 failed' in result[0]
            row['results'] += result
        assert all(sha(Path(p)) == h for p, h in identity.items())
        row['complete'] = True
        print(json.dumps(row), flush=True)
    summary['complete'] = True
except BaseException as error:
    summary['failure'] = repr(error)
    raise
finally:
    summary['ended_ns'] = time.time_ns()
    (S / 'tests-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
