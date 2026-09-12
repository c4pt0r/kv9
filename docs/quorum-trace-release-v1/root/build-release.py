#!/usr/bin/env python3
"""Retain standalone native-batch workload and opt-in quorum-trace WAL server from one source tree."""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path('/tmp/kv9-quorum-message-trace')
spec = importlib.util.spec_from_file_location('batch_builder', ROOT / 'scripts/build-workload.py')
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--release', action='store_true')
    args = parser.parse_args(); out = args.output.resolve()
    if out.is_relative_to(ROOT): raise ValueError('build artifacts must be outside source')
    out.mkdir(parents=True, exist_ok=False)
    before = builder.snapshot()
    if before["dirty"]: raise ValueError("diagnostic release requires clean committed source")
    with builder.cache.BuildCache(ROOT, out, args.release, before) as session:
        builder.build_component(out/'workload', session, binary='kv9-batch-workload', release=args.release)
        command = ['cargo', 'build', '--locked', '--bin', 'kv9', *(['--release'] if args.release else []), '--features', 'quorum-trace', '--message-format=json-render-diagnostics']
        with (out/'kv9-cargo.jsonl').open('w') as stdout, (out/'kv9-build.log').open('w') as stderr:
            session.run(command, stdout=stdout, stderr=stderr)
        if builder.snapshot() != before: raise ValueError('native build source changed')
        session.check_artifacts(out/'kv9-cargo.jsonl')
        records = [json.loads(line) for line in (out/'kv9-cargo.jsonl').read_text().splitlines()]
        expected_features = {'kv9': ['quorum-trace'], 'kv9_server': ['quorum-trace'], 'kv9_raft': ['quorum-trace', 'read-stage-timing'], 'kv9_engine': []}
        for name, features in expected_features.items():
            selected = [r for r in records if r.get('reason') == 'compiler-artifact' and r['target']['name'] == name]
            if len(selected) != 1 or selected[0]['features'] != features or selected[0]['profile']['test'] or selected[0]['profile']['opt_level'] != '3':
                raise ValueError('diagnostic production artifact features/profile differ: ' + name)

        paths = {r['executable'] for r in records if r.get('reason') == 'compiler-artifact' and r.get('target', {}).get('name') == 'kv9' and r.get('executable')}
        if len(paths) != 1: raise ValueError('missing or ambiguous server artifact')
        source = Path(paths.pop())
        if not 0 < source.stat().st_size <= builder.MAX_BINARY: raise ValueError('invalid server binary bound')
        shutil.copy2(source, out/'kv9')
        workload = json.loads((out/'workload/build.json').read_text())
        manifest = dict(version=1, **before, profile='release' if args.release else 'debug',
            binaries={'kv9':dict(sha256=builder.sha((out/'kv9').read_bytes()), command=command)},
            workload_build_sha256=builder.sha((out/'workload/build.json').read_bytes()),
            source_tree_sha256=workload['source_tree_sha256'], rustc=workload['rustc'])
        (out/'build.json').write_text(json.dumps(manifest, sort_keys=True, indent=2)+'\n')
        if builder.snapshot() != before: raise ValueError('source changed during native build retention')
        print('PASS: retained native batch workload and diagnostic server builds')


if __name__ == '__main__': main()
