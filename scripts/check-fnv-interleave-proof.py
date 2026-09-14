#!/usr/bin/env python3
"""Fresh source-bound FNV interleaving proof and standalone Rust equivalence tests."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

if not __debug__:
    raise SystemExit('PYTHONOPTIMIZE=0 is required')

FOUNDATIONS = {'Classical.choice', 'Quot.sound', 'propext'}
NAMESPACE = 'Kv9.FnvInterleave'


class Rejected(Exception):
    pass


def require(value, message):
    if not value:
        raise Rejected(message)


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def save(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')


def command(argv, directory, label, timeout=45):
    record = dict(argv=list(map(str, argv)), cwd=str(directory), started_unix_ns=time.time_ns())
    with (directory / (label + '.stdout')).open('x') as stdout, (directory / (label + '.stderr')).open('x') as stderr:
        process = subprocess.Popen(argv, cwd=directory, stdout=stdout, stderr=stderr)
        record['pid'] = process.pid
        try:
            fields = Path('/proc', str(process.pid), 'stat').read_text().rsplit(')', 1)[1].split()
            record['start_ticks'] = int(fields[19])
            record['cpu_affinity'] = sorted(os.sched_getaffinity(process.pid))
        except FileNotFoundError:
            record['identity_observation'] = 'Child exited before /proc observation; Popen handle remains authoritative.'
        save(directory / (label + '.invocation.json'), record)
        try:
            code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.kill()
            code = process.wait()
            record['timeout'] = True
        record.update(exit_code=code, ended_unix_ns=time.time_ns(), reaped=True,
                      absent=not Path('/proc', str(process.pid)).exists())
        save(directory / (label + '.terminal.json'), record)
    text = (directory / (label + '.stdout')).read_text() + (directory / (label + '.stderr')).read_text()
    require(not record.get('timeout'), 'bounded command timeout: ' + label)
    require(record['absent'], 'owned child remains: ' + label)
    return code, text


def declaration(text, needle):
    require(text.count(needle) == 1, 'missing/duplicate declaration: ' + needle)
    start = text.index(needle)
    if needle.startswith('const '):
        return text[start:text.index(';', start) + 1]
    cursor = text.index('{', start) + 1
    depth = 1
    while depth and cursor < len(text):
        depth += (text[cursor] == '{') - (text[cursor] == '}')
        cursor += 1
    require(depth == 0, 'unclosed declaration: ' + needle)
    return text[start:cursor]


def bind(text, expected):
    for needle, original in expected.items():
        require(re.sub(r'\s+', '', declaration(text, needle)) == re.sub(r'\s+', '', original),
                'source contract mismatch: ' + needle)


def parse_axioms(log, names):
    result = {}
    for name in names:
        qualified = NAMESPACE + '.' + name
        pattern = re.escape("'" + qualified + "'") + r' (?:depends on axioms: \[([^]]*)\]|does not depend on any axioms)'
        matches = list(re.finditer(pattern, log))
        require(len(matches) == 1, 'missing/duplicate axiom report: ' + qualified)
        dependencies = {item.strip() for item in (matches[0][1] or '').split(',') if item.strip()}
        require(dependencies <= FOUNDATIONS, 'untrusted axioms: ' + qualified + ': ' + str(sorted(dependencies)))
        result[qualified] = sorted(dependencies)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--source-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lean', type=Path, required=True)
    parser.add_argument('--rustc', type=Path, required=True)
    args = parser.parse_args()
    root, output = args.source_root.resolve(), args.output.resolve()
    require(not output.exists(), 'fresh output required; preserve every earlier attempt')
    output.mkdir(parents=True)
    result = dict(complete=False, accepted=False, controls=[], started_unix_ns=time.time_ns(),
                  cpu_affinity=sorted(os.sched_getaffinity(0)), runner_pid=os.getpid(),
                  scope='Universal independent-fold equivalence under explicit Rust primitive/iterator mapping; standalone kernel only, no writer/Raft integration proof.')
    try:
        proof = root / 'proofs/lean/fnv-interleave'
        contract = json.loads((proof / 'source-contract.json').read_text())
        module = root / contract['source']
        lean_source = proof / 'Interleave.lean'
        inputs = [module, lean_source, proof / 'source-contract.json', Path(__file__).resolve(), root / 'proofs/lean/lean-toolchain']
        before = {str(p): dict(bytes=p.stat().st_size, sha256=sha(p)) for p in inputs}
        snapshots = output / 'inputs'
        snapshots.mkdir()
        for path in inputs:
            shutil.copyfile(path, snapshots / path.name)
        save(output / 'inputs.json', before)
        require(sha(module) == contract['module_sha256'] and sha(lean_source) == contract['proof_sha256'], 'frozen source/model hash mismatch')
        require(set(contract['foundations']) == FOUNDATIONS, 'foundation allowlist mismatch')
        source = module.read_text()
        model = lean_source.read_text()
        bind(source, contract['declarations'])
        require(re.findall(r'^theorem ([A-Za-z0-9_]+)', model, re.M) == contract['theorems'], 'theorem inventory mismatch')
        require(not re.search(r'\b(sorry|axiom|native_decide|bv_decide|implemented_by)\b', model), 'unsupported proof construct')
        code, parent = command(['git', '-C', str(root), 'show', contract['parent_revision'] + ':' + contract['parent_source']], output, 'original-source')
        require(code == 0 and hashlib.sha256(parent.encode()).hexdigest() == contract['parent_sha256'], 'original committed writer differs')
        bind(parent, {'fn fnv1a(': contract['parent_fnv']})
        expected = (root / 'proofs/lean/lean-toolchain').read_text().strip().split(':v')[1]
        code, version = command([str(args.lean), '--version'], output, 'lean-version', 15)
        require(code == 0 and version.startswith('Lean (version ' + expected + ','), 'Lean version mismatch')
        code, rust_version = command([str(args.rustc), '--version', '--verbose'], output, 'rustc-version', 15)
        require(code == 0, 'rustc unavailable')
        result['tools'] = dict(lean=dict(path=str(args.lean), sha256=sha(args.lean), version=version.strip()),
                               rustc=dict(path=str(args.rustc), sha256=sha(args.rustc), version=rust_version.strip()))

        def lean_check(label, text, names):
            directory = output / label
            directory.mkdir()
            queries = '\n'.join('#print axioms ' + NAMESPACE + '.' + name for name in names)
            (directory / 'Interleave.lean').write_text(text + '\n' + queries + '\n')
            code, log = command([str(args.lean), '-M1024', '-j1', '-DwarningAsError=true', 'Interleave.lean'], directory, 'lean')
            return directory, code, log

        directory, code, log = lean_check('positive', model, contract['theorems'])
        require(code == 0, 'Lean rejected positive proof: ' + log[:2000])
        axioms = parse_axioms(log, contract['theorems'])
        save(directory / 'axioms.json', axioms)
        result['distinct_theorems'] = len(axioms)

        # Compile the actual frozen module and the exact extracted old writer
        # recurrence together. Nothing is imported from a prebuilt KV9 binary.
        rust = output / 'rust'
        rust.mkdir()
        shutil.copyfile(module, rust / 'fnv.rs')
        (rust / 'original.rs').write_text(contract['parent_fnv'].replace('fn fnv1a(', 'fn original_fnv1a(', 1) + '\n')
        (rust / 'tests.rs').write_text('''#[path = "fnv.rs"] mod fnv;
include!("original.rs");
#[test]
fn actual_committed_scalar_matches_all_lanes() {
    let data: [Vec<u8>; 4] = std::array::from_fn(|lane| (0..65536).map(|i| ((i * 71 + lane * 53) ^ (i >> 8)) as u8).collect());
    for sizes in [[0,0,0,0], [0,205,10601,32], [206;4], [10601;4], [65536;4], [205,206,10599,10601]] {
        let inputs = std::array::from_fn(|i| &data[i][..sizes[i]]);
        let expected = inputs.map(original_fnv1a);
        assert_eq!(fnv::fnv1a_four(inputs), expected);
        assert_eq!(inputs.map(fnv::fnv1a), expected);
    }
}
''')
        for profile, options in [('debug', []), ('optimized', ['-Copt-level=3'])]:
            binary = rust / ('tests-' + profile)
            code, log = command([str(args.rustc), '--edition=2021', '-Dwarnings', '--test', 'tests.rs', *options, '-o', str(binary)], rust, 'compile-' + profile, 90)
            require(code == 0, 'Rust test compile failed: ' + log[:2000])
            code, log = command([str(binary), '--test-threads=1'], rust, 'test-' + profile, 45)
            require(code == 0 and '4 passed; 0 failed' in log, 'Rust equivalence tests failed: ' + log[:2000])
        result['rust_tests'] = dict(distinct_tests=4, profiles=['debug', 'optimized'], total_passes=8)

        bad_model = model.replace('(step h1 b)', '(step h1 a)', 1)
        require(bad_model != model, 'cross-lane control mutation missing')
        _, code, log = lean_check('control-cross-lane', bad_model, contract['theorems'])
        require(code != 0 and ('Type mismatch' in log or 'type mismatch' in log or 'unsolved goals' in log) and 'excessive memory' not in log, 'cross-lane proof control did not fail for the intended reason')
        result['controls'].append(dict(name='cross-lane-proof', rejected=True))

        needle = '  exact four_equivalence fnvStep as bs cs ds initial initial initial initial'
        require(model.count(needle) == 1, 'proof-hole control target mismatch')
        _, code, log = lean_check('control-proof-hole', model.replace(needle, '  sorry', 1), contract['theorems'])
        require(code != 0 and 'sorry' in log, 'proof hole not rejected')
        result['controls'].append(dict(name='proof-hole', rejected=True))

        injected = model.replace('end ' + NAMESPACE, 'axiom injectedFalse : False\ntheorem injected_false : False := injectedFalse\nend ' + NAMESPACE)
        _, code, log = lean_check('control-custom-axiom', injected, contract['theorems'] + ['injected_false'])
        require(code == 0, 'custom axiom control must reach semantic inspection')
        try:
            parse_axioms(log, contract['theorems'] + ['injected_false'])
        except Rejected as error:
            require('untrusted axioms:' in str(error), 'unrelated axiom-control failure')
        else:
            raise Rejected('custom false axiom was accepted')
        result['controls'].append(dict(name='custom-axiom', rejected=True))

        for name, old, new in [('wrong-prime', '0x01000193', '0x01000195'),
                               ('wrong-lane', 'u32::from(b[i])', 'u32::from(a[i])'),
                               ('wrong-tail', '&d[common..]', '&d[..common]')]:
            mutated = source.replace(old, new, 1)
            require(mutated != source, 'source control target absent: ' + name)
            directory = output / ('control-' + name)
            directory.mkdir()
            (directory / 'fnv.rs').write_text(mutated)
            try:
                bind(mutated, contract['declarations'])
            except Rejected as error:
                require('source contract mismatch:' in str(error), 'unrelated source-control failure')
                save(directory / 'result.json', dict(rejected=True, reason=str(error)))
            else:
                raise Rejected('bad source accepted: ' + name)
            result['controls'].append(dict(name=name, rejected=True))
        require(all(sha(p) == row['sha256'] for p, row in before.items()), 'proof inputs changed during qualification')
        result.update(complete=True, accepted=True, source_unchanged=True)
    except BaseException as error:
        result['failure'] = repr(error)
        raise
    finally:
        result['ended_unix_ns'] = time.time_ns()
        save(output / 'result.json', result)
        inventory = {str(p.relative_to(output)): dict(bytes=p.stat().st_size, sha256=sha(p))
                     for p in sorted(output.rglob('*')) if p.is_file()}
        save(output / 'inventory.json', dict(complete=result['complete'], files=inventory))
    print(json.dumps(result))


if __name__ == '__main__':
    main()
