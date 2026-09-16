#!/usr/bin/env python3
"""Build an isolated alignment control from previously qualified exact sources."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--experiment", type=Path, required=True)
    parser.add_argument("--qualified", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    output = args.experiment.resolve()
    qualified = args.qualified.resolve()
    prior = json.loads((qualified / "build-summary.json").read_text())
    plan = json.loads((output / "timing-plan.json").read_text())
    assert prior["complete"]
    for path, digest in prior["sources"].items():
        assert sha(Path(path)) == digest
    assert json.loads((qualified / "qualification-audit.json").read_text())["complete"]
    assert not any(os.environ.get(name) for name in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CFLAGS"))
    assert plan["alignment_rustflags"] == "-Cllvm-args=--align-all-functions=6"
    os.environ["RUSTFLAGS"] = plan["alignment_rustflags"]
    os.environ["CARGO_TARGET_DIR"] = str(root / "target")
    os.environ["CARGO_BUILD_JOBS"] = "4"
    spec = importlib.util.spec_from_file_location("cache", root / "scripts/build_cache.py")
    cache = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(cache)
    result = {"complete": False, "started_ns": time.time_ns(), "arms": {},
              "plan_sha256": sha(output / "timing-plan.json"),
              "ordinary_build_summary_sha256": sha(qualified / "build-summary.json"),
              "rustflags": os.environ["RUSTFLAGS"], "sources": dict(prior["sources"]),
              "production_changed": False}
    try:
        for arm in ("baseline", "candidate"):
            work = output / ("source-" + arm)
            shutil.copytree(qualified / ("bench-source-" + arm), work)
            destination = output / ("build-" + arm)
            destination.mkdir(exist_ok=False)
            paths = [work / "Cargo.toml", work / "Cargo.lock", *sorted((work / "src").rglob("*.rs"))]
            identity = dict(prior["sources"])
            identity.update({str(path): sha(path) for path in paths})
            identity[str(Path(__file__).resolve())] = sha(Path(__file__).resolve())
            result["sources"].update(identity)
            (destination / "sources.json").write_text(json.dumps(identity, indent=2) + "\n")
            for path in paths:
                original = qualified / ("bench-source-" + arm) / path.relative_to(work)
                assert path.read_bytes() == original.read_bytes()
            with cache.BuildCache(work, destination, True, identity) as build:
                with (destination / "dependency-clean.stdout").open("x") as out, \
                        (destination / "dependency-clean.stderr").open("x") as err:
                    build.run(["cargo", "clean", "--offline", "--locked", "--profile", "release",
                               "-p", "archery", "-p", "rpds", "-p", "triomphe"], stdout=out, stderr=err)
                with (destination / "cargo.jsonl").open("x") as out, (destination / "cargo.stderr").open("x") as err:
                    build.run(["cargo", "build", "--offline", "--locked", "--release",
                               "--message-format", "json-render-diagnostics"], stdout=out, stderr=err)
                build.check_artifacts(destination / "cargo.jsonl")
                units = [json.loads(line) for line in (destination / "cargo.jsonl").read_text().splitlines()]
                for name in ("archery", "rpds", "triomphe"):
                    found = [u for u in units if u.get("reason") == "compiler-artifact" and u["target"]["name"] == name]
                    assert len(found) == 1 and not found[0]["fresh"]
                    if name == "archery":
                        assert (qualified / (arm + "-archery")).as_uri() in found[0]["package_id"]
                    if name == "triomphe":
                        assert ((qualified / "candidate-triomphe").as_uri() in found[0]["package_id"]
                                if arm == "candidate" else "registry+" in found[0]["package_id"])
                binary = destination / "timing"
                shutil.copyfile(root / "target/release/kv9-outlined-mutation-experiment", binary)
                binary.chmod(0o755)
                result["arms"][arm] = {"path": str(binary), "sha256": sha(binary), "bytes": binary.stat().st_size}
                with (destination / "protocol-tests.stdout").open("x") as out, \
                        (destination / "protocol-tests.stderr").open("x") as err:
                    build.run(["cargo", "test", "--offline", "--locked", "--release", "--all-features",
                               "--bin", "kv9-outlined-mutation-experiment"], stdout=out, stderr=err)
            for path, digest in identity.items():
                assert sha(Path(path)) == digest
            command = ["python3", "-B", str(Path(__file__).with_name("inspect_elf.py")), str(binary),
                       str(output / ("codegen-aligned64-" + arm))]
            subprocess.run(command, check=True, timeout=60)
            inspection = json.loads((output / ("codegen-aligned64-" + arm) / "result.json").read_text())
            assert all(value["mod64"] == 0 for value in inspection["functions"].values())
            print(json.dumps({"arm": arm, "complete": True, "binary": result["arms"][arm]}), flush=True)
        for path, digest in result["sources"].items():
            assert sha(Path(path)) == digest
        result["complete"] = True
    except BaseException as error:
        result["failure"] = repr(error)
        raise
    finally:
        result["ended_ns"] = time.time_ns()
        (output / "build-summary.json").write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
