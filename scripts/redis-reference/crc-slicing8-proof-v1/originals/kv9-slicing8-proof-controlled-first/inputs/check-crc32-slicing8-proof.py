#!/usr/bin/env python3
"""Fresh kernel-only slicing-by-eight CRC proof, actual Rust tables, and source controls.
No Cargo, changes to the source worktree, or use of precompiled proof artifacts.
The historical byte-table checker remains unchanged and rejects this source.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

if not __debug__:
    raise SystemExit('FAIL: PYTHONOPTIMIZE must be 0')

SLICING_NAMES = ['zero_xor','zero_zero','zero_peel','zero_eight_decomposition',
 'packed_bytes','byte_at_xor','slice_xor','zero_comp','zero_slice','old_byte_linear',
 'old_eight_expansion','block_transition','slicing_list_equivalence',
 'slicing_parts_equivalence','slicing_checksum_equivalence','slicing_fragmentation_invariant']
COMPILED_NAMES = ['compiled_slicing_rows','compiled_slicing_columns',
 'compiled_slicing_entries','compiled_slice_entry','compiled_block_transition',
 'compiled_slicing_list_equivalence','compiled_slicing_parts_equivalence',
 'compiled_slicing_checksum_equivalence','compiled_slicing_fragmentation_invariant']


def main():
    parser=argparse.ArgumentParser(description=__doc__,allow_abbrev=False)
    parser.add_argument('--source-root',type=Path,required=True)
    parser.add_argument('--proof-dir',type=Path)
    parser.add_argument('--lean',required=True)
    parser.add_argument('--rustc',default='rustc')
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    root=args.source_root.resolve()
    proof=(args.proof_dir or root/'proofs/lean/crc32-slicing8').resolve()
    out=args.output.resolve()
    if out.exists(): raise SystemExit('FAIL: output must be fresh; preserve previous attempts')
    out.mkdir(parents=True)
    result={'accepted':False,'complete':False,'started_unix_ns':time.time_ns(),
            'scope':'Universal CRC arithmetic under explicit Rust/stdlib source mapping; not whole Rust/WAL/IO correctness',
            'controls':[],'runner_pid':os.getpid(),
            'cpu_affinity':sorted(os.sched_getaffinity(0))}
    b=None
    try:
        contract=json.loads((proof/'slicing-source-contract.json').read_text())
        helper=root/'scripts/check-crc32-proof.py'
        # Reuse the historical strict semantic axiom checker byte-for-byte.
        import hashlib
        def digest(p): return hashlib.sha256(Path(p).read_bytes()).hexdigest()
        assert digest(helper)==contract['byte_inputs']['check-crc32-proof.py'], 'old checker changed'
        spec=importlib.util.spec_from_file_location('crc_byte_checker',helper)
        b=importlib.util.module_from_spec(spec);spec.loader.exec_module(b)
        b.require(b.FOUNDATIONS=={'propext','Classical.choice','Quot.sound'},'axiom rules changed')
        b.NAMES.update(Slicing8=SLICING_NAMES,RustSlicing=COMPILED_NAMES)
        byte=root/'proofs/lean/crc32'
        inputs=[Path(__file__).resolve(),helper,root/contract['source'],
                proof/'slicing-source-contract.json',proof/'Slicing8.lean',proof/'RustSlicing.lean.in',
                byte/'CRC32.lean',byte/'RustTable.lean.in',byte/'source-contract.json',
                root/'proofs/lean/lean-toolchain']
        before={str(p):{'bytes':p.stat().st_size,'sha256':b.digest(p)} for p in inputs}
        snapshots=out/'inputs';snapshots.mkdir()
        for p in inputs: shutil.copyfile(p,snapshots/p.name)
        b.save(out/'source-inputs.json',before)
        for name in ['CRC32.lean','RustTable.lean.in','source-contract.json']:
            b.require(b.digest(byte/name)==contract['byte_inputs'][name],f'old proof input changed: {name}')
        candidate=(snapshots/'wal.rs').read_text()
        b.require(b.digest(snapshots/'wal.rs')==contract['candidate_wal_sha256'],'frozen WAL hash mismatch')
        declarations=b.bind(candidate,contract['candidate'])
        for label,revision,expected in [('byte',contract['byte_revision'],contract['byte']),
                                         ('bitwise',contract['bitwise_revision'],contract['bitwise'])]:
            code,old=b.command(['git','-C',str(root),'show',revision+':'+contract['source']],out,label+'-source')
            b.require(code==0,'committed source unavailable: '+label)
            b.bind(old,expected)
            if label=='byte': b.require(b.digest(out/'byte-source.stdout')==contract['byte_wal_sha256'],'committed ca WAL hash mismatch')
        # Demonstrate that old source acceptance is not silently inherited.
        try: b.bind(candidate,contract['byte'])
        except b.Rejected as error:
            b.require('crc32_parts' in str(error),'historical rejection is not the changed CRC function')
            result['historical_checker_source_rejection']=str(error)
        else: raise b.Rejected('new source unexpectedly matches historical byte contract')
        lean=shutil.which(args.lean); rustc=shutil.which(args.rustc)
        b.require(lean and rustc,'Lean and standalone rustc required')
        code,version=b.command([lean,'--version'],out,'lean-version',timeout=15)
        expected=(snapshots/'lean-toolchain').read_text().strip().split(':v')[1]
        b.require(code==0 and version.startswith(f'Lean (version {expected},'),'Lean version mismatch')
        code,sysroot=b.command([rustc,'--print','sysroot'],out,'rustc-sysroot',timeout=15)
        b.require(code==0,'rustc sysroot unavailable')
        rustc=str((Path(sysroot.strip())/'bin/rustc').resolve())
        code,rversion=b.command([rustc,'--version','--verbose'],out,'rustc-version',timeout=15)
        b.require(code==0,'rustc version unavailable')
        result['toolchain']={'lean':lean,'lean_sha256':b.digest(lean),'lean_version':version.strip(),
            'rustc':rustc,'rustc_sha256':b.digest(rustc),'rustc_version':rversion.strip()}
        core=(snapshots/'CRC32.lean').read_text()
        slicing=(snapshots/'Slicing8.lean').read_text()
        # Development #print queries, if present, must be exactly the declared inventory.
        queries=re.findall(r'^#print axioms Kv9\.CRC32\.([A-Za-z_0-9]+)$',slicing,re.M)
        b.require(not queries or queries==SLICING_NAMES,'unexpected existing axiom queries')
        slicing=re.sub(r'^#print axioms Kv9\.CRC32\.[A-Za-z_0-9]+\n?','',slicing,flags=re.M)
        bt=(snapshots/'RustTable.lean.in').read_text()
        st=(snapshots/'RustSlicing.lean.in').read_text()
        for name,text in [('CRC32',core),('Slicing8',slicing),('RustTable',bt),('RustSlicing',st)]:
            b.require(re.findall(r'^theorem ([A-Za-z_0-9]+)',text,re.M)==b.NAMES[name],f'theorem inventory mismatch: {name}')
        rust_source='\n\n'.join(declarations.values())+'\n'

        def compile_tables(source,work):
            main='''
fn main() {
 std::hint::black_box((crc32(&[]), crc32_parts(&[])));
 for (i,v) in CRC32_TABLE.iter().enumerate() { println!("B {i} {v:08x}"); }
 for (r,row) in CRC32_SLICING.iter().enumerate() {
  for (i,v) in row.iter().enumerate() { println!("S {r} {i} {v:08x}"); }
 }
}
'''
            (work/'table.rs').write_text(source+main)
            code,log=b.command([rustc,'--edition=2021','-Dwarnings','-O','table.rs','-o','table'],work,'rustc')
            b.require(code==0,'extracted Rust compile failed: '+log)
            code,log=b.command([str(work/'table')],work,'table',timeout=15)
            b.require(code==0,'table emitter failed')
            lines=log.splitlines();b.require(len(lines)==2304,'compiled table length mismatch')
            base=[];rows=[[] for _ in range(8)]
            for i,line in enumerate(lines[:256]):
                m=re.fullmatch(r'B ([0-9]+) ([0-9a-f]{8})',line)
                b.require(m and int(m[1])==i,'base table index/value mismatch');base.append(int(m[2],16))
            for n,line in enumerate(lines[256:]):
                r,i=divmod(n,256);m=re.fullmatch(r'S ([0-9]+) ([0-9]+) ([0-9a-f]{8})',line)
                b.require(m and int(m[1])==r and int(m[2])==i,'slicing index/value mismatch');rows[r].append(int(m[3],16))
            values={'base':base,'slicing':rows};b.save(work/'compiled-tables.json',values)
            return values

        def instantiate(template,values):
            b.require(template.count('@SLICING@')==1,'slicing marker mismatch')
            return template.replace('@SLICING@',',\n'.join('['+', '.join(f'0x{v:08x}' for v in row)+']' for row in values['slicing']))

        def positive(work):
            work.mkdir();values=compile_tables(rust_source,work)
            axioms={}
            for name,text in [('CRC32',core),('Slicing8',slicing),
                             ('RustTable',b.table_proof(bt,values['base'])),('RustSlicing',instantiate(st,values))]:
                axioms.update(b.lean_check(lean,name,text,work))
            return values,axioms

        pos=out/'positive';values,axioms=positive(pos)
        result['theorems']=axioms;result['compiled_base_entries']=256;result['compiled_slicing_entries']=2048
        result['checked_theorems']=len(axioms)

        def rejected_lean(name,module,text,marker,dependencies):
            work=out/('control-'+name);work.mkdir()
            for dep in dependencies: shutil.copyfile(pos/(dep+'.olean'),work/(dep+'.olean'))
            try: b.lean_check(lean,module,text,work)
            except b.Rejected as error:
                b.require(marker in str(error),f'{name} failed outside intended assertion: {error}')
                result['controls'].append({'name':name,'rejected':True,'reason':str(error),'module':module})
            else: raise b.Rejected('control accepted: '+name)

        # Corrupt actual Rust const recurrence; all entries are still emitted by rustc.
        work=out/'control-compiled-recurrence';work.mkdir()
        anchor='(prior >> 8)';b.require(rust_source.count(anchor)==1,'recurrence control anchor mismatch')
        bad=compile_tables(rust_source.replace(anchor,'(prior >> 7)'),work)
        b.require(bad['base']==values['base'] and bad['slicing']!=values['slicing'],'recurrence control did not isolate slicing table')
        for dep in ['CRC32','Slicing8','RustTable']:shutil.copyfile(pos/(dep+'.olean'),work/(dep+'.olean'))
        try:b.lean_check(lean,'RustSlicing',instantiate(st,bad),work)
        except b.Rejected as error:
            b.require('Lean rejected RustSlicing' in str(error) and 'decide' in str(error),'recurrence control did not reach finite kernel assertion')
            result['controls'].append({'name':'compiled-recurrence','rejected':True,'reason':str(error)})
        else:raise b.Rejected('bad compiled slicing recurrence accepted')

        # Preserve the original alternative-polynomial finite binding check as well.
        work=out/'control-compiled-polynomial';work.mkdir()
        b.require(rust_source.count('0xEDB8_8320')==1,'polynomial anchor mismatch')
        bad=compile_tables(rust_source.replace('0xEDB8_8320','0x82F6_3B78'),work)
        shutil.copyfile(pos/'CRC32.olean',work/'CRC32.olean')
        try:b.lean_check(lean,'RustTable',b.table_proof(bt,bad['base']),work)
        except b.Rejected as error:
            b.require('Lean rejected RustTable' in str(error) and 'decide' in str(error),'polynomial control failed outside finite table assertion')
            result['controls'].append({'name':'compiled-polynomial','rejected':True,'reason':str(error)})
        else:raise b.Rejected('bad polynomial accepted')

        anchor='(compiledSlicing[7]!)';b.require(st.count(anchor)==1,'row control anchor mismatch')
        rejected_lean('wrong-block-row','RustSlicing',instantiate(st.replace(anchor,'(compiledSlicing[6]!)'),values),
                      'unsolved goals',['CRC32','Slicing8','RustTable'])
        anchor='  obtain ⟨h0,h1,h2,h3⟩ := packed_bytes b0 b1 b2 b3\n  rw [old_eight_expansion, zero_eight_decomposition]\n  simp only [sliceBlock, byte_at_xor, h0, h1, h2, h3, slice_xor]\n  ac_rfl'
        b.require(slicing.count(anchor)==1,'block proof control anchor mismatch')
        rejected_lean('proof-hole','Slicing8',slicing.replace(anchor,'  sorry'),'declaration uses `sorry`',['CRC32'])
        mutant=slicing.replace('namespace Kv9.CRC32','namespace Kv9.CRC32\naxiom fake : False').replace(anchor,'  exact False.elim fake')
        rejected_lean('unapproved-axiom','Slicing8',mutant,'untrusted axioms',['CRC32'])
        for name,old,new in [('big-endian','u32::from_le_bytes','u32::from_be_bytes'),
                             ('tail-shift','(crc >> 8)','(crc >> 7)'),
                             ('missing-complement','    !crc\n}','    crc\n}')]:
            b.require(candidate.count(old)==1,'source mutant anchor mismatch: '+name)
            mutant=candidate.replace(old,new)
            work=out/('control-source-'+name);work.mkdir();(work/'wal-mutant.rs').write_text(mutant)
            try:b.bind(mutant,contract['candidate'])
            except b.Rejected as error:
                b.save(work/'rejection.json',{'rejected':True,'reason':str(error)})
                result['controls'].append({'name':'source-'+name,'rejected':True,'reason':str(error)})
            else:raise b.Rejected('source mutant accepted: '+name)
        restored_values,restored=positive(out/'restored')
        b.require(restored_values==values and restored==axioms,'restored theorem/table mismatch')
        b.require(all(b.digest(p)==before[str(p)]['sha256'] for p in inputs),'input changed during gate')
        result.update(accepted=True,complete=True,restored_checked_theorems=len(restored),fresh_positive_theorem_checks=len(axioms)+len(restored))
        print(f"PASS: {len(axioms)} theorems, 256 + 2048 compiled entries, {len(result['controls'])} rejected controls; fresh restoration",flush=True)
    except Exception as error:
        result.update(complete=True,failure=f'{type(error).__name__}: {error}')
        raise
    finally:
        result['completed_unix_ns']=time.time_ns()
        (out/'result.json').write_text(json.dumps(result,indent=2,sort_keys=True)+'\n')
        import hashlib
        inventory={str(p.relative_to(out)):{'bytes':p.stat().st_size,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()}
                   for p in sorted(out.rglob('*')) if p.is_file() and p.name!='inventory.json'}
        (out/'inventory.json').write_text(json.dumps(inventory,indent=2,sort_keys=True)+'\n')

if __name__=='__main__':
    main()
