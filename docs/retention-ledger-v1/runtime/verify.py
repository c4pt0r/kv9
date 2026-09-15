#!/usr/bin/env python3
"""Independent one-pass gzip EOF/member readback and optional original-byte check."""
import argparse,gzip,hashlib,json,tarfile,time,traceback
from pathlib import Path
ap=argparse.ArgumentParser();ap.add_argument('--originals',action='store_true');args=ap.parse_args()
P=Path(__file__).resolve().parent;start=time.monotonic()
def digest_file(p):
 h=hashlib.sha256();n=0
 with p.open('rb') as f:
  while b:=f.read(1024*1024):h.update(b);n+=len(b)
 return n,h.hexdigest()
def matches(p,r):assert digest_file(p)==(r['bytes'],r['sha256']),str(p)
try:
 package=json.loads((P/'package-result.json').read_text());assert package['complete']
 archive=P/'evidence.tar.gz';matches(archive,package['archive'])
 joined=hashlib.sha256();parts_bytes=0
 for r in package['parts']:
  p=P/Path(r['path']).name;matches(p,r);b=p.read_bytes();joined.update(b);parts_bytes+=len(b)
 assert (parts_bytes,joined.hexdigest())==(package['archive']['bytes'],package['archive']['sha256'])
 selection=json.loads((P/'selection.json').read_text());matches(P/'selection.json',package['selection'])
 wanted={r['member']:r for r in selection['files']};seen=set();total=0;manifest_seen=False
 with gzip.open(archive,'rb') as gz:
  with tarfile.open(fileobj=gz,mode='r|') as tar:
   for member in tar:
    assert member.isfile() and member.name not in seen and member.size<=128*1024**2
    stream=tar.extractfile(member);h=hashlib.sha256();n=0
    if member.name=='manifest.json':
     b=stream.read();assert b==(P/'selection.json').read_bytes();manifest_seen=True;continue
    r=wanted[member.name];assert member.size==r['bytes'];seen.add(member.name)
    original=Path(r['path']).open('rb') if args.originals else None
    try:
     while b:=stream.read(1024*1024):
      n+=len(b);h.update(b)
      if original:assert original.read(len(b))==b,'original-byte mismatch '+member.name
     if original:assert original.read(1)==b''
    finally:
     if original:original.close()
    assert n==r['bytes'] and h.hexdigest()==r['sha256'];total+=n
    assert total<=512*1024**2 and time.monotonic()-start<=120
  trailing=gz.read();assert not trailing.strip(b'\0') and len(trailing)<=10240
 assert manifest_seen and seen==set(wanted) and total==selection['original_bytes']
 result={'complete':True,'archive_sha256':package['archive']['sha256'],'gzip_full_eof':True,'manifest_exact_bytes':True,'member_paths_unique':True,'selected_members':len(seen),'tar_members':len(seen)+1,'selected_original_bytes':total,'all_members_sha256_match':True,'all_original_bytes_compared':args.originals,'all_parts_exact_reconstruct_archive':True,'elapsed_seconds':time.monotonic()-start,'scope':'One independent archive decode/readback; original workload and acceptance controls were not replayed.'}
 with (P/'readback-result.json').open('x') as f:json.dump(result,f,indent=2,sort_keys=True);f.write('\n')
 print(json.dumps(result))
except BaseException as e:
 with (P/'readback-failure.json').open('x') as f:json.dump({'complete':False,'error':repr(e),'traceback':traceback.format_exc()},f,indent=2)
 raise
