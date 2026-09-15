#!/usr/bin/env python3
"""Check pre-upload ownership and journal recovery, strict proofs and isolated fault controls.

No database, object-store operation or hosted CI is started by this runner.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
MODEL = ROOT / 'proofs/tla/checkpoint_owner'
PROOF = ROOT / 'proofs/tlaps/checkpoint_owner'


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


tlc = module('ledger_tlc', 'check-tla.py')
proof = module('ledger_proof', 'check-tlaps.py')
require = proof.require


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def model_case(args, name, model, config, expected=None, mc=None):
    work = args.output / name
    work.mkdir()
    (work / 'CheckpointOwner.tla').write_text(model)
    (work / 'CheckpointOwnerMC.tla').write_text((MODEL / 'CheckpointOwnerMC.tla').read_text() if mc is None else mc)
    (work / 'Run.cfg').write_text(config)
    command = ['java', f'-Djava.io.tmpdir={work}', '-Xmx256m', '-cp', str(args.jar),
               'tlc2.TLC', '-tool', '-workers', '1', '-fp', '0',
               '-metadir', str(work / 'states'), '-config', 'Run.cfg', 'CheckpointOwnerMC.tla']
    record = {'command': command, 'expected': expected,
              'inputs': {p.name: proof.sha(p) for p in work.iterdir()}}
    started = time.monotonic()
    try:
        with (work / 'tlc.log').open('w') as log:
            result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                                    timeout=args.timeout)
        record.update(exit_code=result.returncode, seconds=time.monotonic() - started)
        record['statistics'] = tlc.verdict(
            (work / 'tlc.log').read_text(), result.returncode, expected,
            temporal='PROPERTY' in config, module='CheckpointOwner',
            minimum_distinct=2 if expected else 3,
            minimum_trace_states=1 if expected == 'EventuallyDrained' else 2)
        record['status'] = 'accepted'
    except Exception as error:
        record.update(status='rejected', reason=str(error))
        raise
    finally:
        save(work / 'result.json', record)
    print(f"PASS: {name}: {record['statistics']['distinct']} distinct states", flush=True)
    return record


def proof_case(args, name, model, source, expected=None):
    work = args.output / name
    work.mkdir()
    (work / 'CheckpointOwner.tla').write_text(model)
    (work / 'CheckpointOwnerProof.tla').write_text(source)
    shutil.copyfile(PROOF / 'inventory.json', work / 'inventory.json')
    record = {'expected_rejection': expected,
              'inputs': {p.name: proof.sha(p) for p in work.iterdir()}}
    try:
        proof.check_tree(args, work, record)
        require(expected is None, f'{name}: invalid proof was accepted')
        record['status'] = 'proved'
    except proof.Rejected as error:
        if expected is None or expected not in str(error):
            record.update(status='rejected', reason=str(error))
            raise
        if expected == 'CheckpointOwnerProof: TLAPS exit 10':
            log = (work / 'CheckpointOwnerProof.log').read_text()
            require('obligations failed' in log and 'Could not prove or check' in log,
                    'negative proof failed without an actual unproved obligation')
        record.update(status='expected rejection', reason=str(error))
    finally:
        save(work / 'result.json', record)
    print(f"PASS: {name}: {record['status']}", flush=True)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jar', type=Path, required=True)
    parser.add_argument('--tlapm', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout', type=int, default=180)
    args = parser.parse_args()
    args.jar, args.tlapm, args.output = args.jar.resolve(), args.tlapm.resolve(), args.output.resolve()
    require(0 < args.timeout <= 600, 'timeout must be 1..600 seconds')
    inventory = json.loads((PROOF / 'inventory.json').read_text())
    require(proof.sha(args.jar) == inventory['sany_jar_sha256'], 'TLA jar differs')
    require(subprocess.check_output([str(args.tlapm), '--version'], text=True, timeout=15).strip() == inventory['tlapm_version'], 'TLAPS version differs')
    for name, digest in inventory['source_pins'].items():
        require(proof.sha(ROOT / name) == digest, f'source pin differs: {name}')
    require(not args.output.exists(), 'output must be a new directory')
    args.output.mkdir(parents=True)
    args.classes = args.output / 'auditor'
    args.classes.mkdir()
    args.stdlib = args.tlapm.parent.parent / 'lib/tlapm/stdlib'
    shutil.copyfile(ROOT / 'scripts/ProofAudit.java', args.classes / 'ProofAudit.java')
    subprocess.run(['javac', '-cp', str(args.jar), '-d', str(args.classes), str(args.classes / 'ProofAudit.java')], check=True, timeout=30)
    inputs = ['scripts/check-checkpoint-owner.py', 'scripts/check-tla.py', 'scripts/check-tlaps.py', 'scripts/ProofAudit.java', *inventory['source_pins']]
    inputs += [str(p.relative_to(ROOT)) for folder in (MODEL, PROOF) for p in folder.iterdir() if p.is_file()]
    save(args.output / 'inputs.json', {name: proof.sha(ROOT / name) for name in inputs})
    model = (MODEL / 'CheckpointOwner.tla').read_text()
    source = (PROOF / 'CheckpointOwnerProof.tla').read_text()
    results = {'proof': proof_case(args, 'proof', model, source)}
    valid = (MODEL / 'Owner.cfg').read_text()
    for name, config in [('one-operation', valid.replace('{1,2}', '{1}')), ('two-operations', valid)]:
        results[name] = model_case(args, name, model, config)
    # Restrict negative evidence to an already applied identity, then require
    # an actual trace that clears its journal while retaining Published Pending.
    # This deliberately false extra invariant is a reachability witness, not a
    # fault in the production protocol or one of its safety invariants.
    late_model = proof.replace_once(model,
        'CNegativeEvidence(o) == /\\ cJournal = o /\\ o \\in cPrepared\n',
        'CNegativeEvidence(o) == /\\ cJournal = o /\\ o \\in cPrepared /\\ o \\in cEffects\n')
    late_mc = ('---------------- MODULE CheckpointOwnerMC ----------------\n'
               'EXTENDS CheckpointOwner\n'
               'CNoAppliedNegativeClear == \\A o \\in cCleared :\n'
               '    ~(o \\in cNegative /\\ o \\in cEffects /\\ cPending[o] = 2 /\\ cVersion[o] = 0)\n'
               '=========================================================\n')
    results['late-negative-retains-owner'] = model_case(args,
        'late-negative-retains-owner', late_model,
        valid.replace('{1,2}', '{1}') + '\nINVARIANT CNoAppliedNegativeClear\n',
        'CNoAppliedNegativeClear', mc=late_mc)
    mutations = {
        'io-without-owner': ('cJournal = o /\\ CProtected(o)', 'cJournal = o /\\ TRUE', 'CCovered'),
        'verified-without-visible-bytes': ('cJournal = o /\\ o \\in cRemote', 'cJournal = o /\\ TRUE', 'CType'),
        'apply-without-preparation': ('CApply(o) == /\\ cJournal = o /\\ o \\in cPrepared\n', 'CApply(o) == /\\ cJournal = o /\\ TRUE\n', 'CType'),
        'replace-active-intent': ('cJournal = CNone /\\ o \\notin cUsed', 'TRUE /\\ o \\notin cUsed', 'CSlot'),
        'clear-unknown-intent': ('(o \\in cNegative \\/ (o \\in cPositive /\\ cPending[o] = 4 /\\ cVersion[o] = 2))', 'TRUE', 'CSettled'),
        # Removing the last published owner first violates coverage (which is
        # checked before transfer), as the retained first run demonstrated.
        'quiesce-before-published-successor': ('cPending[o] = 2 /\\ cVersion[o] = 2', 'cPending[o] = 2 /\\ cVersion[o] \\in {1,2}', 'CCovered'),
    }
    for name, (before, after, expected) in mutations.items():
        results[name] = model_case(args, name, proof.replace_once(model, before, after), valid, expected)
    start = source.index('BY CInputs, SMT DEF CInit')
    end = source.index('THEOREM CInvariantStep')
    results['proof-hole'] = proof_case(args, 'proof-hole', model, source[:start] + 'PROOF OMITTED\n' + source[end:], 'proof hole:')
    axiom = proof.replace_once(source, 'EXTENDS CheckpointOwner, TLAPS', 'EXTENDS CheckpointOwner, TLAPS\nAXIOM UnsafeShortcut == FALSE')
    results['custom-axiom'] = proof_case(args, 'custom-axiom', model, axiom, 'unapproved module assumption:')
    unsafe = proof.replace_once(model, *mutations['io-without-owner'][:2])
    results['unowned-io-proof'] = proof_case(args, 'unowned-io-proof', unsafe, source, 'CheckpointOwnerProof: TLAPS exit 10')
    save(args.output / 'summary.json', dict(status='PASS', theorems=7, obligations=43, positive_models=2,
        model_counterexamples=6, reachability_witnesses=1, proof_rejections=3, results=results, source_pins=inventory['source_pins'],
        scope='Durable journal/owner projection under atomic validated local journal recovery, exact ledger receipts and typed seam evidence. Restart stutters on durable state; volatile scheduling is separate. No complete legacy coverage, negative-owner release, destination admission, reader drainage or deletion proof.'))
    print('PASS: checkpoint owner: 7 theorems / 43 obligations; 2 models, 6 counterexamples, 1 reachability witness, 3 proof rejections', flush=True)


if __name__ == '__main__':
    main()
