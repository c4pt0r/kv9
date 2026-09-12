#!/usr/bin/env python3
"""Verify the portable validation archive without extracting files."""
import hashlib
import json
from pathlib import Path
import tarfile

root = Path(__file__).resolve().parent
manifest = json.loads((root / 'manifest.json').read_text())
archive = root / 'evidence.tar.gz'
if hashlib.sha256(archive.read_bytes()).hexdigest() != manifest['archive_sha256']:
    raise ValueError('archive checksum mismatch')
seen = set()
total = 0
with tarfile.open(archive, 'r:gz') as tar:
    for entry in tar:
        if not entry.isfile() or entry.name not in manifest['files'] or entry.name in seen:
            raise ValueError('unexpected archive member')
        expected = manifest['files'][entry.name]
        if entry.size != expected['bytes'] or entry.size > 2 * 1024**2:
            raise ValueError('member size mismatch')
        with tar.extractfile(entry) as stream:
            data = stream.read(entry.size + 1)
        if len(data) != entry.size or hashlib.sha256(data).hexdigest() != expected['sha256']:
            raise ValueError('member checksum mismatch')
        seen.add(entry.name)
        total += entry.size
if seen != set(manifest['files']) or total != manifest['decoded_bytes']:
    raise ValueError('incomplete archive')
print(f'PASS: {len(seen)} original files / {total} decoded bytes; integrity only, no gates rerun')
