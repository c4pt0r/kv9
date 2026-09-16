#!/usr/bin/env python3
"""Finish engine/common tests after removing the unrelated root binary fixture."""
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
work = S / 'tests-engine-source-v2'
out = S / 'tests-engine-v2'
out.mkdir(exist_ok=False)
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
os.environ['CARGO_TARGET_DIR'] = str(R / 'target')
os.environ['CARGO_BUILD_JOBS'] = '4'
assert not any(os.environ.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CFLAGS'))
spec = importlib.util.spec_from_file_location('cache', R / 'scripts/build_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
result = dict(complete=False, started_ns=time.time_ns(), results=[])
try:
    for name in ('engine', 'common'):
        for p in (work / 'crates' / name).rglob('*'):
            if p.is_file():
                assert p.read_bytes() == (R / p.relative_to(work)).read_bytes()
    with (out / 'metadata.json').open('x') as stdout, (out / 'metadata.stderr').open('x') as stderr:
        subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1'], cwd=work, stdout=stdout, stderr=stderr, check=True, timeout=120)
    paths = [Path(__file__).resolve(), work / 'Cargo.toml', work / 'Cargo.lock',
             *sorted(work.rglob('*.rs')), work / 'crates/engine/Cargo.toml', work / 'crates/common/Cargo.toml',
             S / 'candidate-triomphe/Cargo.toml', *sorted((S / 'candidate-triomphe/src').rglob('*.rs'))]
    identity = {str(p): sha(p) for p in paths}
    (out / 'sources.json').write_text(json.dumps(identity, indent=2) + '\n')
    with cache.BuildCache(work, out, False, identity) as build:
        with (out / 'dependency-clean.stdout').open('x') as stdout, (out / 'dependency-clean.stderr').open('x') as stderr:
            build.run(['cargo', 'clean', '--offline', '--locked', '--profile', 'dev', '-p', 'triomphe', '-p', 'archery'], stdout=stdout, stderr=stderr)
        with (out / 'cargo.jsonl').open('x') as stdout, (out / 'cargo.stderr').open('x') as stderr:
            build.run(['cargo', 'test', '--offline', '--locked', '--workspace', '--lib', '--no-run', '--message-format', 'json-render-diagnostics'], stdout=stdout, stderr=stderr)
        build.check_artifacts(out / 'cargo.jsonl')
        units = [json.loads(line) for line in (out / 'cargo.jsonl').read_text().splitlines()]
        for name in ('archery', 'triomphe'):
            matches = [u for u in units if u.get('reason') == 'compiler-artifact' and u['target']['name'] == name]
            assert len(matches) == 1 and not matches[0]['fresh']
            assert ((S / 'candidate-triomphe').as_uri() if name == 'triomphe' else 'registry+') in matches[0]['package_id']
        selected = [u for u in units if u.get('reason') == 'compiler-artifact' and u.get('executable') and u['package_id'] in build.packages]
        assert len(selected) == 2
        binaries = []
        for unit in selected:
            elf = out / (unit['target']['name'] + '-tests')
            shutil.copyfile(unit['executable'], elf)
            elf.chmod(0o755)
            binaries.append(dict(path=str(elf), sha256=sha(elf), bytes=elf.stat().st_size))
    result['binaries'] = binaries
    for binary in binaries:
        name = Path(binary['path']).name
        with (out / (name + '.stdout')).open('x') as stdout, (out / (name + '.stderr')).open('x') as stderr:
            subprocess.run([binary['path'], '--test-threads=4'], cwd=work, stdout=stdout, stderr=stderr, check=True, timeout=900)
        lines = [line for line in (out / (name + '.stdout')).read_text().splitlines() if line.startswith('test result:')]
        assert len(lines) == 1 and '0 failed' in lines[0]
        result['results'] += lines
    assert all(sha(Path(p)) == h for p, h in identity.items())
    result['complete'] = True
except BaseException as error:
    result['failure'] = repr(error)
    raise
finally:
    result['ended_ns'] = time.time_ns()
    (S / 'tests-engine-v2-summary.json').write_text(json.dumps(result, indent=2) + '\n')
prior = json.loads((S / 'tests-summary.json').read_text())
cases = prior['cases'][:2]
assert [c['label'] for c in cases] == ['archery', 'rpds'] and all(c['complete'] for c in cases)
for label in ('archery', 'rpds'):
    pins = json.loads((S / ('tests-' + label) / 'sources.json').read_text())
    assert all(sha(Path(p)) == h for p, h in pins.items())
accepted = dict(complete=True, dependency_cases=cases, engine_common=result,
                original_failure_retained='tests-summary.json',
                fixture_correction='engine-test-fixture-correction.json')
(S / 'tests-accepted.json').write_text(json.dumps(accepted, indent=2) + '\n')
print(json.dumps({'complete': True, 'results': result['results']}))
