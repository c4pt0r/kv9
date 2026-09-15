#!/usr/bin/env python3
"""Prepare the isolated resident-upsert experiment; never patch the active tree."""

import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tomllib


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rpds-crate", type=Path, required=True,
                        help="The registry rpds-1.2.1.crate archive; hash is checked against Cargo.lock")
    args = parser.parse_args()
    here = Path(__file__).resolve().parent
    repo = here.parents[1]
    revision = "de726b6d46391afdf11374a959441329c175b789"
    lock = subprocess.check_output(["git", "-C", str(repo), "show", revision + ":Cargo.lock"])
    packages = tomllib.loads(lock.decode())["package"]
    package = next(p for p in packages if p["name"] == "rpds" and p["version"] == "1.2.1")
    archive = args.rpds_crate.read_bytes()
    if digest(archive) != package["checksum"]:
        raise ValueError("rpds registry archive checksum mismatch")
    packet = repo / "docs/resident-index-experiments-v1"
    corpus_meta = json.loads((packet / "corpus.json").read_text())
    corpus = gzip.decompress((packet / "batches.bin.gz").read_bytes())
    if len(corpus) != corpus_meta["bytes"] or digest(corpus) != corpus_meta["sha256"]:
        raise ValueError("resident-index corpus checksum mismatch")
    output = args.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    source = output / "source"
    source.mkdir()
    tree = subprocess.check_output([
        "git", "-C", str(repo), "archive", revision, "Cargo.toml", "Cargo.lock",
        "rust-toolchain.toml", "crates", "proto", "src",
    ])
    with tarfile.open(fileobj=io.BytesIO(tree)) as tar:
        tar.extractall(source, filter="data")
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
        tar.extractall(output, filter="data")
    dependency = output / "rpds-1.2.1"
    for name in ("rpds-borrowed-upsert.patch", "rpds-differential-test.patch"):
        result = subprocess.run([
            "patch", "--batch", "--fuzz=0", "-p1", "-i", str(here / name),
        ], cwd=dependency, check=True, text=True, capture_output=True)
        (output / (name + ".log")).write_text(result.stdout + result.stderr)
    with (dependency / "Cargo.toml").open("a") as stream:
        stream.write("\n[workspace]\n")
    engine = source / "crates/engine"
    module = engine / "src/mem.rs"
    original = module.read_text()
    marker = "type CfMap = RedBlackTreeMapSync<Vec<u8>, Vec<u8>>;"
    if original.count(marker) != 1:
        raise ValueError("resident-map test module insertion point changed")
    module.write_text(original.replace(marker, marker + "\n\n#[cfg(test)]\nmod value_reuse;"))
    (engine / "src/mem").mkdir(exist_ok=True)
    shutil.copyfile(here / "value_reuse.rs", engine / "src/mem/value_reuse.rs")
    with (engine / "Cargo.toml").open("a") as stream:
        stream.write('\n[target.\'cfg(target_os = "linux")\'.dev-dependencies]\n'
                     'tikv-jemallocator = { version = "=0.6.1", default-features = false }\n')
    (source / ".cargo").mkdir()
    (source / ".cargo/config.toml").write_text(
        "[patch.crates-io]\nrpds = { path = " + json.dumps(str(dependency)) + " }\n")
    (output / "batches.bin").write_bytes(corpus)
    record = {
        "source_revision": revision, "rpds_crate_sha256": digest(archive),
        "corpus_sha256": digest(corpus), "source": str(source),
        "dependency": str(dependency),
        "patched_module_sha256": digest((dependency / "src/map/red_black_tree_map/mod.rs").read_bytes()),
        "harness_sha256": digest((here / "value_reuse.rs").read_bytes()),
        "scope": "Preparation only. The runtime write path is unchanged; candidate calls occur only in the test harness.",
    }
    (output / "preparation.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record, indent=2))


if __name__ == "__main__":
    main()
