#!/usr/bin/env python3
"""Check complete real-wire response-loss histories and an unsafe retry mutant."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts/history"))
from checker import check, coverage, load, verify_witness

SOURCE = "crates/server/src/client.rs"
TEST = "client::tests::persistent_response_loss_history_fixture"
BEFORE = "Outcome::UnknownWrite { reason }\n                    };\n                    break;"
AFTER = "Outcome::UnknownWrite { reason }\n                    };\n                    continue;"


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    original = (ROOT / SOURCE).read_text()
    require(original.count(BEFORE) == 1, "source mutation anchor is not unique")
    mutant = original.replace(BEFORE, AFTER)
    (output / "original.rs").write_text(original)
    (output / "mutant.rs").write_text(mutant)
    sources = [Path(__file__), ROOT / SOURCE, ROOT / "crates/server/src/client/tests.rs",
               ROOT / "scripts/history/checker.py", ROOT / "crates/server/src/workload.rs",
               *sorted((ROOT / "crates/server/src/workload").glob("*.rs"))]
    manifest = dict(revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
                    sources={str(p.relative_to(ROOT)): sha(p.read_bytes()) for p in sources},
                    mutant_sha256=sha(mutant.encode()), runs=[])
    with tempfile.TemporaryDirectory(prefix="kv9-client-history-control.") as temp:
        tree = Path(temp)
        for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates", "src", "proto"):
            path = ROOT / name
            if path.is_dir(): shutil.copytree(path, tree / name)
            else: shutil.copy2(path, tree / name)
        for phase, source, expected in (("baseline", original, "valid"), ("mutant", mutant, "invalid"), ("restored", original, "valid")):
            folder = output / phase
            folder.mkdir()
            (tree / SOURCE).write_text(source)
            env = dict(os.environ, CARGO_TARGET_DIR=os.environ.get("CARGO_TARGET_DIR", str((ROOT / "target").resolve())),
                       KV9_WORKLOAD_HISTORY_FIXTURE=str(folder))
            command = ["cargo", "test", "--locked", "-p", "kv9-server", "--lib", TEST, "--", "--exact", "--ignored"]
            with (folder / "fixture.log").open("w") as log:
                result = subprocess.run(command, cwd=tree, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=180)
            text = (folder / "fixture.log").read_text()
            require(result.returncode == 0 and "running 1 test\n" in text and "1 passed; 0 failed; 0 ignored;" in text,
                    f"{phase}: fixture must compile and run exactly one complete test")
            fixture = json.loads((folder / "fixture.json").read_text())
            summary = fixture["history"]
            history = load(folder / "history.jsonl")
            require(fixture["fixture_only"] and summary["accounting_complete"] and summary["full_history_complete"],
                    f"{phase}: fixture accounting is incomplete")
            require(summary["issued"] == summary["terminal"] == len(history.operations) == 4,
                    f"{phase}: expected all four logical calls")
            require(summary["events"] == len(history.events) == 8 and all(o.response is not None for o in history.operations),
                    f"{phase}: missing terminal history record")
            require(summary["bytes"] == (folder / "history.jsonl").stat().st_size and not summary["independently_checked"],
                    f"{phase}: invalid unverified recorder summary")
            # No kv9 implementation, server effect count or final-value-only
            # oracle is used in this decision. Unknown writes remain in the full
            # model, including the possibility of an effect after timeout.
            verdict = check(history, max_states=200_000, seconds=10)
            require(verdict["verdict"] == expected, f"{phase}: independent history verdict {verdict['verdict']}, expected {expected}")
            if expected == "valid":
                require(verify_witness(history, verdict["witness"]), f"{phase}: independent witness replay failed")
                require(coverage(history)["outcomes"] == {"unknown": 1, "ok": 3}, f"{phase}: unknown outcome was lost")
            else:
                require(verdict["reason"] == "no_legal_execution" and verdict["attempts"][-1]["unknown_effect_limit"] is None
                        and not verdict["attempts"][-1]["guided_unknown"], f"{phase}: invalidity lacks unrestricted exhaustive search")
            (folder / "checker.json").write_text(json.dumps(verdict, indent=2) + "\n")
            manifest["runs"].append(dict(phase=phase, command=command, source_sha256=sha(source.encode()),
                                         fixture_exit_code=result.returncode, verdict=expected, coverage=coverage(history),
                                         history_sha256=sha((folder / "history.jsonl").read_bytes()), states=verdict["states"]))
            (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
            print(f"PASS: {phase} real-wire history is independently {expected}", flush=True)
    require(manifest["runs"][0]["source_sha256"] == manifest["runs"][2]["source_sha256"], "restored source differs")
    print("PASS: complete response-loss history verified; unsafe retry rejected by independent history checking", flush=True)


if __name__ == "__main__":
    main()
