#!/usr/bin/env python3
"""Check the preparation model, axiom inventory, source pins and defect controls."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--lean", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    contract = json.loads((root / "proofs/lean/group-preparation/source-contract.json").read_text())
    assert contract["schema"] == 1
    for name, digest in contract["source_sha256"].items():
        assert hashlib.sha256((root / name).read_bytes()).hexdigest() == digest, name
    source = (root / "proofs/lean/group-preparation/Preparation.lean").read_text()

    def clean(text):
        assert not re.search(r"\b(sorry|admit|axiom|unsafe)\b", text), "proof hole or custom axiom"

    clean(source)
    commands = []

    def compile_source(label, text, expected_success, failure=None):
        directory = output / label
        directory.mkdir()
        path = directory / "Preparation.lean"
        path.write_text(text)
        command = [args.lean, "-DwarningAsError=true", "-o", str(directory / "Preparation.olean"), str(path)]
        result = subprocess.run(command, cwd=directory, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        (directory / "compiler.log").write_text(result.stdout)
        assert (result.returncode == 0) == expected_success, (label, result.stdout)
        if failure:
            assert failure in result.stdout, (label, result.stdout)
        commands.append({"label": label, "command": command, "exit_code": result.returncode})

    compile_source("positive", source, True)
    audit = output / "positive/Audit.lean"
    audit.write_text("import Preparation\n" + "\n".join(f"#print axioms Kv9.GroupPreparation.{name}" for name in contract["theorems"]) + "\n")
    env = dict(os.environ, LEAN_PATH=str(output / "positive"))
    result = subprocess.run([args.lean, "-DwarningAsError=true", str(audit)], cwd=audit.parent, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
    (output / "axioms.log").write_text(result.stdout)
    assert result.returncode == 0, result.stdout
    assert len(result.stdout.splitlines()) == len(contract["theorems"])
    for name in contract["theorems"]:
        line = next(line for line in result.stdout.splitlines() if f"Kv9.GroupPreparation.{name}'" in line)
        if "does not depend on any axioms" not in line:
            names = line.split("[", 1)[1].split("]", 1)[0].split(", ")
            assert set(names) <= {"propext", "Classical.choice", "Quot.sound"}, line
    controls = [
        ("missing-engine-gate", "(raft : s.raftDurable = true) (engine : s.engineDurable = true)", "(raft : s.raftDurable = true)"),
        ("wrong-binding", "binding := some authority", "binding := some (authority + 1)"),
        ("failed-report", "{s with failed := true, reported := false}", "{s with failed := true}"),
        ("ready-reinitialization", "s.phase == .intent && !s.failed", "(s.phase == .intent || s.phase == .ready) && !s.failed"),
    ]
    for label, before, after in controls:
        assert source.count(before) == 1, label
        compile_source(label, source.replace(before, after), False, "unsolved goals")
    for text in (source + "\naxiom injected : False\n", source + "\ntheorem injected : False := by sorry\n"):
        try:
            clean(text)
        except AssertionError:
            pass
        else:
            raise AssertionError("forbidden proof declaration was accepted")
    report = {"accepted": True, "theorems": len(contract["theorems"]), "semantic_controls": len(controls), "hole_and_axiom_controls": 2,
              "source_sha256": contract["source_sha256"], "commands": commands,
              "lean_sha256": hashlib.sha256(Path(args.lean).read_bytes()).hexdigest(),
              "complete_region_manager_gate": False, "new_chaos_acceptance": False, "new_qps": False}
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print("GROUP_PREPARATION_PROOF_ACCEPTED: 9 theorems; 4 semantic and 2 policy controls")


if __name__ == "__main__":
    main()
