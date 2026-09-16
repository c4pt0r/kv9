#!/usr/bin/env python3
"""Build four separately identified engine-interface executables from qualified sources."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import tomllib

R=Path(__file__).resolve().parents[2];S=Path(sys.argv[1]).resolve()
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
load=lambda p:json.loads(p.read_text())
plan=load(S/'plan.json');declared=load(S/'declared-plan.json');Q=Path(plan['qualified_root'])
assert load(Q/'qualification-audit.json')['complete']
assert plan['declared_plan_sha256']==sha(S/'declared-plan.json')
assert plan['orders']==declared['stage_one']['orders'] and len(plan['cases'])==declared['stage_one']['cases']
assert sha(Q/'candidate/crates/engine/src/entry_buffer.rs')==declared['candidate_entry_buffer_sha256']
assert sha(Q/'candidate/crates/engine/src/mem.rs')==declared['candidate_engine_sha256']
qualified=load(Q/'sources.json')
for p,h in qualified.items():assert sha(Path(p))==h,p
correspondence=load(S/'harness-correspondence.json')
for p,h in {**correspondence['origins'],**correspondence['generated']}.items():assert sha(Path(p))==h,p
os.environ['CARGO_TARGET_DIR']=str(R/'target');os.environ['CARGO_BUILD_JOBS']='4'
assert not any(os.environ.get(k) for k in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CFLAGS'))
spec=importlib.util.spec_from_file_location('cache',R/'scripts/build_cache.py')
cache=importlib.util.module_from_spec(spec);spec.loader.exec_module(cache)
registry=lambda p:{(v['name'],v['version'],v['source']):v['checksum'] for v in tomllib.loads(p.read_text())['package'] if v.get('source','').startswith('registry+')}
reference=registry(R/'Cargo.lock')
result=dict(complete=False,started_ns=time.time_ns(),plan_sha256=sha(S/'plan.json'),sources={},arms={'baseline':{},'candidate':{}})
try:
    for profile in ('timing','counting'):
        for arm in ('baseline','candidate'):
            work=S/f'source-{profile}-{arm}';shutil.copytree(S/('harness-'+profile),work)
            out=S/f'build-{profile}-{arm}';out.mkdir()
            manifest=(work/'Cargo.toml').read_text()
            for name in ('engine','common'):
                manifest=manifest.replace('../../crates/'+name,str(Q/arm/'crates'/name))
            (work/'Cargo.toml').write_text(manifest)
            with (out/'metadata.json').open('x') as stdout,(out/'metadata.stderr').open('x') as stderr:
                subprocess.run(['cargo','metadata','--offline','--format-version','1'],cwd=work,stdout=stdout,stderr=stderr,check=True,timeout=120)
            assert all(reference.get(k)==v for k,v in registry(work/'Cargo.lock').items())
            paths=[Path(__file__).resolve(),R/'scripts/entry-buffer-performance/prepare.py',S/'plan.json',S/'declared-plan.json',S/'harness-correspondence.json',work/'Cargo.toml',work/'Cargo.lock',*sorted((work/'src').glob('*.rs'))]
            sources={str(p):sha(p) for p in paths};sources.update(qualified);sources.update(correspondence['generated']);sources.update(correspondence['origins'])
            result['sources'].update(sources)
            (out/'sources.json').write_text(json.dumps(sources,indent=2)+'\n')
            with cache.BuildCache(work,out,True,sources) as build:
                with (out/'dependency-clean.stdout').open('x') as stdout,(out/'dependency-clean.stderr').open('x') as stderr:
                    build.run(['cargo','clean','--offline','--locked','--profile','release','-p','kv9-engine','-p','kv9-common'],stdout=stdout,stderr=stderr)
                with (out/'clippy.stdout').open('x') as stdout,(out/'clippy.stderr').open('x') as stderr:
                    build.run(['cargo','clippy','--offline','--locked','--release','--all-targets','--','-D','warnings'],stdout=stdout,stderr=stderr)
                with (out/'cargo.jsonl').open('x') as stdout,(out/'cargo.stderr').open('x') as stderr:
                    build.run(['cargo','build','--offline','--locked','--release','--message-format','json-render-diagnostics'],stdout=stdout,stderr=stderr)
                build.check_artifacts(out/'cargo.jsonl')
                units=[json.loads(line) for line in (out/'cargo.jsonl').read_text().splitlines()]
                for name in ('kv9_engine','kv9_common','rpds','archery','triomphe'):
                    selected=[u for u in units if u.get('reason')=='compiler-artifact' and u['target']['name']==name]
                    assert len(selected)==1,name
                    if name in ('kv9_engine','kv9_common'):
                        assert not selected[0]['fresh'] and str(Q/arm) in selected[0]['package_id']
                    else:assert 'registry+' in selected[0]['package_id']
                binary=out/'engine-interface'
                shutil.copyfile(R/('target/release/kv9-entry-buffer-'+profile),binary);binary.chmod(0o755)
                result['arms'][arm][profile]=dict(path=str(binary),sha256=sha(binary),bytes=binary.stat().st_size)
            assert all(sha(Path(p))==h for p,h in sources.items())
            print(json.dumps({'profile':profile,'arm':arm,'complete':True,'binary':result['arms'][arm][profile]}),flush=True)
    assert all(sha(Path(p))==h for p,h in result['sources'].items())
    result['complete']=True
except BaseException as error:
    result['failure']=repr(error);raise
finally:
    result['ended_ns']=time.time_ns();(S/'build-summary.json').write_text(json.dumps(result,indent=2)+'\n')
