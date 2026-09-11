"""Finite, source-bound acceptance for the local AppliedRing representation proof."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
MODULE = 'AppliedReceipts'
PROOF = 'AppliedReceiptsProof'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2)
        stream.write('\n')


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


def receipt_sequence(state, variable):
    match = re.search(r'/\\ ' + variable + r' = (.*?)(?=\n/\\ |\Z)', state, re.S)
    require(match is not None, 'counterexample lacks ' + variable)
    value = match.group(1).strip()
    require(value.startswith('<<') and value.endswith('>>'), 'unexpected sequence rendering')
    result = []
    for record in re.findall(r'\[(.*?)\]', value, re.S):
        fields = dict(re.findall(r'(index|term|outcome)\s*\|->\s*([^,\]\s]+)', record))
        require(set(fields) == {'index', 'term', 'outcome'}, 'incomplete receipt payload in trace')
        result.append((int(fields['index']), fields['term'], fields['outcome']))
    require(bool(result) or re.fullmatch(r'<<\s*>>', value) is not None,
            'unparsed receipt sequence')
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    require(not out.exists() and out.parent == HERE.parent and out != HERE,
            'output must be a fresh sibling directory')
    require(os.environ.get('PYTHONOPTIMIZE') == '0' and
            os.environ.get('PYTHONDONTWRITEBYTECODE') == '1', 'wrong Python environment')
    require(set(os.sched_getaffinity(0)) <= set(range(6, 16)) | set(range(22, 32)),
            'wrong background CPU affinity')
    frozen = json.loads((HERE / 'freeze.json').read_text())
    for name, entry in frozen['files'].items():
        require(sha(HERE / name) == entry['sha256'], 'frozen preparation changed: ' + name)
    for name, entry in frozen['source_bindings'].items():
        require(sha(Path(name)) == entry['sha256'], 'bound source changed: ' + name)
    out.mkdir()
    source = out / 'source'
    source.mkdir()
    for name in frozen['files']:
        target = source / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(HERE / name, target)
    shutil.copyfile(HERE / 'freeze.json', source / 'freeze.json')
    save(out / 'invocation.json', dict(argv=[str(Path(__file__).resolve()), '--output', str(out)],
                                     pid=os.getpid(), started_ns=time.time_ns(),
                                     allowed_cpus=sorted(os.sched_getaffinity(0)),
                                     freeze_sha256=sha(HERE / 'freeze.json')))
    records = []
    summary = dict(complete=False, representation_acceptance=False, records=records,
                   source_revision=frozen['source_revision'],
                   scope='Conditional local sequence/certificate/lookup refinement; not whole Rust, Raft, liveness or crash proof.')

    def run(work, label, argv, timeout):
        save(work / (label + '-invocation.json'), dict(argv=argv, cwd=str(work), started_ns=time.time_ns()))
        with (work / (label + '.log')).open('x') as log:
            child = subprocess.Popen(argv, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                                     env=dict(os.environ, LC_ALL='C'))
            save(work / (label + '-live.json'), dict(pid=child.pid, started_ns=time.time_ns()))
            try:
                code = child.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
                save(work / (label + '-terminal.json'), dict(exit_code=child.returncode,
                     terminal=True, reaped=True, timeout=True, ended_ns=time.time_ns()))
                raise RuntimeError(label + ' timeout; retained as inconclusive') from None
        save(work / (label + '-terminal.json'), dict(exit_code=code, terminal=True,
             reaped=True, pid=child.pid, pid_absent=not Path(f'/proc/{child.pid}').exists(),
             ended_ns=time.time_ns()))
        return code, (work / (label + '.log')).read_text()

    try:
        inventory = json.loads((source / 'inventory.json').read_text())
        require(len(inventory['modules'][PROOF]['theorems']) == 13 and
                inventory['modules'][PROOF]['obligations'] == 122, 'wrong reviewed proof count')
        jar = Path('/tmp/kv9-p0-tools/tla2tools-v1.7.4.jar')
        tlapm = Path('/tmp/kv9-p0-tools/tlapm-1.6.0-pre-20260731/bin/tlapm').resolve()
        require(sha(jar) == inventory['sany_jar_sha256'], 'TLC/SANY jar differs')
        require(subprocess.check_output([str(tlapm), '--version'], text=True).strip() ==
                inventory['tlapm_version'], 'TLAPM version differs')
        save(out / 'tools.json', dict(jar=str(jar), jar_sha256=sha(jar), tlapm=str(tlapm),
                                     tlapm_sha256=sha(tlapm), tlapm_version=inventory['tlapm_version']))
        classes = out / 'auditor'
        classes.mkdir()
        code, _ = run(out, 'compile-auditor', ['javac', '-cp', str(jar), '-d', str(classes),
                         str(source / 'helpers/ProofAudit.java')], 30)
        require(code == 0, 'semantic auditor compilation failed')
        proof = load_module('applied_receipt_proof', source / 'helpers/check-tlaps.py')
        tlc = load_module('applied_receipt_tlc', source / 'helpers/check-tla.py')

        class LocalJavaTemps:
            def __getattr__(self, name):
                return getattr(subprocess, name)

            def run(self, command, *positional, **keywords):
                if command[0] == 'java' and 'ProofAudit' in command:
                    temporary = Path(keywords['cwd']) / 'sany-temp'
                    temporary.mkdir(exist_ok=True)
                    command = [command[0], f'-Djava.io.tmpdir={temporary}', *command[1:]]
                return subprocess.run(command, *positional, **keywords)

        proof.subprocess = LocalJavaTemps()
        tool_args = SimpleNamespace(jar=jar, classes=classes,
                                   stdlib=tlapm.parent.parent / 'lib/tlapm/stdlib')

        def case_source(name, mutation=None):
            work = out / name
            work.mkdir()
            for n in [MODULE + '.tla', PROOF + '.tla', 'inventory.json']:
                shutil.copyfile(source / n, work / n)
            if mutation:
                filename, before, after = mutation
                target = work / filename
                target.write_text(proof.replace_once(target.read_text(), before, after))
            return work

        def finite(name, capacity, mutation=None, expected=None):
            work = case_source(name, mutation)
            config = (source / f'AppliedReceiptsCapacity{capacity}.cfg').read_text()
            if expected:
                config = proof.replace_once(config, 'INVARIANTS ARInvariant ARLookupRefinement',
                                            'INVARIANTS ' + expected)
            (work / 'Run.cfg').write_text(config)
            states = work / 'fresh-states'
            states.mkdir()
            record = dict(name=name, kind='TLC', capacity=capacity, expected=expected,
                          sources={n.name: sha(n) for n in work.iterdir() if n.is_file()})
            records.append(record)
            try:
                argv = ['java', '-XX:+UseParallelGC', '-Xmx1g', '-cp', str(jar), 'tlc2.TLC',
                        '-tool', '-workers', '1', '-fp', '0', '-metadir', str(states),
                        '-config', 'Run.cfg', MODULE + '.tla']
                code, output = run(work, 'tlc', argv, 120)
                record.update(exit_code=code, statistics=tlc.verdict(output, code, expected,
                              module=MODULE, minimum_distinct=3))
                if expected:
                    trace = tlc.messages(output, 2217, 4)
                    last = trace[-1]
                    vector, deque = receipt_sequence(last, 'arVector'), receipt_sequence(last, 'arDeque')
                    if expected == 'AROrdering':
                        require(re.search(r'/\\ arOrdered = TRUE', last) is not None and
                                any(deque[i][0] >= deque[i+1][0] for i in range(len(deque)-1)),
                                'certificate control lacks a true-certificate/non-strict sequence')
                        require(vector == deque, 'certificate control also changed FIFO retention')
                    else:
                        require(len(trace) >= 4 and vector != deque and len(vector) == len(deque) == capacity,
                                'wrong-eviction control lacks complete differing retained payloads')
                        previous = receipt_sequence(trace[-2], 'arDeque')
                        require(deque[:-1] == previous[:-1] and vector[:-1] == previous[1:],
                                'wrong-eviction trace is not pop-back versus pop-front')
                    save(work / 'counterexample.json', dict(property=expected, trace=trace,
                         final_vector=vector, final_deque=deque, attributable=True))
                record['accepted_expected_result'] = True
                print('PASS:', name, 'expected=' + str(expected), flush=True)
            finally:
                save(work / 'result.json', record)

        def theorem_case(name, mutation=None, expected=None):
            work = case_source(name, mutation)
            record = dict(name=name, kind='TLAPS/semantic', expected=expected,
                          sources={n.name: sha(n) for n in work.iterdir() if n.is_file()})
            records.append(record)
            try:
                try:
                    proof.audit(tool_args, work, inventory)
                except proof.Rejected as error:
                    record['semantic_rejection'] = str(error)
                    require(expected is not None and str(error) == expected,
                            'semantic rejection outside intended gate: ' + str(error))
                    require(not (work / 'tlaps.log').exists(), 'rejected semantic input reached TLAPS')
                else:
                    require(expected is None, 'invalid semantic control was accepted')
                    cache = work / 'fresh-cache'
                    code, output = run(work, 'tlaps', [str(tlapm), '--strict', '--nofp',
                         '--threads', '1', '--cache-dir', str(cache), str(work / (PROOF + '.tla'))], 300)
                    record.update(exit_code=code, obligations=proof.proof_verdict(output, code, PROOF, 122),
                                  semantic_import_assumption_hole_audit=True)
                record['accepted_expected_result'] = True
                print('PASS:', name, 'expected=' + str(expected), flush=True)
            finally:
                save(work / 'result.json', record)

        finite('capacity-1', 1)
        finite('capacity-2', 2)
        certificate = (MODULE + '.tla',
             '    c /\\ (IF Len(s) = 0 THEN TRUE ELSE s[Len(s)].index < e.index)', '    TRUE')
        wrong_eviction = (MODULE + '.tla',
             '    Append(IF Len(s) = ARCapacity THEN Tail(s) ELSE s, e)',
             '    Append(IF Len(s) = ARCapacity THEN SubSeq(s, 1, Len(s) - 1) ELSE s, e)')
        for label, mutation, expected in [('certificate', certificate, 'AROrdering'),
                                          ('wrong-eviction', wrong_eviction, 'ARRefinement')]:
            finite(label + '-baseline', 2)
            finite(label + '-mutant', 2, mutation, expected)
            finite(label + '-restored', 2)
        hole = (PROOF + '.tla', 'BY ARInvariantAlways, ARLookupObservation, PTL\n', 'PROOF OMITTED\n')
        axiom = (MODULE + '.tla', 'EXTENDS Integers, Sequences\n',
                 'EXTENDS Integers, Sequences\nAXIOM ARUnapproved == FALSE\n')
        for label, mutation, expected in [
                ('proof-hole', hole, 'proof hole: AppliedReceiptsProof.ARLookupAlways'),
                ('unapproved-axiom', axiom, 'unapproved module assumption: AppliedReceipts')]:
            theorem_case(label + '-baseline')
            theorem_case(label + '-mutant', mutation, expected)
            theorem_case(label + '-restored')
        baseline_output = (out / 'proof-hole-baseline/tlaps.log').read_text()
        output_controls = proof.output_controls(baseline_output, PROOF, 122)
        for name, entry in frozen['files'].items():
            require(sha(HERE / name) == sha(source / name) == entry['sha256'], 'preparation source changed: ' + name)
        for name, entry in frozen['source_bindings'].items():
            require(sha(Path(name)) == entry['sha256'], 'Rust/source boundary changed: ' + name)
        require(len(records) == 14 and all(r['accepted_expected_result'] for r in records), 'missing controlled case')
        summary.update(complete=True, representation_acceptance=True, declared_theorems=13,
                       fresh_obligations_per_positive_run=122, positive_proof_runs=4,
                       total_positive_proof_obligations=488, finite_runs=8,
                       semantic_control_triples=2, finite_control_triples=2,
                       proof_output_controls=output_controls, sources_unchanged=True,
                       ended_ns=time.time_ns())
    except BaseException as error:
        summary.update(failure_type=type(error).__name__, failure=str(error), ended_ns=time.time_ns())
        raise
    finally:
        save(out / 'summary.json', summary)
    print(json.dumps({k: v for k, v in summary.items() if k != 'records'}), flush=True)


if __name__ == '__main__':
    main()
