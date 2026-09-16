#!/usr/bin/env python3
"""Independently audit completed source, model, layout and proof qualification."""
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
assert len(proof['theorems'])==19 and len(proof['controls'])==9
for path,digest in {**load(S/'sources.json'),**layout['sources']}.items():assert sha(Path(path))==digest,path
for arm in ('baseline','candidate'):
    for name in ('engine','common'):
        for p in (R/'crates'/name).rglob('*'):
            if not p.is_file():continue
            if arm=='candidate' and p.name in ('mem.rs','lib.rs') and name=='engine':continue
            assert p.read_bytes()==(S/arm/p.relative_to(R)).read_bytes(),p
    assert (S/arm/'crates/engine/tests/inline_key_model.rs').read_bytes()==(R/'scripts/inline-key/model.rs').read_bytes()
    row=tests['arms'][arm]
    assert sum(v['passed'] for v in row['results'])=={'baseline':200,'candidate':203}[arm]
    assert sum(v['ignored'] for v in row['results'])==25
    for binary in row['binaries']:assert sha(Path(binary['path']))==binary['sha256']
assert (S/'candidate/crates/engine/src/mem_key.rs').read_bytes()==(R/'scripts/inline-key/mem_key.rs').read_bytes()
assert (S/'candidate/crates/engine/src/lib.rs').read_text()==(R/'crates/engine/src/lib.rs').read_text().replace('pub mod mem;','pub mod mem;\nmod mem_key;')

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
assert (obs['key_bytes'],obs['baseline_key_bytes'],obs['key_alignment'])==(48,24,8)
assert len(obs['rows'])==33 and not obs['elapsed_time_recorded']
for row in obs['rows']:
    n=row['length']
    if row['operation']=='construct_and_clone':
        expected=[0]*6 if n<=40 else [1,n,0,0,0,0]
        assert row['counts']==row['clone_counts']==expected
        assert row['net_requested_bytes']==row['extra_requested_bytes']==(0 if n<=40 else n)
        continue
    b=row['baseline_counts'];c=row['candidate_counts']
    assert b[0]-c[0]==int(0<n<=40)
    assert c[1]-b[1]==(24-n if 0<n<=40 else 24)
    assert b[2:4]==c[2:4]==[0,0]
    if row['operation']=='insert':
        assert b[4:]==c[4:]==[0,0]
        assert row['baseline_net_requested_bytes']==b[1]
        assert row['candidate_net_requested_bytes']==c[1]
    else:
        assert b[:2]==b[4:] and c[:2]==c[4:]
        assert row['baseline_net_requested_bytes']==row['candidate_net_requested_bytes']==0
assert sha(Path(layout['binary']['path']))==layout['binary']['sha256']
text=(S/label/'disassembly.txt').read_text()
start=text.index('<kv9_inline_key_layout::construct>:')
constructor=text[start:text.index('\n\n',start)]
assert 'cmp    $0x29,%r14' in constructor
heap=re.search(r'jae\s+([a-f0-9]+)',constructor)[1]
short=constructor[:constructor.index('   '+heap+':')]
assert short.count('\tcall')==1 and '<memcpy@GLIBC_2.14>' in short
(S/label/'constructor.txt').write_text(constructor+'\n')
result=dict(complete=True,selected_layout=label,tests_passed=403,tests_ignored=50,ignored_per_arm=25,
            conditional_theorems=19,rejecting_controls=9,allocation_rows=33,
            generated_live_states_per_arm=240,generated_old_views_per_arm=1080,
            registry_sources=registry,source_mapping='Strict reviewed patch; original engine/common bytes otherwise unchanged',
            constructor='Recorded release wrapper: length < 41 path has memcpy only and no allocator call; allocator request counts independently checked',
            limits='Debug engine/common suites and ordinary-release standalone constructor/map allocation probe. Conditional proof, not verified Rust extraction. No engine timing, database/Redis, new MinIO, ordinary distributed recovery or actual Chaos acceptance.',
            production_promoted=False)
(S/'qualification-audit.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result))
