#!/usr/bin/env python3
"""Check sampling-error containment, recovery and shared margin; local Z3 only."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--z3', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir()
    result = {'complete': False, 'solver': str(args.z3.resolve()),
              'solver_sha256': hashlib.sha256(args.z3.read_bytes()).hexdigest(),
              'solver_version': subprocess.check_output([str(args.z3), '--version'], text=True, timeout=15).strip(),
              'cases': []}
    sources = {name: (ROOT / 'proofs/tlaps/leader_lease' / f'sampled-{name}.smt2').read_text()
               for name in ['containment', 'recovery', 'margin']}
    cases = [(name, text, 'unsat') for name, text in sources.items()]
    mutations = [
        ('containment-without-error', 'containment',
         '(assert (<= (* (+ D (* 2 epsilon)) b) (* (- E (* 2 epsilon)) a)))',
         '(assert (<= (* D b) (* E a)))'),
        ('recovery-without-error', 'recovery',
         '(assert (>= (* (- R (* 2 epsilon)) a) (* (+ E (* 2 epsilon)) b)))',
         '(assert (>= (* R a) (* E b)))'),
        ('leader-only-margin', 'margin',
         '(assert (>= (* M a) (* 2 epsilon (+ a b))))',
         '(assert (>= (* M b) (* 2 epsilon (+ a b))))'),
    ]
    for name, source, old, new in mutations:
        assert sources[source].count(old) == 1
        cases.append((name, sources[source].replace(old, new) + '(get-model)\n', 'sat'))
    try:
        for name, source, expected in cases:
            path = args.output / f'{name}.smt2'
            path.write_text(source)
            command = [str(args.z3), '-T:5', '-smt2', str(path)]
            run = subprocess.run(command, capture_output=True, text=True, timeout=15)
            path.with_suffix('.stdout').write_text(run.stdout)
            path.with_suffix('.stderr').write_text(run.stderr)
            row = {'name': name, 'source_sha256': hashlib.sha256(source.encode()).hexdigest(),
                   'argv': command, 'exit_code': run.returncode, 'expected': expected}
            result['cases'].append(row)
            assert run.returncode == 0 and not run.stderr.strip(), (name, run.returncode, run.stderr)
            assert run.stdout.splitlines()[0] == expected and '(error' not in run.stdout, (name, run.stdout)
            if expected == 'sat':
                assert 'define-fun' in run.stdout, 'countermodel missing'
            row['verified'] = True
            print(f'{name}: {expected}', flush=True)
        for name, source in sources.items():
            assert (ROOT / 'proofs/tlaps/leader_lease' / f'sampled-{name}.smt2').read_text() == source
        result['complete'] = True
    finally:
        (args.output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
