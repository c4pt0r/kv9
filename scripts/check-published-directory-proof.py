#!/usr/bin/env python3
"""Audit the source-bound, conditional directory-publication refinement."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import time
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / 'proofs/tlaps/published_directory'
MODULE = 'PublishedDirectoryProof'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    with path.open('x') as stream:
        stream.write(json.dumps(value, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--tlapm', type=Path, required=True)
    parser.add_argument('--jar', type=Path, required=True)
    args = parser.parse_args()
    if not __debug__:
        raise RuntimeError('assertions must be enabled')
    out = args.output.resolve()
    assert not out.is_relative_to(ROOT)
    out.mkdir()
    inventory = json.loads((SOURCE / 'inventory.json').read_text())
    contract = json.loads((SOURCE / 'source-contract.json').read_text())
    before = {name: sha(ROOT / name) for name in contract['files']}
    assert before == contract['files'], 'source differs from reviewed contract'
    assert sha(args.jar) == inventory['sany_jar_sha256']
    assert subprocess.check_output([str(args.tlapm), '--version'], text=True).strip() == inventory['tlapm_version']
    spec = importlib.util.spec_from_file_location('directory_audit', ROOT / 'scripts/check-tlaps.py')
    audit = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(audit)
    result = {'complete': False, 'scope': contract['scope'], 'source': before,
              'tlapm_sha256': sha(args.tlapm), 'jar_sha256': sha(args.jar),
              'script_sha256': sha(Path(__file__)), 'commands': [], 'controls': []}
    copied = out / 'source'
    copied.mkdir()
    for path in SOURCE.iterdir():
        shutil.copyfile(path, copied / path.name)
    for name in before:
        path = out / 'bound-source' / name
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / name, path)

    def run(work, label, command):
        row = {'argv': command, 'cwd': str(work), 'started_ns': time.time_ns()}
        result['commands'].append(row)
        with (work / (label + '.log')).open('x') as log:
            process = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT, timeout=120)
        row.update(exit_code=process.returncode, ended_ns=time.time_ns())
        save(work / (label + '-terminal.json'), row)
        return process.returncode, (work / (label + '.log')).read_text()

    def case(name, old=None, new=None):
        work = out / name
        work.mkdir()
        text = (SOURCE / (MODULE + '.tla')).read_text()
        if old is not None:
            text = audit.replace_once(text, old, new)
        (work / (MODULE + '.tla')).write_text(text)
        return work

    try:
        classes = out / 'auditor'
        classes.mkdir()
        code, _ = run(out, 'compile-auditor', ['javac', '-cp', str(args.jar), '-d', str(classes), str(ROOT / 'scripts/ProofAudit.java')])
        assert code == 0, 'semantic auditor did not compile'
        tools = SimpleNamespace(jar=args.jar, classes=classes, stdlib=args.tlapm.resolve().parent.parent / 'lib/tlapm/stdlib')
        work = case('theorems')
        audit.audit(tools, work, inventory)
        code, output = run(work, 'proof', [str(args.tlapm), '--strict', '--nofp', '--threads', '1', '--cache-dir', str(work / 'fresh-cache'), str(work / (MODULE + '.tla'))])
        audit.proof_verdict(output, code, MODULE, 24)
        controls = audit.output_controls(output, MODULE, 24)
        result.update(theorems=12, obligations=24, output_rejection_controls=controls)
        for name, old, new, expected in [
            ('proof-hole', 'BY SMT DEF ParentSync, FullSync', 'PROOF OMITTED', 'proof hole: PublishedDirectoryProof.SuccessorEquivalent'),
            ('false-axiom', 'EXTENDS Naturals, TLAPS', 'EXTENDS Naturals, TLAPS\nAXIOM FalsePremise == FALSE', 'unapproved module assumption: PublishedDirectoryProof'),
        ]:
            work = case(name, old, new)
            try:
                audit.audit(tools, work, inventory)
            except audit.Rejected as error:
                assert str(error) == expected, str(error)
                result['controls'].append({'name': name, 'rejection': str(error)})
            else:
                raise AssertionError('invalid proof accepted: ' + name)
        assert {name: sha(ROOT / name) for name in before} == before
        result.update(complete=True, source_unchanged=True)
        print('PASS: 12 conditional statements, 24 fresh obligations, semantic and output controls', flush=True)
    except BaseException as error:
        result['failure'] = repr(error)
        raise
    finally:
        result['ended_ns'] = time.time_ns()
        save(out / 'result.json', result)


if __name__ == '__main__':
    main()
