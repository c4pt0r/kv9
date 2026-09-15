#!/usr/bin/env python3
"""Check local recovery anchor binding, strict proofs and isolated fault controls.

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
MODEL = ROOT / 'proofs/tla/anchor_binding'
PROOF = ROOT / 'proofs/tlaps/anchor_binding'


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


tlc = module('anchor_tlc', 'check-tla.py')
proof = module('anchor_proof', 'check-tlaps.py')
require = proof.require


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def model_case(args, name, model, config, expected=None, mc=None):
    work = args.output / name
    work.mkdir()
    (work / 'AnchorBinding.tla').write_text(model)
    (work / 'AnchorBindingMC.tla').write_text((MODEL / 'AnchorBindingMC.tla').read_text() if mc is None else mc)
    (work / 'Run.cfg').write_text(config)
    command = ['java', f'-Djava.io.tmpdir={work}', '-Xmx256m', '-cp', str(args.jar),
               'tlc2.TLC', '-tool', '-workers', '1', '-fp', '0',
               '-metadir', str(work / 'states'), '-config', 'Run.cfg', 'AnchorBindingMC.tla']
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
            temporal='PROPERTY' in config, module='AnchorBinding',
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
    (work / 'AnchorBinding.tla').write_text(model)
    (work / 'AnchorBindingProof.tla').write_text(source)
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
        if expected == 'AnchorBindingProof: TLAPS exit 10':
            log = (work / 'AnchorBindingProof.log').read_text()
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
    inputs = ['scripts/check-anchor-binding.py', 'scripts/check-tla.py', 'scripts/check-tlaps.py', 'scripts/ProofAudit.java', *inventory['source_pins']]
    inputs += [str(p.relative_to(ROOT)) for folder in (MODEL, PROOF) for p in folder.iterdir() if p.is_file()]
    save(args.output / 'inputs.json', {name: proof.sha(ROOT / name) for name in inputs})
    model = (MODEL / 'AnchorBinding.tla').read_text()
    source = (PROOF / 'AnchorBindingProof.tla').read_text()
    results = {'proof': proof_case(args, 'proof', model, source)}
    valid = (MODEL / 'Valid.cfg').read_text()
    for name in ('Valid', 'WrongRoot', 'MissingConfiguration', 'MissingPublication', 'InvalidFrame'):
        results[name] = model_case(args, name, model, (MODEL / f'{name}.cfg').read_text())
    mutations = {
        'bind-decoded-image': ('ABind == /\\ aPhase = "complete"\n         /\\ aCap\' = IF AFrameOK[aSelected] THEN aSelected ELSE 0',
            'ABind == /\\ aPhase = "complete" /\\ aDecoded # 0\n         /\\ aCap\' = IF AFrameOK[aSelected] THEN aDecoded ELSE 0', valid),
        'decode-mints-observation': ('/\\ UNCHANGED <<aSelected, aBase, aPublication, aCap, aPhase, aComplete>>',
            '/\\ aCap\' = aDecoded\' /\\ UNCHANGED <<aSelected, aBase, aPublication, aPhase, aComplete>>', valid),
        'bind-before-completion': ('ABind == /\\ aPhase = "complete"',
            'ABind == /\\ aPhase \\in {"scan", "complete"}', valid),
        'ignore-root': ('aBase\' = IF ARootOK[aSelected] THEN aSelected ELSE 0\n         /\\ aPhase\' = IF ARootOK[aSelected] THEN',
            'aBase\' = IF TRUE THEN aSelected ELSE 0\n         /\\ aPhase\' = IF TRUE THEN', (MODEL / 'WrongRoot.cfg').read_text()),
        'ignore-configuration': ('ok == AConfigurationOK[aSelected] /\\ APublicationOK[aSelected]',
            'ok == TRUE /\\ APublicationOK[aSelected]', (MODEL / 'MissingConfiguration.cfg').read_text()),
        'ignore-publication': ('ok == AConfigurationOK[aSelected] /\\ APublicationOK[aSelected]',
            'ok == AConfigurationOK[aSelected] /\\ TRUE', (MODEL / 'MissingPublication.cfg').read_text()),
        'ignore-frame': ('aCap\' = IF AFrameOK[aSelected] THEN aSelected ELSE 0\n         /\\ aPhase\' = IF AFrameOK[aSelected] THEN',
            'aCap\' = IF TRUE THEN aSelected ELSE 0\n         /\\ aPhase\' = IF TRUE THEN', (MODEL / 'InvalidFrame.cfg').read_text()),
    }
    for name, (before, after, config) in mutations.items():
        results[name] = model_case(args, name, proof.replace_once(model, before, after), config, 'AInvariant')
    start = source.index('BY AInputs, SMT DEF AInit')
    end = source.index('THEOREM AInvariantStep')
    results['proof-hole'] = proof_case(args, 'proof-hole', model, source[:start] + 'PROOF OMITTED\n' + source[end:], 'proof hole:')
    axiom = proof.replace_once(source, 'EXTENDS AnchorBinding, TLAPS', 'EXTENDS AnchorBinding, TLAPS\nAXIOM UnsafeShortcut == FALSE')
    results['custom-axiom'] = proof_case(args, 'custom-axiom', model, axiom, 'unapproved module assumption:')
    unsafe = proof.replace_once(model, *mutations['bind-before-completion'][:2])
    results['premature-binding-proof'] = proof_case(args, 'premature-binding-proof', unsafe, source, 'AnchorBindingProof: TLAPS exit 10')
    too_small = proof.replace_once(source, '<= 1147128', '<= 1147127')
    results['false-size-bound'] = proof_case(args, 'false-size-bound', model, too_small, 'AnchorBindingProof: TLAPS exit 10')
    save(args.output / 'summary.json', dict(status='PASS', theorems=7, obligations=26, positive_models=5,
        model_counterexamples=7, proof_rejections=4, results=results, source_pins=inventory['source_pins'],
        scope='Private local observation from one completed image/base/configuration/publication recovery; public decoding cannot mint it. Lower recovery and bounded canonical codec checks are explicit premises. No Rust refinement, destination installation, outer retention ledger, ownership-chain or object-store power-loss proof.'))
    print('PASS: anchor binding: 7 theorems / 26 obligations; 5 models, 7 counterexamples, 4 proof rejections', flush=True)


if __name__ == '__main__':
    main()
