#!/usr/bin/env python3
"""Check the WAL stream-prefix refinement under explicit std::io premises."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
MODEL = ROOT / 'proofs/smt/wal_vectored'


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--z3', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    assert __debug__
    args.output.mkdir(exist_ok=False)
    source = (ROOT / 'crates/engine/src/wal_segment.rs').read_bytes()
    ancestor = subprocess.check_output([
        'git', 'show', '11113f68f6a5df77da1ffb4fcec850953716ffa3:crates/engine/src/wal_segment.rs'
    ], cwd=ROOT)
    contract = json.loads((MODEL / 'source-contract.json').read_text())
    begin = source.index(b'/// Write the same header/payload/checksum sequence')
    end = source.index(b'fn sync_namespace(', begin)
    helper = source[begin:end]
    assert sha(helper) == contract['helper_sha256']
    before = b'''            .measure(|| {
                self.file.write_all(&header)?;
                self.file.write_all(&payload)?;
                self.file.write_all(&crc)
            })'''
    after = b'''            .measure(|| write_frame(&mut self.file, &header, &payload, &crc))'''
    assert ancestor.count(before) == source.count(after) == 1
    expected = ancestor.replace(
        b'use std::io::{BufReader, Read, Seek, SeekFrom, Write};',
        b'use std::io::{BufReader, IoSlice, Read, Seek, SeekFrom, Write};'
    ).replace(before, after).replace(b'fn sync_namespace(', helper + b'fn sync_namespace(', 1)
    assert expected == source, 'Production or existing test changed beyond the declared helper/call/import'
    models = {name: (MODEL / (name + '.smt2')).read_text()
              for name in ['prefix', 'cursor', 'completion']}
    cases = [(name, text, 'unsat') for name, text in models.items()]
    mutations = [
        ('skipped-byte', 'prefix', '(+ k (- i k))', '(+ k (- i k) 1)'),
        ('advance-too-far', 'cursor', '(define-fun next () Int (+ k n))',
         '(define-fun next () Int (+ k n 1))'),
        ('omit-checksum', 'completion', '(define-fun length () Int (+ header payload checksum))',
         '(define-fun length () Int (+ header payload))'),
    ]
    for name, model, old, new in mutations:
        assert models[model].count(old) == 1
        cases.append((name, models[model].replace(old, new) + '(get-model)\n', 'sat'))
    result = {
        'complete': False, 'source_sha256': sha(source), 'helper_sha256': sha(helper),
        'exact_delta': True, 'solver_sha256': sha(args.z3.read_bytes()),
        'solver_version': subprocess.check_output([str(args.z3), '--version'], text=True).strip(),
        'scope': 'Unbounded prefix/cursor/completion induction under Write and IoSlice contracts; not whole-Rust or Raft proof',
        'cases': [],
    }
    try:
        for name, text, expected in cases:
            path = args.output / (name + '.smt2'); path.write_text(text)
            argv = [str(args.z3), '-T:10', '-smt2', str(path)]
            run = subprocess.run(argv, capture_output=True, text=True, timeout=15)
            path.with_suffix('.stdout').write_text(run.stdout)
            path.with_suffix('.stderr').write_text(run.stderr)
            row = {'name': name, 'argv': argv, 'exit_code': run.returncode,
                   'expected': expected, 'query_sha256': sha(text.encode())}
            result['cases'].append(row)
            assert run.returncode == 0 and not run.stderr.strip()
            assert run.stdout.splitlines()[0] == expected and '(error' not in run.stdout
            if expected == 'sat':
                assert 'define-fun' in run.stdout
            row['verified'] = True
            print(name + ': ' + expected, flush=True)
        assert (ROOT / 'crates/engine/src/wal_segment.rs').read_bytes() == source
        assert all((MODEL / (n + '.smt2')).read_text() == s for n, s in models.items())
        result['complete'] = True
    finally:
        (args.output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
