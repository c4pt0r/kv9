#!/usr/bin/env python3
"""Check pending-edge wake refinement and its unchanged scheduling dependency."""
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
MODELS = ROOT / 'proofs/tla/coalesced_wake'
PROOFS = ROOT / 'proofs/tlaps/coalesced_wake'


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


proof = load('wake_proof_gate', 'check-tlaps.py')
tlc = load('wake_model_gate', 'check-tla.py')
require = proof.require


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def copy_sources(work, inventory, include_proof=False):
    work.mkdir()
    for name, item in inventory['models'].items():
        shutil.copyfile(ROOT / item['source'], work / (name + '.tla'))
    if include_proof:
        for name, source in [('RaftScheduleProof', ROOT / 'proofs/tlaps/raft_schedule'),
                             ('CoalescedWakeProof', PROOFS)]:
            shutil.copyfile(source / (name + '.tla'), work / (name + '.tla'))
        save(work / 'inventory.json', inventory)
    else:
        shutil.copyfile(MODELS / 'CoalescedWakeMC.tla', work / 'CoalescedWakeMC.tla')


def model_case(args, name, config, *, fp=0, temporal=False, mutant=False):
    work = args.output / name
    copy_sources(work, args.inventory)
    (work / 'Run.cfg').write_text(config)
    if mutant:
        path = work / 'CoalescedWake.tla'
        path.write_text(proof.replace_once(path.read_text(),
            'CWDelivered == rsWake \\/ (~rsStopped /\\ ~rsPending /\\ rsPhase = "park")',
            'CWDelivered == rsWake'))
    record = dict(name=name, fingerprint=fp, temporal=temporal, mutant=mutant,
                  sources={p.name: proof.sha(p) for p in sorted(work.iterdir())})
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix='kv9-coalesced-wake-states.') as states:
            command = ['java', f'-Djava.io.tmpdir={states}', '-XX:+UseParallelGC', '-Xmx512m',
                       '-cp', str(args.jar), 'tlc2.TLC', '-tool', '-workers', '1',
                       '-fp', str(fp), '-lncheck', 'final', '-metadir', states,
                       '-coverage', '999', '-config', 'Run.cfg', 'CoalescedWakeMC.tla']
            record['command'] = command
            with (work / 'tlc.log').open('w') as log:
                result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                                        timeout=args.timeout, env=dict(os.environ, LC_ALL='C'))
        output = (work / 'tlc.log').read_text()
        record['exit_code'] = result.returncode
        record['statistics'] = tlc.verdict(output, result.returncode,
            'RSParkedSignal' if mutant else None, temporal=temporal, coverage=not mutant,
            module='CoalescedWake', actions=('CWNotify', 'CWHint'),
            minimum_distinct=2 if mutant else 3)
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
            output = (work / 'CoalescedWakeProof.log').read_text()
            require(re.search(r'PROVE\s+CWDelivered =', output) is not None,
                    'wake mutation did not fail the intended equivalence')
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
    require(set(args.inventory['modules']) == {'RaftScheduleProof', 'CoalescedWakeProof'}, 'proof closure mismatch')
    require(set(args.inventory['models']) == {'RaftSchedule', 'CoalescedWake'}, 'model closure mismatch')
    args.output.mkdir(parents=True)
    args.classes = args.output / 'auditor'
    args.classes.mkdir()
    shutil.copyfile(ROOT / 'scripts/ProofAudit.java', args.classes / 'ProofAudit.java')
    subprocess.run(['javac', '-cp', str(args.jar), '-d', str(args.classes),
                    str(args.classes / 'ProofAudit.java')], check=True, timeout=30)
    records = []
    for config in ('Schedule2.cfg', 'Schedule3.cfg'):
        pair = [model_case(args, f'{Path(config).stem}-fp{fp}', (MODELS / config).read_text(), fp=fp)
                for fp in (0, 1)]
        require(pair[0]['statistics'] == pair[1]['statistics'], 'fingerprint exploration mismatch')
        records.extend(pair)
    records.append(model_case(args, 'service', (MODELS / 'ScheduleService.cfg').read_text(), temporal=True))
    control = '\n'.join(line for line in (MODELS / 'Schedule2.cfg').read_text().splitlines()
                        if not line.startswith(('INVARIANT', 'PROPERTY'))) + '\nINVARIANT RSParkedSignal\n'
    triple = [model_case(args, 'first-wake-' + stage, control, mutant=mutant)
              for stage, mutant in [('before', False), ('mutant', True), ('restored', False)]]
    require(triple[0]['sources'] == triple[2]['sources'] and
            triple[0]['statistics'] == triple[2]['statistics'], 'restored model mismatch')
    records.extend(triple)
    baseline = proof_case(args, 'proof')
    records.append(baseline)
    records.append(proof_case(args, 'proof-missing-first-wake', ('CoalescedWake.tla',
        'CWDelivered == rsWake \\/ (~rsStopped /\\ ~rsPending /\\ rsPhase = "park")',
        'CWDelivered == rsWake'), 'CoalescedWakeProof: TLAPS exit 10'))
    records.append(proof_case(args, 'proof-hole', ('CoalescedWakeProof.tla',
        'BY SMT DEF CWDelivered, RSParkedSignal', 'OMITTED'),
        'proof hole: CoalescedWakeProof.CWWakeEquivalent'))
    records.append(proof_case(args, 'proof-assumption', ('CoalescedWake.tla',
        'EXTENDS RaftSchedule', 'EXTENDS RaftSchedule\nASSUME FALSE'),
        'unapproved module assumption: CoalescedWake'))
    output_controls = proof.output_controls((args.output / 'proof/CoalescedWakeProof.log').read_text(),
                                           'CoalescedWakeProof', 64)
    save(args.output / 'summary.json', dict(complete=True, records=records,
        distinct_theorems=47, baseline_obligations=358, new_theorems=14, new_obligations=64,
        proof_output_controls=output_controls,
        scope='Pending-edge wake refinement; finite models plus parameterized scheduling dependency. '
              'Conditional service fairness is inherited, not an OS scheduling bound or full database proof.'))
    print('PASS: coalesced wake refinement; 14 new theorems / 64 obligations; dependency 33 / 294', flush=True)


if __name__ == '__main__':
    main()
