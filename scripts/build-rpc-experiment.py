#!/usr/bin/env python3
"""Retain a same-source server and workload for the explicit RPC experiment."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("builder", ROOT / "scripts/build-workload.py")
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--release", action="store_true")
    args = parser.parse_args()
    out = args.output.resolve()
    if out.is_relative_to(ROOT):
        raise ValueError("build artifacts must be outside the source tree")
    out.mkdir(parents=True, exist_ok=False)
    before = builder.snapshot()
    command = ["cargo", "build", "--locked", "--bin", "kv9", "--features", "rpc-experiment",
               "--message-format=json-render-diagnostics", *(["--release"] if args.release else [])]
    with (out / "cargo.jsonl").open("w") as stdout, (out / "build.log").open("w") as stderr:
        subprocess.run(command, cwd=ROOT, stdout=stdout, stderr=stderr, timeout=900, check=True)
    records = [json.loads(line) for line in (out / "cargo.jsonl").read_text().splitlines()]
    artifacts = [r for r in records if r.get("reason") == "compiler-artifact" and
                 r.get("target", {}).get("name") == "kv9" and r.get("executable")]
    if len(artifacts) != 1 or artifacts[0]["features"] != ["rpc-experiment"]:
        raise ValueError("missing or incorrect experimental server artifact")
    source = Path(artifacts[0]["executable"])
    if not 0 < source.stat().st_size <= builder.MAX_BINARY:
        raise ValueError("invalid executable size")
    shutil.copy2(source, out / "kv9")
    subprocess.run(["python3", str(ROOT / "scripts/build-workload.py"), "--output", str(out / "workload"),
                    "--rpc-experiment", *(["--release"] if args.release else [])], cwd=ROOT, check=True)
    if builder.snapshot() != before:
        raise ValueError("source changed during RPC experiment build")
    workload = json.loads((out / "workload/build.json").read_text())
    manifest = dict(version=1, **before, profile=workload["profile"], command=command,
                    source_tree_sha256=workload["source_tree_sha256"], binary_sha256=digest(out / "kv9"),
                    workload_build_sha256=digest(out / "workload/build.json"),
                    features=["rpc-experiment"], rustc=workload["rustc"])
    (out / "build.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print("PASS: retained same-source RPC experiment server and workload")


if __name__ == "__main__":
    main()
