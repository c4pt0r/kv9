#!/usr/bin/env python3
"""Verify compact evidence bytes only; no extraction, audit or statistics."""
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import tarfile

def safe(name):
    p = PurePosixPath(name)
    return bool(name) and not p.is_absolute() and str(p) == name and all(x not in ('.', '..') for x in p.parts) and '\\' not in name

def checked(data, expected, name):
    if len(data) != expected['bytes'] or hashlib.sha256(data).hexdigest() != expected['sha256']:
        raise ValueError('byte identity differs: ' + name)

def main():
    root = Path(__file__).resolve().parent
    index = json.loads((root/'inventory.json').read_text())
    for name, expected in index['top_level'].items():
        if not safe(name) or len(PurePosixPath(name).parts) != 1:
            raise ValueError('unsafe top-level path')
        checked((root/name).read_bytes(), expected, name)
    parts = []
    for number, part in enumerate(index['parts'], 1):
        if part['name'] != f'evidence.tar.gz.{number:03d}' or not 0 < part['bytes'] <= 2*1024*1024:
            raise ValueError('unexpected archive part')
        data = (root/part['name']).read_bytes()
        checked(data, part, part['name'])
        parts.append(data)
    seen = set()
    with tarfile.open(fileobj=io.BytesIO(b''.join(parts)), mode='r:gz') as archive:
        for member in archive:
            if not safe(member.name) or not member.isfile() or member.name in seen or member.name not in index['files']:
                raise ValueError('unexpected or unsafe archive member: ' + member.name)
            expected = index['files'][member.name]
            if member.size != expected['bytes']:
                raise ValueError('member size differs: ' + member.name)
            with archive.extractfile(member) as stream:
                actual = hashlib.file_digest(stream, 'sha256').hexdigest()
            if actual != expected['sha256']:
                raise ValueError('member hash differs: ' + member.name)
            seen.add(member.name)
    if seen != set(index['files']):
        raise ValueError('missing archive member')
    print(f'PASS: {len(seen)} exact evidence files, {len(parts)} bounded archive parts; integrity only')

if __name__ == '__main__':
    main()
