#!/usr/bin/env python3
"""Check owner-local submit/pump sequencing and retained notifications."""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
MODEL = ROOT / 'proofs/tla/owner-read-pump'
PROOF = ROOT / 'proofs/tlaps/owner-read-pump'


def module(name, file):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / file)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


tlc = module('owner_tlc', 'check-tla.py')
proof = module('owner_proof', 'check-tlaps.py')
require, digest = proof.require, proof.sha


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def case(args, name, model, source, config=None, expected=None, theorem=None, fp=0):
    out = args.output / name
    out.mkdir()
    (out / 'OwnerReadPump.tla').write_text(model)
    (out / 'OwnerReadPumpProof.tla').write_text(source)
    shutil.copyfile(PROOF / 'inventory.json', out / 'inventory.json')
    if config is not None:
        (out / 'Run.cfg').write_text(config)
    result = dict(name=name, expected=expected, fingerprint=fp,
                  sources={p.name: digest(p) for p in sorted(out.iterdir())})
    try:
        if config is not None:
            with tempfile.TemporaryDirectory(prefix='kv9-owner-read-pump-tlc-') as states:
                command = ['java', '-XX:+UseParallelGC', '-Xmx512m', '-cp', str(args.jar),
                           'tlc2.TLC', '-tool', '-workers', '1', '-fp', str(fp),
                           '-metadir', states, '-config', 'Run.cfg', '-coverage', '999', 'OwnerReadPump.tla']
                result['command'] = command
                with (out / 'tlc.log').open('x') as log:
                    run = subprocess.run(command, cwd=out, stdout=log, stderr=subprocess.STDOUT,
                                         timeout=args.timeout)
            result['exit_code'] = run.returncode
            result['statistics'] = tlc.verdict((out / 'tlc.log').read_text(), run.returncode,
                expected, coverage=expected is None, module='OwnerReadPump',
                actions=('ORNotify', 'ORSubmit', 'ORPump', 'ORAbort'),
                minimum_distinct=2 if expected else 3)
        else:
            try:
                proof.check_tree(args, out, result)
            except proof.Rejected as error:
                require(expected is not None and str(error) == expected,
                        name + ': unexpected proof rejection: ' + str(error))
                if theorem:
                    log = (out / 'OwnerReadPumpProof.log').read_text()
                    counts = re.findall(r'\[ERROR\]: (\d+)/(\d+) obligations failed\.', log)
                    require(len(counts) == 1 and 0 < int(counts[0][0]) < int(counts[0][1]) == 20,
                            'intended control must fail a proper subset of the fixed obligations')
                    lines = source.splitlines()
                    start = next(i + 1 for i, line in enumerate(lines) if line.startswith('THEOREM ' + theorem + ' =='))
                    stop = next((i + 1 for i in range(start, len(lines)) if lines[i].startswith('THEOREM ')), len(lines) + 1)
                    errors = [int(n) for n in re.findall(r'line (\d+), characters? [^\n]+:\n\[ERROR\]: Could not prove or check:', log)]
                    require(any(start <= n < stop for n in errors), 'intended theorem did not fail')
                    require('[WARNING]' not in log, 'unexpected proof warning')
            else:
                require(expected is None, name + ': defective proof accepted')
        result['accepted'] = True
    except BaseException as error:
        result['accepted'] = False
        result['failure'] = repr(error)
        raise
    finally:
        save(out / 'result.json', result)
    print('PASS:', name, flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jar', type=Path, required=True)
    parser.add_argument('--tlapm', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout', type=int, default=120)
    args = parser.parse_args()
    args.jar, args.tlapm, args.output = args.jar.resolve(), args.tlapm.resolve(), args.output.resolve()
    args.stdlib = args.tlapm.parent.parent / 'lib/tlapm/stdlib'
    require(args.timeout > 0 and not args.output.exists(), 'fresh output and positive timeout required')
    inventory = json.loads((PROOF / 'inventory.json').read_text())
    require(digest(args.jar) == inventory['sany_jar_sha256'] == tlc.JAR_SHA256, 'TLC/SANY hash differs')
    require(subprocess.check_output([str(args.tlapm), '--version'], text=True).strip() == inventory['tlapm_version'],
            'TLAPS version differs')
    inputs = [*MODEL.glob('*'), *PROOF.glob('*'), ROOT / 'crates/raft/src/rawnode.rs',
              ROOT / 'crates/raft/src/driver.rs', ROOT / 'crates/raft/src/driver/owner_read_tests.rs',
              ROOT / 'crates/raft/src/async_read.rs', ROOT / 'crates/raft/src/work.rs',
              ROOT / 'docs/OWNER-READ-PUMP.md', Path(__file__), ROOT / 'scripts/check-tla.py', ROOT / 'scripts/check-tlaps.py', ROOT / 'scripts/ProofAudit.java']
    args.output.mkdir()
    args.classes = args.output / 'auditor'
    args.classes.mkdir()
    shutil.copyfile(ROOT / 'scripts/ProofAudit.java', args.classes / 'ProofAudit.java')
    subprocess.run(['javac', '-cp', str(args.jar), '-d', str(args.classes), str(args.classes / 'ProofAudit.java')],
                   check=True, timeout=30)
    result = dict(accepted=False,
        scope='Local same-owner transaction sequencing and external notification retention; no quorum, Rust/Raft composition or liveness claim',
        sources={str(p.relative_to(ROOT)): digest(p) for p in inputs},
        jar_sha256=digest(args.jar), tlapm_sha256=digest(args.tlapm), records=[])
    save(args.output / 'summary.json', result)
    model = (MODEL / 'OwnerReadPump.tla').read_text()
    source = (PROOF / 'OwnerReadPumpProof.tla').read_text()
    try:
        for filename in ['Owner.cfg']:
            config = (MODEL / filename).read_text()
            pair = [case(args, Path(filename).stem + '-fp' + str(fp), model, source, config, fp=fp) for fp in [0, 1]]
            require(pair[0]['statistics'] == pair[1]['statistics'], 'fingerprint exploration differs')
            result['records'].extend(pair)
        result['records'].append(case(args, 'proof-baseline', model, source))
        controls = [
            ('skip-pump', "orPumped' = TRUE", "orPumped' = FALSE", 'ORInvariant', 'ORPumpStep'),
            ('consume-notification',
             "    /\\ UNCHANGED <<orAdmitted, orExternal, orPending>>",
             "    /\\ orPending' = FALSE\n    /\\ UNCHANGED <<orAdmitted, orExternal>>",
             'ORInvariant', 'ORPumpStep'),
        ]
        config = (MODEL / 'Owner.cfg').read_text()
        for name, before, after, property_name, theorem in controls:
            mutant = proof.replace_once(model, before, after)
            for kind in ['model', 'proof']:
                triple = []
                for phase, current in [('baseline', model), ('mutant', mutant), ('restored', model)]:
                    bad = phase == 'mutant'
                    row = case(args, name + '-' + kind + '-' + phase, current, source,
                        config if kind == 'model' else None,
                        expected=(property_name if kind == 'model' else 'OwnerReadPumpProof: TLAPS exit 10') if bad else None,
                        theorem=theorem if bad and kind == 'proof' else None)
                    triple.append(row)
                    result['records'].append(row)
                require(triple[0]['sources'] == triple[2]['sources'], 'restored source differs')
                require([p for p, h in triple[0]['sources'].items() if triple[1]['sources'][p] != h] == ['OwnerReadPump.tla'],
                        'semantic control changed unrelated inputs')
                if kind == 'model':
                    require(triple[0]['statistics'] == triple[2]['statistics'], 'restored exploration differs')
        for name, mutant, rejection in [
            ('proof-hole', proof.replace_once(source, 'BY SMT DEF ORAbort, ORInvariant, ORType, OROrder, ORNoLostNotification', 'OMITTED'),
             'proof hole: OwnerReadPumpProof.ORAbortStep'),
            ('custom-axiom', proof.replace_once(source, 'EXTENDS OwnerReadPump, TLAPS', 'EXTENDS OwnerReadPump, TLAPS\nAXIOM FALSE'),
             'unapproved module assumption: OwnerReadPumpProof')]:
            triple = [case(args, name + '-' + phase, model, current,
                expected=rejection if phase == 'mutant' else None)
                for phase, current in [('baseline', source), ('mutant', mutant), ('restored', source)]]
            require(triple[0]['sources'] == triple[2]['sources'], 'proof restoration differs')
            require([p for p, h in triple[0]['sources'].items() if triple[1]['sources'][p] != h] == ['OwnerReadPumpProof.tla'],
                    'proof control changed unrelated inputs')
            result['records'].extend(triple)
        require(len(result['records']) == 21 and all(digest(ROOT / p) == h for p, h in result['sources'].items()),
                'case inventory/source changed')
        result['accepted'] = True
    finally:
        save(args.output / 'summary.json', result)
    print('PASS: owner read transaction, two semantic controls and two proof-audit controls', flush=True)


if __name__ == '__main__':
    main()
