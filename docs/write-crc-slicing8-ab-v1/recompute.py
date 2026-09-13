#!/usr/bin/env python3
"""Draft: replay exact published summary arithmetic from bounded archived metadata.

Root must review and execute only after actual timing/audit/summary terminals.
No workload, campaign auditor, original binary, or WAL decoder is invoked.
"""
import ast,hashlib,io,json,os,sys,tarfile,tempfile,types
from pathlib import Path,PurePosixPath

CAP=512*1024**2
MEMBER_CAP=64*1024**2
ORIGINAL_HERE='/tmp/kv9-write-crc-slicing8-ab-summary-preparation-first'
def safe(name):
 p=PurePosixPath(name)
 return bool(name) and not p.is_absolute() and str(p)==name and all(x not in ('.','..') for x in p.parts) and '\\' not in name
def checked(data,pin):
 assert len(data)==pin['bytes'] and hashlib.sha256(data).hexdigest()==pin['sha256']
def main():
 assert __debug__ and os.sysconf('SC_CLK_TCK')==100
 root=Path(__file__).resolve().parent;index=json.loads((root/'inventory.json').read_text())
 def top(name):
  assert safe(name) and '/' not in name and index['top_level'][name]['bytes']<=MEMBER_CAP
  path=root/name;assert path.is_file() and not path.is_symlink()
  data=path.read_bytes();checked(data,index['top_level'][name]);return data
 expected_bytes=top('summary.json');expected=json.loads(expected_bytes)
 bindings=json.loads(top('summary-input-hashes.json'));replay=json.loads(top('replay-inputs.json'))
 original=top('summarize.py').decode()
 assert replay['original_summary_directory']==ORIGINAL_HERE
 assert expected['complete'] is True and expected['timed_cohorts']==16 and expected['smoke_cohorts']==8
 assert expected['performance_promotion'] is False
 sources=set(bindings);wanted={n:r for n,r in index['files'].items() if r['source'] in sources}
 assert len(wanted)==len(sources) and sum(r['bytes'] for r in wanted.values())<=CAP
 assert all(dict(bytes=r['bytes'],sha256=r['sha256'])==bindings[r['source']] for r in wanted.values())
 terminal=replay['audit_terminal_path'];assert terminal in bindings
 parts=[];whole=hashlib.sha256();total=0
 assert 0<len(index['parts'])<=256
 for number,row in enumerate(index['parts'],1):
  assert row['name']==f'evidence.tar.gz.{number:03d}' and 0<row['bytes']<=2*1024**2
  p=root/row['name'];assert p.is_file() and not p.is_symlink()
  data=p.read_bytes();checked(data,row);total+=len(data);assert total<=CAP
  whole.update(data);parts.append(data)
 assert total==index['compressed_bytes'] and whole.hexdigest()==index['compressed_sha256']
 assert 0<index['decoded_member_bytes']<=index['limits']['decoded_member_bytes']<=CAP
 # Three exact I/O substitutions. Every arithmetic function and the remaining
 # main validation/arithmetic source stays original, with original path labels.
 replacements={
  'HERE=Path(__file__).resolve().parent':f'HERE=Path({ORIGINAL_HERE!r})',
  'assert args.output.is_absolute()and args.output.parent==HERE;args.output.mkdir(exist_ok=False)':
   'assert args.output.is_absolute()and args.output.parent==REPLAY_ROOT;args.output.mkdir(exist_ok=False)',
  "p=Path(p);assert p.is_file()and not p.is_symlink()and p.stat().st_size<=CAP\n  data=p.read_bytes();":
   "p=Path(p);assert p.is_absolute() and p.parts[1]=='tmp' and '..' not in p.parts\n  actual=REPLAY_ROOT/str(p).lstrip('/');assert actual.is_file()and not actual.is_symlink()and actual.stat().st_size<=CAP\n  data=actual.read_bytes();",
 }
 adapted=original
 for old,new in replacements.items():
  assert adapted.count(old)==1,('original I/O block differs',old)
  adapted=adapted.replace(old,new)
 def helpers(source):
  return {n.name:ast.get_source_segment(source,n) for n in ast.parse(source).body if isinstance(n,ast.FunctionDef) and n.name!='main'}
 assert helpers(original)==helpers(adapted)
 with tempfile.TemporaryDirectory(prefix='kv9-crc-ab-report-recompute-') as temporary:
  destination=Path(temporary);seen=set();seen_all=set();decoded=0
  with tarfile.open(fileobj=io.BytesIO(b''.join(parts)),mode='r:gz') as archive:
   for member in archive:
    assert safe(member.name) and member.isfile() and member.name not in seen_all and member.name in index['files']
    seen_all.add(member.name);row=index['files'][member.name]
    assert member.size==row['bytes'] and 0<=member.size<=MEMBER_CAP
    decoded+=member.size;assert decoded<=index['limits']['decoded_member_bytes']
    if member.name not in wanted:continue
    with archive.extractfile(member) as f:data=f.read(member.size+1)
    checked(data,row);relative=PurePosixPath(row['source'])
    assert relative.is_absolute() and relative.parts[1]=='tmp' and '..' not in relative.parts
    assert not any(x in relative.parts for x in ('.git','target','retention-objects','data','store-existing','store-extended'))
    p=destination/str(relative).lstrip('/');p.parent.mkdir(parents=True,exist_ok=True)
    with p.open('xb') as f:f.write(data)
    seen.add(member.name)
  assert seen==set(wanted) and seen_all==set(index['files']) and decoded==index['decoded_member_bytes']
  module=types.ModuleType('published_crc_ab_summary');module.__file__=str(root/'summarize.py')
  exec(compile(adapted,str(root/'summarize.py'),'exec'),module.__dict__)
  module.REPLAY_ROOT=destination
  argv=[str(root/'summarize.py'),'--output',str(destination/'recomputed'),
        '--audit-sha256',expected['audit_sha256'],'--input-inventory-sha256',expected['audit_input_inventory_sha256'],
        '--timing-session',str(expected['actual_timing_session']),'--audit-terminal',terminal,
        '--audit-terminal-sha256',bindings[terminal]['sha256']]
  prior=sys.argv
  try:sys.argv=argv;module.main()
  finally:sys.argv=prior
  assert (destination/'recomputed/summary.json').read_bytes()==expected_bytes
  assert json.loads((destination/'recomputed/input-hashes.json').read_text())==bindings
 print('PASS: exact original summary arithmetic and input hashes from archived metadata; no live/source/WAL acceptance')
if __name__=='__main__':main()
