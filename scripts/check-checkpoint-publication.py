#!/usr/bin/env python3
"""Check checkpoint publication, strict proofs and isolated fault controls.

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
MODEL = ROOT / 'proofs/tla/checkpoint_publication'
PROOF = ROOT / 'proofs/tlaps/checkpoint_publication'


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


tlc = module('publication_tlc', 'check-tla.py')
proof = module('publication_proof', 'check-tlaps.py')
require = proof.require


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def model_case(args, name, model, config, expected=None, mc=None):
    work = args.output / name
    work.mkdir()
    (work / 'CheckpointPublication.tla').write_text(model)
    (work / 'CheckpointPublicationMC.tla').write_text((MODEL / 'CheckpointPublicationMC.tla').read_text() if mc is None else mc)
    (work / 'Run.cfg').write_text(config)
    command = ['java', f'-Djava.io.tmpdir={work}', '-Xmx256m', '-cp', str(args.jar),
               'tlc2.TLC', '-tool', '-workers', '1', '-fp', '0',
               '-metadir', str(work / 'states'), '-config', 'Run.cfg', 'CheckpointPublicationMC.tla']
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
            temporal='PROPERTY' in config, module='CheckpointPublication',
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
    (work / 'CheckpointPublication.tla').write_text(model)
    (work / 'CheckpointPublicationProof.tla').write_text(source)
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
        if expected == 'CheckpointPublicationProof: TLAPS exit 10':
            log = (work / 'CheckpointPublicationProof.log').read_text()
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
    inputs = ['scripts/check-checkpoint-publication.py', 'scripts/check-tla.py',
              'scripts/check-tlaps.py', 'scripts/ProofAudit.java',
              'proofs/tla/checkpoint_publication/CheckpointPublication.tla', 'proofs/tla/checkpoint_publication/CheckpointPublicationMC.tla',
              'proofs/tla/checkpoint_publication/Winner.cfg', 'proofs/tla/checkpoint_publication/Loser.cfg', 'proofs/tlaps/checkpoint_publication/CheckpointPublicationProof.tla',
              'proofs/tlaps/checkpoint_publication/inventory.json', *inventory['source_pins']]
    save(args.output / 'inputs.json', {name: proof.sha(ROOT / name) for name in inputs})
    model = (MODEL / 'CheckpointPublication.tla').read_text()
    source = (PROOF / 'CheckpointPublicationProof.tla').read_text()
    results = {}
    results['proof'] = proof_case(args, 'proof', model, source)
    for name in ('Winner', 'Loser'):
        results[name] = model_case(args, name, model, (MODEL / f'{name}.cfg').read_text())
    winner = (MODEL / 'Winner.cfg').read_text()
    loser = (MODEL / 'Loser.cfg').read_text()
    mutations = {
        'proposal-presence': (r'p \in cpWinners /\ CPObservedImage[p] = CPSelected',
                              'CPObservedImage[p] = CPSelected', loser),
        'wrong-command-image': ('CPMatches(i) == CPImage[i] = CPObservedImage[i] /\\',
                                 'CPMatches(i) ==', loser.replace('CPObservedImage <- Images', 'CPObservedImage <- WrongImages')),
        'wrong-publication-term': (r'/\ CPTerm[i] = CPReportedTerm[i]', '',
                                    winner.replace('CPReportedTerm <- Terms', 'CPReportedTerm <- SkewedTerms')),
        'partial-recovery': (r'CPGrant == /\ cpPhase = "complete"',
                             r'CPGrant == /\ cpPhase \in {"scan", "complete"}', winner),
        'missing-final-history': (r'IF cpSeen # 0 /\ CPFinalHistoryMatches THEN',
                                   'IF cpSeen # 0 THEN', winner.replace('CPFinalHistoryMatches = TRUE', 'CPFinalHistoryMatches = FALSE')),
    }
    for name, (before, after, config) in mutations.items():
        results[name] = model_case(args, name, proof.replace_once(model, before, after), config, 'CPInvariant')
    start = source.index('BY CPInputs, SMT DEF CPInit')
    end = source.index('THEOREM CPInvariantStep')
    hole = source[:start] + 'PROOF OMITTED\n' + source[end:]
    results['proof-hole'] = proof_case(args, 'proof-hole', model, hole, 'proof hole:')
    axiom = proof.replace_once(source, 'EXTENDS CheckpointPublication, TLAPS',
                               'EXTENDS CheckpointPublication, TLAPS\nAXIOM UnsafeShortcut == FALSE')
    results['custom-axiom'] = proof_case(args, 'custom-axiom', model, axiom,
                                      'unapproved module assumption:')
    bad_scan = proof.replace_once(model, *mutations['proposal-presence'][:2])
    results['unsafe-selection-proof'] = proof_case(args, 'unsafe-selection-proof', bad_scan,
                                                  source, 'CheckpointPublicationProof: TLAPS exit 10')
    save(args.output / 'summary.json', {
        'status': 'PASS', 'tlapm_sha256': proof.sha(args.tlapm),
        'jar_sha256': proof.sha(args.jar), 'theorems': 9, 'obligations': 36,
        'positive_models': 2, 'model_counterexamples': 5, 'proof_rejections': 3,
        'source_pins': inventory['source_pins'], 'results': results,
        'scope': 'Successful local checkpoint publication from atomic apply history and matching committed command, completed recovery and finite scan rank. Does not prove full portable anchors, root/range admission, physical power loss or replicated retention.'
    })
    print('PASS: checkpoint publication: 9 theorems / 36 obligations, two models, five counterexamples, three proof rejections', flush=True)



if __name__ == '__main__':
    main()
