#!/usr/bin/env python3
"""Independently audit completed source, model, layout and proof qualification."""
import difflib
import hashlib
import json
from pathlib import Path
import re
import tarfile
import tomllib
import sys

R=Path(__file__).resolve().parents[2];S=Path(sys.argv[1]).resolve()
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
load=lambda p:json.loads(p.read_text())
label=sys.argv[2] if len(sys.argv)>2 else 'layout'
tests=load(S/'tests-summary.json');layout=load(S/(label+'-summary.json'));proof=load(S/'proof/result.json')
assert tests['complete'] and layout['complete'] and proof['accepted']
assert len(proof['theorems'])==23 and len(proof['controls'])==13
for path,digest in {**load(S/'sources.json'),**layout['sources']}.items():assert sha(Path(path))==digest,path
for arm in ('baseline','candidate'):
    for name in ('engine','common'):
        for p in (R/'crates'/name).rglob('*'):
            if not p.is_file():continue
            if arm=='candidate' and p.name in ('mem.rs','lib.rs') and name=='engine':continue
            assert p.read_bytes()==(S/arm/p.relative_to(R)).read_bytes(),p
    assert (S/arm/'crates/engine/tests/inline_key_model.rs').read_bytes()==(R/'scripts/inline-key/model.rs').read_bytes()
    row=tests['arms'][arm]
    assert sum(v['passed'] for v in row['results'])=={'baseline':202,'candidate':206}[arm]
    assert sum(v['ignored'] for v in row['results'])==25
    for binary in row['binaries']:assert sha(Path(binary['path']))==binary['sha256']
for arm in ('baseline','candidate'):
    assert (S/arm/'crates/engine/tests/entry_buffer_model.rs').read_bytes()==(R/'scripts/entry-buffer/model.rs').read_bytes()
assert (S/'candidate/crates/engine/src/entry_buffer.rs').read_bytes()==(R/'scripts/entry-buffer/entry_buffer.rs').read_bytes()==(S/'layout-source/entry_buffer.rs').read_bytes()
assert (S/'candidate/crates/engine/src/lib.rs').read_text()==(R/'crates/engine/src/lib.rs').read_text().replace('pub mod mem;','pub mod mem;\nmod entry_buffer;')
base=(S/'baseline/crates/engine/src/mem.rs').read_text()
changed=(S/'candidate/crates/engine/src/mem.rs').read_text()
patch=''.join(difflib.unified_diff(base.splitlines(True),changed.splitlines(True),fromfile='baseline/mem.rs',tofile='candidate/mem.rs'))
assert patch==(S/'mem.patch').read_text()
production=changed.split('#[cfg(test)]',1)[0]
assert production.count('get_key_value(key)')==2
assert production.count('insert_mut(EntryBuffer::from_slices(key, value), ())')==2
assert production.count('validate_entry_lengths(&batch)?;')==2
assert '.get_mut(' not in production and '.get(key)' not in production
for piece in production.split('validate_entry_lengths(&batch)?;')[1:]:
    assert piece.lstrip().startswith('for m in batch.mutations()')
replicated=production[production.index('fn write_applied('):]
assert replicated.index('if let Some')<replicated.index('validate_entry_lengths(&batch)?;')
assert 'map(|(entry, _)| entry.key().to_vec())' in production
assert production.count('entry.value().to_vec()')==6
assert production.count('entry.key().iter().chain(entry.value().iter())')==1

# Verify selected upstream sources against their Cargo.lock-pinned archives,
# not merely the registry package identifier reported by Cargo.
packages=tomllib.loads((R/'Cargo.lock').read_text())['package'];registry=[]
metadata=load(S/'candidate-metadata.json')
for name in ('archery','rpds','triomphe'):
    p=next(p for p in metadata['packages'] if p['name']==name)
    lock=next(p2 for p2 in packages if p2['name']==name and p2['version']==p['version'])
    archive=next((Path.home()/'.cargo/registry/cache').glob(f'*/{name}-{p["version"]}.crate'))
    assert sha(archive)==lock['checksum']
    root=Path(p['manifest_path']).parent;count=0
    with tarfile.open(archive,'r:gz') as tar:
        for member in tar:
            if not member.isfile():continue
            relative=Path(*Path(member.name).parts[1:])
            if relative.parts[0]=='src' or relative.name=='Cargo.toml':
                assert tar.extractfile(member).read()==(root/relative).read_bytes(),relative
                count+=1
    registry.append(dict(name=name,version=p['version'],archive_sha256=sha(archive),source_files_checked=count))

