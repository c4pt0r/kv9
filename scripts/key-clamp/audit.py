#!/usr/bin/env python3
"""Audit changed-source tests/proof and exact allocation equivalence to the control."""
import hashlib
import json
from pathlib import Path
import sys
import tarfile
import tomllib

R=Path(__file__).resolve().parents[2];S=Path(sys.argv[1]).resolve()
Q=Path('/mnt/data/kv9-work/entry-buffer-qualification-20260916-first')
load=lambda p:json.loads(p.read_text());sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
tests=load(S/'tests-summary.json');layout=load(S/'layout-summary.json');proof=load(S/'proof/result.json')
assert tests['complete'] and layout['complete'] and proof['accepted']
assert len(proof['theorems'])==26 and len(proof['controls'])==15
for p,h in {**load(S/'sources.json'),**layout['sources']}.items():assert sha(Path(p))==h,p
row=tests['arms']['candidate'];assert row['complete']
assert sum(r['passed'] for r in row['results'])==206 and sum(r['ignored'] for r in row['results'])==25
for elf in row['binaries']:assert sha(Path(elf['path']))==elf['sha256']
for name in ('inline_key_model.rs','entry_buffer_model.rs'):
    assert (S/'candidate/crates/engine/tests'/name).read_bytes()==(Q/'candidate/crates/engine/tests'/name).read_bytes()
changes=[]
for p in (Q/'candidate').rglob('*'):
    if not p.is_file():continue
    rel=p.relative_to(Q/'candidate');new=S/'candidate'/rel
    if p.read_bytes()!=new.read_bytes():
        assert str(rel)=='crates/engine/src/entry_buffer.rs'
        assert new.read_text()==p.read_text().replace('&self.bytes[..self.key_len]','&self.bytes[..self.key_len.min(self.bytes.len())]')
        changes.append(str(rel))
assert changes==['crates/engine/src/entry_buffer.rs']
assert (S/'layout-source/entry_buffer.rs').read_bytes()==(S/'candidate/crates/engine/src/entry_buffer.rs').read_bytes()
assert layout['observation']==load(Q/'layout-summary.json')['observation']==load(S/'layout/observations.json')
assert len(layout['observation']['rows'])==198 and not layout['observation']['elapsed_time_recorded']
assert sha(Path(layout['binary']['path']))==layout['binary']['sha256']
packages=tomllib.loads((R/'Cargo.lock').read_text())['package'];registry=[]
metadata=load(S/'candidate-metadata.json')
for name in ('archery','rpds','triomphe'):
    package=next(p for p in metadata['packages'] if p['name']==name)
    lock=next(p for p in packages if p['name']==name and p['version']==package['version'])
    archive=next((Path.home()/'.cargo/registry/cache').glob(f'*/{name}-{package["version"]}.crate'))
    assert sha(archive)==lock['checksum'];root=Path(package['manifest_path']).parent;count=0
    with tarfile.open(archive,'r:gz') as tar:
        for member in tar:
            if not member.isfile():continue
            rel=Path(*Path(member.name).parts[1:])
            if rel.parts[0]=='src' or rel.name=='Cargo.toml':
                assert tar.extractfile(member).read()==(root/rel).read_bytes();count+=1
    registry.append(dict(name=name,version=package['version'],archive_sha256=sha(archive),source_files_checked=count))
result=dict(complete=True,new_candidate_tests_passed=206,existing_ignored=25,reused_tests={'baseline':202,'single_buffer_control':206},reused_ignored_per_arm=25,
            old_control_tests_reexecuted=False,conditional_theorems=26,rejecting_controls=15,
            allocation_rows=198,allocation_observations_exactly_equal_to_control=True,source_changes=changes,
            generated_live_states=240,generated_old_views=1080,payload_model_tests=2,registry_sources=registry,
            limits='Source-bound conditional refinement, not verified Rust extraction. No new timing, database/Redis, MinIO/recovery/Chaos acceptance or production promotion.')
(S/'qualification-audit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
