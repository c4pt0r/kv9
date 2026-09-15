#!/usr/bin/env python3
"""Read every portable archive member without extracting paths or running helpers."""
import gzip
import hashlib
import json
from pathlib import Path
import tarfile

HERE = Path(__file__).resolve().parent


def require(ok, message):
    if not ok:
        raise ValueError(message)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    manifest = json.loads((HERE / "manifest.json").read_text())
    require(set(manifest["runs"]) == {"accepted", "noop-gap-failure"},
            "unexpected run population")
    for name, expected in manifest["files"].items():
        rel = Path(name)
        require(not rel.is_absolute() and ".." not in rel.parts,
                "unsafe package path")
        path = HERE / rel
        require(path.is_file() and not path.is_symlink()
                and all(not (HERE / parent).is_symlink() for parent in rel.parents),
                "nonordinary file")
        require(path.stat().st_size == expected["bytes"]
                and digest(path) == expected["sha256"], "package bytes differ: " + name)
    reports = {}
    for label, spec in manifest["runs"].items():
        root = HERE / label
        inventory = json.loads((root / "input-inventory.json").read_text())
        expected = {row["member"]: row for row in inventory}
        require(len(expected) == len(inventory) == spec["members"],
                "duplicate or missing inventory member")
        seen, decoded, records = set(), 0, {}
        with gzip.open(root / "original-evidence.tar.gz", "rb") as stream:
            with tarfile.open(fileobj=stream, mode="r|") as archive:
                for member in archive:
                    require(member.isfile() and member.name in expected
                            and member.name not in seen, "unexpected archive member")
                    row = expected[member.name]
                    require(member.size == row["bytes"], "member size differs")
                    sha, count = hashlib.sha256(), 0
                    keep = member.name in {"run/result.json", "audit/audit.json"}
                    chunks = []
                    with archive.extractfile(member) as contents:
                        while block := contents.read(1024 * 1024):
                            count += len(block)
                            sha.update(block)
                            if keep:
                                chunks.append(block)
                    require(count == row["bytes"] and sha.hexdigest() == row["sha256"],
                            "member bytes differ: " + member.name)
                    if keep:
                        records[member.name] = json.loads(b"".join(chunks))
                    seen.add(member.name)
                    decoded += count
            while stream.read(1024 * 1024):
                pass  # Consume gzip EOF/checksum and all tar padding.
        require(seen == set(expected) and decoded == spec["decoded_bytes"],
                "incomplete archive population")
        run, audit = records["run/result.json"], records["audit/audit.json"]
        accepted = label == "accepted"
        require(run["complete"] is accepted and run["accepted"] is accepted
                and audit["complete"] is True and audit["accepted"] is accepted,
                "original outcome changed")
        terminal = json.loads((root / "runtime-terminal.json").read_text())
        require(terminal["exit_code"] == (0 if accepted else 1)
                and terminal["result_sha256"] == expected["run/result.json"]["sha256"],
                "actual runtime terminal differs")
        cleanup = json.loads((root / "cleanup-result.json").read_text())
        require(cleanup["complete"] and cleanup["namespace_absent"]
                and cleanup["namespace_uid"] == audit["namespace_uid"]
                and all(row["absent"] for row in cleanup["processes"]),
                "owned cleanup incomplete")
        reports[label] = dict(members=len(seen), decoded_bytes=decoded,
                              original_runtime_exit_code=terminal["exit_code"],
                              supplement_accepted=accepted)
    print(json.dumps(dict(complete=True, runs=reports)))


if __name__ == "__main__":
    main()
