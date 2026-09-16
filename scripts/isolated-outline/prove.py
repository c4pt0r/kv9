#!/usr/bin/env python3
"""Bind extraction to original archery and check unchanged dynamic-guard composition."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

S = Path(sys.argv[1]).resolve()
lean = sys.argv[2]
R = Path(__file__).resolve().parents[2]
P = R / 'proofs/lean/isolated-outline/Isolated.lean'
Q = Path(json.loads((S / 'qualification-plan.json').read_text())['qualified_prior_root'])
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
spec = importlib.util.spec_from_file_location('outline', R / 'scripts/check-outlined-mutation-proof.py')
outline = importlib.util.module_from_spec(spec)
spec.loader.exec_module(outline)
require = outline.require
contract = json.loads((R / 'proofs/lean/outlined-mutation-path/source-contract.json').read_text())
source = S / 'candidate-triomphe/src/arc.rs'
require(sha(source) == contract['sources']['candidate-triomphe/src/arc.rs'], 'extraction source changed')
require(sha(S / 'registry/triomphe-0.1.16/src/arc.rs') == contract['sources']['baseline-triomphe/src/arc.rs'], 'baseline source changed')
build = json.loads((S / 'build-summary.json').read_text())
require(build['complete'], 'release pair required')
for path, digest in build['sources'].items():
    require(sha(Path(path)) == digest, 'compiled source changed: ' + path)
output = S / 'proof-isolated'
output.mkdir(exist_ok=False)
bindings = {str(p): sha(p) for p in [P, Path(__file__).resolve(), source,
            S / 'registry/triomphe-0.1.16/src/arc.rs',
            S / 'registry/archery-1.2.3/src/shared_pointer/kind/arct/mod.rs',
            S / 'registry/archery-1.2.3/src/shared_pointer/kind/erased_ptr.rs']}
for path in bindings:
    dest = output / 'inputs' / Path(path).relative_to(R if Path(path).is_relative_to(R) else S)
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(path, dest)
(output / 'source-inputs.json').write_text(json.dumps(bindings, indent=2) + '\n')
names = ['dynamic_guard_extraction', 'dynamic_observed_extraction', 'dynamic_unwind_restores_current_owner']
proof = P.read_text()
require(re.findall(r'^theorem (\w+)', proof, re.M) == names, 'theorem inventory changed')
result = dict(complete=False, accepted=False, started_ns=time.time_ns(), controls=[],
              scope='Conditional original dynamic-guard composition of the qualified extraction. Source mapping is reviewed, not verified Rust extraction. Original Arc, pointer provenance, ordinary unwinding and compiler correctness remain premises.')

def check(text, name):
    work = output / name
    work.mkdir()
    queries = '\n'.join('#print axioms Kv9.IsolatedOutline.' + n for n in names)
    (work / 'Isolated.lean').write_text(text + '\n' + queries + '\n')
    code, log = outline.command([lean, '-DwarningAsError=true', '-o', 'Isolated.olean', 'Isolated.lean'], work, 'lean',
        env=dict(os.environ, LEAN_PATH=str(S / 'proof-dependency/positive') + ':' + str(S / 'proof-dependency/dependency')))
    require(code == 0, 'Lean rejected isolated proof: ' + log[:2000])
    axioms = {}
    for n in names:
        qualified = 'Kv9.IsolatedOutline.' + n
        matches = list(re.finditer(re.escape("'" + qualified + "'") + r' (?:depends on axioms: \[([^]]*)\]|does not depend on any axioms)', log))
        require(len(matches) == 1, 'axiom report missing')
        deps = {x.strip() for x in (matches[0][1] or '').split(',') if x.strip()}
        require(deps <= outline.guard.FOUNDATIONS, 'untrusted axioms')
        axioms[qualified] = sorted(deps)
    (work / 'axioms.json').write_text(json.dumps(axioms, indent=2) + '\n')
    return axioms

try:
    with (output / 'dependency.stdout').open('x') as out, (output / 'dependency.stderr').open('x') as err:
        subprocess.run(['python3', str(R / 'scripts/check-outlined-mutation-proof.py'), '--lean', lean, '--work', str(Q), '--output', str(S / 'proof-dependency')], stdout=out, stderr=err, check=True, timeout=180)
    dependency = json.loads((S / 'proof-dependency/result.json').read_text())
    require(dependency['accepted'], 'dependency proof failed')
    result['dependency_theorems'] = len(dependency['guard_theorems']) + len(dependency['adapter_theorems'])
    result['dependency_controls'] = len(dependency['controls'])
    result['theorems'] = check(proof, 'positive')
    for name, old, new, marker in [
        ('wrong-branch', 'outlined unique cloneReplace expose owner', 'outlined (!unique) cloneReplace expose owner', 'Lean rejected isolated proof'),
        ('restore-initial-owner', ')).slot = during.current', ')).slot = owner.current', 'Lean rejected isolated proof'),
        ('proof-hole', '  rw [extraction_equivalence]', '  sorry', 'Lean rejected isolated proof'),
        ('custom-axiom', 'namespace Kv9.IsolatedOutline', 'namespace Kv9.IsolatedOutline\naxiom fake : False', 'untrusted axioms'),
    ]:
        require(proof.count(old) == 1, 'control anchor mismatch: ' + name)
        changed = proof.replace(old, new)
        if name == 'custom-axiom':
            changed = changed.replace('  rw [extraction_equivalence]', '  exact False.elim fake')
        try:
            check(changed, 'control-' + name)
        except outline.guard.helpers.Rejected as error:
            require(marker in str(error), 'control failed outside intended gate: ' + name)
            result['controls'].append(dict(name=name, rejected=True, reason=str(error)))
        else:
            raise outline.guard.helpers.Rejected('invalid control accepted: ' + name)
    # The original pointer source is a separate binding, not the patched source
    # named in the dependency proof's historical combined-candidate contract.
    original = S / 'registry/archery-1.2.3/src/shared_pointer/kind/erased_ptr.rs'
    changed = (Q / 'candidate-archery/src/shared_pointer/kind/erased_ptr.rs').read_bytes()
    require(hashlib.sha256(changed).hexdigest() != sha(original), 'source control does not change the guard')
    try:
        outline.guard.bind(changed, sha(original))
    except outline.guard.helpers.Rejected as error:
        require('reviewed source hash mismatch' in str(error), 'wrong source rejection')
        result['controls'].append(dict(name='substitute-static-pointer-guard', rejected=True, reason=str(error)))
    else:
        raise outline.guard.helpers.Rejected('changed guard accepted')
    require(all(sha(Path(p)) == h for p, h in bindings.items()), 'proof input changed')
    result.update(complete=True, accepted=True, source_bindings=len(bindings))
except BaseException as error:
    result['failure'] = repr(error)
    raise
finally:
    result['ended_ns'] = time.time_ns()
    (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({'accepted': True, 'new_theorems': len(result['theorems']), 'new_controls': len(result['controls']), 'dependency_theorems': result['dependency_theorems'], 'dependency_controls': result['dependency_controls']}))
