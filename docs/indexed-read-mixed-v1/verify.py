#!/usr/bin/env python3
"""Verify compact diagnostic evidence without extraction or runtime execution."""
import hashlib
import io
import json
from pathlib import Path
import tarfile


def main():
    root = Path(__file__).resolve().parent
    index = json.loads((root / "inventory.json").read_text())
    parts = []
    for part in index["parts"]:
        data = (root / part["name"]).read_bytes()
        if len(data) != part["bytes"] or hashlib.sha256(data).hexdigest() != part["sha256"]:
            raise ValueError("archive part differs: " + part["name"])
        parts.append(data)
    seen = set()
    with tarfile.open(fileobj=io.BytesIO(b"".join(parts)), mode="r:gz") as archive:
        for member in archive:
            if not member.isfile() or member.name in seen or member.name not in index["files"]:
                raise ValueError("unexpected archive member: " + member.name)
            expected = index["files"][member.name]
            if member.size != expected["bytes"]:
                raise ValueError("member size differs: " + member.name)
            with archive.extractfile(member) as stream:
                actual = hashlib.file_digest(stream, "sha256").hexdigest()
            if actual != expected["sha256"]:
                raise ValueError("member hash differs: " + member.name)
            seen.add(member.name)
    if seen != set(index["files"]):
        raise ValueError("missing archive member")
    print(f"PASS: {len(seen)} exact evidence files; integrity only")


if __name__ == "__main__":
    main()
