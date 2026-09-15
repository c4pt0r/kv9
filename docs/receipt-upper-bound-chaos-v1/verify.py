#!/usr/bin/env python3
"""Independent portable member/hash/cap readback; no extraction or correctness-test replay."""
import argparse, collections, hashlib, io, json, tarfile, time
from pathlib import Path, PurePosixPath
def safe(n):
 p=PurePosixPath(n);return bool(n)and not p.is_absolute()and str(p)==n and all(x not in ('.','..')for x in p.parts)and '\\'not in n
def checked(b,v,n):
 if len(b)!=v['bytes']or hashlib.sha256(b).hexdigest()!=v['sha256']:raise ValueError('byte identity differs: '+n)
def main():
 a=argparse.ArgumentParser(description=__doc__);a.add_argument('--output',type=Path,required=True);args=a.parse_args()
 assert __debug__
 assert not args.output.exists()
 root=Path(__file__).resolve().parent;started=time.time_ns();result=dict(complete=False,started_ns=started)
 try:
  raw=(root/'inventory.json').read_bytes();index=json.loads(raw);M=1024**2
  assert index['limits']==dict(member_bytes=16*M,decoded_bytes=64*M,compressed_bytes=32*M,root_allocated_bytes=64*M,part_bytes=2*M,seconds=1200,host_floor_bytes=8*1024**3)
  for n,v in index['top_level'].items():
   assert safe(n)and len(PurePosixPath(n).parts)==1;checked((root/n).read_bytes(),v,n)
  parts=[];compressed=0;h=hashlib.sha256()
  for number,part in enumerate(index['parts'],1):
   assert number<=16 and part['name']==f'evidence.tar.gz.{number:03d}'and 0<part['bytes']<=2*M
   b=(root/part['name']).read_bytes();checked(b,part,part['name']);parts.append(b);h.update(b);compressed+=len(b)
   assert compressed<=32*M
  assert compressed==index['compressed_bytes']and h.hexdigest()==index['compressed_sha256']
  seen=set();decoded=0;summary=json.loads((root/'summary.json').read_text())
  history_names={v['archive_name']:label for label,v in summary['histories'].items()};histories={}
  with tarfile.open(fileobj=io.BytesIO(b''.join(parts)),mode='r:gz')as archive:
   for member in archive:
    assert safe(member.name)and member.isfile()and member.name not in seen and member.name in index['files']
    v=index['files'][member.name];assert 0<=member.size==v['bytes']<=16*M
    decoded+=member.size;assert decoded<=64*M and time.time_ns()-started<1200*10**9
    with archive.extractfile(member)as stream:
     if member.name in history_names:
      b=stream.read();checked(b,v,member.name);assert b.endswith(b'\n')
      pending=set();done=set();invokes=0;returns=0;seq=0;outcomes=collections.Counter()
      for number,line in enumerate(b.splitlines()):
       row=json.loads(line)
       if number==0:assert row['type']=='header';continue
       assert type(row['seq'])is int and row['seq']==seq;seq+=1
       ident=row['id'];assert type(ident)is int and ident>=0
       if row['type']=='invoke':assert ident not in pending and ident not in done;pending.add(ident);invokes+=1
       else:
        assert row['type']=='return'and ident in pending;pending.remove(ident);done.add(ident);returns+=1
        assert row['outcome']in ['ok','unknown','refused'];outcomes[row['outcome']]+=1
      assert not pending and invokes==returns
      label=history_names[member.name];expected=summary['histories'][label]
      assert invokes==expected['invokes']and returns==expected['returns']and dict(outcomes)==expected['outcomes']
      histories[label]=dict(invokes=invokes,returns=returns,outcomes=dict(outcomes),sha256=v['sha256'],bytes=v['bytes'])
     else:
      actual=hashlib.file_digest(stream,'sha256').hexdigest();assert actual==v['sha256'],member.name
    seen.add(member.name)
  assert seen==set(index['files'])and len(seen)==index['decoded_member_count']and decoded==index['decoded_member_bytes']
  assert set(histories)==set(summary['histories'])and sum(v['invokes']for v in histories.values())==summary['operations']
  result.update(complete=True,members=len(seen),decoded_bytes=decoded,compressed_bytes=compressed,compressed_sha256=h.hexdigest(),
   inventory_sha256=hashlib.sha256(raw).hexdigest(),histories=histories,scope='Exact included bytes, three complete-history counts and bounded archive readback. No semantic/lifecycle/workload replay; omissions remain original-archive authority.')
 except BaseException as error:result['failure']=repr(error);raise
 finally:
  result['ended_ns']=time.time_ns()
  with args.output.open('x')as f:json.dump(result,f,indent=2,sort_keys=True);f.write('\n')
 print(json.dumps(result))
if __name__=='__main__':main()
