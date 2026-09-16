#!/usr/bin/env python3
"""Inspect recorded measured ELFs; compare actual insertion bound-failure branches."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

S=Path(sys.argv[1]).resolve();out=S/'codegen-review';out.mkdir(exist_ok=True)
load=lambda p:json.loads(p.read_text());sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
assert load(S/'analysis.json')['complete'] and not load(S/'analysis.json')['stage_one_gate_passed']
build=load(S/'build-summary.json');rows={}
for arm in ('baseline','candidate'):
    binary=build['arms'][arm]['timing'];assert sha(Path(binary['path']))==binary['sha256']
    root=S/('build-timing-'+arm);path=root/'disassembly.txt'
    if not path.exists():
        with path.open('x') as f:subprocess.run(['objdump','-Cd',binary['path']],stdout=f,check=True,timeout=30)
    text=path.read_text();a=text.index('<rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins>:');function=text[a:text.index('\n\n',a)]
    (root/'insert.txt').write_text(function+'\n')
    a=function.index('mov    %rax,(%rcx)')+len('mov    %rax,(%rcx)');b=function.index('<memcmp@GLIBC_2.2.5>')+len('<memcmp@GLIBC_2.2.5>')
    region=function[a:b].strip();(out/(arm+'-comparison.txt')).write_text(region+'\n')
    branches=re.findall(r'^\s*([a-f0-9]+):.*\tja\s+([a-f0-9]+)',region,re.M)
    rows[arm]=dict(elf_sha256=binary['sha256'],insert_sha256=sha(root/'insert.txt'),comparison_bounds_branches=branches)
assert len(rows['baseline']['comparison_bounds_branches'])==0 and len(rows['candidate']['comparison_bounds_branches'])==2
result=dict(complete=True,observed=rows,conclusion='The actual candidate insertion comparison retains two unsigned key_len > buffer_len failure branches before memcmp; the baseline comparison has none. This is a static code-generation difference, not a measured causal attribution of the failed timing gate.',next_hypothesis='Use safe key_len.min(bytes.len()) in the key accessor only. Valid private entries already satisfy key_len <= bytes.len(), so this may express the range proof to LLVM without adding unsafe access. First check actual engine codegen; qualify the source invariant and model before any timing. Keep the full preflight and value accessor unchanged.')
p=out/'review.json';encoded=json.dumps(result,indent=2)+'\n'
if p.exists():assert p.read_text()==encoded
else:p.write_text(encoded)
print(json.dumps({'complete':True,'failure_branches':{'baseline':0,'candidate':2},'measured_causal_attribution':False}))
