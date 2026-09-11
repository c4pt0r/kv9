#!/usr/bin/env python3
"""Verify exact retained reporting bytes without extracting archive members."""
import hashlib
import io
import json
from pathlib import Path
import tarfile

root = Path(__file__).resolve().parent
index = json.loads((root / 'inventory.json').read_text())


def check(data, expected):
    if len(data) != expected['bytes'] or hashlib.sha256(data).hexdigest() != expected['sha256']:
        raise ValueError('retained byte binding differs')


data = (root / index['archive']).read_bytes()
check(data, index)
with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as archive:
    members = archive.getmembers()
    if len(members) != len(index['files']) or {m.name for m in members} != set(index['files']):
        raise ValueError('archive membership differs')
    for member in members:
        if not member.isfile():
            raise ValueError('non-file archive member')
        check(archive.extractfile(member).read(), index['files'][member.name])
print(f'PASS: {len(members)} exact retained reporting files; no runtime or proof rerun')
