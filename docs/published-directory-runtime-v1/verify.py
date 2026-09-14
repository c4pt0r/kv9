#!/usr/bin/env python3
"""Verify this finite archive without extracting any member."""
import hashlib
import json
from pathlib import Path
import tarfile
root = Path(__file__).resolve().parent
manifest = json.loads((root / 'inventory.json').read_text())
archive = root / 'original-evidence.tar.gz'
assert archive.stat().st_size == manifest['archive_bytes']
assert hashlib.sha256(archive.read_bytes()).hexdigest() == manifest['archive_sha256']
expected = {row['path']: row for row in manifest['files']}
assert len(expected) == len(manifest['files'])
seen = set()
with tarfile.open(archive, 'r:gz') as stream:
    for member in stream:
        assert member.isfile() and member.name in expected and member.name not in seen
        row = expected[member.name]
        assert member.size == row['bytes'] <= 64 * 1024**2
        with stream.extractfile(member) as data:
            assert hashlib.file_digest(data, 'sha256').hexdigest() == row['sha256']
        seen.add(member.name)
assert seen == set(expected)
assert sum(row['bytes'] for row in expected.values()) == manifest['decoded_bytes']
print(f'PASS: {len(seen)} original members verified without extraction')
