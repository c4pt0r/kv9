#!/usr/bin/env python3
"""Verify portable FNV writer evidence bytes without extraction or execution."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path, PurePosixPath
import tarfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--root', type=Path, required=True)
parser.add_argument('--inventory-sha256', required=True)
args = parser.parse_args()
assert __debug__, 'Use PYTHONOPTIMIZE=0'
payload = (args.root / 'inventory.json').read_bytes()
assert hashlib.sha256(payload).hexdigest() == args.inventory_sha256
index = json.loads(payload)
archive = args.root / index['archive']['name']
assert archive.name == 'original-evidence.tar.gz' and archive.stat().st_size == index['archive']['bytes'] < 4 * 1024**2
with archive.open('rb') as stream:
    assert hashlib.file_digest(stream, 'sha256').hexdigest() == index['archive']['sha256']
assert len(index['files']) < 500 and index['decoded_bytes'] < 16 * 1024**2
seen, decoded = set(), 0
with gzip.open(archive, 'rb') as gz:
    with tarfile.open(fileobj=gz, mode='r|') as tar:
        for item in tar:
            name = PurePosixPath(item.name)
            assert not name.is_absolute() and '..' not in name.parts and str(name) == item.name
            assert item.isfile() and item.name in index['files'] and item.name not in seen
            expected = index['files'][item.name]
            assert item.size == expected['bytes'] < 4 * 1024**2
            stream = tar.extractfile(item)
            data = stream.read(item.size + 1)
            assert len(data) == item.size and hashlib.sha256(data).hexdigest() == expected['sha256']
            decoded += len(data)
            assert decoded <= index['decoded_bytes']
            seen.add(item.name)
    # Drain the bounded gzip trailer, validating compressed CRC and EOF.
    tail = gz.read(16 * 1024 + 1)
    assert len(tail) <= 16 * 1024 and not tail.strip(b'\0') and not gz.read(1)
assert seen == set(index['files']) and decoded == index['decoded_bytes']
print(json.dumps(dict(complete=True, files=len(seen), decoded_bytes=decoded,
                      compressed_bytes=archive.stat().st_size,
                      scope='Exact portable bytes only; no runtime, proof or performance re-execution.')))
