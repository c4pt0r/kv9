#!/usr/bin/env python3
"""Check universal frame layout under explicit Rust vector/compiler premises."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
MODEL = ROOT/'proofs/smt/raft_frame_buffer'

def sha(data):
    return hashlib.sha256(data).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--z3', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    assert __debug__
    args.output.mkdir(exist_ok=False)
    source = (ROOT/'crates/raft/src/storage.rs').read_bytes()
    ancestor = subprocess.check_output(['git','show','11113f68f6a5df77da1ffb4fcec850953716ffa3:crates/raft/src/storage.rs'],cwd=ROOT)
    before = b'''        let mut body = Vec::with_capacity(1 + payload.len());
        body.push(kind);
        body.extend_from_slice(payload);
        let mut rec = Vec::with_capacity(8 + body.len());
        rec.extend_from_slice(&(body.len() as u32).to_be_bytes());
        rec.extend_from_slice(&fnv1a(&body).to_be_bytes());
        rec.extend_from_slice(&body);'''
    after = b'''        let body_len = 1 + payload.len();
        let mut rec = Vec::with_capacity(8 + body_len);
        rec.extend_from_slice(&(body_len as u32).to_be_bytes());
        rec.extend_from_slice(&[0; 4]);
        rec.push(kind);
        rec.extend_from_slice(payload);
        // Hash the final body in place; filling the checksum cannot change it.
        let checksum = fnv1a(&rec[8..]);
        rec[4..8].copy_from_slice(&checksum.to_be_bytes());'''
    # All production storage code, including refusal, FNV, writer failures,
    # synchronization and publication, is identical outside this replacement.
    old_prod = ancestor.split(b'#[cfg(test)]',1)[0]
    new_prod = source.split(b'#[cfg(test)]',1)[0]
    assert old_prod.count(before) == 1 and new_prod.count(after) == 1
    assert old_prod.replace(before,after) == new_prod
    sources = {name:(MODEL/f'{name}.smt2').read_text() for name in ('equivalence','bounds')}
    base = sources['equivalence']
    body = base.replace('(assert (distinct historical finished))',
                        '(assert (distinct body (seq.extract finished 8 body-length)))')
    assert body != base
    cases = [('frame-equivalence',base,'unsat'),('body-preserved',body,'unsat'),('representable-bounds',sources['bounds'],'unsat')]
    mutants = [
        ('wrong-hash-offset','(seq.extract staged 8 body-length))','(seq.extract staged 7 body-length))'),
        ('wrong-checksum-range','(seq.extract staged 0 4)','(seq.extract staged 0 5)'),
        ('omitted-kind','(define-fun staged () Bytes (seq.++ header (be32 #x00000000) (seq.unit kind) payload))',
         '(define-fun staged () Bytes (seq.++ header (be32 #x00000000) payload))'),
    ]
    for name,old,new in mutants:
        # Only the hash-body declaration changes for the offset control.
        if name == 'wrong-hash-offset':
            old = '(define-fun hash-body () Bytes '+old
            new = '(define-fun hash-body () Bytes '+new
        assert base.count(old) == 1, name
        cases.append((name,base.replace(old,new)+'(get-model)\n','sat'))
    result = dict(complete=False,source_sha256=sha(source),ancestor_production_sha256=sha(old_prod),candidate_production_sha256=sha(new_prod),production_delta_exact=True,solver=str(args.z3),solver_sha256=sha(args.z3.read_bytes()),solver_version=subprocess.check_output([str(args.z3),'--version'],text=True,timeout=15).strip(),cases=[],scope='Universal byte-sequence layout and length arithmetic under Rust Vec/slice/serialization and compiler premises; not whole-Rust, WAL durability or Raft proof')
    try:
        for name,text,expected in cases:
            path=args.output/f'{name}.smt2';path.write_text(text)
            argv=[str(args.z3),'-T:10','-smt2',str(path)]
            run=subprocess.run(argv,capture_output=True,text=True,timeout=15)
            path.with_suffix('.stdout').write_text(run.stdout)
            path.with_suffix('.stderr').write_text(run.stderr)
            row=dict(name=name,source_sha256=sha(text.encode()),argv=argv,exit_code=run.returncode,expected=expected)
            result['cases'].append(row)
            assert run.returncode == 0 and not run.stderr.strip() and run.stdout.splitlines()[0] == expected and '(error' not in run.stdout,(name,run.stdout,run.stderr)
            if expected == 'sat':assert 'define-fun' in run.stdout
            row['verified']=True
            print(name+': '+expected,flush=True)
        assert (ROOT/'crates/raft/src/storage.rs').read_bytes() == source
        for name,text in sources.items():assert (MODEL/f'{name}.smt2').read_text() == text
        result['complete']=True
    finally:
        (args.output/'result.json').write_text(json.dumps(result,indent=2)+'\n')

if __name__ == '__main__':
    main()
