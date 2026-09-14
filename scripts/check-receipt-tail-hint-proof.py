#!/usr/bin/env python3
"""Check source-bound receipt refinement, finite controls and semantic auditing."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'proofs/tlaps/receipt-tail-hint'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def save(path, value):
    with path.open('x') as stream:
        stream.write(json.dumps(value, indent=2) + '\n')


def source_binding(contract):
    for name, expected in contract['files'].items():
        assert sha(ROOT / name) == expected, 'source contract differs: ' + name
    text = (ROOT / 'crates/raft/src/driver.rs').read_text()
    for old, new in contract['reverse_driver_edits']:
        assert text.count(old) == 1, 'driver mapping is not unique'
        text = text.replace(old, new)
    parent = subprocess.check_output(
        ['git', 'show', contract['parent'] + ':crates/raft/src/driver.rs'], cwd=ROOT)
    assert text.encode() == parent, 'surrounding driver semantics changed'
    return {'files': contract['files'], 'parent': contract['parent'],
            'surrounding_driver_byte_identical': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--tlapm', type=Path, required=True)
    parser.add_argument('--jar', type=Path, required=True)
    args = parser.parse_args()
    assert __debug__, 'assertions must be enabled'
    out = args.output.resolve()
    assert not out.is_relative_to(ROOT), 'output must be outside source'
    out.mkdir()
    contract = json.loads((SOURCE / 'source-contract.json').read_text())
    before = source_binding(contract)
    inventory = json.loads((SOURCE / 'inventory.json').read_text())
    assert sha(args.jar) == inventory['sany_jar_sha256']
    assert subprocess.check_output([str(args.tlapm), '--version'], text=True).strip() == inventory['tlapm_version']
    audit = load('receipt_proof_audit', ROOT / 'scripts/check-tlaps.py')
    tlc = load('receipt_tlc_audit', ROOT / 'scripts/check-tla.py')
    result = {'complete': False, 'source_binding': before, 'commands': [], 'cases': [],
              'scope': 'Conditional receipt representation and lookup refinement; not whole Rust/Raft or performance acceptance.'}
    original_sources = out / 'source'
    original_sources.mkdir()
    for p in SOURCE.iterdir():
        shutil.copyfile(p, original_sources / p.name)
    for name in contract['files']:
        dst = out / 'bound-source' / name
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / name, dst)
    save(out / 'invocation.json', {'argv': os.sys.argv, 'started_ns': time.time_ns(),
         'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
         'script_sha256': sha(Path(__file__)), 'tlapm_sha256': sha(args.tlapm),
         'jar_sha256': sha(args.jar)})

    def run(work, label, argv, timeout=180):
        row = {'label': label, 'argv': argv, 'cwd': str(work), 'started_ns': time.time_ns()}
        result['commands'].append(row)
        with (work / (label + '.log')).open('x') as log:
            process = subprocess.Popen(argv, cwd=work, stdout=log, stderr=subprocess.STDOUT)
            row['pid'] = process.pid
            try:
                row['exit_code'] = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
                row['exit_code'] = process.returncode
                row['timed_out'] = True
                raise RuntimeError(label + ' timed out; original output retained') from None
            finally:
                row['ended_ns'] = time.time_ns()
                row['pid_absent'] = not Path('/proc', str(process.pid)).exists()
                save(work / (label + '-terminal.json'), row)
        return row['exit_code'], (work / (label + '.log')).read_text()

    def case(name, mutation=None):
        work = out / name
        work.mkdir()
        for p in SOURCE.glob('*.tla'):
            shutil.copyfile(p, work / p.name)
        if mutation:
            filename, old, new = mutation
            p = work / filename
            p.write_text(audit.replace_once(p.read_text(), old, new))
        return work

    try:
        classes = out / 'auditor'
        classes.mkdir()
        code, _ = run(out, 'compile-auditor', ['javac', '-cp', str(args.jar), '-d', str(classes), str(ROOT / 'scripts/ProofAudit.java')])
        assert code == 0
        tool_args = SimpleNamespace(jar=args.jar, classes=classes, stdlib=args.tlapm.resolve().parent.parent / 'lib/tlapm/stdlib')
        work = case('theorems')
        audit.audit(tool_args, work, inventory)
        for module, item in inventory['modules'].items():
            code, output = run(work, module, [str(args.tlapm), '--strict', '--nofp', '--threads', '1',
                '--cache-dir', str(work / ('cache-' + module)), str(work / (module + '.tla'))])
            audit.proof_verdict(output, code, module, item['obligations'])
            audit.output_controls(output, module, item['obligations'])
        result['cases'].append({'theorems': 18, 'obligations': 158, 'output_rejection_controls': 6})
        print('PASS: 18 theorem statements, 158 fresh obligations', flush=True)

        finite_cases = [
            ('capacity-1', None, 1, None),
            ('capacity-2', None, 2, None),
            ('missing-index-guard', ('ReceiptTailHint.tla', 'THEN s[RTHPosition(s, key)].index = key', 'THEN TRUE'), 2, 'RTHRefinement'),
            ('ignored-certificate', ('ReceiptTailHint.tla', 'IF c THEN RTHOrdered(s, key) ELSE ARFirst(s, key)', 'RTHOrdered(s, key)'), 2, 'RTHRefinement'),
            ('restored', None, 2, None),
        ]
        for name, mutation, capacity, expected in finite_cases:
            work = case(name, mutation)
            cfg = (SOURCE / 'ReceiptTailHint.cfg').read_text().replace('ARCapacity = 2', 'ARCapacity = ' + str(capacity))
            (work / 'Run.cfg').write_text(cfg)
            code, output = run(work, 'tlc', ['java', '-Xmx512m', '-cp', str(args.jar), 'tlc2.TLC', '-tool',
                '-workers', '1', '-fp', '0', '-metadir', str(work / 'states'), '-config', 'Run.cfg', 'ReceiptTailHint.tla'])
            stats = tlc.verdict(output, code, expected, module='ReceiptTailHint', minimum_distinct=3)
            result['cases'].append({'name': name, 'expected_violation': expected, 'statistics': stats})
            print('PASS: ' + name, flush=True)

        controls = [
            ('proof-hole', ('ReceiptTailHintProof.tla', 'BY ARInvariantAlways, RTHLookupObservation, PTL', 'PROOF OMITTED'),
             'proof hole: ReceiptTailHintProof.RTHLookupAlways'),
            ('false-axiom', ('ReceiptTailHint.tla', 'EXTENDS AppliedReceipts', 'EXTENDS AppliedReceipts\nAXIOM Wrong == FALSE'),
             'unapproved module assumption: ReceiptTailHint'),
        ]
        for name, mutation, expected in controls:
            work = case(name, mutation)
            try:
                audit.audit(tool_args, work, inventory)
            except audit.Rejected as error:
                assert str(error) == expected, str(error)
            else:
                raise AssertionError('invalid proof accepted: ' + name)
            result['cases'].append({'name': name, 'semantic_rejection': expected})
            print('PASS: ' + name, flush=True)
        assert source_binding(contract) == before
        result.update(complete=True, source_unchanged=True)
    except BaseException as error:
        result['failure'] = repr(error)
        raise
    finally:
        result['ended_ns'] = time.time_ns()
        save(out / 'result.json', result)


if __name__ == '__main__':
    main()
