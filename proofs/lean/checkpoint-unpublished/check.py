#!/usr/bin/env python3
"""Check unpublished checkpoint retirement component lemmas; complete closure/GC stays open."""
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
    "initial_valid", "step_preserves", "reachable_valid", "retired_never_authorized_or_used",
    "retired_has_closed_gate_and_terminal_phase", "closed_never_reopens",
    "retirement_evidence_never_disappears", "published_receipt_excludes_retirement",
    "retirement_keeps_global_protection",
]
INPUTS = ["crates/meta/src/retention.rs", "crates/meta/src/retention/checkpoint_intent.rs",
          "crates/meta/src/retention/checkpoint_admission.rs",
          "crates/meta/src/retention/checkpoint_admission/retirement.rs",
          "crates/server/src/checkpoint_retention/execution.rs",
          "crates/server/src/checkpoint_retention/retirement.rs",
          "crates/server/src/checkpoint_retention.rs", "crates/server/src/runtime/checkpoint_retention.rs",
          "crates/server/src/runtime/data_checkpoint/worker.rs", "crates/server/src/remote_storage.rs",
          "proofs/tla/checkpoint_unpublished/CheckpointUnpublished.tla",
          "proofs/tla/checkpoint_unpublished/Unpublished.cfg",
          "proofs/lean/checkpoint-unpublished/Unpublished.lean",
          "proofs/lean/checkpoint-unpublished/check.py"]


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
    source = (ROOT / "proofs/lean/checkpoint-unpublished/Unpublished.lean").read_text()
    clean(source)
    require(re.findall(r"^theorem (\w+)", source, re.M) == THEOREMS, "theorem inventory differs")
    (args.output / "Unpublished.lean").write_text(source)
    with tempfile.TemporaryDirectory(prefix="kv9-unpublished-proof-") as temporary:
        work = Path(temporary)

        def compile_lean(label, candidate, accepted):
            (work / "Unpublished.lean").write_text(candidate)
            result = run(label, [str(args.lean), "-DwarningAsError=true", "-o", "Unpublished.olean", "Unpublished.lean"], work)
            require((result.returncode == 0) == accepted, f"unexpected result: {label}")
            if not accepted:
                require("Type mismatch" in result.stdout or "unsolved goals" in result.stdout
                        or "omega could not prove" in result.stdout, f"nonsemantic failure: {label}")

        compile_lean("positive", source, True)
        (work / "Audit.lean").write_text("import Unpublished\n" + "\n".join(
            f"#print axioms Kv9.CheckpointUnpublished.{name}" for name in THEOREMS) + "\n")
        audit = run("axioms", [str(args.lean), "Audit.lean"], work, dict(os.environ, LEAN_PATH=str(work)))
        require(audit.returncode == 0 and len(audit.stdout.splitlines()) == len(THEOREMS), "incomplete axiom audit")
        for line in audit.stdout.splitlines():
            if "does not depend on any axioms" not in line:
                axioms = line.split("[", 1)[1].split("]", 1)[0].split(", ")
                require(set(axioms) <= {"propext", "Classical.choice", "Quot.sound"}, "untrusted axiom")
        controls = {
            "retire-without-close": ("| retire (s) (closed : s.closed = true)", "| retire (s) (closed : True)"),
            "retire-published": ("| retire (s) (closed : s.closed = true) (held : s.phase = .held)", "| retire (s) (closed : s.closed = true) (held : s.phase = .held ∨ s.phase = .published)"),
            "receipt-without-commit": ("| receipt (s) (committed : s.published = true)", "| receipt (s) (committed : True)"),
            "use-without-receipt": ("| use (s) (confirmed : s.receipt = true)", "| use (s) (confirmed : True)"),
            "reenter-released": ("| enter (s) (available : s.phase = .absent)", "| enter (s) (available : s.phase = .absent ∨ s.phase = .released)"),
            "invent-generic-release-evidence": ("| release (s) (drained : s.phase = .quiesced) : Step s {s with phase := .released}", "| release (s) (drained : s.phase = .quiesced) : Step s {s with phase := .released, retired := true}"),
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
    report.update(component_accepted=True, theorems=THEOREMS, semantic_controls=6,
                  policy_controls=2, lean_sha256=digest(args.lean))
    (args.output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print("UNPUBLISHED_COMPONENT_LEMMAS_ACCEPTED; complete publisher closure and full TTL phase remain open")


if __name__ == "__main__":
    main()
