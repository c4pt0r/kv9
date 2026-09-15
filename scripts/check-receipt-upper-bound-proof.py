#!/usr/bin/env python3
"""Check the source-bound receipt upper-bound refinement and its fault controls."""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'proofs/tlaps/receipt-upper-bound'
RUNTIME_PATHS = ['crates', 'src', 'proto', '.cargo', 'Cargo.toml', 'Cargo.lock',
                 'rust-toolchain.toml']


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
    changed = subprocess.check_output(
        ['git', 'diff', '--name-only', contract['parent'], '--', *RUNTIME_PATHS],
        cwd=ROOT, text=True).splitlines()
    untracked = subprocess.check_output(
        ['git', 'ls-files', '--others', '--exclude-standard', '--', *RUNTIME_PATHS],
        cwd=ROOT, text=True).splitlines()
    assert sorted(set(changed + untracked)) == contract['changed_runtime_files'], \
        'unbound runtime change'
    driver = (ROOT / 'crates/raft/src/driver.rs').read_text()
    for candidate, original in contract['reverse_driver_edits']:
        assert candidate != original and driver.count(candidate) == 1, \
            'driver mapping is not unique'
        driver = driver.replace(candidate, original)
    parent = subprocess.check_output(
        ['git', 'show', contract['parent'] + ':crates/raft/src/driver.rs'], cwd=ROOT)
    assert driver.encode() == parent, 'surrounding driver semantics changed'
    return {'files': contract['files'], 'parent': contract['parent'],
            'changed_runtime_files': contract['changed_runtime_files'],
            'surrounding_driver_byte_identical': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--tlapm', type=Path, required=True)
    parser.add_argument('--jar', type=Path, required=True)
    args = parser.parse_args()
    assert __debug__, 'assertions must be enabled'
    args.output, args.tlapm, args.jar = (p.resolve() for p in
                                       (args.output, args.tlapm, args.jar))
    out = args.output
    assert not out.is_relative_to(ROOT), 'output must be outside source'
    out.mkdir()
    contract = json.loads((SOURCE / 'source-contract.json').read_text())
    before = source_binding(contract)
    inventory = json.loads((SOURCE / 'inventory.json').read_text())
    assert sha(args.jar) == inventory['sany_jar_sha256'], 'untrusted SANY/TLC jar'
    assert subprocess.check_output([str(args.tlapm), '--version'], text=True,
                                   timeout=15).strip() == inventory['tlapm_version']
    audit = load('upper_bound_tlaps', ROOT / 'scripts/check-tlaps.py')
    tlc = load('upper_bound_tlc', ROOT / 'scripts/check-tla.py')
    result = {'complete': False, 'source_binding': before, 'commands': [], 'cases': [],
              'scope': 'Receipt representation and complete first-match lookup refinement; '
                       'not whole Rust/Raft, liveness or performance acceptance.'}
    snapshot = out / 'source'
    snapshot.mkdir()
    for p in SOURCE.iterdir():
        assert p.is_file(), 'unexpected proof source directory'
        shutil.copyfile(p, snapshot / p.name)
    for name in contract['files']:
        dst = out / 'bound-source' / name
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / name, dst)
    save(out / 'invocation.json', {'argv': os.sys.argv, 'started_ns': time.time_ns(),
         'revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT,
                                             text=True).strip(),
         'script_sha256': sha(Path(__file__)), 'tlapm_sha256': sha(args.tlapm),
         'jar_sha256': sha(args.jar), 'contract_sha256': sha(SOURCE / 'source-contract.json')})

    def run(work, label, argv, timeout=180):
        row = {'label': label, 'argv': argv, 'cwd': str(work), 'started_ns': time.time_ns()}
        result['commands'].append(row)
        with (work / (label + '.log')).open('x') as log:
            process = subprocess.Popen(argv, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                                       start_new_session=True)
            row['pid'] = process.pid
            try:
                row['exit_code'] = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                row['timed_out'] = True
                raise RuntimeError(label + ' timed out; original output retained') from None
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                process.wait()
                row.update(exit_code=process.returncode, ended_ns=time.time_ns(),
                           pid_absent=not Path('/proc', str(process.pid)).exists())
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
        code, _ = run(out, 'compile-auditor', ['javac', '-cp', str(args.jar), '-d',
                    str(classes), str(ROOT / 'scripts/ProofAudit.java')])
        assert code == 0
        tool_args = SimpleNamespace(jar=args.jar, classes=classes,
            stdlib=args.tlapm.parent.parent / 'lib/tlapm/stdlib')
        work = case('theorems')
        audit.audit(tool_args, work, inventory)
        rejection_controls = 0
        for module, item in inventory['modules'].items():
            code, output = run(work, module, [str(args.tlapm), '--strict', '--nofp',
                '--threads', '1', '--cache-dir', str(work / ('cache-' + module)),
                str(work / (module + '.tla'))])
            audit.proof_verdict(output, code, module, item['obligations'])
            rejection_controls += audit.output_controls(output, module, item['obligations'])
        statements = sum(len(item['theorems']) for item in inventory['modules'].values())
        obligations = sum(item['obligations'] for item in inventory['modules'].values())
        result['cases'].append({'theorems': statements, 'obligations': obligations,
                                'output_rejection_controls': rejection_controls})
        print(f'PASS: {statements} theorem statements, {obligations} fresh obligations', flush=True)

        finite_cases = [
            ('capacity-1', None, 1, None),
            ('capacity-2', None, 2, None),
            ('decreasing-bound', ('ReceiptUpperBound.tla',
             'UBAdvance(b, e) == IF e.index > b THEN e.index ELSE b',
             'UBAdvance(b, e) == e.index'), 2, 'UBLookupRefinement'),
            ('non-strict-guard', ('ReceiptUpperBound.tla',
             'UBLookup(s, b, key) == IF key > b',
             'UBLookup(s, b, key) == IF key >= b'), 2, 'UBLookupRefinement'),
        ]
        for name, mutation, capacity, expected in finite_cases:
            work = case(name, mutation)
            cfg = (SOURCE / 'ReceiptUpperBound.cfg').read_text().replace(
                'UBCapacity = 2', 'UBCapacity = ' + str(capacity))
            if expected:
                # Require an actual wrong lookup, not merely an internal bound violation.
                cfg = audit.replace_once(cfg, 'INVARIANT UBInvariant\n', '')
            (work / 'Run.cfg').write_text(cfg)
            code, output = run(work, 'tlc', ['java', '-Xmx512m', '-cp', str(args.jar),
                'tlc2.TLC', '-tool', '-workers', '1', '-fp', '0', '-metadir',
                str(work / 'states'), '-config', 'Run.cfg', 'ReceiptUpperBound.tla'])
            stats = tlc.verdict(output, code, expected, module='ReceiptUpperBound',
                                minimum_distinct=3 if expected is None else 2)
            result['cases'].append({'name': name, 'expected_violation': expected,
                                    'statistics': stats})
            print('PASS: ' + name, flush=True)

        controls = [
            ('proof-hole', ('ReceiptUpperBoundProof.tla',
             'BY UBInvariantAlways, UBLookupObservation, PTL', 'PROOF OMITTED'),
             'proof hole: ReceiptUpperBoundProof.UBLookupAlways'),
            ('false-axiom', ('ReceiptUpperBound.tla', 'EXTENDS Integers, Sequences',
             'EXTENDS Integers, Sequences\nAXIOM Wrong == FALSE'),
             'unapproved module assumption: ReceiptUpperBound'),
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
        assert sha(SOURCE / 'source-contract.json') == sha(snapshot / 'source-contract.json')
        result.update(complete=True, source_unchanged=True)
    except BaseException as error:
        result['failure'] = repr(error)
        raise
    finally:
        result['ended_ns'] = time.time_ns()
        save(out / 'result.json', result)


if __name__ == '__main__':
    main()
