#!/usr/bin/env python3
"""Recompute the published arithmetic from hash-bound archived reporting inputs."""
import hashlib,io,json,os,subprocess,sys,tarfile,tempfile
from pathlib import Path

def main():
    root=Path(__file__).resolve().parent
    index=json.loads((root/'inventory.json').read_text())
    expected=json.loads((root/'summary.json').read_text())
    sources=set(expected['input_bindings'])
    sources.add(expected['audit_path'])
    sources.add(str(Path(expected['audit_path']).with_name('input-inventory.json')))
    wanted={name:row for name,row in index['files'].items() if row['source'] in sources}
    assert len(wanted)==len(sources)==74
    assert sum(row['bytes'] for row in wanted.values())<512*1024**2
    data=[]
    assert len(index['parts'])<=256
    for row in index['parts']:
        assert row['name']==f'evidence.tar.gz.{len(data)+1:03d}' and 0<row['bytes']<=2*1024**2
        p=root/row['name'];assert not p.is_symlink()
        b=p.read_bytes();assert len(b)==row['bytes'] and hashlib.sha256(b).hexdigest()==row['sha256']
        data.append(b)
    assert sum(map(len,data))==index['compressed_bytes']<=512*1024**2
    env=dict(os.environ,PYTHONDONTWRITEBYTECODE='1',PYTHONOPTIMIZE='0')
    with tempfile.TemporaryDirectory(prefix='kv9-write-report-recompute-') as temporary:
        destination=Path(temporary);seen=set()
        with tarfile.open(fileobj=io.BytesIO(b''.join(data)),mode='r:gz') as tar:
            for member in tar:
                if member.name not in wanted:continue
                assert member.name not in seen and member.isfile()
                row=wanted[member.name];assert member.size==row['bytes']
                b=tar.extractfile(member).read();assert len(b)==row['bytes'] and hashlib.sha256(b).hexdigest()==row['sha256']
                relative=Path(row['source']).relative_to('/')
                assert relative.parts[0]=='tmp' and all(x not in ('.','..') for x in relative.parts)
                p=destination/relative;p.parent.mkdir(parents=True,exist_ok=True)
                with p.open('xb') as out:out.write(b)
                seen.add(member.name)
        assert seen==set(wanted)
        command=[sys.executable,'-B',str(root/'summarize.py'),'--root',str(destination),'--output',str(destination/'result')]
        subprocess.run(command,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=True,timeout=60)
        assert (destination/'result/summary.json').read_bytes()==(root/'summary.json').read_bytes()
    print('PASS: identical pooled/per-repeat rates, latency bounds, outcomes and sampled CPU from 74 archived inputs; no workload or WAL replay')

if __name__=='__main__':main()
