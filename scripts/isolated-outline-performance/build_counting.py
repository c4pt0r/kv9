#!/usr/bin/env python3
"""Build allocation-only companions; retain the existing timing ELFs unchanged."""
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
H = R / 'scripts/engine-interface-counting'
T = R / 'scripts/engine-interface-experiment'
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
plan = json.loads((S / 'plan.json').read_text())
declared = json.loads((S / 'declared-plan.json').read_text())
Q = Path(plan['qualified_dependency_root'])
qualified = json.loads((Q / 'build-summary.json').read_text())
assert json.loads((Q / 'qualification-audit.json').read_text())['complete']
assert qualified['complete']
for path, digest in qualified['sources'].items():
    assert sha(Path(path)) == digest, path
for key in ('cases', 'orders', 'gates', 'timing_scopes', 'timing_binaries', 'allocation_companion'):
    assert plan[key] == declared[key], key
for arm in ('baseline', 'candidate'):
    assert plan['timing_binaries'][arm] == qualified['arms'][arm]['binary']
    assert sha(Path(plan['timing_binaries'][arm]['path'])) == plan['timing_binaries'][arm]['sha256']

# A strict source projection checks all operation and correctness paths. Only
# instrumentation/metadata differences are erased; all remaining bytes match.
original = (T / 'src/main.rs').read_text()
counted = (H / 'src/main.rs').read_text()
normalized = counted.replace('//! Allocation-only companion; no elapsed-time observations are emitted.', original.splitlines()[0])
normalized = normalized.replace('mod counting;\n', 'use std::time::Instant;\n')
normalized = normalized.replace('static ALLOCATOR: counting::Allocator = counting::Allocator;', 'static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;')
start = normalized.index('// All buffers are reserved before the counted operation windows.')
end = normalized.index('fn apply_pass(', start)
old_start = original.index('#[derive(Default)]\nstruct Metrics')
old_end = original.index('fn apply_pass(', old_start)
normalized = normalized[:start] + original[old_start:old_end] + normalized[end:]
assert normalized.count('let started = begin();') == 4
normalized = normalized.replace('let started = begin();', 'let started = Instant::now();')
normalized = normalized.replace('    let snapshot_bytes = std::mem::size_of_val(engine.snapshot().unwrap().as_ref());\n', '')
normalized = normalized.replace('"snapshot_bytes":snapshot_bytes,', '')
normalized = normalized.replace('    result["counting_build"] = json!(true);\n', '')
normalized = normalized.replace('let mut measured = match spec["operation"].as_str().unwrap() {', 'let measured = match spec["operation"].as_str().unwrap() {')
block = '''        for row in &mut measured {
            let scope = std::mem::replace(&mut row["unit"], json!("allocator_counts_per_window"));
            row["operation_window"] = scope;
        }
'''
assert normalized.count(block) == 1
normalized = normalized.replace(block, '')
assert normalized == original, 'counting changed an operation or validation path'
assert (H / 'src/counting.rs').read_bytes() == (R / 'scripts/resident-outlined-mutation/src/counting.rs').read_bytes()
assert (H / 'src/probes.rs').read_bytes() == (T / 'src/probes.rs').read_bytes()
(S / 'harness-correspondence.json').write_text(json.dumps(dict(complete=True,
    original_sha256=sha(T / 'src/main.rs'), counted_sha256=sha(H / 'src/main.rs'),
    normalized_sha256=hashlib.sha256(normalized.encode()).hexdigest(),
    counter_sha256=sha(H / 'src/counting.rs'), probes_sha256=sha(H / 'src/probes.rs'),
    scope='Exact source projection excluding allocator/metric implementation, four begin hooks, output metadata and one untimed snapshot-size observation. All operation/window boundaries and state checks are otherwise identical.'), indent=2) + '\n')

