#!/usr/bin/env python3
"""Strict prefix-order proof with exact reviewed Rust source binding and controls."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import time

ROOT = Path(__file__).resolve().parents[1]
PROOF = ROOT / 'proofs/lean/packed-prefix'
NAMES = ['shared_prefix_order', 'common_left', 'common_right', 'prefix_trans',
         'common_shortens_existing', 'common_admits_new', 'prefix_interval',
         'endpoint_prefix_covers_interval', 'prefix_bound', 'prefix_take',
         'prefix_drop', 'drop_shared_prefix', 'guarded_order', 'subset_preserves_prefix',
         'insertion_preserves_shortened_prefix', 'endpoints_cover_node',
         'checked_slices_in_bounds', 'empty_prefix_fallback']
FOUNDATIONS = {'propext', 'Classical.choice', 'Quot.sound'}
spec = importlib.util.spec_from_file_location('helpers', ROOT / 'scripts/check-crc32-proof.py')
helpers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)
require, save, command = helpers.require, helpers.save, helpers.command

def digest(data):
    return hashlib.sha256(data).hexdigest()

def bind(source, contract):
    for needle, declaration in contract['declarations'].items():
        require(helpers.declaration(source, needle) == declaration,
                'source declaration mismatch: ' + needle)
    require(digest(source.encode()) == contract['sha256'], 'source hash mismatch')

def check(lean, source, output):
    output.mkdir()
    require(re.findall(r'^theorem (\w+)', source, re.M) == NAMES, 'theorem inventory mismatch')
    queries = '\n'.join('#print axioms Kv9.PackedPrefix.' + name for name in NAMES)
    (output / 'Prefix.lean').write_text(source + '\n' + queries + '\n')
    code, log = command([lean, '-DwarningAsError=true', '-o', 'Prefix.olean', 'Prefix.lean'],
                        output, 'lean', env=dict(os.environ, LEAN_PATH=str(output)))
    require(code == 0, 'Lean rejected prefix proof: ' + log[:3000])
    axioms = {}
    for name in NAMES:
        qualified = 'Kv9.PackedPrefix.' + name
        pattern = re.escape("'" + qualified + "'") + r' (?:depends on axioms: \[([^]]*)\]|does not depend on any axioms)'
        matches = list(re.finditer(pattern, log))
        require(len(matches) == 1, 'missing/duplicate axiom report: ' + qualified)
        dependencies = {s.strip() for s in (matches[0][1] or '').split(',') if s.strip()}
        require(dependencies <= FOUNDATIONS, 'untrusted axioms: ' + qualified)
        axioms[qualified] = sorted(dependencies)
    save(output / 'axioms.json', axioms)
    return axioms

def main():
    ap = argparse.ArgumentParser(allow_abbrev=False)
    ap.add_argument('--lean', required=True)
    ap.add_argument('--output', type=Path, required=True)
    args = ap.parse_args()
    lean = shutil.which(args.lean)
    require(lean, 'Lean required')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    result = dict(complete=False, accepted=False, started_ns=time.time_ns(), controls=[],
                  scope='Universal prefix/order/cache-set lemmas with reviewed exact Rust mapping. '
                        'Conditional on ordered node intervals and valid safe Rust slices/Arc ownership. '
                        'Not a full proof of B+tree mutators, Rust/compiler refinement, Raft or durability.')
    contract = json.loads((PROOF / 'source-contract.json').read_text())
    files = [Path(__file__).resolve(), ROOT / 'scripts/check-crc32-proof.py',
             PROOF / 'Prefix.lean', PROOF / 'source-contract.json',
             ROOT / 'proofs/lean/lean-toolchain', ROOT / contract['source']]
    try:
        before = {str(p.relative_to(ROOT)): digest(p.read_bytes()) for p in files}
        for path in files:
            dst = output / 'inputs' / path.relative_to(ROOT)
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, dst)
        save(output / 'source-inputs.json', before)
        code, version = command([lean, '--version'], output, 'version', timeout=15)
        expected = (ROOT / 'proofs/lean/lean-toolchain').read_text().strip().split(':v')[1]
        require(code == 0 and version.startswith('Lean (version ' + expected + ','), 'Lean version mismatch')
        result['toolchain'] = dict(path=lean, sha256=digest(Path(lean).read_bytes()), version=version.strip())
        source = (ROOT / contract['source']).read_text()
        proof = (PROOF / 'Prefix.lean').read_text()
        bind(source, contract)
        require(digest(proof.encode()) == contract['proof_sha256'], 'reviewed proof changed')
        result['theorems'] = check(lean, proof, output / 'positive')
        def rejected(name, action, marker):
            try:
                action()
            except helpers.Rejected as error:
                require(marker in str(error), name + ' rejected outside intended gate: ' + str(error))
                result['controls'].append(dict(name=name, rejected=True, reason=str(error)))
            else:
                raise helpers.Rejected('invalid control accepted: ' + name)
        for name, old, new in [
            ('reversed-suffix-order', 'cmp (p ++ a) (p ++ b) = cmp a b', 'cmp (p ++ a) (p ++ b) = cmp b a'),
            ('incorrect-slice-bound', 'p.length ≤ stored.length ∧ p.length ≤ query.length', 'p.length + 1 ≤ stored.length ∧ p.length ≤ query.length'),
            ('proof-hole', 'cmp (stored.drop 0) (query.drop 0) = cmp stored query := by simp', 'cmp (stored.drop 0) (query.drop 0) = cmp stored query := by sorry'),
        ]:
            require(proof.count(old) == 1, 'control anchor mismatch: ' + name)
            changed = proof.replace(old, new)
            rejected(name, lambda: check(lean, changed, output / ('control-' + name)), 'Lean rejected prefix proof')
        changed = proof.replace('namespace Kv9.PackedPrefix', 'namespace Kv9.PackedPrefix\naxiom fake : False')
        changed = changed.replace('cmp (stored.drop 0) (query.drop 0) = cmp stored query := by simp',
                                  'cmp (stored.drop 0) (query.drop 0) = cmp stored query := False.elim fake')
        rejected('custom-axiom', lambda: check(lean, changed, output / 'control-custom-axiom'), 'untrusted axioms')
        for name, old, new in [
            ('unchecked-query', 'if query.starts_with(&reference[..prefix])', 'if true'),
            ('skip-extra-byte', 'a[skip..].cmp(&b[skip..])', 'a[skip + 1..].cmp(&b[skip + 1..])'),
            ('separator-instead-of-subtree-end', 'children.last().unwrap().node.last_key()', '&children.last().unwrap().minimum'),
            ('missing-insertion-cache-update', 'current.admit_prefix(&entry.key);', '// omitted cache admission'),
        ]:
            require(source.count(old) == 1, 'source control anchor mismatch: ' + name)
            changed = source.replace(old, new)
            (output / ('control-' + name + '.rs')).write_text(changed)
            rejected(name, lambda: bind(changed, contract), 'source declaration mismatch')
        require(all(digest((ROOT / name).read_bytes()) == h for name, h in before.items()), 'inputs changed during proof execution')
        result.update(accepted=True, complete=True)
    except Exception as error:
        result['failure'] = repr(error)
        raise
    finally:
        result['ended_ns'] = time.time_ns()
        save(output / 'result.json', result)
    print(json.dumps(result))

if __name__ == '__main__':
    main()
