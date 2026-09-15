#!/usr/bin/env python3
"""Verify every retained prefix-package byte without extracting archive paths."""
import gzip
import hashlib
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent


def require(ok, message):
    if not ok:
        raise ValueError(message)


def main():
    manifest = json.loads((HERE / "manifest.json").read_text())
    require(manifest["original_runtime_exit_code"] == 1
            and manifest["supplement_accepted"] is False, "original failure changed")
    for name, expected in manifest["files"].items():
        require(Path(name).name == name, "unsafe package name")
        path = HERE / name
        require(path.is_file() and not path.is_symlink(), "nonordinary package file")
        with path.open("rb") as stream:
            actual = hashlib.file_digest(stream, "sha256").hexdigest()
        require(path.stat().st_size == expected["bytes"]
                and actual == expected["sha256"], "package file differs: " + name)
    inventory = json.loads((HERE / "input-inventory.json").read_text())
    expected = {row["member"]: row for row in inventory}
    require(len(expected) == len(inventory) == manifest["archive_members"],
            "duplicate or missing inventory members")
    seen = set()
    decoded = 0
    with gzip.open(HERE / "original-evidence.tar.gz", "rb") as stream:
        with tarfile.open(fileobj=stream, mode="r|") as archive:
            for member in archive:
                require(member.isfile() and member.name in expected
                        and member.name not in seen, "unexpected archive member")
                row = expected[member.name]
                require(member.size == row["bytes"], "member size differs")
                digest = hashlib.sha256()
                count = 0
                with archive.extractfile(member) as contents:
                    while block := contents.read(1024 * 1024):
                        count += len(block)
                        digest.update(block)
                require(count == row["bytes"] and digest.hexdigest() == row["sha256"],
                        "member content differs: " + member.name)
                seen.add(member.name)
                decoded += count
        while stream.read(1024 * 1024):
            pass  # Consume gzip EOF and its checksum, including tar padding.
    require(seen == set(expected) and decoded == manifest["archive_decoded_bytes"],
            "incomplete archive population")
    print(json.dumps(dict(complete=True, members=len(seen), decoded_bytes=decoded,
                          original_runtime_exit_code=1, supplement_accepted=False)))


if __name__ == "__main__":
    main()
