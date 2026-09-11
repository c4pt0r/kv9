#!/usr/bin/env python3
"""Retain workload, default database and explicit calibration binaries from one source tree."""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('workload_builder', ROOT/'scripts/build-workload.py')
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--debug', action='store_true', help='functional smoke only; full measurements require release')
    args = parser.parse_args()
    out = args.output.resolve()
    if out.is_relative_to(ROOT): raise ValueError('build artifacts must be outside the source tree')
    out.mkdir(parents=True, exist_ok=False)
    before = builder.snapshot()
    with builder.cache.BuildCache(ROOT, out, not args.debug, before) as session:
        builder.build_component(out/'workload', session, release=not args.debug)
        manifest = dict(version=1, **before, profile='debug' if args.debug else 'release', binaries={})
        for target, selector in [('kv9', ['--bin', 'kv9']),
                                 ('workload-loopback', ['-p', 'kv9-server', '--example', 'workload-loopback'])]:
            command = ['cargo', 'build', '--locked', *selector, *([] if args.debug else ['--release']),
                       '--message-format=json-render-diagnostics']
            with (out/(target+'-cargo.jsonl')).open('w') as stdout, (out/(target+'-build.log')).open('w') as stderr:
                session.run(command, stdout=stdout, stderr=stderr)
            if builder.snapshot() != before: raise ValueError('source changed while building benchmark binaries')
            session.check_artifacts(out/(target+'-cargo.jsonl'))
            records = [json.loads(line) for line in (out/(target+'-cargo.jsonl')).read_text().splitlines()]
            paths = {r['executable'] for r in records if r.get('reason')=='compiler-artifact' and
                     r.get('target', {}).get('name')==target and r.get('executable')}
            if len(paths)!=1: raise ValueError('missing or ambiguous benchmark executable')
            source = Path(paths.pop())
            if not 0 < source.stat().st_size <= builder.MAX_BINARY: raise ValueError('invalid executable size')
            shutil.copy2(source, out/target)
            manifest['binaries'][target] = dict(sha256=builder.sha((out/target).read_bytes()), command=command)
        workload = json.loads((out/'workload/build.json').read_text())
        manifest['workload_build_sha256'] = builder.sha((out/'workload/build.json').read_bytes())
        manifest['source_tree_sha256'] = workload['source_tree_sha256']
        manifest['rustc'] = workload['rustc']
        (out/'build.json').write_text(json.dumps(manifest, sort_keys=True, indent=2)+'\n')
        if builder.snapshot() != before: raise ValueError('source changed during benchmark retention')
        print('PASS: retained database, workload and explicit loopback calibration builds')


if __name__=='__main__': main()
