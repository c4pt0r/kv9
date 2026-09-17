#!/usr/bin/env python3
"""Check the child-population model and explicit defect controls."""
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
        (root / "proofs/lean/child-population/source-contract.json").read_text()
    )
    assert contract["schema"] == 1
    for name, digest in contract["source_sha256"].items():
        assert hashlib.sha256((root / name).read_bytes()).hexdigest() == digest, name
    source = (root / "proofs/lean/child-population/Population.lean").read_text()

    def clean(text):
        assert not re.search(r"\b(sorry|admit|axiom|unsafe)\b", text), "proof hole or custom axiom"

    clean(source)
    commands = []

    def compile_source(label, text, success):
        directory = output / label
        directory.mkdir()
        path = directory / "Population.lean"
        path.write_text(text)
        command = [args.lean, "-DwarningAsError=true", "-o", str(directory / "Population.olean"), str(path)]
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
    audit.write_text("import Population\n" + "\n".join(
        f"#print axioms Kv9.ChildPopulation.{name}" for name in contract["theorems"]) + "\n")
    env = dict(os.environ, LEAN_PATH=str(audit.parent))
    result = subprocess.run([args.lean, "-DwarningAsError=true", str(audit)], cwd=audit.parent,
                            env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
    (output / "axioms.log").write_text(result.stdout)
    assert result.returncode == 0, result.stdout
    assert len(result.stdout.splitlines()) == len(contract["theorems"])
    for name in contract["theorems"]:
        line = next(line for line in result.stdout.splitlines()
                    if f"Kv9.ChildPopulation.{name}'" in line)
        if "does not depend on any axioms" not in line:
            names = line.split("[", 1)[1].split("]", 1)[0].split(", ")
            assert set(names) <= {"propext", "Classical.choice", "Quot.sound"}, line
    controls = [
        ('copy-without-the-fence',
         '| copyLow {s} (f : s.sealed = true) :',
         '| copyLow {s} (f : s.sealed = s.sealed) :'),
        ('copy-inexact',
         'Step s {s with lowCopied := true, lowExact := true}',
         'Step s {s with lowCopied := true}'),
        ('copy-commits-divergence',
         'Step s {s with highCopied := true, highExact := true}',
         'Step s {s with highCopied := true, highExact := true, divergentCommitted := true}'),
        ('copy-serves',
         'Step s {s with lowCopied := true, lowExact := true}',
         'Step s {s with lowCopied := true, lowExact := true, servedFromChild := true}'),
        ('recopy-diverges',
         '| recopy {s} (f : s.sealed = true) (done : s.lowCopied = true) : Step s s',
         '| recopy {s} (f : s.sealed = true) (done : s.lowCopied = true) : Step s {s with lowExact := false}'),
        ('fence-copies-without-verification',
         '| fence {s} : Step s {s with sealed := true}',
         '| fence {s} : Step s {s with sealed := true, lowCopied := true}'),
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
        "verified_rust_extraction": False, "copy_without_fence_or_divergence": False,
        "new_chaos_acceptance": False, "new_qps": False,
    }
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"CHILD_POPULATION_PROOF_ACCEPTED: {len(contract['theorems'])} theorems; {len(controls)} semantic and 2 policy controls")


if __name__ == "__main__":
    main()
