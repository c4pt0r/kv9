#!/usr/bin/env python3
"""Independent reporting archive byte verifier, with complete gzip EOF readback."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent


def main():
    blob = (HERE / 'analysis-members.json').read_bytes()
    manifest = json.loads(blob)
    expected = {r['member']: r for r in manifest['members']}
    assert len(expected) == manifest['original_files'] < 512
    assert sum(r['bytes'] for r in expected.values()) == manifest['original_bytes'] <= 64 * 1024 * 1024
    expected['members.json'] = {'bytes': len(blob), 'sha256': hashlib.sha256(blob).hexdigest()}
    archive = HERE / 'analysis.tar.gz'
    assert archive.stat().st_size <= 32 * 1024 * 1024
    encoded = archive.read_bytes()
    with gzip.GzipFile(fileobj=io.BytesIO(encoded), mode='rb') as gz:
        decoded = gz.read(80 * 1024 * 1024 + 1)
        assert len(decoded) <= 80 * 1024 * 1024 and gz.read(1) == b''
    seen, total = set(), 0
    with tarfile.open(fileobj=io.BytesIO(decoded), mode='r:') as tf:
        for member in tf:
            assert member.isfile() and member.name in expected and member.name not in seen
            assert not Path(member.name).is_absolute() and '..' not in Path(member.name).parts
            row = expected[member.name]
            assert member.size == row['bytes'] <= 8 * 1024 * 1024
            data = tf.extractfile(member).read()
            assert len(data) == row['bytes'] and hashlib.sha256(data).hexdigest() == row['sha256']
            total += len(data)
            seen.add(member.name)
    assert seen == set(expected)
    result = {'complete': True, 'archive': archive.name, 'archive_bytes': len(encoded),
              'archive_sha256': hashlib.sha256(encoded).hexdigest(),
              'gzip_eof_verified': True, 'decoded_tar_bytes': len(decoded),
              'files': len(seen), 'original_files': manifest['original_files'],
              'original_bytes': manifest['original_bytes'], 'member_bytes_including_inventory': total,
              'members_sha256': hashlib.sha256(blob).hexdigest(),
              'scope': 'One full reporting-archive decode and exact member readback; no audit, tests or payload decoding.'}
    with (HERE / 'analysis-verification.json').open('x') as out:
        out.write(json.dumps(result, indent=2, sort_keys=True) + '\n')
    print(json.dumps(result, sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except BaseException as exc:
        (HERE / 'analysis-verification-failure.json').write_text(json.dumps({'complete': False, 'error': repr(exc)}) + '\n')
        raise
