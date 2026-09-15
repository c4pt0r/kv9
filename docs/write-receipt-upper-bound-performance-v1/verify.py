#!/usr/bin/env python3
"""Independent full gzip EOF, tar termination and member SHA readback; no extraction."""
import argparse,gzip,hashlib,io,json,tarfile,time,os
from pathlib import Path,PurePosixPath
HERE=Path(__file__).resolve().parent
def safe(n):
    p=PurePosixPath(n);return bool(n)and not p.is_absolute()and str(p)==n and all(x not in ('.','..')for x in p.parts)and '\\'not in n
def main():
    a=argparse.ArgumentParser();a.add_argument('--members-sha256',required=True);a.add_argument('--archive-sha256',required=True);a.add_argument('--output',type=Path,required=True);args=a.parse_args()
    assert __debug__ and args.output.is_absolute()and not args.output.exists()
    started=time.monotonic();result=dict(complete=False)
    try:
        v=os.statvfs(HERE);assert v.f_bavail*v.f_frsize>=8*1024**3
        blob=(HERE/'members.json').read_bytes();assert len(blob)<=8*1024**2 and hashlib.sha256(blob).hexdigest()==args.members_sha256
        manifest=json.loads(blob);assert manifest['member_cap_bytes']==8*1024**2 and manifest['total_cap_bytes']==80*1024**2
        assert manifest['compressed_cap_bytes']==32*1024**2 and manifest['decoded_tar_cap_bytes']==80*1024**2
        expected={r['member']:r for r in manifest['members']};assert len(expected)==manifest['original_files']<512
        assert sum(r['bytes']for r in expected.values())==manifest['original_bytes']<=80*1024**2
        expected['members.json']=dict(bytes=len(blob),sha256=hashlib.sha256(blob).hexdigest())
        archive=HERE/'evidence.tar.gz';assert archive.is_file()and not archive.is_symlink()and archive.stat().st_size<=32*1024**2
        encoded=archive.read_bytes();assert hashlib.sha256(encoded).hexdigest()==args.archive_sha256
        with gzip.GzipFile(fileobj=io.BytesIO(encoded),mode='rb')as gz:
            decoded=gz.read(80*1024**2+1);assert len(decoded)<=80*1024**2 and gz.read(1)==b''
        seen=set();total=0
        with tarfile.open(fileobj=io.BytesIO(decoded),mode='r:')as tf:
            for member in tf:
                assert time.monotonic()-started<=1200
                assert member.isfile()and safe(member.name)and member.name in expected and member.name not in seen
                row=expected[member.name];assert member.size==row['bytes']<=8*1024**2
                data=tf.extractfile(member).read();assert len(data)==row['bytes']and hashlib.sha256(data).hexdigest()==row['sha256']
                total+=len(data);seen.add(member.name)
            end=tf.offset
        assert seen==set(expected)
        assert len(decoded)%512==0 and len(decoded)-end>=1024 and not any(decoded[end:]),'nonempty/truncated data after tar members'
        result.update(complete=True,archive_bytes=len(encoded),archive_sha256=args.archive_sha256,members_sha256=args.members_sha256,gzip_eof_verified=True,tar_termination_verified=True,decoded_tar_bytes=len(decoded),files=len(seen),original_files=manifest['original_files'],original_bytes=manifest['original_bytes'],member_bytes_including_inventory=total,report_inputs_included=manifest['report_inputs_included'],scope='One complete reporting archive decode and exact bounded member readback. No audit, workload, controls, WAL payload decoding or independent source-tree/ELF replay.')
    except BaseException as e:result['failure']=repr(e);raise
    finally:
        result['elapsed_seconds']=time.monotonic()-started
        with args.output.open('x')as out:json.dump(result,out,indent=2,sort_keys=True);out.write('\n')
    print(json.dumps(result,sort_keys=True))
if __name__=='__main__':main()
