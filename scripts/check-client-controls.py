#!/usr/bin/env python3
"""Exercise isolated persistent-client regressions on real gRPC tests."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SOURCE = "crates/server/src/client.rs"
CASES = [
    ("per-rpc-channel", "let mut client = self.0.clients[peer].clone();",
     'let mut client = Kv9Client::new(Endpoint::from_shared(format!("http://{}", '
     'self.0.config.peers[peer].address)).unwrap().connect_lazy());',
     "persistent_connections_are_observed_by_the_server",
     "per-RPC TCP connection creation is not persistent reuse"),
    ("retry-unknown-write",
     "Outcome::UnknownWrite { reason }\n                    };\n                    break;",
     "Outcome::UnknownWrite { reason }\n                    };\n                    continue;",
     "unknown_write_is_not_replayed_after_an_intervening_read_and_write",
     "unknown v0 was dispatched again"),
    ("reset-logical-deadline",
     "for ordinal in 1..=self.0.config.max_attempts {",
     "for ordinal in 1..=self.0.config.max_attempts {\n"
     "            let deadline = Instant::now() + Duration::from_millis(self.0.config.deadline_ms);",
     "logical_deadline_is_not_reset_by_refusals",
     "logical deadline was restarted on retry"),
]


def sha(text):
    return hashlib.sha256(text.encode()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get("CARGO_TARGET_DIR", str((ROOT / "target").resolve())))
    original = (ROOT / SOURCE).read_text()
    manifest = dict(revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                    runner_sha256=sha(Path(__file__).read_text()),
                    test_sha256=sha((ROOT / "crates/server/src/client/tests.rs").read_text()), controls=[])
    with tempfile.TemporaryDirectory(prefix="kv9-client-controls.") as temp:
        tree = Path(temp)
        for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates", "src", "proto"):
            source = ROOT / name
            if source.is_dir():
                shutil.copytree(source, tree / name)
            else:
                shutil.copy2(source, tree / name)
        for name, before, after, test, expected in CASES:
            if original.count(before) != 1 or before == after:
                raise RuntimeError(f"{name}: source anchor is not unique")
            mutant = original.replace(before, after)
            folder = output / name
            folder.mkdir()
            (folder / "original.rs").write_text(original)
            (folder / "mutant.rs").write_text(mutant)
            case = dict(name=name, source=SOURCE, source_sha256=sha(original), mutant_sha256=sha(mutant),
                        test=f"client::tests::{test}", expected_failure=expected, runs=[])
            for phase, source in (("baseline", original), ("mutant", mutant), ("restored", original)):
                (tree / SOURCE).write_text(source)
                command = ["cargo", "test", "--locked", "-p", "kv9-server", "--lib",
                           case["test"], "--", "--exact"]
                with (folder / f"{phase}.log").open("w") as log:
                    result = subprocess.run(command, cwd=tree, env=env, stdout=log,
                                            stderr=subprocess.STDOUT, timeout=180)
                text = (folder / f"{phase}.log").read_text()
                if "running 1 test\n" not in text:
                    raise RuntimeError(f"{name}/{phase}: expected exactly one selected test")
                if phase == "mutant":
                    if result.returncode == 0 or expected not in text or "1 failed;" not in text:
                        raise RuntimeError(f"{name}: mutant missed intended assertion")
                elif result.returncode or "1 passed; 0 failed;" not in text:
                    raise RuntimeError(f"{name}/{phase}: valid source rejected")
                case["runs"].append(dict(phase=phase, command=command, exit_code=result.returncode, source_sha256=sha(source)))
            manifest["controls"].append(case)
            (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
            print(f"PASS: {name} baseline, intended assertion failure and restored source", flush=True)
    print("PASS: 3 isolated persistent-client source controls checked", flush=True)


if __name__ == "__main__":
    main()
