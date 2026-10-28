#!/usr/bin/env python3
"""Check retirement safety/progress models and semantic defect controls.

Retains exact source hashes and compiler/checker logs. This is an abstract
model gate, not Rust extraction, production catalog GC or Chaos Mesh evidence.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
JAR_SHA256 = "936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88"
THEOREMS = [
    "initial_safe", "step_safe", "run_safe", "running_slot_is_not_free",
    "borrowed_payload_stays_open", "deletion_requires_fence_and_closed_owners",
    "lock_retained_while_open", "fence_survives_every_transition",
    "retired_group_cannot_dispatch", "drained_group_can_close",
]
INPUTS = [
    "crates/raft/src/driver/pool.rs", "crates/raft/src/grpc/groups.rs",
    "crates/raft/src/work.rs", "crates/server/src/region_manager.rs",
    "crates/server/src/region_manager/tests.rs",
    "proofs/lean/group-retirement/Retirement.lean",
    "proofs/tla/group-retirement/Retirement.tla",
    "proofs/tla/group-retirement/Safety.cfg", "proofs/tla/group-retirement/Drain.cfg",
    "scripts/prove-group-retirement.py",
]


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def clean(source):
    require(not re.search(r"\b(sorry|admit|axiom|unsafe)\b", source), "proof policy violation")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lean", type=Path, required=True)
    parser.add_argument("--jar", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--lean-only", action="store_true",
                        help="Check Lean only; leaves the combined TLA/Lean gate unaccepted")
    args = parser.parse_args()
    args.lean, args.jar, args.output = (p.resolve() for p in (args.lean, args.jar, args.output))
    require(digest(args.jar) == JAR_SHA256, "wrong TLC jar")
    args.output.mkdir(parents=True, exist_ok=False)
    hashes = {name: digest(ROOT / name) for name in INPUTS}
    report = dict(accepted=False, source_sha256=hashes, commands=[],
                  lean_accepted=False, tla_accepted=False,
                  rust_extraction=False, chaos_mesh=False, catalog_gc=False)
    report_path = args.output / "result.json"

    def run(label, command, cwd, env=None):
        result = subprocess.run(command, cwd=cwd, env=env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        (args.output / f"{label}.log").write_text(result.stdout)
        report["commands"].append(dict(label=label, command=list(map(str, command)),
                                        exit_code=result.returncode))
        report_path.write_text(json.dumps(report, indent=2) + "\n")
        return result

    version = run("lean-version", [str(args.lean), "--version"], ROOT)
    require(version.returncode == 0 and "version 4.33.1" in version.stdout, "wrong Lean version")
    lean_source = (ROOT / INPUTS[5]).read_text()
    clean(lean_source)
    tla_source = (ROOT / INPUTS[6]).read_text()
    (args.output / "Retirement.lean").write_text(lean_source)
    (args.output / "Retirement.tla").write_text(tla_source)

    with tempfile.TemporaryDirectory(prefix="kv9-retirement-proof-") as temporary:
        work = Path(temporary)

        def compile_lean(label, source, positive):
            path = work / "Retirement.lean"
            path.write_text(source)
            result = run(label, [str(args.lean), "-DwarningAsError=true", "-o",
                                str(work / "Retirement.olean"), str(path)], work)
            require((result.returncode == 0) == positive, f"{label}: unexpected proof result")
            if not positive:
                require("unsolved goals" in result.stdout or "Type mismatch" in result.stdout,
                        f"{label}: rejected outside the semantic proof")

        compile_lean("lean-positive", lean_source, True)
        audit = work / "Audit.lean"
        audit.write_text("import Retirement\n" + "\n".join(
            f"#print axioms Kv9.GroupRetirement.{name}" for name in THEOREMS) + "\n")
        result = run("lean-axioms", [str(args.lean), str(audit)], work,
                     dict(os.environ, LEAN_PATH=str(work)))
        require(result.returncode == 0 and len(result.stdout.splitlines()) == len(THEOREMS),
                "incomplete axiom audit")
        for line in result.stdout.splitlines():
            if "does not depend on any axioms" not in line:
                names = line.split("[", 1)[1].split("]", 1)[0].split(", ")
                require(set(names) <= {"propext", "Classical.choice", "Quot.sound"}, "untrusted axiom")

        lean_controls = {
            "running-slot-reuse": ("(stopped : s.stopped = true) (idle : s.running = false)",
                                   "(stopped : s.stopped = true) (idle : True)"),
            "close-with-reader": ("(unborrowed : s.external = 0) (durable : s.fenced = true)",
                                  "(unborrowed : True) (durable : s.fenced = true)"),
            "delete-open-payload": ("| delete {s} (durable : s.fenced = true) (closed : s.opened = false)",
                                    "| delete {s} (durable : s.fenced = true) (closed : True)"),
            "unlock-open-payload": ("| unlock {s} (closed : s.opened = false)", "| unlock {s} (closed : True)"),
            "resurrect-retired": ("(notRetired : s.fenced = false)", "(notRetired : True)"),
        }
        for label, (before, after) in lean_controls.items():
            require(before in lean_source, f"missing Lean control {label}")
            compile_lean(f"lean-negative-{label}", lean_source.replace(before, after, 1), False)
        for bad in ("axiom bad : False", "theorem bad : False := by sorry"):
            try:
                clean(lean_source + "\n" + bad)
            except RuntimeError:
                pass
            else:
                raise RuntimeError("proof policy control accepted")

        report.update(lean_accepted=True, theorems=THEOREMS,
                      lean_semantic_controls=len(lean_controls), policy_controls=2,
                      lean_sha256=digest(args.lean), jar_sha256=JAR_SHA256)
        report_path.write_text(json.dumps(report, indent=2) + "\n")
        if args.lean_only:
            require(hashes == {name: digest(ROOT / name) for name in INPUTS}, "sources changed during proof")
            print("GROUP_RETIREMENT_LEAN_ACCEPTED: 10 theorems; 5 semantic controls; TLA gate remains open")
            return

        def check_tla(label, source, config, positive):
            case = work / label
            case.mkdir()
            (case / "Retirement.tla").write_text(source)
            config_source = (ROOT / "proofs/tla/group-retirement" / config).read_text()
            (case / "Run.cfg").write_text(config_source)
            (args.output / f"{label}.cfg").write_text(config_source)
            result = run(label, ["java", "-XX:+UseParallelGC", "-Xmx512m", "-cp", str(args.jar),
                                 "tlc2.TLC", "-tool", "-workers", "1", "-config", "Run.cfg",
                                 "-metadir", str(case / "states"), "Retirement.tla"], case)
            require("TLC2 Version 2.19 of 08 August 2024" in result.stdout, "wrong TLC version")
            if positive:
                require(result.returncode == 0 and "No error has been found" in result.stdout,
                        f"{label}: model not accepted")
            else:
                require(result.returncode != 0 and "Invariant Safe is violated" in result.stdout,
                        f"{label}: missing expected counterexample")

        check_tla("tlc-safety", tla_source, "Safety.cfg", True)
        check_tla("tlc-drain", tla_source, "Drain.cfg", True)
        tla_controls = {
            "running-slot-reuse": ("Unregister == /\\ stopped /\\ ~running", "Unregister == /\\ stopped"),
            "close-with-reader": ("/\\ external = 0 /\\ fenced", "/\\ fenced"),
            "delete-open-payload": ("Delete == /\\ fenced /\\ ~opened", "Delete == /\\ fenced"),
            "unlock-open-payload": ("Unlock == /\\ ~opened", "Unlock == /\\ TRUE"),
            "resurrect-retired": ("Recover == /\\ ~fenced", "Recover == /\\ TRUE"),
        }
        for label, (before, after) in tla_controls.items():
            require(tla_source.count(before) == 1, f"missing TLA control {label}")
            check_tla(f"tlc-negative-{label}", tla_source.replace(before, after), "Safety.cfg", False)
    require(hashes == {name: digest(ROOT / name) for name in INPUTS}, "sources changed during proof")
    report.update(accepted=True, tla_accepted=True, theorems=THEOREMS, lean_semantic_controls=len(lean_controls),
                  tla_semantic_controls=len(tla_controls), policy_controls=2,
                  lean_sha256=digest(args.lean), jar_sha256=JAR_SHA256)
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    print("GROUP_RETIREMENT_MODELS_ACCEPTED: 10 theorems; safety/drain; 10 semantic controls")


if __name__ == "__main__":
    main()
