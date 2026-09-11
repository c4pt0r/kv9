#!/usr/bin/env python3
"""Check credit admission's bounded models and parameterized safety projection.

The existing SANY dependency/assumption audit and strict fresh TLAPS verifier
remain authoritative. Every mutation retains normal, defective and restored
inputs. These checks do not establish grouped-read/Rust/Ready refinement.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
MODEL = ROOT / 'proofs/tla/read-credit'
PROOF = ROOT / 'proofs/tlaps/read-credit'
INPUTS = [
    'proofs/tla/read-admission/ReadAdmission.tla',
    'proofs/tlaps/read-admission/ReadAdmissionProof.tla',
    'proofs/tla/read-credit/ReadCredit.tla',
    'proofs/tla/read-credit/ReadCreditMC.tla',
    'proofs/tlaps/read-credit/ReadCreditProof.tla',
    'proofs/tlaps/read-credit/inventory.json',
]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


tlc = module('credit_tlc', 'check-tla.py')
proof = module('credit_proof', 'check-tlaps.py')
require, digest = tlc.require, tlc.digest


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def proof_location(source, theorem, marker):
    lines = source.splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith('THEOREM ' + theorem + ' =='))
    end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith('THEOREM ')), len(lines))
    matches = [i + 1 for i in range(start + 1, end) if marker in lines[i]]
    require(len(matches) == 1, 'intended proof obligation location is not unique')
    return matches[0] + (1 if marker == '<1>1.' else 0)


def case(args, name, model, config=None, expected=None, location=None, fp=0,
         coverage=False, temporal=False, proof_source=None):
    work = args.output / name
    work.mkdir()
    for relative in INPUTS:
        shutil.copyfile(ROOT / relative, work / Path(relative).name)
    (work / 'ReadCredit.tla').write_text(model)
    if proof_source is not None:
        (work / 'ReadCreditProof.tla').write_text(proof_source)
    if config is not None:
        (work / 'Run.cfg').write_text(config)
    record = dict(name=name, expected=expected, fingerprint=fp,
                  sources={p.name: digest(p) for p in sorted(work.iterdir())})
    try:
        if config is None:
            try:
                proof.check_tree(args, work, record)
            except proof.Rejected as error:
                require(expected is not None and str(error) == expected,
                        f'{name}: unexpected proof rejection: {error}')
                if location is not None:
                    output = (work / 'ReadCreditProof.log').read_text()
                    require(re.search(rf'line {location}, characters? [^\n]+:\n\[ERROR\]: Could not prove or check:', output),
                            f'{name}: intended obligation did not fail')
                    counts = re.findall(r'\[ERROR\]: (\d+)/(\d+) obligations failed\.', output)
                    require(len(counts) == 1 and 0 < int(counts[0][0]) < int(counts[0][1]),
                            'missing nontrivial proof failure counts')
                    require('[WARNING]' not in output, 'unexpected proof warning')
                record['verdict'] = 'expected rejection'
            else:
                require(expected is None, 'invalid proof accepted')
                record['verdict'] = 'proved'
        else:
            with tempfile.TemporaryDirectory(prefix='kv9-credit-tlc-') as states:
                command = ['java', '-XX:+UseParallelGC', '-Xmx512m', '-cp', str(args.jar),
                           'tlc2.TLC', '-tool', '-workers', '1', '-fp', str(fp),
                           '-metadir', states, '-config', 'Run.cfg']
                if coverage:
                    command += ['-coverage', '999']
                command += ['ReadCreditMC.tla']
                record['command'] = command
                with (work / 'tlc.log').open('w') as log:
                    result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                                            timeout=args.timeout, env=dict(os.environ, LC_ALL='C'))
            output = (work / 'tlc.log').read_text()
            record['exit_code'] = result.returncode
            record['statistics'] = tlc.verdict(
                output, result.returncode,
                'EventuallyDrained' if expected == 'RCMCAdmission' else expected,
                temporal=temporal, coverage=coverage, module='ReadCredit',
                actions=('RCPoll', 'RCReadStep', 'RCOtherAdmission', 'RCRelease', 'RCCancel'),
                action_property=expected in ('RCAdmissionEffects', 'RCCancellationEffects'),
                minimum_distinct=2 if expected else 3)
            if coverage:
                hits = re.findall(r'^<RCMCReset line [^\n]+ of module ReadCreditMC>: (\d+):(\d+)$', output, re.M)
                require(len(hits) == 1 and int(hits[0][1]) > 0, 'missing bounded reset action coverage')
            record['verdict'] = 'accepted'
    except Exception as error:
        record.update(verdict='rejected', error=repr(error))
        raise
    finally:
        save(work / 'result.json', record)
    print('PASS:', name, record['verdict'], flush=True)
    return record


def triple(records, changed):
    before, mutant, restored = records
    require(before['sources'] == restored['sources'], 'restoration differs')
    require([name for name, sha in before['sources'].items() if mutant['sources'][name] != sha] == [changed],
            'mutation changed unrelated inputs')
    if 'statistics' in before:
        require(before['statistics'] == restored['statistics'], 'restored exploration differs')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jar', required=True, type=Path)
    parser.add_argument('--tlapm', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--timeout', type=int, default=120)
    args = parser.parse_args()
    args.jar, args.tlapm, args.output = args.jar.resolve(), args.tlapm.resolve(), args.output.resolve()
    args.stdlib = args.tlapm.parent.parent / 'lib/tlapm/stdlib'
    require(args.timeout > 0 and not args.output.exists(), 'positive timeout and fresh output required')
    inventory = json.loads((PROOF / 'inventory.json').read_text())
    require(digest(args.jar) == inventory['sany_jar_sha256'] == tlc.JAR_SHA256, 'TLC/SANY hash mismatch')
    require(subprocess.check_output([str(args.tlapm), '--version'], text=True).strip() == inventory['tlapm_version'],
            'TLAPS version mismatch')
    args.output.mkdir(parents=True)
    args.classes = args.output / 'auditor'
    args.classes.mkdir()
    shutil.copyfile(ROOT / 'scripts/ProofAudit.java', args.classes / 'ProofAudit.java')
    subprocess.run(['javac', '-cp', str(args.jar), '-d', str(args.classes), str(args.classes / 'ProofAudit.java')],
                   check=True, timeout=30)
    bound = INPUTS + ['scripts/check-read-credit-protocol.py', 'scripts/check-tla.py', 'scripts/check-tlaps.py',
                     'scripts/ProofAudit.java'] + [str(p.relative_to(ROOT)) for p in sorted(MODEL.glob('*.cfg'))]
    summary = dict(accepted=False, scope='Single-invocation credit abstraction and conditional admission; not whole-system refinement',
                   sources={p: digest(ROOT / p) for p in bound}, jar_sha256=digest(args.jar),
                   tlapm_sha256=digest(args.tlapm), records=[])
    save(args.output / 'summary.json', summary)
    model = (MODEL / 'ReadCredit.tla').read_text()
    source = (PROOF / 'ReadCreditProof.tla').read_text()
    try:
        for filename in ('Credit2.cfg', 'Credit3.cfg', 'CreditProgress.cfg'):
            config = (MODEL / filename).read_text()
            temporal = filename == 'CreditProgress.cfg'
            pair = [case(args, f'{Path(filename).stem}-fp{fp}', model, config, fp=fp,
                         coverage=not temporal, temporal=temporal) for fp in (0, 1)]
            require(pair[0]['statistics'] == pair[1]['statistics'], 'fingerprint exploration differs')
            summary['records'].extend(pair)
        summary['records'].append(case(args, 'proof-baseline', model))
        controls = [
            ('bypass-credit', '~raLeader \\/ raCommitTerm # raTerm \\/ ~rcOccupied',
             '~raLeader \\/ raCommitTerm # raTerm \\/ TRUE',
             'Credit2.cfg', 'RCAdmissionEffects', 'RCAdmissionCreditProof', 'BY SMT'),
            ('cancel-refunds-credit', '            /\\ UNCHANGED <<raVars, rcOccupied>>',
             '            /\\ rcOccupied\' = FALSE /\\ UNCHANGED raVars',
             'Credit2.cfg', 'RCCancellationEffects', 'RCCancellationKeepsCredit', 'BY SMT'),
            ('missing-release-fairness', 'WF_rcVars(RCRelease) /\\ WF_rcVars(RCPoll)',
             'WF_rcVars(RCPoll)',
             'CreditProgress.cfg', 'RCMCAdmission', 'RCReleaseProgress', '<1> QED'),
            ('competing-refill', 'RCProgressNext == RCPoll \\/ RCRelease',
             'RCProgressNext == RCPoll \\/ RCRelease \\/ RCOtherAdmission',
             'CreditProgress.cfg', 'RCMCAdmission', 'RCAdmissionProgress', '<1>1.'),
        ]
        for name, before, after, filename, violation, theorem, marker in controls:
            mutant = tlc.replace_once(model, before, after)
            config = (MODEL / filename).read_text()
            for kind in ('model', 'proof'):
                records = []
                for phase, current in (('baseline', model), ('mutant', mutant), ('restored', model)):
                    bad = phase == 'mutant'
                    record = case(args, f'{name}-{kind}-{phase}', current,
                                  config if kind == 'model' else None,
                                  expected=(violation if kind == 'model' else 'ReadCreditProof: TLAPS exit 10') if bad else None,
                                  location=proof_location(source, theorem, marker) if bad and kind == 'proof' else None,
                                  temporal=filename == 'CreditProgress.cfg')
                    records.append(record)
                triple(records, 'ReadCredit.tla')
                summary['records'].extend(records)
            save(args.output / 'summary.json', summary)
        for name, bad, expected in (
            ('proof-hole', tlc.replace_once(source, 'BY SMT DEF RCCancel', 'OMITTED'), 'proof hole: ReadCreditProof.RCCancellationKeepsCredit'),
            ('custom-axiom', tlc.replace_once(source, 'EXTENDS ReadCredit, ReadAdmissionProof, TLAPS',
                                            'EXTENDS ReadCredit, ReadAdmissionProof, TLAPS\nAXIOM FALSE'),
             'unapproved module assumption: ReadCreditProof')):
            records = [case(args, f'{name}-{phase}', model, proof_source=current,
                            expected=expected if phase == 'mutant' else None)
                       for phase, current in (('baseline', source), ('mutant', bad), ('restored', source))]
            triple(records, 'ReadCreditProof.tla')
            summary['records'].extend(records)
        require(all(digest(ROOT / p) == sha for p, sha in summary['sources'].items()), 'source changed during checks')
        summary['accepted'] = True
    finally:
        save(args.output / 'summary.json', summary)
    print('PASS: credit models, safety projection, conditional progress, four semantic controls and two audit controls', flush=True)


if __name__ == '__main__':
    main()
