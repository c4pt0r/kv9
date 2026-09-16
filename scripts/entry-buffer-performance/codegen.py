#!/usr/bin/env python3
"""Inspect one safe accessor change after the failed screen; do not run workloads."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

R=Path(__file__).resolve().parents[2];S=Path(sys.argv[1]).resolve()
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
load=lambda p:json.loads(p.read_text())
assert load(S/'analysis.json')['complete'] and not load(S/'analysis.json')['stage_one_gate_passed']
assert load(S/'codegen-review/review.json')['complete']
Q=Path(load(S/'plan.json')['qualified_root'])
for p,h in load(Q/'sources.json').items():assert sha(Path(p))==h,p
D=S/'key-clamp-codegen';D.mkdir(exist_ok=False)
shutil.copytree(Q/'candidate',D/'candidate')
p=D/'candidate/crates/engine/src/entry_buffer.rs';old=p.read_text()
anchor='&self.bytes[..self.key_len]';assert old.count(anchor)==1
p.write_text(old.replace(anchor,'&self.bytes[..self.key_len.min(self.bytes.len())]'))
work=D/'harness';shutil.copytree(S/'harness-timing',work)
x=(work/'Cargo.toml').read_text().replace('kv9-entry-buffer-timing','kv9-entry-buffer-clamp-codegen')
for name in ('engine','common'):x=x.replace('../../crates/'+name,str(D/'candidate/crates'/name))
(work/'Cargo.toml').write_text(x)
out=D/'build';out.mkdir()
os.environ['CARGO_TARGET_DIR']=str(R/'target');os.environ['CARGO_BUILD_JOBS']='4'
assert not any(os.environ.get(k) for k in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CFLAGS'))
with (out/'metadata.json').open('x') as f:
    subprocess.run(['cargo','metadata','--offline','--format-version','1'],cwd=work,stdout=f,check=True,timeout=120)
sources={str(p):sha(p) for root in (D/'candidate',work) for p in root.rglob('*') if p.is_file()}
sources[str(Path(__file__).resolve())]=sha(Path(__file__).resolve())
(out/'sources.json').write_text(json.dumps(sources,indent=2)+'\n')
spec=importlib.util.spec_from_file_location('cache',R/'scripts/build_cache.py');cache=importlib.util.module_from_spec(spec);spec.loader.exec_module(cache)
with cache.BuildCache(work,out,True,sources) as build:
    with (out/'clean.stdout').open('x') as stdout,(out/'clean.stderr').open('x') as stderr:
        build.run(['cargo','clean','--offline','--locked','--profile','release','-p','kv9-engine','-p','kv9-common'],stdout=stdout,stderr=stderr)
    with (out/'clippy.stdout').open('x') as stdout,(out/'clippy.stderr').open('x') as stderr:
        build.run(['cargo','clippy','--offline','--locked','--release','--all-targets','--','-D','warnings'],stdout=stdout,stderr=stderr)
    with (out/'cargo.jsonl').open('x') as stdout,(out/'cargo.stderr').open('x') as stderr:
        build.run(['cargo','build','--offline','--locked','--release','--message-format','json-render-diagnostics'],stdout=stdout,stderr=stderr)
    build.check_artifacts(out/'cargo.jsonl')
    binary=D/'engine-interface';shutil.copyfile(R/'target/release/kv9-entry-buffer-clamp-codegen',binary)
with (out/'disassembly.txt').open('x') as f:subprocess.run(['objdump','-Cd',str(binary)],stdout=f,check=True,timeout=30)
text=(out/'disassembly.txt').read_text();a=text.index('<rpds::map::red_black_tree_map::Node<K,V,P>::insert::ins>:');function=text[a:text.index('\n\n',a)]
(out/'insert.txt').write_text(function+'\n');prefix=function[:function.index('<memcmp@GLIBC_2.2.5>')]
branches=re.findall(r'\tja\s+([a-f0-9]+)',prefix)
assert not branches,'Accessor control did not eliminate unsigned bound-failure branches'
assert 'cmov' in prefix
for path,h in sources.items():assert sha(Path(path))==h,path
result=dict(complete=True,workloads_executed=0,proof_and_model_qualified=False,production_promoted=False,
            change='One key() expression only: safe min(key_len, buffer_len) before slicing; value accessor and batch preflight unchanged',
            binary=dict(path=str(binary),sha256=sha(binary),bytes=binary.stat().st_size),
            key_source_sha256=sha(p),unsigned_failure_branches_before_first_memcmp=branches,
            limits='Static engine codegen control only. No cause, speedup, semantic qualification or DB result established. Prove the valid-entry invariant and run independent models before timing.')
(D/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
