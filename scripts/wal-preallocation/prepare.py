#!/usr/bin/env python3
"""Prepare a fresh isolated WAL encoder experiment with the server's allocator."""

import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    output = args.output.resolve()
    contract = json.loads((root / "proofs/lean/wal-preallocation/source-contract.json").read_text())
    for name, entry in contract["sources"].items():
        if digest((root / name).read_bytes()) != entry["sha256"]:
            raise ValueError("candidate source differs from reviewed contract: " + name)
    corpus = gzip.decompress((root / "docs/resident-index-experiments-v1/batches.bin.gz").read_bytes())
    expected = "ccb8790572eea92f20bc0cf8ef1a8bd66bfecb0c6106514551d4439b6869ab7e"
    if digest(corpus) != expected:
        raise ValueError("retained corpus checksum mismatch")
    revision = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    source = output / "source"
    source.mkdir()
    tree = subprocess.check_output([
        "git", "-C", str(root), "archive", revision,
        "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates", "proto", "src",
    ])
    with tarfile.open(fileobj=io.BytesIO(tree)) as archive:
        archive.extractall(source, filter="data")
    overlays = ["Cargo.toml", "crates/engine/Cargo.toml", "crates/engine/src/wal.rs",
                "crates/engine/src/wal_segment.rs", "crates/engine/src/wal/preallocation_tests.rs"]
    inputs = {}
    for name in overlays:
        data = (root / name).read_bytes()
        destination = source / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
        inputs[name] = digest(data)
    with (source / "crates/engine/Cargo.toml").open("a") as stream:
        stream.write('\n[target.\'cfg(target_os = "linux")\'.dev-dependencies]\n'
                     'tikv-jemallocator = { version = "=0.6.1", default-features = false }\n')
    with (source / "crates/engine/src/wal/preallocation_tests.rs").open("a") as stream:
        stream.write('\n#[cfg(target_os = "linux")]\n#[global_allocator]\n'
                     'static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;\n')
    (output / "batches.bin").write_bytes(corpus)
    record = {"source_revision": revision, "overlays": inputs, "corpus_sha256": expected,
              "source": str(source), "allocator": "tikv-jemallocator =0.6.1, default-features=false",
              "scope": "Preparation only. Uses an actual test-only Linux global allocator; no benchmark ran.",
              "isolated_manifest_sha256": digest((source / "crates/engine/Cargo.toml").read_bytes()),
              "isolated_harness_sha256": digest((source / "crates/engine/src/wal/preallocation_tests.rs").read_bytes())}
    (output / "preparation.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record, indent=2))


if __name__ == "__main__":
    main()
