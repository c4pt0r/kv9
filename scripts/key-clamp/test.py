#!/usr/bin/env python3
"""Build and execute the changed candidate suite; retain unchanged controls without rerun."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

R=Path(__file__).resolve().parents[2]
S=Path(sys.argv[1]).resolve()
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
sources=json.loads((S/'sources.json').read_text())
assert all(sha(Path(p))==h for p,h in sources.items())
assert not any(os.environ.get(k) for k in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CFLAGS'))
os.environ['CARGO_TARGET_DIR']=str(R/'target')
os.environ['CARGO_BUILD_JOBS']='4'
spec=importlib.util.spec_from_file_location('cache',R/'scripts/build_cache.py')
cache=importlib.util.module_from_spec(spec);spec.loader.exec_module(cache)
result=dict(complete=False,started_ns=time.time_ns(),arms={},sources=sources)
try:
    for arm in ('candidate',):
        work=S/arm;out=S/('tests-'+arm);out.mkdir(exist_ok=False)
        row=dict(complete=False,binaries=[],results=[]);result['arms'][arm]=row
        with cache.BuildCache(work,out,False,sources) as build:
            with (out/'clippy.stdout').open('x') as stdout,(out/'clippy.stderr').open('x') as stderr:
                build.run(['cargo','clippy','--offline','--locked','--workspace','--all-targets','--','-D','warnings'],stdout=stdout,stderr=stderr)
            with (out/'cargo.jsonl').open('x') as stdout,(out/'cargo.stderr').open('x') as stderr:
                build.run(['cargo','test','--offline','--locked','--workspace','--all-targets','--no-run','--message-format','json-render-diagnostics'],stdout=stdout,stderr=stderr)
            build.check_artifacts(out/'cargo.jsonl')
            units=[json.loads(line) for line in (out/'cargo.jsonl').read_text().splitlines()]
            for name in ('rpds','archery','triomphe'):
                selected=[u for u in units if u.get('reason')=='compiler-artifact' and u['target']['name']==name]
                assert len(selected)==1 and 'registry+' in selected[0]['package_id'],name
            for u in units:
                if u.get('reason')!='compiler-artifact' or not u.get('executable') or u['package_id'] not in build.packages:continue
                elf=out/Path(u['executable']).name;shutil.copyfile(u['executable'],elf);elf.chmod(0o755)
                row['binaries'].append(dict(path=str(elf),sha256=sha(elf),bytes=elf.stat().st_size))
        assert row['binaries']
        for binary in row['binaries']:
            elf=Path(binary['path'])
            with (out/(elf.name+'.stdout')).open('x') as stdout,(out/(elf.name+'.stderr')).open('x') as stderr:
                subprocess.run([str(elf),'--test-threads=4'],cwd=work,stdout=stdout,stderr=stderr,check=True,timeout=900)
            lines=[line for line in (out/(elf.name+'.stdout')).read_text().splitlines() if line.startswith('test result:')]
            assert len(lines)==1 and '0 failed' in lines[0]
            counts=re.search(r'(\d+) passed; (\d+) failed; (\d+) ignored',lines[0])
            assert counts
            row['results'].append(dict(binary=elf.name,line=lines[0],passed=int(counts[1]),ignored=int(counts[3])))
        row['complete']=True
        print(json.dumps({'arm':arm,'complete':True,'passed':sum(x['passed'] for x in row['results']),'ignored':sum(x['ignored'] for x in row['results'])}),flush=True)
    assert all(sha(Path(p))==h for p,h in sources.items())
    result['complete']=True
except BaseException as error:
    result['failure']=repr(error);raise
finally:
    result['ended_ns']=time.time_ns()
    (S/'tests-summary.json').write_text(json.dumps(result,indent=2)+'\n')
