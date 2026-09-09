#!/usr/bin/env python3
"""Require terminal accounting and serialized completions in complete histories."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "crates/server/src/workload/recorder.rs"
TEST = "workload::recorder::tests::complete_history_preserves_unknowns_and_out_of_order_terminals"


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    original = (ROOT / SOURCE).read_text()
    cases = [
        ("missing-terminal-accounting", "state.terminal += 1;", "// terminal accounting deliberately omitted",
         "complete workload lost a terminal record"),
        ("missing-terminal-serialization",
         'if state.file.as_mut().unwrap().write_all(&event).is_err() {\n                return invalidate(&mut state, "history terminal write failed");\n            }',
         "// terminal serialization deliberately omitted",
         "complete history lost serialized terminal records"),
    ]
    manifest = dict(revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                    source=SOURCE, source_sha256=sha(original), runner_sha256=sha(Path(__file__).read_text()), controls=[])
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get("CARGO_TARGET_DIR", str((ROOT / "target").resolve())))
    with tempfile.TemporaryDirectory(prefix="kv9-workload-controls.") as temp:
        tree = Path(temp)
        for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates", "src", "proto"):
            path = ROOT / name
            if path.is_dir(): shutil.copytree(path, tree / name)
            else: shutil.copy2(path, tree / name)
        for name, before, after, expected in cases:
            if before == after or original.count(before) != 1:
                raise RuntimeError(f"{name}: source anchor is not unique")
            mutant = original.replace(before, after)
            folder = output / name
            folder.mkdir()
            (folder / "original.rs").write_text(original)
            (folder / "mutant.rs").write_text(mutant)
            case = dict(name=name, test=TEST, expected_failure=expected, mutant_sha256=sha(mutant), runs=[])
            for phase, source in (("baseline", original), ("mutant", mutant), ("restored", original)):
                (tree / SOURCE).write_text(source)
                command = ["cargo", "test", "--locked", "-p", "kv9-server", "--lib", TEST, "--", "--exact"]
                with (folder / f"{phase}.log").open("w") as log:
                    result = subprocess.run(command, cwd=tree, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=180)
                text = (folder / f"{phase}.log").read_text()
                if "running 1 test\n" not in text:
                    raise RuntimeError(f"{name}/{phase}: expected exactly one test")
                if phase == "mutant":
                    if result.returncode == 0 or expected not in text or "1 failed;" not in text:
                        raise RuntimeError(f"{name}: missing intended assertion failure")
                elif result.returncode != 0 or "1 passed; 0 failed;" not in text:
                    raise RuntimeError(f"{name}/{phase}: valid source rejected")
                case["runs"].append(dict(phase=phase, command=command, exit_code=result.returncode, source_sha256=sha(source)))
            manifest["controls"].append(case)
            (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
            print(f"PASS: {name} baseline, intended assertion failure and restored source", flush=True)
    print("PASS: 2 isolated workload terminal/history controls checked", flush=True)


if __name__ == "__main__":
    main()
