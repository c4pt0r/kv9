#!/usr/bin/env python3
"""Check conditional guard equivalence, source bindings and rejecting controls."""
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
PROOF = ROOT / 'proofs/lean/static-pointer-callback'
NAMES = ['callback_adapter_equal', 'guard_equivalence', 'transaction_equivalence',
         'normal_equivalence', 'unwind_equivalence', 'current_slot_restored',
         'heap_unchanged_by_guard', 'views_unchanged_by_guard', 'exit_preserved',
         'one_reference_transferred', 'validity_restored', 'retained_views_preserved',
         'unwind_after_replacement', 'clone_failure_preserves_ownership',
         'replacement_valid', 'stale_writeback_is_invalid',
         'dropping_current_reference_is_invalid', 'missing_unwind_writeback_is_stale']
FOUNDATIONS = {'propext', 'Classical.choice', 'Quot.sound'}
spec = importlib.util.spec_from_file_location('helpers', ROOT / 'scripts/check-crc32-proof.py')
helpers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)
require, save, command = helpers.require, helpers.save, helpers.command
sha = lambda data: hashlib.sha256(data).hexdigest()


def bind(data, digest):
    require(sha(data) == digest, 'reviewed source hash mismatch')


def check(lean, source, output):
    output.mkdir()
    require(re.findall(r'^theorem (\w+)', source, re.M) == NAMES, 'theorem inventory mismatch')
    queries = '\n'.join('#print axioms Kv9.StaticPointerCallback.' + name for name in NAMES)
    (output / 'WriteBack.lean').write_text(source + '\n' + queries + '\n')
    code, log = command([lean, '-DwarningAsError=true', '-o', 'WriteBack.olean', 'WriteBack.lean'],
                        output, 'lean', env=dict(os.environ, LEAN_PATH=str(output)))
    require(code == 0, 'Lean rejected guard proof: ' + log[:3000])
    axioms = {}
    for name in NAMES:
        qualified = 'Kv9.StaticPointerCallback.' + name
        pattern = re.escape("'" + qualified + "'") + r' (?:depends on axioms: \[([^]]*)\]|does not depend on any axioms)'
        matches = list(re.finditer(pattern, log))
        require(len(matches) == 1, 'missing/duplicate axiom report: ' + qualified)
        dependencies = {s.strip() for s in (matches[0][1] or '').split(',') if s.strip()}
        require(dependencies <= FOUNDATIONS, 'untrusted axioms: ' + qualified)
        axioms[qualified] = sorted(dependencies)
    save(output / 'axioms.json', axioms)
    return axioms


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument('--lean', required=True)
    parser.add_argument('--baseline', type=Path, required=True)
    parser.add_argument('--candidate', type=Path, required=True)
    parser.add_argument('--registry', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    lean = shutil.which(args.lean)
    require(lean, 'Lean required')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    result = dict(complete=False, accepted=False, started_ns=time.time_ns(), controls=[],
                  scope='Conditional normal/unwind guard and ownership-token equivalence. '
                        'Exact source binding is a review boundary, not verified Rust extraction. '
                        'Rust provenance, smart-pointer contracts, ManuallyDrop, unwinding and '
                        'compiler correctness remain explicit premises; no Raft/durability claim.')
    try:
        contract = json.loads((PROOF / 'source-contract.json').read_text())
        proof = (PROOF / 'WriteBack.lean').read_text()
        bind(proof.encode(), contract['proof_sha256'])
        bindings = {}
        roots = {'baseline': args.baseline, 'candidate': args.candidate, 'registry': args.registry}
        for group, paths in contract['sources'].items():
            for relative, digest in paths.items():
                path = roots[group] / relative
                data = path.read_bytes()
                bind(data, digest)
                bindings[str(path)] = digest
                dest = output / 'inputs' / group / relative
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(data)
        for path in [Path(__file__).resolve(), ROOT / 'scripts/check-crc32-proof.py',
                     PROOF / 'WriteBack.lean', PROOF / 'source-contract.json',
                     ROOT / 'proofs/lean/lean-toolchain']:
            bindings[str(path)] = sha(path.read_bytes())
            dest = output / 'inputs/repo' / path.relative_to(ROOT)
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, dest)
        save(output / 'source-inputs.json', bindings)
        code, version = command([lean, '--version'], output, 'version', timeout=15)
        expected = (ROOT / 'proofs/lean/lean-toolchain').read_text().strip().split(':v')[1]
        require(code == 0 and version.startswith('Lean (version ' + expected + ','), 'Lean version mismatch')
        result['toolchain'] = dict(path=lean, sha256=sha(Path(lean).read_bytes()), version=version.strip())
        result['theorems'] = check(lean, proof, output / 'positive')

        def rejected(name, action, marker):
            try:
                action()
            except helpers.Rejected as error:
                require(marker in str(error), name + ' rejected outside intended gate: ' + str(error))
                result['controls'].append(dict(name=name, rejected=True, reason=str(error)))
            else:
                raise helpers.Rejected('invalid control accepted: ' + name)

        anchor = '⟨adapter done.state.current, done.state.counts, done.state.views, done.exit⟩'
        for name, old, new in [
            ('stale-slot', anchor, '⟨0, done.state.counts, done.state.views, done.exit⟩'),
            ('extra-owner-drop', anchor, '⟨adapter done.state.current, fun a => done.state.counts a - token done.state.current a, done.state.views, done.exit⟩'),
            ('missing-unwind-writeback', anchor, '⟨(match done.exit with | .unwind => 0 | .normal _ => adapter done.state.current), done.state.counts, done.state.views, done.exit⟩'),
            ('proof-hole', '(fun owned => asPtr owned) = asPtr := rfl', '(fun owned => asPtr owned) = asPtr := by sorry'),
        ]:
            require(proof.count(old) == 1, 'control anchor mismatch: ' + name)
            changed = proof.replace(old, new)
            rejected(name, lambda: check(lean, changed, output / ('control-' + name)), 'Lean rejected guard proof')
        changed = proof.replace('namespace Kv9.StaticPointerCallback', 'namespace Kv9.StaticPointerCallback\naxiom fake : False')
        changed = changed.replace('(fun owned => asPtr owned) = asPtr := rfl',
                                  '(fun owned => asPtr owned) = asPtr := False.elim fake')
        rejected('custom-axiom', lambda: check(lean, changed, output / 'control-custom-axiom'), 'untrusted axioms')
        guard_path = 'src/shared_pointer/kind/erased_ptr.rs'
        source = (args.candidate / guard_path).read_text()
        for name, old, new in [
            ('missing-writeback', '*self.slot = (self.as_ptr)(&self.owned);', '// missing write-back'),
            ('missing-manually-drop', 'owned: ManuallyDrop<P>,', 'owned: P,'),
            ('erased-callback-again', 'as_ptr: impl Fn(&P) -> *const T + Copy,', 'as_ptr: fn(&P) -> *const T,'),
        ]:
            require(source.count(old) == 1, 'source control anchor mismatch: ' + name)
            changed = source.replace(old, new).encode()
            (output / ('control-' + name + '.rs')).write_bytes(changed)
            rejected(name, lambda: bind(changed, contract['sources']['candidate'][guard_path]), 'reviewed source hash mismatch')
        require(all(sha(Path(path).read_bytes()) == digest for path, digest in bindings.items()), 'proof inputs changed')
        result.update(complete=True, accepted=True, source_bindings=len(bindings))
    except Exception as error:
        result['failure'] = repr(error)
        raise
    finally:
        result['ended_ns'] = time.time_ns()
        save(output / 'result.json', result)
    print(json.dumps(result))


if __name__ == '__main__':
    main()
