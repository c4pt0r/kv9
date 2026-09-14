#!/usr/bin/env python3
"""Root-only byte-exact splitting/readback of the already accepted gzip archive. No codec."""
import argparse,hashlib,json,os
from pathlib import Path
SOURCE=Path('/tmp/kv9-frame-crc-chaos-preparation-20260914-first/archive-first/evidence.tar.gz')
BYTES=92343933
SHA='b3bc55144f53c83bbdcb5369c2388a4905e024e3a221e94a723477e46a88ecbc'
PART=2*1024**2
def check(ok,message):
 if not ok:raise ValueError(message)
def verify(output):
 p=output/'parts.json'
 check(p.is_file() and not p.is_symlink() and p.stat().st_size<65536,'index shape')
 index=json.loads(p.read_text());parts=index['parts']
 check(index['original_bytes']==BYTES and index['original_sha256']==SHA,'original binding')
 check(len(parts)==(BYTES+PART-1)//PART,'part count')
 digest=hashlib.sha256();total=0
 for i,row in enumerate(parts,1):
  name=f'chaos-original.tar.gz.{i:03d}';check(row['name']==name,'part order')
  path=output/name;check(path.is_file() and not path.is_symlink() and 0<path.stat().st_size<=PART,'part shape')
  data=path.read_bytes();check(len(data)==row['bytes'] and hashlib.sha256(data).hexdigest()==row['sha256'],'part identity')
  total+=len(data);check(total<=BYTES,'total overflow');digest.update(data)
 check({p.name for p in output.glob('chaos-original.tar.gz.*')}=={p['name'] for p in parts},'extra parts')
 check(total==BYTES and digest.hexdigest()==SHA,'whole original identity')
 return dict(complete=True,bytes=total,sha256=digest.hexdigest(),parts=len(parts),scope='Exact original compressed bytes; original archive member acceptance is separately retained.')
def main():
 ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('--output',type=Path,required=True);ap.add_argument('--verify-only',action='store_true');args=ap.parse_args()
 output=args.output.absolute();check(not any(p.is_symlink() for p in [output,*output.parents]),'symlink destination')
 if not args.verify_only:
  check(SOURCE.is_file() and not SOURCE.is_symlink() and SOURCE.stat().st_size==BYTES,'source shape')
  space=os.statvfs('/tmp');check(space.f_bavail*space.f_frsize>=96*1024**3+BYTES,'retention floor plus copy reservation')
  output.mkdir(parents=True,exist_ok=False)
  parts=[];digest=hashlib.sha256();count=0
  with SOURCE.open('rb') as stream:
   while data:=stream.read(PART):
    count+=len(data);check(count<=BYTES,'original overflow');digest.update(data)
    name=f'chaos-original.tar.gz.{len(parts)+1:03d}'
    with (output/name).open('xb') as f:f.write(data)
    parts.append(dict(name=name,bytes=len(data),sha256=hashlib.sha256(data).hexdigest()))
  check(count==BYTES and digest.hexdigest()==SHA,'original hash changed')
  (output/'parts.json').write_text(json.dumps(dict(original_source=str(SOURCE),original_bytes=BYTES,original_sha256=SHA,parts=parts),indent=2)+'\n')
 print(json.dumps(verify(output)))
if __name__=='__main__':main()
