#!/usr/bin/env python3
"""Check durable checkpoint sweep component lemmas; complete closure/GC stays open."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
THEOREMS = [
    "initial_valid", "step_preserves", "reachable_valid", "passed_prefix_has_no_held",
    "done_excludes_indexed_held", "cursor_never_moves_backwards", "sequence_never_moves_backwards",
    "sweep_keeps_global_protection", "done_allows_published",
]
INPUTS = ["crates/meta/src/retention.rs", "crates/meta/src/retention/checkpoint_admission.rs",
          "crates/meta/src/retention/checkpoint_admission/sweep.rs",
          "crates/server/src/runtime/checkpoint_sweep.rs", "crates/server/src/runtime.rs",
          "crates/server/src/maintenance.rs",
          "proofs/tla/checkpoint_sweep/CheckpointSweep.tla", "proofs/tla/checkpoint_sweep/Sweep.cfg",
          "proofs/lean/checkpoint-sweep/Sweep.lean", "proofs/lean/checkpoint-sweep/check.py"]


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
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.lean = args.lean.resolve()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    hashes = {name: digest(ROOT / name) for name in INPUTS}
    report = dict(component_accepted=False, phase_accepted=False, rust_extraction=False,
                  tla_checked=False, source_sha256=hashes, commands=[])

    def run(label, command, cwd, env=None):
        result = subprocess.run(command, cwd=cwd, env=env, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        (args.output / f"{label}.log").write_text(result.stdout)
        report["commands"].append(dict(label=label, command=list(map(str, command)), exit_code=result.returncode))
        (args.output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
        return result

    version = run("version", [str(args.lean), "--version"], ROOT)
    require(version.returncode == 0 and "version 4.33.1" in version.stdout, "wrong Lean version")
    source = (ROOT / "proofs/lean/checkpoint-sweep/Sweep.lean").read_text()
    clean(source)
    require(re.findall(r"^theorem (\w+)", source, re.M) == THEOREMS, "theorem inventory differs")
    (args.output / "Sweep.lean").write_text(source)
    with tempfile.TemporaryDirectory(prefix="kv9-sweep-proof-") as temporary:
        work = Path(temporary)

        def compile_lean(label, candidate, accepted):
            (work / "Sweep.lean").write_text(candidate)
            result = run(label, [str(args.lean), "-DwarningAsError=true", "-o", "Sweep.olean", "Sweep.lean"], work)
            require((result.returncode == 0) == accepted, f"unexpected result: {label}")
            if not accepted:
                require("type mismatch" in result.stdout.lower() or "unsolved goals" in result.stdout
                        or "omega could not prove" in result.stdout
                        or "Tactic `rfl` failed" in result.stdout, f"nonsemantic failure: {label}")

        compile_lean("positive", source, True)
        (work / "Audit.lean").write_text("import Sweep\n" + "\n".join(
            f"#print axioms Kv9.CheckpointSweep.{name}" for name in THEOREMS) + "\n")
        audit = run("axioms", [str(args.lean), "Audit.lean"], work, dict(os.environ, LEAN_PATH=str(work)))
        require(audit.returncode == 0 and len(audit.stdout.splitlines()) == len(THEOREMS), "incomplete axiom audit")
        for line in audit.stdout.splitlines():
            if "does not depend on any axioms" not in line:
                axioms = line.split("[", 1)[1].split("]", 1)[0].split(", ")
                require(set(axioms) <= {"propext", "Classical.choice", "Quot.sound"}, "untrusted axiom")
        controls = {
            "skip-held-page": ("(pageClear : ∀ i, s.frontier ≤ i → i < next → s.held i = false)",
                               "(pageClear : ∀ i, s.frontier ≤ i → i < next → True)"),
            "late-held-after-close": ("if j = i then false else s.held j", "if j = i then true else s.held j"),
            "false-eof": ("done := decide (next = bound)", "done := true"),
            "rewind-cursor": ("(forward : s.frontier ≤ next)", "(forward : True)"),
            "restart-drops-progress": ("| restart (s) : Step bound s s",
                                       "| restart (s) : Step bound s {s with frontier := 0, sequence := 0, done := false}"),
            "drop-global-protection": ("{s with published := false}", "{s with published := false, covered := false}"),
            "stale-cas-accepted": ("(cas : expected = s.sequence)", "(cas : True)"),
        }
        for label, (before, after) in controls.items():
            require(before in source, f"missing control: {label}")
            compile_lean(label, source.replace(before, after, 1), False)
        for extra in ("axiom bad : False", "theorem bad : False := by sorry"):
            try:
                clean(source + "\n" + extra)
            except RuntimeError:
                pass
            else:
                raise RuntimeError("proof policy control accepted")
    require(hashes == {name: digest(ROOT / name) for name in INPUTS}, "source changed")
    report.update(component_accepted=True, theorems=THEOREMS, semantic_controls=7,
                  policy_controls=2, lean_sha256=digest(args.lean))
    (args.output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print("SWEEP_COMPONENT_LEMMAS_ACCEPTED; complete publisher closure and full TTL phase remain open")


if __name__ == "__main__":
    main()
