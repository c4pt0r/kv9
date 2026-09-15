#!/usr/bin/env python3
"""Independent portable byte readback and complete-history population check; no extraction or live acceptance."""
import hashlib,io,json,tarfile
from collections import Counter
from pathlib import Path,PurePosixPath
M=1024**2
MAX_MEMBER=128*M;MAX_BYTES=512*M;MAX_FILES=10000
REV='a6ac335ef4567f2e1a2b33da1a723d694cd5b7d9'
def safe(name):
 p=PurePosixPath(name)
 return bool(name)and not p.is_absolute()and str(p)==name and all(x not in('.','..')for x in p.parts)and '\\'not in name
def checked(data,expected,name):
 if len(data)!=expected['bytes']or hashlib.sha256(data).hexdigest()!=expected['sha256']:raise ValueError('byte identity differs: '+name)
def main():
 root=Path(__file__).resolve().parent
 index=json.loads((root/'inventory.json').read_text());summary=json.loads((root/'summary.json').read_text())
 if index['source_revision']!=REV or summary['revision']!=REV or not summary['complete']:raise ValueError('revision/completion differs')
 if not 0<len(index['files'])<=MAX_FILES or len(index['parts'])>256:raise ValueError('file/part bound exceeded')
 if sum(v['bytes']for v in index['files'].values())>MAX_BYTES or any(type(v['bytes'])is not int or not 0<=v['bytes']<=MAX_MEMBER for v in index['files'].values()):raise ValueError('member/aggregate bound exceeded')
 for name,expected in index['top_level'].items():
  if not safe(name)or len(PurePosixPath(name).parts)!=1:raise ValueError('unsafe top-level path')
  checked((root/name).read_bytes(),expected,name)
 parts=[];whole=hashlib.sha256();stored=0
 for number,part in enumerate(index['parts'],1):
  if part['name']!=f'evidence.tar.gz.{number:03d}'or not 0<part['bytes']<=2*M:raise ValueError('unexpected archive part')
  data=(root/part['name']).read_bytes();checked(data,part,part['name']);whole.update(data);stored+=len(data)
  if stored>MAX_BYTES:raise ValueError('compressed bound exceeded')
  parts.append(data)
 if stored!=index['compressed_bytes']or whole.hexdigest()!=index['compressed_sha256']:raise ValueError('whole compressed identity differs')
 histories={};seen=set();total=0
 artifact=Path(summary['full_archive']['roots'][0])
 if not str(artifact).startswith('/tmp/kv9-chaos-e2e.'):raise ValueError('unexpected original artifact')
 expected_paths={str(artifact/rel):label for label,rel in [('cli','history.jsonl'),('persistent','persistent-run/history.jsonl'),('native','native-run/history.jsonl')]}
 with tarfile.open(fileobj=io.BytesIO(b''.join(parts)),mode='r:gz')as archive:
  for member in archive:
   if not safe(member.name)or not member.isfile()or member.name in seen or member.name not in index['files']:raise ValueError('unexpected/unsafe member: '+member.name)
   expected=index['files'][member.name]
   if member.size!=expected['bytes']:raise ValueError('member size differs: '+member.name)
   label=expected_paths.get(expected['source']);digest=hashlib.sha256();count=0
   with archive.extractfile(member)as stream:
    if label is None:
     while True:
      data=stream.read(M)
      if not data:break
      digest.update(data);count+=len(data)
    else:
     if label in histories:raise ValueError('duplicate original history')
     invokes=set();returns=set();outcomes=Counter()
     for line in stream:
      digest.update(line);count+=len(line);event=json.loads(line)
      if event.get('type')not in('invoke','return'):continue
      identity=json.dumps(event['id'],sort_keys=True,separators=(',',':'))
      target=invokes if event['type']=='invoke'else returns
      if identity in target:raise ValueError('duplicate history event identity')
      target.add(identity)
      if event['type']=='return':outcomes[event['outcome']]+=1
     if invokes!=returns:raise ValueError('incomplete invocation/return population')
     histories[label]=dict(bytes=count,sha256=digest.hexdigest(),invokes=len(invokes),returns=len(returns),outcomes=dict(outcomes))
   if count!=member.size or digest.hexdigest()!=expected['sha256']:raise ValueError('member byte identity differs: '+member.name)
   seen.add(member.name);total+=count
 if seen!=set(index['files'])or len(seen)!=index['decoded_member_count']or total!=index['decoded_member_bytes']:raise ValueError('missing or mismatched inventory')
 if histories!=summary['histories']:raise ValueError('original complete-history populations differ')
 outcomes=Counter()
 for h in histories.values():outcomes.update(h['outcomes'])
 if dict(outcomes)!=summary['outcomes']or sum(outcomes.values())!=summary['operations']:raise ValueError('combined history outcome accounting differs')
 print(json.dumps(dict(complete=True,members=len(seen),parts=len(parts),decoded_bytes=total,history_operations=summary['operations'],outcomes=dict(outcomes),scope='Independent byte identity and original complete-history population accounting. Original semantic checker reports are bound; no live/source/ELF/WAL or cluster replay.')))
if __name__=='__main__':main()
