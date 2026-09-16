#!/usr/bin/env python3
"""Build the same current MemEngine harness with isolated dependency variants."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import tomllib

parser = argparse.ArgumentParser(allow_abbrev=False)
parser.add_argument("--work", type=Path, required=True)
S = parser.parse_args().work.resolve()
R = Path(__file__).resolve().parents[2]
H = R / 'scripts/resident-selected-profile'
os.environ['CARGO_TARGET_DIR'] = str(R / 'target')
os.environ['CARGO_BUILD_JOBS'] = '4'
assert not any(os.environ.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CFLAGS'))
assert json.loads((S / 'proof/result.json').read_text())['accepted']
assert json.loads((S / 'tests-summary.json').read_text())['complete']
spec = importlib.util.spec_from_file_location('cache', R / 'scripts/build_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
registry = lambda path: {(p['name'], p['version'], p['source']): p['checksum'] for p in tomllib.loads(path.read_text())['package'] if p.get('source', '').startswith('registry+')}
reference = registry(H / 'Cargo.lock')
summary = {'complete': False, 'started_ns': time.time_ns(), 'cases': [], 'timing_eligible': False}
try:
    for arm in ('baseline', 'candidate'):
        work = S / ('codegen-' + arm)
        work.mkdir(exist_ok=False)
        (work / 'src').mkdir()
        shutil.copyfile(H / 'src/main.rs', work / 'src/main.rs')
        manifest = (H / 'Cargo.toml').read_text()
        for crate in ('engine', 'common'):
            manifest = manifest.replace('../../crates/' + crate, str(R / 'crates' / crate))
        manifest += '\n[patch.crates-io]\narchery = { path = "../' + arm + '-archery" }\n'
        (work / 'Cargo.toml').write_text(manifest)
        shutil.copyfile(H / 'Cargo.lock', work / 'Cargo.lock')
        output = S / ('build-codegen-' + arm)
        output.mkdir(exist_ok=False)
        row = {'arm': arm, 'complete': False}
        summary['cases'].append(row)
        with (output / 'resolve.json').open('x') as out, (output / 'resolve.stderr').open('x') as err:
            subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1'], cwd=work, stdout=out, stderr=err, check=True, timeout=120)
        resolved = registry(work / 'Cargo.lock')
        assert all(reference.get(key) == value for key, value in resolved.items())
        assert len(reference) == len(resolved) + 1
        paths = [work / 'Cargo.toml', work / 'Cargo.lock', work / 'src/main.rs',
                 S / (arm + '-archery') / 'Cargo.toml', *sorted((S / (arm + '-archery') / 'src').rglob('*.rs'))]
        for crate in ('engine', 'common'):
            paths += [R / 'crates' / crate / 'Cargo.toml', *sorted((R / 'crates' / crate / 'src').rglob('*.rs'))]
        identity = {str(path): sha(path) for path in paths}
        (output / 'sources.json').write_text(json.dumps(identity, indent=2) + '\n')
        for path in paths:
            relative = Path('repo') / path.relative_to(R) if path.is_relative_to(R) else Path('experiment') / path.relative_to(S)
            destination = output / 'source' / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, destination)
        with cache.BuildCache(work, output, True, identity) as build:
            with (output / 'dependency-clean.stdout').open('x') as out, (output / 'dependency-clean.stderr').open('x') as err:
                build.run(['cargo', 'clean', '--offline', '--locked', '--profile', 'release', '-p', 'kv9-engine', '-p', 'kv9-common', '-p', 'archery', '-p', 'rpds'], stdout=out, stderr=err)
            command = ['cargo', 'build', '--offline', '--locked', '--release', '--message-format', 'json-render-diagnostics']
            with (output / 'cargo.jsonl').open('x') as out, (output / 'cargo.stderr').open('x') as err:
                build.run(command, stdout=out, stderr=err)
            build.check_artifacts(output / 'cargo.jsonl')
            units = [json.loads(line) for line in (output / 'cargo.jsonl').read_text().splitlines()]
            for name in ('kv9_engine', 'kv9_common', 'archery', 'rpds'):
                matches = [unit for unit in units if unit.get('reason') == 'compiler-artifact' and unit['target']['name'] == name]
                assert len(matches) == 1 and not matches[0]['fresh']
                if name == 'archery':
                    assert (S / (arm + '-archery')).as_uri() in matches[0]['package_id']
            elf = output / 'resident-selected-profile'
            shutil.copyfile(R / 'target/release/kv9-resident-selected-profile', elf)
            elf.chmod(0o755)
            row['binary'] = {'sha256': sha(elf), 'bytes': elf.stat().st_size}
        for tool, filename, flags in [('nm', 'symbols.txt', ['-S', '-C', '--defined-only']), ('objdump', 'disassembly.txt', ['-d', '-C']), ('readelf', 'elf-notes.txt', ['-n'])]:
            with (output / filename).open('x') as out:
                subprocess.run([tool, *flags, str(elf)], stdout=out, check=True, timeout=120)
        assert all(sha(Path(path)) == digest for path, digest in identity.items())
        row['complete'] = True
        print(json.dumps(row), flush=True)
    summary['complete'] = True
except BaseException as error:
    summary['failure'] = repr(error)
    raise
finally:
    summary['ended_ns'] = time.time_ns()
    (S / 'codegen-build-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