obs=layout['observation'];assert obs==load(S/label/'observations.json')
assert (obs['entry_buffer_bytes'],obs['entry_buffer_alignment'],obs['candidate_key_value_pair_bytes'],obs['baseline_key_value_pair_bytes'])==(24,8,24,48)
assert len(obs['rows'])==198 and not obs['elapsed_time_recorded']
expected_cells={(k,v,op) for k in [0,1,27,35,39,40,41,128,136,1024,4096] for v in [0,1,7,128,4096,65536] for op in ['construct_and_clone','insert','overwrite']}
assert {(r['key_length'],r['value_length'],r['operation']) for r in obs['rows']}==expected_cells
for row in obs['rows']:
    k,v=row['key_length'],row['value_length'];total=k+v
    buffers=int(k>0)+int(v>0);combined=int(total>0)
    if row['operation']=='construct_and_clone':
        assert row['counts']==row['clone_counts']==[combined,total,0,0,0,0]
        assert row['baseline_counts']==[buffers,total,0,0,0,0]
        assert row['net_requested_bytes']==row['extra_requested_bytes']==total
        continue
    b=row['baseline_counts'];c=row['candidate_counts']
    # The actual one-entry insertion requests an Entry and tree Node. Overwrite
    # replaces just the Entry; allocation/free request totals are paired.
    objects=2 if row['operation']=='insert' else 1
    bb=total+56*objects;cb=bb-24
    assert b[:4]==[buffers+objects,bb,0,0]
    assert c[:4]==[combined+objects,cb,0,0]
    assert row['baseline_extra_requested_bytes']==bb
    assert row['candidate_extra_requested_bytes']==cb
    if row['operation']=='insert':
        assert b[4:]==c[4:]==[0,0]
        assert row['baseline_net_requested_bytes']==bb
        assert row['candidate_net_requested_bytes']==cb
    else:
        assert b[:2]==b[4:] and c[:2]==c[4:]
        assert row['baseline_net_requested_bytes']==row['candidate_net_requested_bytes']==0
assert sha(Path(layout['binary']['path']))==layout['binary']['sha256']
text=(S/label/'disassembly.txt').read_text()
start=text.index('<kv9_entry_buffer_layout::construct>:')
constructor=text[start:text.index('\n\n',start)]
first_call=constructor.index('\tcall')
assert re.search(r'\tadd\s+',constructor[:first_call])
assert re.search(r'\tjs\s+',constructor[:first_call])
assert re.search(r'\tje\s+',constructor[:first_call])
assert 'memcpy@GLIBC_2.14' in constructor and 'into_boxed_slice' in constructor
(S/label/'constructor.txt').write_text(constructor+'\n')
result=dict(complete=True,selected_layout=label,tests_passed=408,tests_ignored=50,ignored_per_arm=25,
            conditional_theorems=23,rejecting_controls=13,allocation_rows=198,
            generated_live_states_per_arm=240,generated_old_views_per_arm=1080,
            additional_payload_model_tests_per_arm=2,registry_sources=registry,
            source_mapping='Strict reviewed patch; original engine/common bytes otherwise unchanged; same inherited/payload models in both arms',
            constructor='Recorded release wrapper checks the combined length and zero length before its first call. Allocator request counts independently checked; no reallocation in 66 constructor/clone cases.',
            length_limit='New explicit combined-buffer representability preflight; individual public key/value limits unchanged. Unrepresentable batches fail before mutation; replicated-position refusal stays first. Safe slices make unsigned sum overflow unreachable in this recorded constructor; scalar checked_len tests cover overflow.',
            limits='Debug engine/common suites and ordinary-release standalone constructor/map allocation probe. Conditional proof, not verified Rust extraction. No engine timing, database/Redis, new MinIO, ordinary distributed recovery or actual Chaos acceptance.',
            production_promoted=False)
(S/'qualification-audit.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result))
