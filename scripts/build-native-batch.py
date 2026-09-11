#!/usr/bin/env python3
"""Retain standalone native-batch workload and default WAL server from one source tree."""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
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
    with builder.cache.BuildCache(ROOT, out, args.release, before) as session:
        builder.build_component(out/'workload', session, binary='kv9-batch-workload', release=args.release)
        command = ['cargo', 'build', '--locked', '--bin', 'kv9', *(['--release'] if args.release else []), '--message-format=json-render-diagnostics']
        with (out/'kv9-cargo.jsonl').open('w') as stdout, (out/'kv9-build.log').open('w') as stderr:
            session.run(command, stdout=stdout, stderr=stderr)
        if builder.snapshot() != before: raise ValueError('native build source changed')
        session.check_artifacts(out/'kv9-cargo.jsonl')
        records = [json.loads(line) for line in (out/'kv9-cargo.jsonl').read_text().splitlines()]
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
        print('PASS: retained native batch workload and default server builds')


if __name__ == '__main__': main()
