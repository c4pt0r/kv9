#!/usr/bin/env python3
"""Check the install-evidence model and explicit defect controls."""
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
    contract = json.loads(
        (root / "proofs/lean/install-evidence/source-contract.json").read_text()
    )
    assert contract["schema"] == 1
    for name, digest in contract["source_sha256"].items():
        assert hashlib.sha256((root / name).read_bytes()).hexdigest() == digest, name
    source = (root / "proofs/lean/install-evidence/Evidence.lean").read_text()

    def clean(text):
        assert not re.search(r"\b(sorry|admit|axiom|unsafe)\b", text), "proof hole or custom axiom"

    clean(source)
    commands = []

    def compile_source(label, text, success):
        directory = output / label
        directory.mkdir()
        path = directory / "Evidence.lean"
        path.write_text(text)
        command = [args.lean, "-DwarningAsError=true", "-o", str(directory / "Evidence.olean"), str(path)]
        result = subprocess.run(command, cwd=directory, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, timeout=60)
        (directory / "compiler.log").write_text(result.stdout)
        assert (result.returncode == 0) == success, (label, result.stdout)
        if not success:
            assert any(marker in result.stdout for marker in
                       ("error: unsolved goals", "error: Type mismatch", "error: type mismatch",
                        "type mismatch", "error: Tactic", "error: omega",
                        "error: `grind` failed", "made no progress")), result.stdout
            assert not any(marker in result.stdout for marker in
                           ("unexpected token", "Unknown identifier", "unknown constant", "unknown namespace")), result.stdout
        commands.append({"label": label, "command": command, "exit_code": result.returncode})

    compile_source("positive", source, True)
    audit = output / "positive/Audit.lean"
    audit.write_text("import Evidence\n" + "\n".join(
        f"#print axioms Kv9.InstallEvidence.{name}" for name in contract["theorems"]) + "\n")
    env = dict(os.environ, LEAN_PATH=str(audit.parent))
    result = subprocess.run([args.lean, "-DwarningAsError=true", str(audit)], cwd=audit.parent,
                            env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
    (output / "axioms.log").write_text(result.stdout)
    assert result.returncode == 0, result.stdout
    assert len(result.stdout.splitlines()) == len(contract["theorems"])
    for name in contract["theorems"]:
        line = next(line for line in result.stdout.splitlines()
                    if f"Kv9.InstallEvidence.{name}'" in line)
        if "does not depend on any axioms" not in line:
            names = line.split("[", 1)[1].split("]", 1)[0].split(", ")
            assert set(names) <= {"propext", "Classical.choice", "Quot.sound"}, line
    controls = [
        ('quiesce-without-evidence',
         '(e : s.evidence = true) (pub : s.sourcePhase = 2)',
         '(e : s.evidence = s.evidence) (pub : s.sourcePhase = 2)'),
        ('record-without-adoption',
         '(a : s.adopted = true) (pub : s.sourcePhase = 2)',
         '(a : s.adopted = s.adopted) (pub : s.sourcePhase = 2)'),
        ('record-drops-the-destination-pin',
         'Step s {s with evidence := true}',
         'Step s {s with evidence := true, destinationPinned := false}'),
        ('record-with-a-divergent-subject',
         'Step s {s with evidence := true}',
         'Step s {s with evidence := true, wrongSubject := true}'),
        ('release-without-quiescence',
         '| release {s} (q : s.sourcePhase = 3) :',
         '| release {s} (q : s.sourcePhase = s.sourcePhase) :'),
        ('pin-quiesces-immediately',
         '{s with sourcePhase := 2, destinationPinned := true}',
         '{s with sourcePhase := 3, destinationPinned := true}'),
        ('quiesce-releases-the-destination',
         'Step s {s with sourcePhase := 3}',
         'Step s {s with sourcePhase := 3, destinationPinned := false}'),
        ('adopt-without-pins',
         '(m : s.migration = true) (p : s.destinationPinned = true)',
         '(m : s.migration = true) (p : s.destinationPinned = s.destinationPinned)'),
        ('release-regresses-the-phase',
         'Step s {s with sourcePhase := 4}',
         'Step s {s with sourcePhase := 0}'),
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
        "verified_rust_extraction": False, "quiesce_without_evidence_or_pin_drop": False,
        "new_chaos_acceptance": False, "new_qps": False,
    }
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"INSTALL_EVIDENCE_PROOF_ACCEPTED: {len(contract['theorems'])} theorems; {len(controls)} semantic and 2 policy controls")


if __name__ == "__main__":
    main()
