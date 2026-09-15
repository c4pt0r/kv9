#!/usr/bin/env python3
"""Independent bounded original observer metadata readback; no extraction or test replay."""
import argparse,hashlib,io,json,os,stat,tarfile,time
from pathlib import Path,PurePosixPath
M=1024**2
LIMITS=dict(member_bytes=16*M,decoded_bytes=96*M,compressed_bytes=8*M,root_allocated_bytes=24*M,part_bytes=2*M,seconds=1200,host_floor_bytes=8*1024**3)
def safe(n):
 p=PurePosixPath(n);return bool(n)and not p.is_absolute()and str(p)==n and all(x not in ('.','..')for x in p.parts)and '\\'not in n
def checked(b,v,n):
 if len(b)!=v['bytes']or hashlib.sha256(b).hexdigest()!=v['sha256']:raise ValueError('byte identity differs: '+n)
def identity(s):return(s.st_dev,s.st_ino,s.st_size,s.st_mtime_ns,s.st_ctime_ns,s.st_mode)
def main():
 a=argparse.ArgumentParser(description=__doc__);a.add_argument('--output',type=Path,required=True);a.add_argument('--original-inputs',action='store_true');args=a.parse_args()
 assert __debug__ and not args.output.exists()
 root=Path(__file__).resolve().parent;started=time.time_ns();result=dict(complete=False,started_ns=started)
 def guard():
  fs=os.statvfs(args.output.parent);assert fs.f_bavail*fs.f_frsize>=LIMITS['host_floor_bytes']
  assert time.time_ns()-started<LIMITS['seconds']*10**9
 try:
  guard();raw=(root/'inventory.json').read_bytes();index=json.loads(raw)
  assert index['limits']==LIMITS
  for n,v in index['top_level'].items():
   assert safe(n)and len(PurePosixPath(n).parts)==1;checked((root/n).read_bytes(),v,n)
  parts=[];compressed=0;h=hashlib.sha256()
  for number,part in enumerate(index['parts'],1):
   assert number<=4 and part['name']==f'evidence.tar.gz.{number:03d}'and 0<part['bytes']<=2*M
   b=(root/part['name']).read_bytes();checked(b,part,part['name']);parts.append(b);h.update(b);compressed+=len(b)
   assert compressed<=LIMITS['compressed_bytes']
  assert compressed==index['compressed_bytes']and h.hexdigest()==index['compressed_sha256']
  summary=json.loads((root/'summary.json').read_text());mandatory_name='original/'+summary['mandatory_inputs']['path'].lstrip('/')
  seen=set();decoded=0;mandatory=None
  with tarfile.open(fileobj=io.BytesIO(b''.join(parts)),mode='r:gz')as archive:
   for member in archive:
    assert safe(member.name)and member.isfile()and member.name not in seen and member.name in index['files']
    v=index['files'][member.name];assert 0<=member.size==v['bytes']<=LIMITS['member_bytes']
    decoded+=member.size;assert decoded<=LIMITS['decoded_bytes'];guard()
    with archive.extractfile(member)as stream:
     b=stream.read(LIMITS['member_bytes']+1);checked(b,v,member.name);assert not stream.read(1)
     if member.name==mandatory_name:
      checked(b,summary['mandatory_inputs'],member.name);mandatory=json.loads(b)
    seen.add(member.name)
  assert seen==set(index['files'])and len(seen)==index['decoded_member_count']and decoded==index['decoded_member_bytes']
  assert mandatory is not None and len(mandatory)==261 and sum(v['bytes']for v in mandatory.values())==59567269
  for p,v in mandatory.items():
   n='original/'+p.lstrip('/');assert n in index['files']and all(index['files'][n][k]==v[k]for k in ['bytes','sha256'])
  originals_checked=0
  if args.original_inputs:
   for n,v in index['files'].items():
    p=Path(v['original_path']);before=p.lstat();assert stat.S_ISREG(before.st_mode)and before.st_size==v['bytes']
    with os.fdopen(os.open(p,os.O_RDONLY|os.O_NOFOLLOW),'rb')as f:
     assert identity(os.fstat(f.fileno()))==identity(before)
     b=f.read(LIMITS['member_bytes']+1);checked(b,v,str(p));assert not f.read(1)
     assert identity(os.fstat(f.fileno()))==identity(before)
    assert identity(p.lstat())==identity(before);originals_checked+=1;guard()
  result.update(complete=True,members=len(seen),decoded_bytes=decoded,compressed_bytes=compressed,compressed_sha256=h.hexdigest(),inventory_sha256=hashlib.sha256(raw).hexdigest(),mandatory_analysis_inputs=261,mandatory_analysis_bytes=59567269,original_inputs_checked=originals_checked,scope='Every archived member and all 261 mandatory original analysis-input pins verified; optional original-input mode rechecks actual selected files. No observer, histogram, lookup, lifecycle, workload or control replay; local WAL/ELF payloads are omitted and unchanged.')
 except BaseException as error:result['failure']=repr(error);raise
 finally:
  result['ended_ns']=time.time_ns()
  with args.output.open('x')as f:json.dump(result,f,indent=2,sort_keys=True);f.write('\n')
 print(json.dumps(result))
if __name__=='__main__':main()
