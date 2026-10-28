#!/usr/bin/env python3
"""Check TTL component lemmas and defect controls; no full protocol acceptance."""
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
    "clock_never_regresses", "old_sample_keeps_floor", "expiry_never_reverses",
    "no_early_expiry", "delayed_apply_keeps_deadline", "cleanup_preserves_new_version",
    "cleanup_preserves_unexpired", "cleanup_requires_exact_current",
    "cleanup_does_not_move_clock", "inherited_floor_keeps_source_expiry",
]
INPUTS = ["crates/common/src/raw_ttl.rs", "crates/txn/src/raw/ttl.rs",
          "proofs/lean/raw-ttl/Ttl.lean", "proofs/lean/raw-ttl/check.py"]


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
    source = (ROOT / "proofs/lean/raw-ttl/Ttl.lean").read_text()
    clean(source)
    require(re.findall(r"^theorem (\w+)", source, re.M) == THEOREMS, "theorem inventory differs")
    (args.output / "Ttl.lean").write_text(source)
    with tempfile.TemporaryDirectory(prefix="kv9-ttl-proof-") as temporary:
        work = Path(temporary)

        def compile_lean(label, candidate, accepted):
            (work / "Ttl.lean").write_text(candidate)
            result = run(label, [str(args.lean), "-DwarningAsError=true", "-o", "Ttl.olean", "Ttl.lean"], work)
            require((result.returncode == 0) == accepted, f"unexpected result: {label}")
            if not accepted:
                require("Type mismatch" in result.stdout or "unsolved goals" in result.stdout
                        or "omega could not prove" in result.stdout, f"nonsemantic failure: {label}")

        compile_lean("positive", source, True)
        (work / "Audit.lean").write_text("import Ttl\n" + "\n".join(
            f"#print axioms Kv9.RawTtl.{name}" for name in THEOREMS) + "\n")
        audit = run("axioms", [str(args.lean), "Audit.lean"], work, dict(os.environ, LEAN_PATH=str(work)))
        require(audit.returncode == 0 and len(audit.stdout.splitlines()) == len(THEOREMS), "incomplete axiom audit")
        for line in audit.stdout.splitlines():
            if "does not depend on any axioms" not in line:
                axioms = line.split("[", 1)[1].split("]", 1)[0].split(", ")
                require(set(axioms) <= {"propext", "Classical.choice", "Quot.sound"}, "untrusted axiom")
        controls = {
            "clock-regression": ("clock := max s.clock observed", "clock := observed"),
            "cleanup-new-version": ("s.current = some observed ∧ observed.deadline ≤ s.clock", "observed.deadline ≤ s.clock"),
            "cleanup-live-value": ("s.current = some observed ∧ observed.deadline ≤ s.clock", "s.current = some observed"),
            "clock-bound-missing": ("(clockBound : written ≤ upper)", "(clockBound : True)"),
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
    report.update(component_accepted=True, theorems=THEOREMS, semantic_controls=4,
                  policy_controls=2, lean_sha256=digest(args.lean))
    (args.output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print("TTL_COMPONENT_LEMMAS_ACCEPTED; full TTL phase remains open")


if __name__ == "__main__":
    main()