os.environ['CARGO_TARGET_DIR'] = str(R / 'target')
os.environ['CARGO_BUILD_JOBS'] = '4'
assert not any(os.environ.get(k) for k in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CFLAGS'))
spec = importlib.util.spec_from_file_location('cache', R / 'scripts/build_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)
with (S / 'harness-metadata.json').open('x') as out, (S / 'harness-metadata.stderr').open('x') as err:
    subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1'], cwd=H, stdout=out, stderr=err, check=True, timeout=120)
registry = lambda p: {(v['name'], v['version'], v['source']): v['checksum'] for v in tomllib.loads(p.read_text())['package'] if v.get('source', '').startswith('registry+')}
reference = registry(R / 'Cargo.lock')
summary = dict(complete=False, started_ns=time.time_ns(), sources={}, arms={},
               plan_sha256=sha(S / 'plan.json'), timing_reused=True,
               qualified_build_sha256=sha(Q / 'build-summary.json'))
try:
    for arm in ('baseline', 'candidate'):
        work = S / ('source-counting-' + arm)
        work.mkdir(exist_ok=False)
        shutil.copytree(H / 'src', work / 'src')
        manifest = (H / 'Cargo.toml').read_text()
        for name in ('common', 'engine'):
            manifest = manifest.replace(f'../../crates/{name}', str(R / 'crates' / name))
        if arm == 'candidate':
            manifest += f'\n[patch.crates-io]\ntriomphe = {{ path = "{Q / "candidate-triomphe"}" }}\n'
        (work / 'Cargo.toml').write_text(manifest)
        shutil.copyfile(H / 'Cargo.lock', work / 'Cargo.lock')
        output = S / ('build-counting-' + arm)
        output.mkdir(exist_ok=False)
        row = dict(work=str(work), output=str(output), timing=plan['timing_binaries'][arm])
        summary['arms'][arm] = row
        with (output / 'metadata.json').open('x') as out, (output / 'metadata.stderr').open('x') as err:
            subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1'], cwd=work, stdout=out, stderr=err, check=True, timeout=120)
        assert all(reference.get(k) == v for k, v in registry(work / 'Cargo.lock').items())
        paths = [Path(__file__).resolve(), H / 'Cargo.toml', H / 'Cargo.lock', *sorted((H / 'src').rglob('*.rs')),
                 work / 'Cargo.toml', work / 'Cargo.lock', *sorted((work / 'src').rglob('*.rs')),
                 S / 'plan.json', S / 'declared-plan.json', S / 'reference-inputs.py', S / 'harness-correspondence.json']
        identity = {str(p): sha(p) for p in paths}
        identity.update(qualified['sources'])
        summary['sources'].update(identity)
        (output / 'sources.json').write_text(json.dumps(identity, indent=2) + '\n')
        for path in paths:
            relative = Path('repo') / path.relative_to(R) if path.is_relative_to(R) else Path('experiment') / path.relative_to(S)
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
                assert len(selected) == 1 and not selected[0]['fresh']
                if name in ('archery', 'rpds'):
                    assert 'registry+' in selected[0]['package_id']
                elif name == 'triomphe':
                    assert ((Q / 'candidate-triomphe').as_uri() if arm == 'candidate' else 'registry+') in selected[0]['package_id']
            binary = output / 'counting'
            shutil.copyfile(R / 'target/release/kv9-engine-interface-counting', binary)
            binary.chmod(0o755)
            row['counting'] = dict(path=str(binary), sha256=sha(binary), bytes=binary.stat().st_size)
        assert all(sha(Path(p)) == h for p, h in identity.items())
        print(json.dumps({'arm': arm, 'counting': row['counting']}), flush=True)
    left = registry(S / 'source-counting-baseline/Cargo.lock')
    right = registry(S / 'source-counting-candidate/Cargo.lock')
    removed = set(left) - set(right)
    assert len(removed) == 1 and next(iter(removed))[:2] == ('triomphe', '0.1.16')
    assert {k: v for k, v in left.items() if k not in removed} == right
    summary['complete'] = True
except BaseException as error:
    summary['failure'] = repr(error)
    raise
finally:
    summary['ended_ns'] = time.time_ns()
    (S / 'build-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
