#!/usr/bin/env python3
"""Check scoped radix point-operation proofs and rejecting controls locally."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

REPO = Path(__file__).resolve().parents[2]
PROOFS = REPO / 'proofs/lean/radix'
OUT = Path(sys.argv[1]).resolve()
LEAN = Path(sys.argv[2]).resolve()
OUT.mkdir(parents=True, exist_ok=False)
assert OUT.is_relative_to(Path('/mnt/data/kv9-work'))
contract = json.loads((PROOFS / 'source-contract.json').read_text())
sha = lambda data: hashlib.sha256(data).hexdigest()

def require(condition, message):
    if not condition:
        raise ValueError(message)

def bind(data, expected):
    require(sha(data) == expected, 'reviewed source hash mismatch')

def source_check():
    for name, expected in contract['sources'].items():
        bind((REPO / name).read_bytes(), expected)

source_check()
bind(LEAN.read_bytes(), contract['lean_sha256'])
sources = {name: (PROOFS / name).read_text() for name in contract['modules']}
names = [name for module in contract['modules']
         for name in re.findall(r'^\s*theorem\s+(\w+)', sources[module], re.M)]
require(names == contract['theorems'] and len(names) == len(set(names)), 'theorem inventory mismatch')
result = {'complete': False, 'accepted': False, 'started_ns': time.time_ns(), 'controls': [],
          'sources': contract['sources'], 'scope': contract['scope'],
          'complete_algorithm_gate': False, 'lean_sha256': sha(LEAN.read_bytes())}
(OUT / 'source-contract.json').write_text(json.dumps(contract, indent=2) + '\n')
(OUT / 'lean-version.txt').write_bytes(subprocess.check_output([str(LEAN), '--version']))

def check(modules, label):
    work = OUT / label
    work.mkdir()
    for name, source in modules.items():
        (work / name).write_text(source)
    audit = 'import History\n' + '\n'.join('#print axioms Kv9.Radix.' + name for name in names) + '\n'
    (work / 'Audit.lean').write_text(audit)
    commands = []
    for name in [*contract['modules'], 'Audit.lean']:
        argv = [str(LEAN), '-DwarningAsError=true', '-o', name.replace('.lean', '.olean'), name]
        with (work / (name + '.log')).open('x') as output:
            process = subprocess.run(argv, cwd=work, env=dict(os.environ, LEAN_PATH=str(work)),
                                     stdout=output, stderr=subprocess.STDOUT, timeout=120)
        commands.append({'argv': argv, 'exit_code': process.returncode})
        (work / 'commands.json').write_text(json.dumps(commands, indent=2) + '\n')
        if process.returncode:
            log = (work / (name + '.log')).read_text()
            require('unexpected token' not in log and 'Unknown identifier' not in log,
                    'control failed to parse instead of testing a proof')
            raise ValueError('Lean rejected proof in ' + name)
    log = (work / 'Audit.lean.log').read_text()
    axioms = {}
    for name in names:
        qualified = 'Kv9.Radix.' + name
        matches = list(re.finditer(re.escape("'" + qualified + "'") +
            r' (?:depends on axioms: \[([^]]*)\]|does not depend on any axioms)', log))
        require(len(matches) == 1, 'axiom inventory missing: ' + name)
        used = {x.strip() for x in (matches[0][1] or '').split(',') if x.strip()}
        require(used <= {'propext', 'Classical.choice', 'Quot.sound'}, 'untrusted axiom dependency')
        axioms[qualified] = sorted(used)
    (work / 'axioms.json').write_text(json.dumps(axioms, indent=2) + '\n')
    return axioms

try:
    result['theorems'] = check(sources, 'positive')
    controls = [
        ('drop-leaf-value', 'Tree.lean',
         '| .leaf entry, _ => if query = entry.key then some entry.value else none',
         '| .leaf entry, _ => if query = entry.key then none else none'),
        ('ignore-terminal', 'Tree.lean', '| some [] => terminal.map Entry.value', '| some [] => none'),
        ('wrong-edge-route', 'Tree.lean',
         'if target = byte then treeLookup query child suffix else forestLookup query tail target suffix',
         'if target ≠ byte then treeLookup query child suffix else forestLookup query tail target suffix'),
        ('omit-collapsed-edge', 'Normalize.lean',
         'some (.branch (pfx ++ byte :: childPrefix) terminal children)',
         'some (.branch (pfx ++ childPrefix) terminal children)'),
        ('retain-deleted-leaf', 'Delete.lean', '| .leaf _, _ => none', '| .leaf entry, _ => some (.leaf entry)'),
        ('retain-deleted-terminal', 'Delete.lean',
         '| some [] => normalize (.branch pfx none children)',
         '| some [] => normalize (.branch pfx terminal children)'),
        ('retain-old-leaf-value', 'Insert.lean',
         'if old.key = fresh.key then (.leaf fresh, false) else (splitLeaf path old fresh, true)',
         'if old.key = fresh.key then (.leaf old, false) else (splitLeaf path old fresh, true)'),
        ('retain-old-terminal-value', 'Insert.lean',
         '| [] => (.branch pfx (some fresh) children, terminal.isNone)',
         '| [] => (.branch pfx terminal children, terminal.isNone)'),
        ('wrong-inserted-flag', 'Insert.lean',
         'else (splitLeaf path old fresh, true)', 'else (splitLeaf path old fresh, false)'),
        ('keep-consumed-split-edge', 'SplitBranch.lean',
         '| byte :: rest => branchSplitCases shared byte rest terminal children fresh (remaining.drop shared.length)',
         '| byte :: rest => branchSplitCases shared byte (byte :: rest) terminal children fresh (remaining.drop shared.length)'),
        ('omit-common-byte', 'Common.lean',
         'if a = b then a :: common as bs else []', 'if a = b then common as bs else []'),
        ('reverse-history', 'History.lean',
         'history.foldl applyMutation root', 'history.reverse.foldl applyMutation root'),
        ('proof-hole', 'Tree.lean', 'exact ⟨[], by simp⟩', 'sorry'),
        ('custom-axiom', 'Tree.lean', 'exact ⟨[], by simp⟩', 'exact False.elim injectedFalse'),
    ]
    for label, module, old, new in controls:
        require(sources[module].count(old) == 1, 'control anchor mismatch: ' + label)
        changed = dict(sources)
        changed[module] = changed[module].replace(old, new)
        if label == 'custom-axiom':
            changed[module] = changed[module].replace('namespace Kv9.Radix',
                'namespace Kv9.Radix\naxiom injectedFalse : False', 1)
        expected = 'untrusted axiom dependency' if label == 'custom-axiom' else 'Lean rejected proof'
        try:
            check(changed, 'control-' + label)
        except ValueError as error:
            require(expected in str(error), 'control rejected at wrong gate: ' + label + ': ' + str(error))
            result['controls'].append({'name': label, 'rejected': True, 'reason': str(error)})
        else:
            raise ValueError('invalid proof accepted: ' + label)
        print(json.dumps(result['controls'][-1]), flush=True)
    rust = (REPO / 'scripts/resident-radix/src/lib.rs').read_bytes()
    for label, old, new in [
        ('source-collapse-byte', b'prefix.push(edge.byte);', b'prefix.push(edge.byte.wrapping_add(1));'),
        ('source-cow-removal', b'match Arc::make_mut(node)', b'match Arc::get_mut(node).unwrap()'),
        ('source-recursive-release', b'pending.append(&mut branch.edges);', b'branch.edges.clear();'),
    ]:
        require(rust.count(old) == 1, 'source control anchor mismatch: ' + label)
        changed = rust.replace(old, new)
        (OUT / (label + '.rs')).write_bytes(changed)
        try:
            bind(changed, contract['sources']['scripts/resident-radix/src/lib.rs'])
        except ValueError as error:
            result['controls'].append({'name': label, 'rejected': True, 'reason': str(error)})
        else:
            raise ValueError('invalid source accepted')
    source_check()
    result.update(complete=True, accepted=True)
except BaseException as error:
    result['failure'] = repr(error)
    raise
finally:
    result['ended_ns'] = time.time_ns()
    (OUT / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({'accepted': True, 'theorems': len(result['theorems']),
                  'rejecting_controls': len(result['controls']), 'complete_algorithm_gate': False}))
