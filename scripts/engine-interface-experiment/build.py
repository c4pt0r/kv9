#!/usr/bin/env python3
"""Retain a serial, ordinary-release pair against qualified dependency sources."""
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
H = Path(__file__).resolve().parent
Q = Path(json.loads((S / 'plan.json').read_text())['qualified_dependency_root'])
attempt = sys.argv[2]
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
assert json.loads((Q / 'tests-summary-corrected.json').read_text())['complete']
assert json.loads((Q / 'proof/result.json').read_text())['accepted']
assert json.loads((Q / 'release-smokes-summary.json').read_text())['complete']
for path, digest in json.loads((Q / 'proof/source-inputs.json').read_text()).items():
    assert sha(Path(path)) == digest, path
os.environ['CARGO_TARGET_DIR'] = str(R / 'target')
os.environ['CARGO_BUILD_JOBS'] = '4'
assert not any(os.environ.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CFLAGS'))
spec = importlib.util.spec_from_file_location('cache', R / 'scripts/build_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
assert (H / 'src/probes.rs').read_bytes() == (R / 'scripts/resident-outlined-mutation/src/probes.rs').read_bytes()
with (S / f'harness-metadata-{attempt}.json').open('x') as out, (S / f'harness-metadata-{attempt}.stderr').open('x') as err:
    subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1'], cwd=H, stdout=out, stderr=err, check=True, timeout=120)

def registry(path):
    return {(v['name'], v['version'], v['source']): v['checksum']
            for v in tomllib.loads(path.read_text())['package']
            if v.get('source', '').startswith('registry+')}

reference = registry(R / 'Cargo.lock')
summary = {'complete': False, 'started_ns': time.time_ns(), 'attempt': attempt,
           'arms': {}, 'sources': {}, 'plan_sha256': sha(S / 'plan.json')}
try:
    for arm in ('baseline', 'candidate'):
        work = S / f'source-{arm}-{attempt}'
        work.mkdir(exist_ok=False)
        shutil.copytree(H / 'src', work / 'src')
        manifest = (H / 'Cargo.toml').read_text()
        for name in ('common', 'engine'):
            manifest = manifest.replace(f'../../crates/{name}', str(R / 'crates' / name))
        manifest += f'\n[patch.crates-io]\narchery = {{ path = "{Q / (arm + "-archery")}" }}\n'
        if arm == 'candidate':
            manifest += f'triomphe = {{ path = "{Q / "candidate-triomphe"}" }}\n'
        (work / 'Cargo.toml').write_text(manifest)
        shutil.copyfile(H / 'Cargo.lock', work / 'Cargo.lock')
        output = S / f'build-{arm}-{attempt}'
        output.mkdir(exist_ok=False)
        artifacts = {'work': str(work), 'output': str(output)}
        summary['arms'][arm] = artifacts
        with (output / 'metadata.json').open('x') as out, (output / 'metadata.stderr').open('x') as err:
            subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1'], cwd=work, stdout=out, stderr=err, check=True, timeout=120)
        assert all(reference.get(key) == value for key, value in registry(work / 'Cargo.lock').items())
        paths = [H / 'Cargo.toml', H / 'Cargo.lock', H / 'build.py', *sorted((H / 'src').rglob('*.rs')),
                 R / 'Cargo.toml', R / 'Cargo.lock', R / 'scripts/build_cache.py',
                 S / 'plan.json', S / 'reference-inputs.py',
                 work / 'Cargo.toml', work / 'Cargo.lock', *sorted((work / 'src').rglob('*.rs')),
                 Q / (arm + '-archery') / 'Cargo.toml', *sorted((Q / (arm + '-archery') / 'src').rglob('*.rs'))]
        for name in ('common', 'engine'):
            paths += [R / 'crates' / name / 'Cargo.toml', *sorted((R / 'crates' / name / 'src').rglob('*.rs'))]
        if arm == 'candidate':
            paths += [Q / 'candidate-triomphe/Cargo.toml', *sorted((Q / 'candidate-triomphe/src').rglob('*.rs'))]
        identity = {str(path): sha(path) for path in paths}
        summary['sources'].update(identity)
        (output / 'sources.json').write_text(json.dumps(identity, indent=2) + '\n')
        for path in paths:
            relative = (Path('repo') / path.relative_to(R) if path.is_relative_to(R)
                        else Path('qualification') / path.relative_to(Q) if path.is_relative_to(Q)
                        else Path('experiment') / path.relative_to(S))
            dest = output / 'source' / relative
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, dest)
        with cache.BuildCache(work, output, True, identity) as build:
            command = ['cargo', 'clean', '--offline', '--locked', '--profile', 'release']
            for name in ('archery', 'rpds', 'triomphe', 'kv9-engine', 'kv9-common'):
                command += ['-p', name]
            with (output / 'dependency-clean.stdout').open('x') as out, (output / 'dependency-clean.stderr').open('x') as err:
                build.run(command, stdout=out, stderr=err)
            with (output / 'clippy.stdout').open('x') as out, (output / 'clippy.stderr').open('x') as err:
                build.run(['cargo', 'clippy', '--offline', '--locked', '--release', '--all-targets', '--', '-D', 'warnings'], stdout=out, stderr=err)
            with (output / 'timing.jsonl').open('x') as out, (output / 'timing.stderr').open('x') as err:
                build.run(['cargo', 'build', '--offline', '--locked', '--release', '--message-format', 'json-render-diagnostics'], stdout=out, stderr=err)
            build.check_artifacts(output / 'timing.jsonl')
            units = [json.loads(line) for line in (output / 'timing.jsonl').read_text().splitlines()]
            for name in ('archery', 'rpds', 'triomphe', 'kv9_engine', 'kv9_common'):
                selected = [u for u in units if u.get('reason') == 'compiler-artifact' and u['target']['name'] == name]
                assert len(selected) == 1 and not selected[0]['fresh'], name
                if name == 'archery':
                    assert (Q / (arm + '-archery')).as_uri() in selected[0]['package_id']
                if name == 'triomphe':
                    assert ((Q / 'candidate-triomphe').as_uri() if arm == 'candidate' else 'registry+') in selected[0]['package_id']
            binary = output / 'timing'
            shutil.copyfile(R / 'target/release/kv9-engine-interface-experiment', binary)
            binary.chmod(0o755)
            artifacts['timing'] = {'path': str(binary), 'sha256': sha(binary), 'bytes': binary.stat().st_size}
        assert all(sha(Path(path)) == digest for path, digest in identity.items())
        print(json.dumps({'arm': arm, 'artifacts': artifacts}), flush=True)
    left = registry(Path(summary['arms']['baseline']['work']) / 'Cargo.lock')
    right = registry(Path(summary['arms']['candidate']['work']) / 'Cargo.lock')
    removed = set(left) - set(right)
    assert len(removed) == 1 and next(iter(removed))[:2] == ('triomphe', '0.1.16')
    assert {k: v for k, v in left.items() if k not in removed} == right
    assert all(sha(Path(path)) == digest for path, digest in summary['sources'].items())
    summary['complete'] = True
except BaseException as error:
    summary['failure'] = repr(error)
    raise
finally:
    summary['ended_ns'] = time.time_ns()
    (S / f'build-summary-{attempt}.json').write_text(json.dumps(summary, indent=2) + '\n')
