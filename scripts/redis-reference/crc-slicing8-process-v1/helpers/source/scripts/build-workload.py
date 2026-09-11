#!/usr/bin/env python3
"""Build and retain the workload executable with bounded source/build provenance."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
MAX_BINARY = 512 * 1024 * 1024


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def sha(data):
    return hashlib.sha256(data).hexdigest()


def snapshot():
    names = subprocess.check_output(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], cwd=ROOT)
    paths = sorted(set(names.decode().strip("\0").split("\0")))
    if len(paths) > 10_000:
        raise RuntimeError("source inventory exceeds its file count bound")
    sources = {}
    total = 0
    for name in paths:
        path = ROOT / name
        if not path.exists():
            sources[name] = None  # A tracked deletion is part of a dirty build.
            continue
        if path.is_symlink() or not path.is_file():
            raise RuntimeError("source inventory requires regular files")
        with path.open("rb") as stream:
            data = stream.read(2 * 1024 * 1024 + 1)
        total += len(data)
        if len(data) > 2 * 1024 * 1024 or total > 64 * 1024 * 1024:
            raise RuntimeError("source inventory exceeds its byte bound")
        sources[name] = sha(data)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=all"], cwd=ROOT))
    return dict(revision=revision, dirty=dirty, sources=sources)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--binary", choices=("kv9-workload", "kv9-batch-workload", "kv9-batch-benchmark", "kv9-redis-batch-reference"), default="kv9-workload")
    parser.add_argument("--rpc-experiment", action="store_true",
                        help="explicitly compile the opt-in RPC transport experiment")
    args = parser.parse_args()
    output = args.output.resolve()
    if output.is_relative_to(ROOT):
        raise RuntimeError("build artifacts must be outside the source tree")
    output.mkdir(parents=True, exist_ok=False)
    before = snapshot()
    if args.binary == "kv9-redis-batch-reference":
        if args.rpc_experiment:
            raise RuntimeError("Redis reference has no RPC experiment feature")
        command = ["cargo", "build", "--locked", "--manifest-path", "scripts/redis-reference/Cargo.toml", "--bin", args.binary, "--message-format=json-render-diagnostics"]
    else:
        command = ["cargo", "build", "--locked", "-p", "kv9-server", "--bin", args.binary, "--message-format=json-render-diagnostics"]
    if args.release:
        command.append("--release")
    if args.rpc_experiment:
        command.extend(["--features", "rpc-experiment"])
    rustc = subprocess.check_output(["rustc", "-vV"], cwd=ROOT, text=True)
    # These are build inputs, not a dump of credentials or the process environment.
    build_environment = {key: os.environ[key] for key in (
        "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"
    ) if key in os.environ}
    with (output / "cargo.jsonl").open("w") as stdout, (output / "build.log").open("w") as stderr:
        result = subprocess.run(command, cwd=ROOT, stdout=stdout, stderr=stderr, timeout=900)
    if result.returncode:
        raise RuntimeError("workload build failed; see retained build.log")
    after = snapshot()
    if before != after:
        raise RuntimeError("source or revision changed during build")
    executables = set()
    with (output / "cargo.jsonl").open() as stream:
        for line in stream:
            item = json.loads(line)
            if item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == args.binary and item.get("executable"):
                executables.add(item["executable"])
    if len(executables) != 1:
        raise RuntimeError("build did not identify exactly one workload executable")
    binary = Path(executables.pop())
    if not 0 < binary.stat().st_size <= MAX_BINARY:
        raise RuntimeError("workload executable exceeds its size bound")
    retained = output / args.binary
    shutil.copy2(binary, retained)
    with retained.open("rb") as stream:
        binary_sha = hashlib.file_digest(stream, "sha256").hexdigest()
    source_bytes = canonical(before["sources"])
    manifest = dict(version=1, revision=before["revision"], dirty=before["dirty"],
                    source_tree_sha256=sha(source_bytes), binary_sha256=binary_sha,
                    profile="release" if args.release else "debug", rustc=rustc)
    inventory = dict(version=1, **before, command=command, build_environment=build_environment,
                     source_tree_sha256=manifest["source_tree_sha256"], binary_sha256=binary_sha)
    for name, value in (("build.json", manifest), ("sources.json", inventory)):
        data = canonical(value) + b"\n"
        if len(data) > (65_536 if name == "build.json" else 2_097_152):
            raise RuntimeError("build provenance exceeds its size bound")
        (output / name).write_bytes(data)
    print(f"PASS: retained workload executable and build provenance in {output}")


if __name__ == "__main__":
    main()
