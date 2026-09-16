#!/usr/bin/env python3
"""Check concrete heap graph and root/child COW primitives with rejecting controls."""
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
assert OUT.is_relative_to(Path('/mnt/data/kv9-work'))
OUT.mkdir(parents=True, exist_ok=False)
contract = json.loads((PROOFS / 'heap-cow-contract.json').read_text())
sha = lambda data: hashlib.sha256(data).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def bind_source(name, data):
    require(sha(data) == contract['sources'][name], 'source hash mismatch: ' + name)


def source_check():
    for name in contract['sources']:
        bind_source(name, (REPO / name).read_bytes())


source_check()
require(sha(LEAN.read_bytes()) == contract['lean_sha256'], 'compiler hash mismatch')
sources = {name: (PROOFS / name).read_text() for name in contract['modules']}
names = [name for module in contract['modules']
         for name in re.findall(r'^\s*theorem\s+(\w+)', sources[module], re.M)]
require(names == contract['theorems'] and len(names) == len(set(names)), 'theorem inventory mismatch')
require(all('set_option autoImplicit false' in source for source in sources.values()),
        'implicit theorem parameters are not disabled')
result = {'complete': False, 'accepted': False, 'started_ns': time.time_ns(),
          'sources': contract['sources'], 'scope': contract['scope'], 'controls': [],
          'complete_algorithm_gate': False, 'lean_sha256': contract['lean_sha256']}
(OUT / 'source-contract.json').write_text(json.dumps(contract, indent=2) + '\n')
(OUT / 'lean-version.txt').write_bytes(subprocess.check_output([str(LEAN), '--version']))


def check(modules, label):
    work = OUT / label
    work.mkdir()
    for name, source in modules.items():
        (work / name).write_text(source)
    (work / 'Audit.lean').write_text('import HeapExamples\n' +
        '\n'.join('#print axioms Kv9.Radix.' + name for name in names) + '\n')
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
            require(not any(message in log for message in [
                'unexpected token', 'Unknown identifier', 'unknown tactic',
            ]), 'control parse/name failure: ' + name)
            raise ValueError('Lean rejected proof in ' + name)
    log = (work / 'Audit.lean.log').read_text()
    axioms = {}
    for name in names:
        qualified = 'Kv9.Radix.' + name
        matches = list(re.finditer(re.escape("'" + qualified + "'") +
            r' (?:depends on axioms: \[([^]]*)\]|does not depend on any axioms)', log))
        require(len(matches) == 1, 'missing axiom inventory: ' + name)
        used = {x.strip() for x in (matches[0][1] or '').split(',') if x.strip()}
        require(used <= {'propext', 'Classical.choice', 'Quot.sound'}, 'untrusted axiom dependency')
        axioms[qualified] = sorted(used)
    (work / 'axioms.json').write_text(json.dumps(axioms, indent=2) + '\n')
    return axioms


try:
    result['theorems'] = check(sources, 'positive')
    for control in contract['controls']:
        label, module = control['name'], control['module']
        old, new = control['old'], control['new']
        require(sources[module].count(old) == 1, 'control anchor mismatch: ' + label)
        changed = dict(sources)
        changed[module] = changed[module].replace(old, new)
        if control['kind'] == 'axiom':
            changed[module] = changed[module].replace('namespace Kv9.Radix',
                'namespace Kv9.Radix\naxiom injectedHeapFalse : False', 1)
        expected = 'untrusted axiom dependency' if control['kind'] == 'axiom' else 'Lean rejected proof'
        try:
            check(changed, 'control-' + label)
        except ValueError as error:
            require(expected in str(error), 'control rejected at wrong gate: ' + label + ': ' + str(error))
            if control['kind'] == 'semantic':
                logs = '\n'.join(p.read_text() for p in (OUT / ('control-' + label)).glob('*.log'))
                require(any(message in logs for message in [
                    'unsolved goals', 'Type mismatch', 'Application type mismatch', 'not definitionally equal to target',
                    'Tactic `', 'omega could not prove',
                ]), 'no substantive proof failure: ' + label)
            result['controls'].append({'name': label, 'kind': control['kind'],
                                       'rejected': True, 'reason': str(error)})
        else:
            raise ValueError('invalid model/proof accepted: ' + label)
        print(json.dumps(result['controls'][-1]), flush=True)
    rust = (REPO / 'scripts/resident-radix/src/lib.rs').read_text()
    for control in contract['source_controls']:
        label, old, new = control['name'], control['old'], control['new']
        require(rust.count(old) == 1, 'source anchor mismatch: ' + label)
        changed = rust.replace(old, new).encode()
        (OUT / (label + '.rs')).write_bytes(changed)
        try:
            bind_source('scripts/resident-radix/src/lib.rs', changed)
        except ValueError as error:
            require('source hash mismatch:' in str(error), 'unexpected source control error: ' + label)
            result['controls'].append({'name': label, 'kind': 'source-binding', 'rejected': True,
                                      'reason': 'reviewed source hash mismatch; not a semantic Rust proof'})
        else:
            raise ValueError('source substitution accepted: ' + label)
    source_check()
    result.update(complete=True, accepted=True)
except BaseException as error:
    result['failure'] = repr(error)
    raise
finally:
    result['ended_ns'] = time.time_ns()
    (OUT / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({'accepted': True, 'theorems': len(result['theorems']),
                  'new_theorems': len(contract['new_theorems']),
                  'rejecting_controls': len(result['controls']), 'complete_algorithm_gate': False}))
