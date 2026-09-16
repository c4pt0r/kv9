#!/usr/bin/env python3
"""Prepare the changed accessor candidate and bind reusable control evidence."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

R=Path(__file__).resolve().parents[2];S=Path(sys.argv[1]).resolve();S.mkdir(exist_ok=False)
Q=Path('/mnt/data/kv9-work/entry-buffer-qualification-20260916-first')
P=Path('/mnt/data/kv9-work/entry-buffer-performance-20260916-first')
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
load=lambda p:json.loads(p.read_text())
assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=R,text=True).strip()=='b23506dc5ee43d52b3a82a0c7f2b3d5abe0003ad'
plan=load(P/'next-key-clamp-plan.json');shutil.copyfile(P/'next-key-clamp-plan.json',S/'declared-plan.json')
for p,h in load(Q/'sources.json').items():assert sha(Path(p))==h,p
old_tests=load(Q/'tests-summary.json');assert old_tests['complete']
for arm in ('baseline','candidate'):
    assert old_tests['arms'][arm]['complete']
    for elf in old_tests['arms'][arm]['binaries']:assert sha(Path(elf['path']))==elf['sha256']
source=Path(plan['candidate_root']);shutil.copytree(source,S/'candidate')
changes=[]
for p in (Q/'candidate').rglob('*'):
    if not p.is_file():continue
    relative=p.relative_to(Q/'candidate');candidate=S/'candidate'/relative
    if p.read_bytes()!=candidate.read_bytes():
        assert str(relative)=='crates/engine/src/entry_buffer.rs'
        assert candidate.read_text()==p.read_text().replace('&self.bytes[..self.key_len]','&self.bytes[..self.key_len.min(self.bytes.len())]')
        changes.append(str(relative))
assert changes==['crates/engine/src/entry_buffer.rs']
assert sha(S/'candidate/crates/engine/src/entry_buffer.rs')==plan['candidate_key_sha256']
with (S/'candidate-metadata.json').open('x') as stdout,(S/'candidate-metadata.stderr').open('x') as stderr:
    subprocess.run(['cargo','metadata','--offline','--locked','--format-version','1'],cwd=S/'candidate',stdout=stdout,stderr=stderr,check=True,timeout=120)
inputs=load(Q/'sources.json')
for p in [Q/'tests-summary.json',Q/'qualification-audit.json',Q/'layout-summary.json',P/'post-screen-audit.json',P/'key-clamp-codegen/result.json',P/'next-key-clamp-plan.json']:
    inputs[str(p)]=sha(p)
sources=dict(inputs)
for p in (S/'candidate').rglob('*'):
    if p.is_file():sources[str(p)]=sha(p)
for p in Path(__file__).parent.glob('*.py'):sources[str(p.resolve())]=sha(p)
(S/'sources.json').write_text(json.dumps(sources,indent=2)+'\n')
result=dict(complete=True,qualified_source_roots={'baseline':str(Q/'baseline'),'single_buffer_control':str(Q/'candidate'),'clamped_candidate':str(S/'candidate')},
            source_changes=changes,reused_test_evidence={'baseline':202,'single_buffer_control':206},reused_ignored_per_arm=25,
            new_candidate_tests='pending',old_tests_reexecuted=False,candidate_key_sha256=plan['candidate_key_sha256'],
            limits='Exact unchanged control source/evidence reuse; changed candidate requires new tests, source-bound proof and allocator qualification. No timing yet.')
(S/'preparation.json').write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps(result))
