#!/usr/bin/env python3
"""Check bounded owner-poll scheduling refinement and explicit fault controls."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
MODELS = ROOT / 'proofs/tla/owner_poll'
PROOFS = ROOT / 'proofs/tlaps/owner_poll'


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


proof = load('owner_poll_proof_gate', 'check-tlaps.py')
tlc = load('owner_poll_model_gate', 'check-tla.py')
require = proof.require
GOOD_HINT = '''OPHintReady ==
    /\\ rsPhase = "wait" /\\ opLeft > 0 /\\ opHint
    /\\ opLeft' = 0
    /\\ UNCHANGED <<rsVars, opHint>>'''
BAD_HINT = '''OPHintReady ==
    /\\ rsPhase = "wait" /\\ opLeft > 0 /\\ opHint
    /\\ opLeft' = 0 /\\ rsPending' = FALSE
    /\\ UNCHANGED <<rsQueue, rsProducer, rsStopped, rsPhase,
                    rsRedo, rsWake, rsClaimed, opHint>>'''


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def copy_sources(work, inventory, include_proof=False):
    work.mkdir()
    for name, item in inventory['models'].items():
        shutil.copyfile(ROOT / item['source'], work / (name + '.tla'))
    if include_proof:
        for name, source in [('RaftScheduleProof', ROOT / 'proofs/tlaps/raft_schedule'),
                             ('OwnerPollProof', PROOFS)]:
            shutil.copyfile(source / (name + '.tla'), work / (name + '.tla'))
        save(work / 'inventory.json', inventory)
    else:
        shutil.copyfile(MODELS / 'OwnerPollMC.tla', work / 'OwnerPollMC.tla')


def model_case(args, name, config, *, fp=0, temporal=False, mutant=False):
    work = args.output / name
    copy_sources(work, args.inventory)
    (work / 'Run.cfg').write_text(config)
    if mutant:
        path = work / 'OwnerPoll.tla'
        path.write_text(proof.replace_once(path.read_text(), GOOD_HINT, BAD_HINT))
    record = dict(name=name, fingerprint=fp, temporal=temporal, mutant=mutant,
                  sources={p.name: proof.sha(p) for p in sorted(work.iterdir())})
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix='kv9-owner-poll-states.') as states:
            command = ['java', f'-Djava.io.tmpdir={states}', '-XX:+UseParallelGC', '-Xmx512m',
                       '-cp', str(args.jar), 'tlc2.TLC', '-tool', '-workers', '1',
                       '-fp', str(fp), '-lncheck', 'final', '-metadir', states,
                       '-coverage', '999', '-config', 'Run.cfg', 'OwnerPollMC.tla']
            record['command'] = command
            with (work / 'tlc.log').open('w') as log:
                result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                                        timeout=args.timeout, env=dict(os.environ, LC_ALL='C'))
        output = (work / 'tlc.log').read_text()
        record['exit_code'] = result.returncode
        record['statistics'] = tlc.verdict(output, result.returncode,
            'RSNoLostWake' if mutant else None, temporal=temporal, coverage=not mutant,
            module='OwnerPoll', actions=('OPSpin', 'OPHintReady'), minimum_distinct=3)
        record['verdict'] = 'expected rejection' if mutant else 'accepted'
    except BaseException as error:
        record.update(verdict='rejected', reason=repr(error))
        raise
    finally:
        record['seconds'] = time.monotonic() - started
        save(work / 'result.json', record)
    print(f"PASS: {name}: {record['verdict']}", flush=True)
    return record


def proof_case(args, name, mutation=None, expected=None):
    work = args.output / name
    copy_sources(work, args.inventory, include_proof=True)
    if mutation:
        filename, old, new = mutation
        path = work / filename
        path.write_text(proof.replace_once(path.read_text(), old, new))
    record = dict(name=name, sources={p.name: proof.sha(p) for p in sorted(work.iterdir())})
    try:
        proof.check_tree(args, work, record)
    except proof.Rejected as error:
        record.update(verdict='rejected', reason=str(error))
        require(expected is not None and str(error) == expected, f'{name}: unexpected rejection: {error}')
        if expected.endswith('TLAPS exit 10'):
            output = (work / 'OwnerPollProof.log').read_text()
            require(re.search(r'PROVE\s+OPLocalNext => UNCHANGED rsVars', output) is not None,
                    'hint mutation did not fail the intended stuttering proof')
            require(re.search(r'\[ERROR\]: [1-9]\d*/\d+ obligations failed\.', output) is not None,
                    'missing failed proof count')
        record['verdict'] = 'expected rejection'
    else:
        require(expected is None, f'{name}: invalid proof accepted')
        record['verdict'] = 'proved'
    finally:
        save(work / 'result.json', record)
    print(f"PASS: {name}: {record['verdict']}", flush=True)
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--jar', required=True, type=Path)
    parser.add_argument('--tlapm', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--timeout', type=int, default=180)
    args = parser.parse_args()
    args.jar, args.tlapm, args.output = args.jar.resolve(), args.tlapm.resolve(), args.output.resolve()
    args.stdlib = args.tlapm.parent.parent / 'lib/tlapm/stdlib'
    args.inventory = json.loads((PROOFS / 'inventory.json').read_text())
    require(args.timeout > 0 and not args.output.exists(), 'new output and positive timeout required')
    require(proof.sha(args.jar) == args.inventory['sany_jar_sha256'] == tlc.JAR_SHA256, 'SANY/TLC checksum mismatch')
    version = subprocess.check_output([str(args.tlapm), '--version'], text=True, timeout=15).strip()
    require(version == args.inventory['tlapm_version'], 'TLAPS version mismatch')
    require(set(args.inventory['modules']) == {'RaftScheduleProof', 'OwnerPollProof'}, 'proof closure mismatch')
    require(set(args.inventory['models']) == {'RaftSchedule', 'OwnerPoll'}, 'model closure mismatch')
    args.output.mkdir(parents=True)
    args.classes = args.output / 'auditor'
    args.classes.mkdir()
    shutil.copyfile(ROOT / 'scripts/ProofAudit.java', args.classes / 'ProofAudit.java')
    subprocess.run(['javac', '-cp', str(args.jar), '-d', str(args.classes),
                    str(args.classes / 'ProofAudit.java')], check=True, timeout=30)
    records = []
    summary = dict(complete=False, records=records)
    try:
        for config in ('Schedule2.cfg', 'Schedule3.cfg'):
            pair = [model_case(args, f'{Path(config).stem}-fp{fp}', (MODELS / config).read_text(), fp=fp)
                    for fp in (0, 1)]
            require(pair[0]['statistics'] == pair[1]['statistics'], 'fingerprint exploration mismatch')
            records.extend(pair)
        records.append(model_case(args, 'service', (MODELS / 'ScheduleService.cfg').read_text(), temporal=True))
        control = '\n'.join(line for line in (MODELS / 'Schedule2.cfg').read_text().splitlines()
                            if not line.startswith(('INVARIANT', 'PROPERTY'))) + '\nINVARIANT RSNoLostWake\n'
        records.append(model_case(args, 'bad-hint-clears-pending', control, mutant=True))
        records.append(proof_case(args, 'proof'))
        records.append(proof_case(args, 'proof-bad-hint', ('OwnerPoll.tla', GOOD_HINT, BAD_HINT),
                                  'OwnerPollProof: TLAPS exit 10'))
        records.append(proof_case(args, 'proof-hole', ('OwnerPollProof.tla',
            'BY SMT DEF OPLocalNext, OPSpin, OPHintReady', 'OMITTED'),
            'proof hole: OwnerPollProof.OPLocalStutters'))
        records.append(proof_case(args, 'proof-assumption', ('OwnerPoll.tla',
            'EXTENDS RaftSchedule', 'EXTENDS RaftSchedule\nASSUME FALSE'),
            'unapproved module assumption: OwnerPoll'))
        output_controls = proof.output_controls((args.output / 'proof/OwnerPollProof.log').read_text(),
                                               'OwnerPollProof', 49)
        summary.update(complete=True, new_theorems=14, new_obligations=49,
            dependency_theorems=33, dependency_obligations=294,
            proof_output_controls=output_controls,
            scope='Bounded optional polling refines the original scheduling actions. Advisory hints grant no authority. '
                  'Original service fairness and monotonic local time are operational assumptions; no clock-derived lease or OS time bound.')
    except BaseException as error:
        summary['failure'] = repr(error)
        raise
    finally:
        save(args.output / 'summary.json', summary)
    print('PASS: owner-poll refinement; 14 new theorems / 49 obligations; dependency 33 / 294', flush=True)


if __name__ == '__main__':
    main()
