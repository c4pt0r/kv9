#!/usr/bin/env python3
"""Independent bounded byte verifier. No extraction, source audit or WAL decode."""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import tarfile

M = 1024**2
TOTAL = 512*M


def require(value, reason):
    if not value:
        raise ValueError(reason)


def safe(name):
    p = PurePosixPath(name)
    return bool(name) and not p.is_absolute() and str(p) == name and '\\' not in name and all(x not in ('.', '..') for x in p.parts)


def bounded(path, cap):
    require(path.is_file() and not path.is_symlink() and path.stat().st_size <= cap, 'invalid file: '+str(path))
    return path.read_bytes()


class Parts(io.RawIOBase):
    def __init__(self, root, parts):
        self.root, self.parts = root, iter(parts)
        self.current = None

    def readable(self):
        return True

    def readinto(self, buffer):
        while True:
            if self.current is None:
                part = next(self.parts, None)
                if part is None:
                    return 0
                self.current = (self.root/part['name']).open('rb')
            n = self.current.readinto(buffer)
            if n:
                return n
            self.current.close()
            self.current = None

    def close(self):
        if self.current:
            self.current.close()
        super().close()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--root', type=Path, default=Path(__file__).resolve().parent)
    ap.add_argument('--inventory-sha256', required=True)
    args = ap.parse_args()
    root = args.root
    require(not any(p.is_symlink() for p in (root, *root.parents)), 'symlink root')
    raw = bounded(root/'inventory.json', 64*M)
    require(hashlib.sha256(raw).hexdigest() == args.inventory_sha256, 'inventory SHA differs')
    index = json.loads(raw)
    files, parts = index['files'], index['parts']
    require(0 < len(files) <= 10000 and len(files) == index['decoded_member_count'], 'member count')
    require(0 < len(parts) <= 256, 'part count')
    total = sum(v['bytes'] for v in files.values())
    require(total == index['decoded_member_bytes'] <= TOTAL, 'decoded cap')
    require(all(safe(n) and 0 <= v['bytes'] <= 64*M for n,v in files.items()), 'unsafe index member')
    for name, expected in index['top_level'].items():
        require(safe(name) and '/' not in name, 'top-level path')
        data = bounded(root/name, 64*M)
        require(len(data) == expected['bytes'] and hashlib.sha256(data).hexdigest() == expected['sha256'], 'top-level identity: '+name)
    digest, compressed = hashlib.sha256(), 0
    for number, part in enumerate(parts, 1):
        require(part['name'] == f'evidence.tar.gz.{number:03d}', 'part sequence')
        data = bounded(root/part['name'], 2*M)
        require(0 < len(data) == part['bytes'] <= 2*M, 'part size')
        require(hashlib.sha256(data).hexdigest() == part['sha256'], 'part hash')
        compressed += len(data)
        require(compressed <= TOTAL, 'compressed cap')
        digest.update(data)
    require(compressed == index['compressed_bytes'] and digest.hexdigest() == index['compressed_sha256'], 'whole compressed identity')
    require({p.name for p in root.glob('evidence.tar.gz.*')} == {p['name'] for p in parts}, 'unexpected archive part')
    seen, decoded_tar = set(), [0]
    with Parts(root, parts) as raw_parts, io.BufferedReader(raw_parts, buffer_size=M) as joined, gzip.GzipFile(fileobj=joined, mode='rb') as gz:
        class Limited:
            def read(self, amount):
                data = gz.read(min(amount, M))
                decoded_tar[0] += len(data)
                require(decoded_tar[0] <= TOTAL+32*M, 'decoded tar envelope cap')
                return data
        limited = Limited()
        with tarfile.open(fileobj=limited, mode='r|') as archive:
            for member in archive:
                require(safe(member.name) and member.isfile() and member.name in files and member.name not in seen, 'unexpected archive member')
                expected = files[member.name]
                require(member.size == expected['bytes'], 'member size')
                digest, count = hashlib.sha256(), 0
                with archive.extractfile(member) as stream:
                    while True:
                        block = stream.read(M)
                        if not block:
                            break
                        count += len(block)
                        require(count <= expected['bytes'], 'member overflow')
                        digest.update(block)
                require(count == expected['bytes'] and digest.hexdigest() == expected['sha256'], 'member bytes/hash: '+member.name)
                seen.add(member.name)
        # Finish gzip, checking its trailer and bounding any bytes after tar EOF.
        while True:
            block = limited.read(M)
            if not block:
                break
            require(not any(block), 'nonzero trailing tar payload')
    require(seen == set(files), 'missing member')
    print(json.dumps(dict(complete=True, files=len(seen), decoded_bytes=total, compressed_bytes=compressed,
                         parts=len(parts), scope='Exact portable metadata bytes only; no original source, runtime, WAL or performance acceptance.')))


if __name__ == '__main__':
    main()
