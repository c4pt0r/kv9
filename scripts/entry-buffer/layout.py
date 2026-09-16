#!/usr/bin/env python3
"""Inspect the qualified single-buffer constructor and private map allocation requests."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

R=Path(__file__).resolve().parents[2];S=Path(sys.argv[1]).resolve()
label=sys.argv[2] if len(sys.argv)>2 else 'layout'
assert label.replace('-','').isalnum()
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
assert json.loads((S/'tests-summary.json').read_text())['complete']
assert (R/'scripts/entry-buffer/entry_buffer.rs').read_bytes()==(S/'candidate/crates/engine/src/entry_buffer.rs').read_bytes()
work=S/(label+'-source');work.mkdir(exist_ok=False)
out=S/label;out.mkdir(exist_ok=False)
for source,name in [(R/'scripts/entry-buffer/layout.rs','main.rs'),(R/'scripts/entry-buffer/entry_buffer.rs','entry_buffer.rs'),
                    (R/'scripts/engine-interface-counting/src/counting.rs','counting.rs')]:
    shutil.copyfile(source,work/name)
(work/'Cargo.toml').write_text('''[package]
name = "kv9-entry-buffer-layout"
version = "0.0.0"
edition = "2021"
publish = false
[workspace]
[[bin]]
name = "kv9-entry-buffer-layout"
path = "main.rs"
[profile.release]
lto = "thin"
codegen-units = 1
[dependencies]
rpds = "=1.2.1"
serde_json = "1"
tikv-jemallocator = { version = "=0.6.1", default-features = false }
''')
shutil.copyfile(R/'scripts/engine-interface-counting/Cargo.lock',work/'Cargo.lock')
os.environ['CARGO_TARGET_DIR']=str(R/'target');os.environ['CARGO_BUILD_JOBS']='4'
assert not any(os.environ.get(k) for k in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CFLAGS'))
with (out/'metadata.json').open('x') as stdout,(out/'metadata.stderr').open('x') as stderr:
    subprocess.run(['cargo','metadata','--offline','--format-version','1'],cwd=work,stdout=stdout,stderr=stderr,check=True,timeout=120)
sources={str(p):sha(p) for p in work.iterdir() if p.is_file()}
sources[str(Path(__file__).resolve())]=sha(Path(__file__).resolve())
(out/'sources.json').write_text(json.dumps(sources,indent=2)+'\n')
spec=importlib.util.spec_from_file_location('cache',R/'scripts/build_cache.py');cache=importlib.util.module_from_spec(spec);spec.loader.exec_module(cache)
result=dict(complete=False,started_ns=time.time_ns(),sources=sources)
try:
    with cache.BuildCache(work,out,True,sources) as build:
        with (out/'clippy.stdout').open('x') as stdout,(out/'clippy.stderr').open('x') as stderr:
            build.run(['cargo','clippy','--offline','--locked','--release','--all-targets','--','-D','warnings'],stdout=stdout,stderr=stderr)
        with (out/'cargo.jsonl').open('x') as stdout,(out/'cargo.stderr').open('x') as stderr:
            build.run(['cargo','build','--offline','--locked','--release','--message-format','json-render-diagnostics'],stdout=stdout,stderr=stderr)
        build.check_artifacts(out/'cargo.jsonl')
        elf=out/'layout';shutil.copyfile(R/'target/release/kv9-entry-buffer-layout',elf);elf.chmod(0o755)
        result['binary']=dict(path=str(elf),sha256=sha(elf),bytes=elf.stat().st_size)
    with (out/'observations.json').open('x') as stdout,(out/'observations.stderr').open('x') as stderr:
        subprocess.run([str(elf)],stdout=stdout,stderr=stderr,check=True,timeout=30)
    observation=json.loads((out/'observations.json').read_text());assert observation['complete'] and not observation['elapsed_time_recorded']
    with (out/'disassembly.txt').open('x') as stdout:
        subprocess.run(['objdump','-Cd',str(elf)],stdout=stdout,check=True,timeout=60)
    assert all(sha(Path(p))==h for p,h in sources.items())
    result.update(complete=True,observation=observation)
except BaseException as error:
    result['failure']=repr(error);raise
finally:
    result['ended_ns']=time.time_ns();(S/(label+'-summary.json')).write_text(json.dumps(result,indent=2)+'\n')
print(json.dumps({'complete':True,'candidate_pair_bytes':observation['candidate_key_value_pair_bytes'],'baseline_pair_bytes':observation['baseline_key_value_pair_bytes'],'rows':len(observation['rows'])}))
