#!/usr/bin/env python3
"""Check exact shared-path extraction, guard composition and source bindings."""
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
PROOF = ROOT / 'proofs/lean/outlined-mutation-path'
spec = importlib.util.spec_from_file_location('guard', ROOT / 'scripts/check-static-pointer-callback-proof.py')
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)
require, save, command = guard.require, guard.save, guard.command
sha = lambda data: hashlib.sha256(data).hexdigest()
NAMES = ['extraction_equivalence', 'observe_once_equivalence', 'unique_path_does_not_clone', 'shared_success_exposes_new_owner', 'clone_unwind_preserved', 'unwind_before_replacement_preserves_slot', 'heap_effects_preserved', 'guarded_extraction_equivalence']


def check(lean, source, output, dependency):
    output.mkdir()
    require(re.findall(r'^theorem (\w+)', source, re.M) == NAMES, 'theorem inventory mismatch')
    queries = '\n'.join('#print axioms Kv9.OutlinedMutation.' + name for name in NAMES)
    (output / 'Outline.lean').write_text(source + '\n' + queries + '\n')
    code, log = command([lean, '-DwarningAsError=true', '-o', 'Outline.olean', 'Outline.lean'],
                        output, 'lean', env=dict(os.environ, LEAN_PATH=str(dependency)))
    require(code == 0, 'Lean rejected outline proof: ' + log[:3000])
    axioms = {}
    for name in NAMES:
        qualified = 'Kv9.OutlinedMutation.' + name
        pattern = re.escape("'" + qualified + "'") + r' (?:depends on axioms: \[([^]]*)\]|does not depend on any axioms)'
        matches = list(re.finditer(pattern, log))
        require(len(matches) == 1, 'missing/duplicate axiom report: ' + qualified)
        dependencies = {x.strip() for x in (matches[0][1] or '').split(',') if x.strip()}
        require(dependencies <= guard.FOUNDATIONS, 'untrusted axioms: ' + qualified)
        axioms[qualified] = sorted(dependencies)
    save(output / 'axioms.json', axioms)
    return axioms


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument('--lean', required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    lean = shutil.which(args.lean)
    require(lean, 'Lean required')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    result = {'complete': False, 'accepted': False, 'started_ns': time.time_ns(), 'controls': [],
              'scope': 'Conditional abstract normal/unwind callback and guard equivalence. '
                       'Source review is not verified Rust extraction; Rust provenance, smart-pointer '
                       'contracts, pure from_mut conversion, ordinary unwinding and compiler correctness remain premises.'}
    try:
        contract = json.loads((PROOF / 'source-contract.json').read_text())
        model = (ROOT / 'proofs/lean/static-pointer-callback/WriteBack.lean').read_text()
        proof = (PROOF / 'Outline.lean').read_text()
        guard.bind(model.encode(), contract['guard_model_sha256'])
        guard.bind(proof.encode(), contract['proof_sha256'])
        bindings = {}
        for relative, digest in contract['sources'].items():
            path = args.work / relative
            data = path.read_bytes()
            guard.bind(data, digest)
            bindings[str(path)] = digest
            dest = output / 'inputs/work' / relative
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(data)
        for path in [Path(__file__).resolve(), ROOT / 'scripts/check-static-pointer-callback-proof.py',
                     ROOT / 'scripts/check-crc32-proof.py', ROOT / 'proofs/lean/lean-toolchain',
                     ROOT / 'proofs/lean/static-pointer-callback/WriteBack.lean',
                     PROOF / 'Outline.lean', PROOF / 'source-contract.json']:
            bindings[str(path)] = sha(path.read_bytes())
            dest = output / 'inputs/repo' / path.relative_to(ROOT)
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, dest)
        save(output / 'source-inputs.json', bindings)
        code, version = command([lean, '--version'], output, 'version', timeout=15)
        expected = (ROOT / 'proofs/lean/lean-toolchain').read_text().strip().split(':v')[1]
        require(code == 0 and version.startswith('Lean (version ' + expected + ','), 'Lean version mismatch')
        result['toolchain'] = {'version': version.strip(), 'path': lean, 'sha256': sha(Path(lean).read_bytes())}
        result['guard_theorems'] = guard.check(lean, model, output / 'dependency')
        result['adapter_theorems'] = check(lean, proof, output / 'positive', output / 'dependency')

        def rejected(name, action, marker):
            try:
                action()
            except guard.helpers.Rejected as error:
                require(marker in str(error), name + ' rejected outside intended gate: ' + str(error))
                result['controls'].append({'name': name, 'rejected': True, 'reason': str(error)})
            else:
                raise guard.helpers.Rejected('invalid control accepted: ' + name)

        for name, old, new in [
            ('clone-unique-owner', 'if unique then ⟨owner, .normal ()⟩ else sharedHelper cloneReplace owner', 'if unique then sharedHelper cloneReplace owner else sharedHelper cloneReplace owner'),
            ('restore-initial-owner', 'else sharedHelper cloneReplace owner', 'else { sharedHelper cloneReplace owner with state := owner }'),
            ('duplicate-observation', '  outlined unique cloneReplace expose state', '  outlined (observe state).1 cloneReplace expose (observe state).2'),
            ('lose-unwind', '| .unwind => .unwind', '| .unwind => .normal (expose done.state)'),
            ('proof-hole', 'cases unique <;> rfl', 'cases unique <;> sorry'),
        ]:
            require(proof.count(old) == 1, 'control anchor mismatch: ' + name)
            changed = proof.replace(old, new)
            rejected(name, lambda: check(lean, changed, output / ('control-' + name), output / 'dependency'), 'Lean rejected outline proof')
        changed = proof.replace('namespace Kv9.OutlinedMutation', 'namespace Kv9.OutlinedMutation\naxiom fake : False')
        changed = changed.replace('cases unique <;> rfl', 'exact False.elim fake')
        rejected('custom-axiom', lambda: check(lean, changed, output / 'control-custom-axiom', output / 'dependency'), 'untrusted axioms')
        relative = 'candidate-triomphe/src/arc.rs'
        source = (args.work / relative).read_text()
        baseline = (args.work / 'baseline-triomphe/src/arc.rs').read_text()
        for needle in ('pub fn is_unique(', 'pub fn count('):
            require(guard.helpers.declaration(source, needle) == guard.helpers.declaration(baseline, needle), 'changed uniqueness observation')
        old_body = guard.helpers.declaration(baseline, 'pub fn make_mut(')
        new_body = guard.helpers.declaration(source, 'pub fn make_mut(')
        helper = guard.helpers.declaration(source, 'fn clone_shared_for_mutation(')
        require(old_body.count('this.is_unique()') == new_body.count('this.is_unique()') == 1, 'uniqueness check count changed')
        require(old_body[old_body.index('        unsafe {'):] == new_body[new_body.index('        unsafe {'):], 'mutable reference exposure changed')
        require(helper == 'fn clone_shared_for_mutation(this: &mut Self) {\n        *this = Arc::new(T::clone(this));\n    }', 'clone-before-replacement operation changed')
        for name, old, new in [
            ('relaxed-count', 'this.inner().count.load(Acquire)', 'this.inner().count.load(Relaxed)'),
            ('skip-shared-replacement', 'Self::clone_shared_for_mutation(this);', '// skipped replacement'),
            ('duplicate-count', 'if !this.is_unique() {\n            // Same clone-before-replacement', 'if !this.is_unique() && !this.is_unique() {\n            // Same clone-before-replacement'),
        ]:
            require(source.count(old) == 1, 'source control anchor mismatch: ' + name)
            changed = source.replace(old, new).encode()
            (output / ('control-' + name + '.rs')).write_bytes(changed)
            rejected(name, lambda: guard.bind(changed, contract['sources'][relative]), 'reviewed source hash mismatch')
        require(all(sha(Path(path).read_bytes()) == digest for path, digest in bindings.items()), 'proof inputs changed')
        result.update(complete=True, accepted=True, source_bindings=len(bindings))
    except Exception as error:
        result['failure'] = repr(error)
        raise
    finally:
        result['ended_ns'] = time.time_ns()
        save(output / 'result.json', result)
    print(json.dumps({'accepted': True, 'guard_theorems': len(result['guard_theorems']),
                      'adapter_theorems': len(result['adapter_theorems']), 'controls': len(result['controls'])}))


if __name__ == '__main__':
    main()
