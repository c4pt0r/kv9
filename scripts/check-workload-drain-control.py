#!/usr/bin/env python3
"""Require a stopped workload to drain an applied write until its terminal response."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "crates/server/src/workload/runner.rs"
TEST = "client::tests::workload_stop_drains_an_applied_write_until_its_terminal_response"
BEFORE = 'if interrupted(options) { shared.stop("stop_file"); }'
AFTER = 'if interrupted(options) { shared.stop("stop_file"); tasks.abort_all(); }'
EXPECTED = "stop abandoned the issued write before its terminal response"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    original = (ROOT / SOURCE).read_text()
    if original.count(BEFORE) != 1:
        raise RuntimeError("drain mutation anchor is not unique")
    mutant = original.replace(BEFORE, AFTER)
    sha = lambda text: hashlib.sha256(text.encode()).hexdigest()
    manifest = dict(revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                    source=SOURCE, source_sha256=sha(original), mutant_sha256=sha(mutant), test=TEST,
                    expected_failure=EXPECTED, sources={p: sha((ROOT / p).read_text()) for p in (
                        "scripts/check-workload-drain-control.py", "crates/server/src/client/tests.rs")}, runs=[])
    (args.output / "original.rs").write_text(original)
    (args.output / "mutant.rs").write_text(mutant)
    with tempfile.TemporaryDirectory(prefix="kv9-workload-drain-control.") as temp:
        tree = Path(temp)
        for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates", "src", "proto"):
            source = ROOT / name
            if source.is_dir():
                shutil.copytree(source, tree / name)
            else:
                shutil.copy2(source, tree / name)
        for phase, source in (("baseline", original), ("mutant", mutant), ("restored", original)):
            (tree / SOURCE).write_text(source)
            command = ["cargo", "test", "--locked", "-p", "kv9-server", "--lib", TEST, "--", "--exact"]
            env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get("CARGO_TARGET_DIR", str((ROOT / "target").resolve())))
            with (args.output / (phase + ".log")).open("w") as log:
                result = subprocess.run(command, cwd=tree, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=180)
            text = (args.output / (phase + ".log")).read_text()
            if "running 1 test\n" not in text:
                raise RuntimeError(phase + ": fixture must compile and select exactly one test")
            if phase == "mutant":
                if result.returncode != 101 or EXPECTED not in text or "1 failed;" not in text:
                    raise RuntimeError("drain mutant lacks its intended assertion failure")
            elif result.returncode or "1 passed; 0 failed;" not in text:
                raise RuntimeError(phase + ": correct drain was rejected")
            manifest["runs"].append(dict(phase=phase, command=command, source_sha256=sha(source), exit_code=result.returncode))
            (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print("PASS: stopped-workload drain control compiled, failed its intended assertion and passed after restoration")


if __name__ == "__main__":
    main()
