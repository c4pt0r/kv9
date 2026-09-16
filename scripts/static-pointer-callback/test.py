#!/usr/bin/env python3
"""Retain sequential baseline/candidate dependency test evidence."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

parser = argparse.ArgumentParser(allow_abbrev=False)
parser.add_argument("--work", type=Path, required=True)
ROOT = parser.parse_args().work.resolve()
REPO = Path(__file__).resolve().parents[2]
os.environ['CARGO_TARGET_DIR'] = str(REPO / 'target')
os.environ['CARGO_BUILD_JOBS'] = '4'
assert not any(os.environ.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CFLAGS'))
spec = importlib.util.spec_from_file_location('cache', REPO / 'scripts/build_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
summary = {'complete': False, 'started_ns': time.time_ns(), 'cases': []}

try:
    for name in ('baseline-archery', 'candidate-archery', 'candidate-rpds'):
        source = ROOT / name
        output = ROOT / ('tests-' + name)
        output.mkdir(exist_ok=False)
        row = {'name': name, 'output': str(output), 'complete': False}
        summary['cases'].append(row)
        with (output / 'resolve.json').open('x') as out, (output / 'resolve.stderr').open('x') as err:
            command = ['cargo', 'metadata', '--offline', '--all-features', '--format-version', '1']
            p = subprocess.run(command, cwd=source, stdout=out, stderr=err, timeout=120)
        row['resolve_exit_code'] = p.returncode
        if p.returncode:
            raise RuntimeError('Offline metadata failed: ' + name)
        paths = [source / 'Cargo.toml', source / 'Cargo.lock', *sorted((source / 'src').rglob('*.rs'))]
        if name == 'candidate-rpds':
            dep = ROOT / 'candidate-archery'
            paths += [dep / 'Cargo.toml', *sorted((dep / 'src').rglob('*.rs'))]
        identity = {str(p): sha(p) for p in paths}
        (output / 'sources.json').write_text(json.dumps(identity, indent=2) + '\n')
        for path in paths:
            dest = output / 'source' / path.relative_to(ROOT)
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, dest)
        with cache.BuildCache(source, output, False, identity) as build:
            if name == 'candidate-rpds':
                with (output / 'dependency-clean.stdout').open('x') as out, (output / 'dependency-clean.stderr').open('x') as err:
                    build.run(['cargo', 'clean', '--offline', '--locked', '--profile', 'dev', '-p', 'archery'], stdout=out, stderr=err)
            command = ['cargo', 'test', '--offline', '--locked', '--all-features', '--lib', '--no-run', '--message-format', 'json-render-diagnostics']
            with (output / 'cargo.jsonl').open('x') as out, (output / 'cargo.stderr').open('x') as err:
                build.run(command, stdout=out, stderr=err)
            build.check_artifacts(output / 'cargo.jsonl')
            units = [json.loads(line) for line in (output / 'cargo.jsonl').read_text().splitlines()]
            selected = [u for u in units if u.get('reason') == 'compiler-artifact' and u.get('executable') and u['package_id'] in build.packages]
            assert len(selected) == 1
            if name == 'candidate-rpds':
                archery = [u for u in units if u.get('reason') == 'compiler-artifact' and u['target']['name'] == 'archery']
                assert len(archery) == 1 and not archery[0]['fresh']
                assert (ROOT / 'candidate-archery').as_uri() in archery[0]['package_id']
            elf = output / 'tests'
            shutil.copyfile(selected[0]['executable'], elf)
            elf.chmod(0o755)
            row['binary'] = {'sha256': sha(elf), 'bytes': elf.stat().st_size}
        command = [str(elf), '--test-threads=4']
        with (output / 'tests.stdout').open('x') as out, (output / 'tests.stderr').open('x') as err:
            p = subprocess.run(command, cwd=source, stdout=out, stderr=err, timeout=900)
        row['test_exit_code'] = p.returncode
        assert p.returncode == 0, name
        row['result'] = [line for line in (output / 'tests.stdout').read_text().splitlines() if line.startswith('test result:')]
        assert len(row['result']) == 1
        assert all(sha(Path(p)) == value for p, value in identity.items())
        row['complete'] = True
        print(json.dumps(row), flush=True)
    summary['complete'] = True
except BaseException as error:
    summary['failure'] = repr(error)
    raise
finally:
    summary['ended_ns'] = time.time_ns()
    (ROOT / 'tests-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
