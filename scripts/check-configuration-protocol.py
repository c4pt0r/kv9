#!/usr/bin/env python3
"""Check configuration-at-cut selection, strict proofs and isolated fault controls.

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
MODEL = ROOT / 'proofs/tla/configuration_cut'
PROOF = ROOT / 'proofs/tlaps/configuration_cut'


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


tlc = module('configuration_tlc', 'check-tla.py')
proof = module('configuration_proof', 'check-tlaps.py')
require = proof.require


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def model_case(args, name, model, config, expected=None, mc=None):
    work = args.output / name
    work.mkdir()
    (work / 'ConfigurationCut.tla').write_text(model)
    (work / 'ConfigurationCutMC.tla').write_text((MODEL / 'ConfigurationCutMC.tla').read_text() if mc is None else mc)
    (work / 'Run.cfg').write_text(config)
    command = ['java', f'-Djava.io.tmpdir={work}', '-Xmx256m', '-cp', str(args.jar),
               'tlc2.TLC', '-tool', '-workers', '1', '-fp', '0',
               '-metadir', str(work / 'states'), '-config', 'Run.cfg', 'ConfigurationCutMC.tla']
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
            temporal='PROPERTY' in config, module='ConfigurationCut',
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
    (work / 'ConfigurationCut.tla').write_text(model)
    (work / 'ConfigurationCutProof.tla').write_text(source)
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
        if expected == 'ConfigurationCutProof: TLAPS exit 10':
            log = (work / 'ConfigurationCutProof.log').read_text()
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
    version = subprocess.check_output([str(args.tlapm), '--version'], text=True, timeout=15).strip()
    require(version == inventory['tlapm_version'], 'TLAPS version differs')
    for name, digest in inventory['source_pins'].items():
        require(proof.sha(ROOT / name) == digest, f'source pin differs: {name}')
    require(not args.output.exists(), 'output must be a new directory')
    args.output.mkdir(parents=True)
    args.classes = args.output / 'auditor'
    args.classes.mkdir()
    args.stdlib = args.tlapm.parent.parent / 'lib/tlapm/stdlib'
    shutil.copyfile(ROOT / 'scripts/ProofAudit.java', args.classes / 'ProofAudit.java')
    subprocess.run(['javac', '-cp', str(args.jar), '-d', str(args.classes),
                    str(args.classes / 'ProofAudit.java')], check=True, timeout=30)
    inputs = ['scripts/check-configuration-protocol.py', 'scripts/check-tla.py',
              'scripts/check-tlaps.py', 'scripts/ProofAudit.java',
              'proofs/tla/configuration_cut/ConfigurationCut.tla', 'proofs/tla/configuration_cut/ConfigurationCutMC.tla',
              'proofs/tla/configuration_cut/Cut2.cfg', 'proofs/tla/configuration_cut/Cut3.cfg',
              'proofs/tla/configuration_cut/Inspect3.cfg', 'proofs/tlaps/configuration_cut/ConfigurationCutProof.tla',
              'proofs/tlaps/configuration_cut/inventory.json', *inventory['source_pins']]
    save(args.output / 'inputs.json', {name: proof.sha(ROOT / name) for name in inputs})
    model = (MODEL / 'ConfigurationCut.tla').read_text()
    source = (PROOF / 'ConfigurationCutProof.tla').read_text()
    results = {}
    results['proof'] = proof_case(args, 'proof', model, source)
    for name in ('Cut2', 'Cut3', 'Inspect3'):
        results[name] = model_case(args, name, model, (MODEL / f'{name}.cfg').read_text())
    config = (MODEL / 'Cut3.cfg').read_text()
    mutations = {
        'future-configuration': (r'/\ s \in {0} \cup hcApplied /\ s <= q',
                                 r'/\ s \in {0} \cup hcApplied'),
        'unapplied-configuration': (r'ELSE IF \E i \in HCChanges : s < i /\ i <= q THEN "Unavailable"',
                                    'ELSE IF FALSE THEN "Unavailable"'),
        'failed-writer': ('IF ~hcWriter THEN "Invalid"', 'IF FALSE THEN "Invalid"'),
        'wrong-cut-term': ('IF claim # HCTerms[q] THEN "Invalid"', 'IF FALSE THEN "Invalid"'),
        'unknown-history': ('IF ~hcKnown THEN "Unavailable"', 'IF FALSE THEN "Unavailable"'),
        'wrong-entry-kind': (r'IF s # 0 /\ s \notin HCChanges THEN "Invalid"', 'IF FALSE THEN "Invalid"'),
        'publish-before-durable': (r'HCPublish == /\ hcWriter /\ hcPending # 0 /\ hcPending \in hcDurable',
                                    r'HCPublish == /\ hcWriter /\ hcPending # 0'),
    }
    for name, (before, after) in mutations.items():
        changed = proof.replace_once(model, before, after)
        results[name] = model_case(args, name, changed, config, 'HCInvariant')
    unfair = proof.replace_once((MODEL / 'ConfigurationCutMC.tla').read_text(),
                                 r'/\ WF_hcVars(HCQueries)', '')
    results['unfair-inspection'] = model_case(args, 'unfair-inspection', model,
        (MODEL / 'Inspect3.cfg').read_text(), 'EventuallyDrained', mc=unfair)
    start = source.index('BY HCInputs, SMT DEF HCInit')
    end = source.index('THEOREM HCInvariantStep')
    hole = source[:start] + 'PROOF OMITTED\n' + source[end:]
    results['proof-hole'] = proof_case(args, 'proof-hole', model, hole, 'proof hole:')
    axiom = proof.replace_once(source, 'EXTENDS ConfigurationCut, TLAPS',
                               'EXTENDS ConfigurationCut, TLAPS\nAXIOM UnsafeShortcut == FALSE')
    results['custom-axiom'] = proof_case(args, 'custom-axiom', model, axiom,
                                      'unapproved module assumption:')
    bad_scan = proof.replace_once(model, *mutations['unapplied-configuration'])
    results['unsafe-selection-proof'] = proof_case(args, 'unsafe-selection-proof', bad_scan,
                                                  source, 'ConfigurationCutProof: TLAPS exit 10')
    save(args.output / 'summary.json', {
        'status': 'PASS', 'tlapm_sha256': proof.sha(args.tlapm),
        'jar_sha256': proof.sha(args.jar), 'theorems': 11, 'obligations': 95,
        'positive_models': 3, 'model_counterexamples': 8, 'proof_rejections': 3,
        'source_pins': inventory['source_pins'], 'results': results,
        'scope': 'Committed configuration selection and stable local inspection; trusted retained Raft prefix and full applied payload. No complete anchor, install, replicated ledger or new Chaos acceptance.'
    })
    print('PASS: configuration cut: 11 theorems / 95 obligations, three models, eight counterexamples, three proof rejections', flush=True)



if __name__ == '__main__':
    main()
