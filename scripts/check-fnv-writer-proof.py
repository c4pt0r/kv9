#!/usr/bin/env python3
"""Source-bound bounded WAL staging composition, with kernel and rejection gates."""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import shutil
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lean', type=Path, required=True)
    parser.add_argument('--rustc', type=Path, required=True)
    args = parser.parse_args()
    root, out = args.source_root.resolve(), args.output.resolve()
    spec = importlib.util.spec_from_file_location('kernel_gate', root / 'scripts/check-fnv-interleave-proof.py')
    k = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(k)
    k.require(not out.exists(), 'fresh output required')
    out.mkdir(parents=True)
    result = dict(complete=False, accepted=False, controls=[])
    try:
        contract_path = root / 'proofs/lean/fnv-writer/source-contract.json'
        c = json.loads(contract_path.read_text())
        result['scope'] = c['scope']
        k.require(set(c['foundations']) == k.FOUNDATIONS, 'foundation allowlist differs')
        paths = [root / p for p in c['files']] + [contract_path, Path(__file__).resolve(),
                root / 'scripts/check-fnv-interleave-proof.py', root / 'proofs/lean/lean-toolchain']
        before = {str(p.relative_to(root)): k.sha(p) for p in paths}
        k.save(out / 'source-before.json', before)
        for p in paths:
            target = out / 'inputs' / p.relative_to(root)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(p, target)
        k.require(all(before[p] == digest for p, digest in c['files'].items()), 'source/model hash mismatch')
        source = (root / c['source']).read_text()
        k.bind(source, c['declarations'])
        code, parent = k.command(['git', '-C', str(root), 'show', c['parent_revision'] + ':' + c['source']], out, 'parent-storage')
        k.require(code == 0, 'committed parent source unavailable')
        for needle in c['unchanged_from_parent']:
            k.require(k.declaration(source, needle) == k.declaration(parent, needle), 'persistence boundary changed: ' + needle)
        result['unchanged_boundaries'] = c['unchanged_from_parent']
        code, _ = k.command([sys.executable, str(root / 'scripts/check-fnv-interleave-proof.py'),
                            '--source-root', str(root), '--output', str(out / 'kernel'),
                            '--lean', str(args.lean), '--rustc', str(args.rustc)], out, 'kernel-gate', 240)
        k.require(code == 0 and json.loads((out / 'kernel/result.json').read_text())['accepted'], 'kernel gate rejected')
        base = (root / 'proofs/lean/fnv-interleave/Interleave.lean').read_text()
        writer = (root / 'proofs/lean/fnv-writer/Writer.lean').read_text()
        k.require(re.findall(r'^theorem ([A-Za-z0-9_]+)', writer, re.M) == c['theorems'], 'theorem inventory differs')
        k.require(not re.search(r'\b(sorry|axiom|native_decide|bv_decide|implemented_by)\b', writer), 'unsupported proof construct')
        k.NAMESPACE = 'Kv9.FnvWriter'

        def lean_check(label, model, names):
            directory = out / label
            directory.mkdir()
            queries = '\n'.join('#print axioms ' + k.NAMESPACE + '.' + name for name in names)
            (directory / 'Combined.lean').write_text(base + '\n' + model + '\n' + queries + '\n')
            return k.command([str(args.lean), '-M1024', '-j1', '-DwarningAsError=true', 'Combined.lean'], directory, 'lean')

        code, log = lean_check('positive', writer, c['theorems'])
        k.require(code == 0, 'writer proof rejected: ' + log[:2000])
        axioms = k.parse_axioms(log, c['theorems'])
        k.save(out / 'axioms.json', axioms)
        result['distinct_writer_theorems'] = len(axioms)
        for name, old, new in [
            ('reordered-pending', 's.pending ++ [b]', 'b :: s.pending'),
            ('budget-overrun', 'bodyBytes s.pending + b.length ≤ budget', 'bodyBytes s.pending + b.length ≤ budget + 1'),
            ('missing-full-flush', 'if s.pending.length = 4 then flush s else s',
             'if s.pending.length = 5 then flush s else s'),
        ]:
            k.require(writer.count(old) == 1, 'model mutation target differs: ' + name)
            code, log = lean_check('control-' + name, writer.replace(old, new, 1), c['theorems'])
            k.require(code != 0 and any(s in log for s in ['unsolved goals', 'Type mismatch', 'omega could not prove'])
                      and 'excessive memory' not in log, 'model mutation did not fail as intended: ' + name)
            result['controls'].append(dict(name=name, rejected=True))
        for name, old, new in [
            ('wrong-lane-result', 'sums[i]', 'sums[(i + 1) % 4]'),
            ('wrong-budget', 'const ENTRY_BODY_BUDGET: usize = 64 * 1024;', 'const ENTRY_BODY_BUDGET: usize = 128 * 1024;'),
            ('missing-sync', 'let result = metrics.sync.measure(|| file.sync_data());',
             'let result = metrics.sync.measure(|| Ok(()));'),
        ]:
            k.require(source.count(old) == 1, 'source mutation target differs: ' + name)
            mutated = source.replace(old, new, 1)
            directory = out / ('control-' + name)
            directory.mkdir()
            (directory / 'storage.rs').write_text(mutated)
            try:
                k.bind(mutated, c['declarations'])
            except k.Rejected as error:
                k.require('source contract mismatch:' in str(error), 'unrelated source rejection')
                k.save(directory / 'result.json', dict(rejected=True, reason=str(error)))
            else:
                raise k.Rejected('mutated source accepted: ' + name)
            result['controls'].append(dict(name=name, rejected=True))
        injected = writer.replace('end Kv9.FnvWriter', 'axiom injectedFalse : False\ntheorem injected_false : False := injectedFalse\nend Kv9.FnvWriter')
        code, log = lean_check('control-custom-axiom', injected, c['theorems'] + ['injected_false'])
        k.require(code == 0, 'custom axiom did not reach semantic inspection')
        try:
            k.parse_axioms(log, c['theorems'] + ['injected_false'])
        except k.Rejected as error:
            k.require('untrusted axioms:' in str(error), 'unrelated axiom rejection')
        else:
            raise k.Rejected('custom false axiom accepted')
        result['controls'].append(dict(name='custom-axiom', rejected=True))
        after = {str(p.relative_to(root)): k.sha(p) for p in paths}
        k.save(out / 'source-after.json', after)
        k.require(before == after, 'source changed during proof')
        result.update(complete=True, accepted=True, source_unchanged=True)
    except BaseException as error:
        result['failure'] = repr(error)
        raise
    finally:
        k.save(out / 'result.json', result)
        k.save(out / 'inventory.json', {str(p.relative_to(out)): dict(bytes=p.stat().st_size, sha256=k.sha(p))
                                      for p in sorted(out.rglob('*')) if p.is_file()})
    print(json.dumps(result))


if __name__ == '__main__':
    main()
