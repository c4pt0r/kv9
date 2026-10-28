#!/usr/bin/env python3
"""Check Raw split population barrier lemmas; full TTL transfer stays open."""
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
    "ownership_is_permanent", "population_after_binding_is_inert",
    "ttl_cannot_use_default_cf_population", "changed_copy_requires_unbound_non_ttl",
    "any_delayed_copy_history_is_inert", "deleted_child_key_cannot_be_resurrected_by_copy",
    "sealed_parent_value_is_immutable",
]
INPUTS = ["crates/common/src/data_range.rs", "crates/raft/src/command.rs",
          "crates/raft/src/command/population.rs", "crates/raft/src/state_machine.rs",
          "crates/raft/src/state_machine/data_range.rs", "crates/raft/src/state_machine/data_range/population.rs",
          "crates/raft/src/state_machine/data_range/population/tests.rs", "crates/server/src/runtime.rs",
          "crates/server/src/runtime/ttl_activation/integration/split_population.rs",
          "proofs/lean/split-population/Population.lean", "proofs/lean/split-population/check.py",
          "proofs/tla/split_population/SplitPopulation.tla", "proofs/tla/split_population/Population.cfg"]


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
    source = (ROOT / "proofs/lean/split-population/Population.lean").read_text()
    clean(source)
    require(re.findall(r"^theorem (\w+)", source, re.M) == THEOREMS, "theorem inventory differs")
    (args.output / "Population.lean").write_text(source)
    with tempfile.TemporaryDirectory(prefix="kv9-population-proof-") as temporary:
        work = Path(temporary)

        def compile_lean(label, candidate, accepted):
            (work / "Population.lean").write_text(candidate)
            result = run(label, [str(args.lean), "-DwarningAsError=true", "-o", "Population.olean", "Population.lean"], work)
            require((result.returncode == 0) == accepted, f"unexpected result: {label}")
            if not accepted:
                require("type mismatch" in result.stdout.lower() or "unsolved goals" in result.stdout
                        or "omega could not prove" in result.stdout
                        or "Tactic `rfl` failed" in result.stdout, f"nonsemantic failure: {label}")

        compile_lean("positive", source, True)
        (work / "Audit.lean").write_text("import Population\n" + "\n".join(
            f"#print axioms Kv9.SplitPopulation.{name}" for name in THEOREMS) + "\n")
        audit = run("axioms", [str(args.lean), "Audit.lean"], work, dict(os.environ, LEAN_PATH=str(work)))
        require(audit.returncode == 0 and len(audit.stdout.splitlines()) == len(THEOREMS), "incomplete axiom audit")
        for line in audit.stdout.splitlines():
            if "does not depend on any axioms" not in line:
                axioms = line.split("[", 1)[1].split("]", 1)[0].split(", ")
                require(set(axioms) <= {"propext", "Classical.choice", "Quot.sound"}, "untrusted axiom")
        controls = {
            "copy-after-binding": ("s.owned = false ∧ s.ttl = false then", "s.ttl = false then"),
            "copy-ignores-ttl": ("s.owned = false ∧ s.ttl = false then", "s.owned = false then"),
            "seal-forgets-ownership": ("{s with sealed := true}", "{s with sealed := true, owned := false}"),
            "client-ignores-seal": ("s.owned = true ∧ s.sealed = false then {s with value}", "s.owned = true then {s with value}"),
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
    print("POPULATION_COMPONENT_LEMMAS_ACCEPTED; complete TTL split transfer remains open")


if __name__ == "__main__":
    main()
