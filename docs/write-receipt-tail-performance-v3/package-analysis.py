#!/usr/bin/env python3
"""Publish the separately frozen analysis without executing its calculation."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import stat
import tarfile

HERE = Path(__file__).resolve().parent
SOURCE = Path('/tmp/kv9-receipt-tail-performance-summary-20260915-first')


def main():
    blob = (SOURCE / 'inventory.json').read_bytes()
    assert hashlib.sha256(blob).hexdigest() == '09dbf0fcdd85fe67f4f6fd5f9c6b84601e2e51da0f65565c76ef791ecc3d6ab3'
    old = json.loads(blob)
    assert old['complete'] and old['files'] == len(old['rows']) == 8
    rows, contents = [], {'inventory.json': blob}
    for row in old['rows']:
        path = Path(row['path'])
        assert path.parent == SOURCE and stat.S_ISREG(path.lstat().st_mode)
        data = path.read_bytes()
        assert len(data) == row['bytes'] and hashlib.sha256(data).hexdigest() == row['sha256']
        assert len(data) <= 1024 * 1024
        contents[path.name] = data
    for name, data in sorted(contents.items()):
        rows.append({'member': name, 'source': str(SOURCE / name), 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()})
    manifest = {'scope': 'Separate exact frozen analysis; histogram helper and all 60 inputs are in evidence.tar.gz.',
                'original_files': len(rows), 'original_bytes': sum(r['bytes'] for r in rows), 'members': rows}
    assert manifest['original_bytes'] <= 1024 * 1024
    data = (json.dumps(manifest, indent=2, sort_keys=True) + '\n').encode()
    (HERE / 'analysis-members.json').write_bytes(data)
    contents['members.json'] = data
    with (HERE / 'analysis.tar.gz').open('xb') as raw:
        with gzip.GzipFile(filename='', mode='wb', fileobj=raw, compresslevel=6, mtime=0) as gz:
            with tarfile.open(fileobj=gz, mode='w', format=tarfile.USTAR_FORMAT) as tf:
                for name, data in sorted(contents.items()):
                    info = tarfile.TarInfo(name)
                    info.size, info.mode, info.mtime = len(data), 0o644, 0
                    tf.addfile(info, io.BytesIO(data))
    for name in ('summary.json', 'input-hashes.json'):
        (HERE / name).write_bytes(contents[name])
    (HERE / 'ANALYSIS.md').write_bytes(contents['README.md'])
    print(json.dumps({'complete': True, 'original_files': len(rows), 'original_bytes': manifest['original_bytes'], 'archive_members': len(contents)}))


if __name__ == '__main__':
    main()
