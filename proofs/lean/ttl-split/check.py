#!/usr/bin/env python3
"""Check TTL import cursor/clock/completion lemmas; full production transfer stays open."""
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
    "initial_valid", "step_preserves", "reachable_valid", "watermark_never_regresses",
    "importing_keeps_source_watermark", "publication_requires_complete_commitment",
    "completed_cursor_cannot_change", "completion_is_permanent", "copied_count_never_regresses",
]
INPUTS = ["crates/common/src/raw_ttl/split.rs", "crates/raft/src/command.rs",
          "crates/raft/src/command/ttl_split.rs", "crates/raft/src/driver/ttl_split.rs",
          "crates/raft/src/state_machine/ttl_split.rs", "crates/raft/src/state_machine/ttl_split/tests.rs",
          "crates/raft/src/state_machine/ttl_split/tests/atomic.rs", "crates/raft/src/state_machine/ttl_split/tests/replicated.rs",
          "crates/raft/src/state_machine/checkpoint_recovery/data_group.rs",
          "crates/raft/src/state_machine/checkpoint_recovery/data_group/tail.rs",
          "crates/raft/src/state_machine/checkpoint_recovery/data_group/tail/tests.rs",
          "crates/engine/src/lib.rs", "crates/engine/src/mem.rs",
          "proofs/lean/ttl-split/Transfer.lean", "proofs/lean/ttl-split/check.py",
          "proofs/tla/ttl_split/Transfer.tla", "proofs/tla/ttl_split/Transfer.cfg"]


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
    source = (ROOT / "proofs/lean/ttl-split/Transfer.lean").read_text()
    clean(source)
    require(re.findall(r"^theorem (\w+)", source, re.M) == THEOREMS, "theorem inventory differs")
    (args.output / "Transfer.lean").write_text(source)
    with tempfile.TemporaryDirectory(prefix="kv9-ttl-transfer-proof-") as temporary:
        work = Path(temporary)

        def compile_lean(label, candidate, accepted):
            (work / "Transfer.lean").write_text(candidate)
            result = run(label, [str(args.lean), "-DwarningAsError=true", "-o", "Transfer.olean", "Transfer.lean"], work)
            require((result.returncode == 0) == accepted, f"unexpected result: {label}")
            if not accepted:
                require("type mismatch" in result.stdout.lower() or "unsolved goals" in result.stdout
                        or "omega could not prove" in result.stdout
                        or "Tactic `rfl` failed" in result.stdout, f"nonsemantic failure: {label}")

        compile_lean("positive", source, True)
        (work / "Audit.lean").write_text("import Transfer\n" + "\n".join(
            f"#print axioms Kv9.TtlSplit.{name}" for name in THEOREMS) + "\n")
        audit = run("axioms", [str(args.lean), "Audit.lean"], work, dict(os.environ, LEAN_PATH=str(work)))
        require(audit.returncode == 0 and len(audit.stdout.splitlines()) == len(THEOREMS), "incomplete axiom audit")
        for line in audit.stdout.splitlines():
            if "does not depend on any axioms" not in line:
                axioms = line.split("[", 1)[1].split("]", 1)[0].split(", ")
                require(set(axioms) <= {"propext", "Classical.choice", "Quot.sound"}, "untrusted axiom")
        controls = {
            "finish-without-count": ("(count : s.copied = p.rows)", "(count : True)"),
            "finish-without-digest": ("(digest : s.chain = p.digest)", "(digest : True)"),
            "append-after-complete": ("(copying : s.complete = false)", "(copying : True)"),
            "clock-before-complete": ("(ready : s.complete = true)", "(ready : True)"),
            "forget-inherited-clock": ("⟨0, p.seed, p.floor, false, false⟩", "⟨0, p.seed, 0, false, false⟩"),
            "copy-beyond-count": ("(within : s.copied + rows ≤ p.rows)", "(within : True)"),
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
    print("TTL_TRANSFER_COMPONENT_LEMMAS_ACCEPTED; source/gate/runtime formal refinement and full phase remain open")


if __name__ == "__main__":
    main()
