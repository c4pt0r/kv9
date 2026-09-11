#!/usr/bin/env python3
"""Real, offline two-workspace cache regression; never uses the project's target."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('cache_under_test', ROOT / 'scripts/build_cache.py')
CACHE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CACHE)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    if out.is_relative_to(ROOT):
        raise ValueError('regression output must be new and outside the source tree')
    out.mkdir(parents=True, exist_ok=False)
    target = out / 'tiny-target'
    # The caller's production/shared target is deliberately not used by this fixture.
    os.environ.update(CARGO_TARGET_DIR=str(target), CARGO_BUILD_JOBS='1',
                      CARGO_NET_OFFLINE='true', CARGO_INCREMENTAL='0')
    result = {'complete': False, 'helper_sha256': sha(ROOT/'scripts/build_cache.py'),
              'fixture_sha256': sha(Path(__file__)), 'target': str(target), 'commands': []}

    def command(argv, cwd, name):
        row = {'argv': argv, 'cwd': str(cwd), 'started_ns': time.time_ns()}
        result['commands'].append(row)
        with (out/(name+'.stdout')).open('xb') as stdout, (out/(name+'.stderr')).open('xb') as stderr:
            completed = subprocess.run(argv, cwd=cwd, stdout=stdout, stderr=stderr, timeout=120)
        row.update(exit_code=completed.returncode, ended_ns=time.time_ns())
        if completed.returncode:
            raise RuntimeError('fixture command failed: ' + name)
        return (out/(name+'.stdout')).read_text().strip()

    try:
        external = out / 'external-dependency'
        (external/'src').mkdir(parents=True)
        (external/'Cargo.toml').write_text('[package]\nname="cached-external"\nversion="0.1.0"\nedition="2021"\n[workspace]\n')
        (external/'src/lib.rs').write_text('pub fn zero() -> u32 { 0 }\n')
        for label, value in [('a', 1), ('b', 2)]:
            tree = out / label
            (tree/'engine/src').mkdir(parents=True)
            (tree/'src').mkdir()
            (tree/'Cargo.toml').write_text('[package]\nname="cache-app"\nversion="0.1.0"\nedition="2021"\n'
                '[workspace]\nmembers=["engine"]\nresolver="2"\n[dependencies]\ncache-engine={path="engine"}\n')
            (tree/'src/main.rs').write_text('fn main() { println!("{}", cache_engine::value()); }\n')
            (tree/'engine/Cargo.toml').write_text('[package]\nname="cache-engine"\nversion="0.1.0"\nedition="2021"\n'
                '[dependencies]\ncached-external={path="../../external-dependency"}\n')
            (tree/'engine/src/lib.rs').write_text('pub fn value() -> u32 { cached_external::zero() + '+str(value)+' }\n')
            command(['cargo', 'generate-lockfile', '--offline'], tree, label+'-lockfile')
        # Both different sources precede A's actual compilation, as in the incident.
        old_ns = time.time_ns() - 3600 * 1_000_000_000
        for label in ['a', 'b']:
            for path in (out/label).rglob('*'):
                if path.is_file():
                    os.utime(path, ns=(old_ns, old_ns))

        def build(label, safe):
            tree = out / label
            directory = out / (label + ('-safe' if safe else '-unsafe'))
            directory.mkdir()
            argv = ['cargo', 'build', '--locked', '--release', '--message-format=json-render-diagnostics']
            identity = {str(p.relative_to(tree)): sha(p) for p in tree.rglob('*') if p.is_file()}
            if safe:
                with CACHE.BuildCache(tree, directory, True, identity) as session:
                    with (directory/'cargo.jsonl').open('x') as stdout, (directory/'build.log').open('x') as stderr:
                        session.run(argv, stdout=stdout, stderr=stderr, timeout=120)
                    session.check_artifacts(directory/'cargo.jsonl')
                    shutil.copy2(target/'release/cache-app', directory/'cache-app')
                    if label == 'a':
                        probe = out/'lock-probe'
                        probe.mkdir()
                        script = (
                            'import importlib.util,sys;from pathlib import Path\n'
                            's=importlib.util.spec_from_file_location("c",sys.argv[1]);m=importlib.util.module_from_spec(s);s.loader.exec_module(m)\n'
                            'try:\n'
                            ' with m.BuildCache(Path(sys.argv[2]),Path(sys.argv[3]),True,{}): pass\n'
                            'except RuntimeError as e:\n'
                            ' if str(e)!="another retained build owns this Cargo target": raise\n'
                            ' print(str(e))\n'
                            'else: raise RuntimeError("concurrent owner incorrectly accepted")\n')
                        text = command([sys.executable, '-c', script, str(ROOT/'scripts/build_cache.py'),
                                        str(tree), str(probe)], tree, 'lock-contention')
                        if text != 'another retained build owns this Cargo target' or (probe/'cache-clean.stdout').exists():
                            raise RuntimeError('lock contention reached invalidation')
            else:
                with (directory/'cargo.jsonl').open('x') as stdout, (directory/'build.log').open('x') as stderr:
                    completed = subprocess.run(argv, cwd=tree, stdout=stdout, stderr=stderr, timeout=120)
                if completed.returncode:
                    raise RuntimeError('unsafe negative-control build failed')
                shutil.copy2(target/'release/cache-app', directory/'cache-app')
            actual = command([str(directory/'cache-app')], tree, directory.name+'-behavior')
            result[directory.name] = {'behavior': actual, 'binary_sha256': sha(directory/'cache-app')}
            return actual

        if build('a', True) != '1':
            raise RuntimeError('baseline compiled wrong implementation')
        external_before = {str(p): [sha(p), p.stat().st_mtime_ns] for p in
                           (target/'release/deps').glob('libcached_external-*') if p.suffix in ('.rlib', '.rmeta')}
        if not external_before:
            raise RuntimeError('external dependency cache was not populated')
        if build('b', False) != '1':
            raise RuntimeError('unsafe older-mtime negative control did not reproduce; preserve for review')
        if build('b', True) != '2':
            raise RuntimeError('safe helper reused the stale implementation')
        external_after = {str(p): [sha(p), p.stat().st_mtime_ns] for p in
                          (target/'release/deps').glob('libcached_external-*') if p.suffix in ('.rlib', '.rmeta')}
        if external_after != external_before:
            raise RuntimeError('first-party invalidation changed the external dependency cache')
        unsafe_output = target / 'retained-output-probe'
        unsafe_output.mkdir()
        before_rejection = sha(out / 'b-safe/cache-app')
        try:
            with CACHE.BuildCache(out / 'b', unsafe_output, True, {}):
                raise RuntimeError('output inside Cargo target was accepted')
        except RuntimeError as error:
            if str(error) != 'retained output must be outside the Cargo target':
                raise
        if (unsafe_output / 'cache-clean.stdout').exists():
            raise RuntimeError('unsafe output rejection reached invalidation')
        if sha(target / 'release/cache-app') != before_rejection:
            raise RuntimeError('unsafe output rejection changed the compiled executable')
        if sha(ROOT/'scripts/build_cache.py') != result['helper_sha256']:
            raise RuntimeError('helper changed during qualification')
        result.update(complete=True, negative_control_stale=True, safe_behavior_correct=True,
                      lock_contention_rejected_before_clean=True,
                      output_inside_target_rejected_before_clean=True,
                      external_cache_unchanged=external_after)
    except BaseException as error:
        result['failure'] = repr(error)
        raise
    finally:
        result['ended_ns'] = time.time_ns()
        save(out/'result.json', result)
    print('PASS: older-mtime stale negative control, first-party rebuild, shared lock, safe output path and dependency-cache preservation')


if __name__ == '__main__':
    main()
