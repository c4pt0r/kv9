#!/usr/bin/env python3
"""Check Durable activation desire and bounded reconciliation and explicit defect controls."""
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
    contract = json.loads((root / "proofs/lean/group-control/source-contract.json").read_text())
    assert contract["schema"] == 1
    for name, digest in contract["source_sha256"].items():
        assert hashlib.sha256((root / name).read_bytes()).hexdigest() == digest, name
    source = (root / "proofs/lean/group-control/Control.lean").read_text()

    def clean(text):
        assert not re.search(r"\b(sorry|admit|axiom|unsafe)\b", text), "proof hole or custom axiom"

    clean(source)
    commands = []

    def compile_source(label, text, success):
        directory = output / label
        directory.mkdir()
        path = directory / "Control.lean"
        path.write_text(text)
        command = [args.lean, "-DwarningAsError=true", "-o", str(directory / "Control.olean"), str(path)]
        result = subprocess.run(command, cwd=directory, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, timeout=60)
        (directory / "compiler.log").write_text(result.stdout)
        assert (result.returncode == 0) == success, (label, result.stdout)
        if not success:
            assert "error" in result.stdout and "unexpected token" not in result.stdout, result.stdout
        commands.append({"label": label, "command": command, "exit_code": result.returncode})

    compile_source("positive", source, True)
    audit = output / "positive/Audit.lean"
    audit.write_text("import Control\n" + "\n".join(
        f"#print axioms Kv9.GroupControl.{name}" for name in contract["theorems"]) + "\n")
    env = dict(os.environ, LEAN_PATH=str(audit.parent))
    result = subprocess.run([args.lean, "-DwarningAsError=true", str(audit)], cwd=audit.parent,
                            env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
    (output / "axioms.log").write_text(result.stdout)
    assert result.returncode == 0, result.stdout
    assert len(result.stdout.splitlines()) == len(contract["theorems"])
    for name in contract["theorems"]:
        line = next(line for line in result.stdout.splitlines() if f"Kv9.GroupControl.{name}'" in line)
        if "does not depend on any axioms" not in line:
            names = line.split("[", 1)[1].split("]", 1)[0].split(", ")
            assert set(names) <= {"propext", "Classical.choice", "Quot.sound"}, line
    controls = [
        ('creation-without-desire', '(committed : s.desire = some localBinding)', '(committed : s.creation = some localBinding)'),
        ('failed-start', '(healthy : s.failed = false)', '(healthy : True)'),
        ('rebound-creation', '(compatible : s.creation = none ∨ s.creation = some binding)', '(compatible : True)'),
        ('wrong-desire-binding', 'creation := some binding, desire := some binding, staged := none', 'creation := some binding, desire := some (binding + 1), staged := none'),
        ('uncommitted-publication', 'Step localBinding s {s with staged := some binding}', 'Step localBinding s {s with staged := some binding, desire := some binding}'),
        ('restart-loses-desire', '| restart {s} : Step localBinding s {s with running := false, failed := false, staged := none, acknowledged := false}', '| restart {s} : Step localBinding s {s with running := false, failed := false, staged := none, acknowledged := false, desire := none}'),
        ('unbounded-turn', 'if eligible c then [c] else select cs', 'if eligible c then c :: select cs else select cs'),
        ('wrong-local-store', 'c.exactLocalStore && !c.running && !c.failed', '!c.running && !c.failed'),
        ('failed-slot-retry', 'c.exactLocalStore && !c.running && !c.failed', 'c.exactLocalStore && !c.running'),
    ]
    for label, before, after in controls:
        assert source.count(before) == 1, label
        compile_source(label, source.replace(before, after), False)
    for invalid in (source + "\naxiom injected : False\n", source + "\ntheorem injected : False := by sorry\n"):
        try:
            clean(invalid)
        except AssertionError:
            pass
        else:
            raise AssertionError("forbidden proof declaration was accepted")
    report = {
        "accepted": True, "theorems": len(contract["theorems"]), "semantic_controls": len(controls),
        "hole_and_axiom_controls": 2, "source_sha256": contract["source_sha256"], "commands": commands,
        "lean_sha256": hashlib.sha256(Path(args.lean).read_bytes()).hexdigest(),
        "verified_rust_extraction": False, "complete_region_manager_gate": False,
        "new_chaos_acceptance": False, "new_qps": False,
    }
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print("GROUP_CONTROL_PROOF_ACCEPTED: 12 theorems; 9 semantic and 2 policy controls")


if __name__ == "__main__":
    main()
