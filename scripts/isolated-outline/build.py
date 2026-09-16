#!/usr/bin/env python3
"""Compile the one-dependency proposal without executing a performance workload."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import tomllib

S = Path(sys.argv[1]).resolve()
R = Path(__file__).resolve().parents[2]
H = R / 'scripts/engine-interface-experiment'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
plan = json.loads((S / 'qualification-plan.json').read_text())
assert not plan['timing_authorized'] and plan['original_archery_unmodified']
for path, digest in plan['source_pins'].items():
    assert sha(Path(path)) == digest, path
os.environ['CARGO_TARGET_DIR'] = str(R / 'target')
os.environ['CARGO_BUILD_JOBS'] = '4'
assert not any(os.environ.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CFLAGS'))
spec = importlib.util.spec_from_file_location('cache', R / 'scripts/build_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
registry = lambda path: {(v['name'], v['version'], v['source']): v['checksum']
                         for v in tomllib.loads(path.read_text())['package']
                         if v.get('source', '').startswith('registry+')}
reference = registry(R / 'Cargo.lock')
summary = dict(complete=False, started_ns=time.time_ns(), timing_executed=False,
               sources={}, arms={}, plan_sha256=sha(S / 'qualification-plan.json'))
try:
    for arm in ('baseline', 'candidate'):
        work = S / ('source-' + arm)
        work.mkdir(exist_ok=False)
        shutil.copytree(H / 'src', work / 'src')
        manifest = (H / 'Cargo.toml').read_text()
        for name in ('common', 'engine'):
            manifest = manifest.replace(f'../../crates/{name}', str(R / 'crates' / name))
        if arm == 'candidate':
            manifest += f'\n[patch.crates-io]\ntriomphe = {{ path = "{S / "candidate-triomphe"}" }}\n'
        (work / 'Cargo.toml').write_text(manifest)
        shutil.copyfile(H / 'Cargo.lock', work / 'Cargo.lock')
        output = S / ('build-' + arm)
        output.mkdir(exist_ok=False)
        row = dict(work=str(work), output=str(output))
        summary['arms'][arm] = row
        with (output / 'metadata.json').open('x') as out, (output / 'metadata.stderr').open('x') as err:
            subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1'], cwd=work, stdout=out, stderr=err, check=True, timeout=120)
        graph = json.loads((output / 'metadata.json').read_text())
        assert all(reference.get(k) == v for k, v in registry(work / 'Cargo.lock').items())
        paths = [Path(__file__).resolve(), R / 'scripts/build_cache.py',
                 S / 'qualification-plan.json', H / 'Cargo.toml', H / 'Cargo.lock',
                 *sorted((H / 'src').rglob('*.rs')), R / 'Cargo.toml', R / 'Cargo.lock',
                 work / 'Cargo.toml', work / 'Cargo.lock', *sorted((work / 'src').rglob('*.rs'))]
        for name in ('common', 'engine'):
            paths += [R / 'crates' / name / 'Cargo.toml', *sorted((R / 'crates' / name / 'src').rglob('*.rs'))]
        for name in ('archery', 'rpds', 'triomphe'):
            packages = [p for p in graph['packages'] if p['name'] == name]
            assert len(packages) == 1
            package = packages[0]
            dep = Path(package['manifest_path']).parent
            if name == 'triomphe' and arm == 'candidate':
                assert package['source'] is None and dep == S / 'candidate-triomphe'
            else:
                assert package['source'].startswith('registry+')
                original = Path(plan['archives'][name]['source'])
                for source in [original / 'Cargo.toml', *sorted((original / 'src').rglob('*.rs'))]:
                    assert source.read_bytes() == (dep / source.relative_to(original)).read_bytes(), source
            paths += [dep / 'Cargo.toml', *sorted((dep / 'src').rglob('*.rs'))]
        identity = {str(p): sha(p) for p in paths}
        summary['sources'].update(identity)
        (output / 'sources.json').write_text(json.dumps(identity, indent=2) + '\n')
        for path in paths:
            relative = (Path('repo') / path.relative_to(R) if path.is_relative_to(R)
                        else Path('experiment') / path.relative_to(S) if path.is_relative_to(S)
                        else Path('registry') / path.relative_to('/home/dongxu/.cargo/registry/src'))
            dest = output / 'source' / relative
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, dest)
        with cache.BuildCache(work, output, True, identity) as build:
            command = ['cargo', 'clean', '--offline', '--locked', '--profile', 'release']
            for name in ('kv9-engine', 'kv9-common', 'archery', 'rpds', 'triomphe'):
                command += ['-p', name]
            with (output / 'dependency-clean.stdout').open('x') as out, (output / 'dependency-clean.stderr').open('x') as err:
                build.run(command, stdout=out, stderr=err)
            with (output / 'clippy.stdout').open('x') as out, (output / 'clippy.stderr').open('x') as err:
                build.run(['cargo', 'clippy', '--offline', '--locked', '--release', '--all-targets', '--', '-D', 'warnings'], stdout=out, stderr=err)
            with (output / 'cargo.jsonl').open('x') as out, (output / 'cargo.stderr').open('x') as err:
                build.run(['cargo', 'build', '--offline', '--locked', '--release', '--message-format', 'json-render-diagnostics'], stdout=out, stderr=err)
            build.check_artifacts(output / 'cargo.jsonl')
            units = [json.loads(line) for line in (output / 'cargo.jsonl').read_text().splitlines()]
            for name in ('kv9_engine', 'kv9_common', 'archery', 'rpds', 'triomphe'):
                selected = [u for u in units if u.get('reason') == 'compiler-artifact' and u['target']['name'] == name]
                assert len(selected) == 1 and not selected[0]['fresh'], name
                if name in ('archery', 'rpds'):
                    assert 'registry+' in selected[0]['package_id']
                elif name == 'triomphe':
                    assert ((S / 'candidate-triomphe').as_uri() if arm == 'candidate' else 'registry+') in selected[0]['package_id']
            binary = output / 'engine-interface'
            shutil.copyfile(R / 'target/release/kv9-engine-interface-experiment', binary)
            binary.chmod(0o755)
            row['binary'] = dict(path=str(binary), sha256=sha(binary), bytes=binary.stat().st_size)
        for tool, filename, flags in [('nm', 'symbols.txt', ['-SC']), ('objdump', 'disassembly.txt', ['-dC', '-Mintel']), ('readelf', 'relocations.txt', ['-Wr'])]:
            with (output / filename).open('x') as out:
                subprocess.run([tool, *flags, str(binary)], stdout=out, check=True, timeout=120)
        assert all(sha(Path(p)) == h for p, h in identity.items())
        row['complete'] = True
        print(json.dumps({'arm': arm, 'binary': row['binary']}), flush=True)
    left = registry(S / 'source-baseline/Cargo.lock')
    right = registry(S / 'source-candidate/Cargo.lock')
    removed = set(left) - set(right)
    assert len(removed) == 1 and next(iter(removed))[:2] == ('triomphe', '0.1.16')
    assert {k: v for k, v in left.items() if k not in removed} == right
    assert all(sha(Path(p)) == h for p, h in summary['sources'].items())
    summary['complete'] = True
except BaseException as error:
    summary['failure'] = repr(error)
    raise
finally:
    summary['ended_ns'] = time.time_ns()
    (S / 'build-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
