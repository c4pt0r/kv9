#!/usr/bin/env python3
"""Check the restore model and explicit defect controls."""
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
        (root / "proofs/lean/restore/source-contract.json").read_text()
    )
    assert contract["schema"] == 1
    for name, digest in contract["source_sha256"].items():
        assert hashlib.sha256((root / name).read_bytes()).hexdigest() == digest, name
    source = (root / "proofs/lean/restore/Restore.lean").read_text()

    def clean(text):
        assert not re.search(r"\b(sorry|admit|axiom|unsafe)\b", text), "proof hole or custom axiom"

    clean(source)
    commands = []

    def compile_source(label, text, success):
        directory = output / label
        directory.mkdir()
        path = directory / "Restore.lean"
        path.write_text(text)
        command = [args.lean, "-DwarningAsError=true", "-o", str(directory / "Restore.olean"), str(path)]
        result = subprocess.run(command, cwd=directory, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, timeout=300)
        (directory / "compiler.log").write_text(result.stdout)
        assert (result.returncode == 0) == success, (label, result.stdout)
        if not success:
            assert any(marker in result.stdout for marker in
                       ("error: unsolved goals", "error: Type mismatch", "error: type mismatch",
                        "type mismatch", "error: Tactic", "error: omega", "error: simp",
                        "error: `grind` failed", "made no progress", "error: The rfl tactic",
                        "error: unknown", "error: Case")), result.stdout
            assert not any(marker in result.stdout for marker in
                           ("unexpected token", "Unknown identifier", "unknown constant", "unknown namespace")), result.stdout
        commands.append({"label": label, "command": command, "exit_code": result.returncode})

    compile_source("positive", source, True)
    audit = output / "positive/Audit.lean"
    audit.write_text("import Restore\n" + "\n".join(
        f"#print axioms Kv9.Restore.{name}" for name in contract["theorems"]) + "\n")
    env = dict(os.environ, LEAN_PATH=str(audit.parent))
    result = subprocess.run([args.lean, "-DwarningAsError=true", str(audit)], cwd=audit.parent,
                            env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300)
    (output / "axioms.log").write_text(result.stdout)
    assert result.returncode == 0, result.stdout
    # Lean wraps a long axiom list onto continuation lines that start with a space.
    lines = []
    for raw in result.stdout.splitlines():
        if raw.startswith(" ") and lines:
            lines[-1] += raw
        else:
            lines.append(raw)
    assert len(lines) == len(contract["theorems"]), lines
    for name in contract["theorems"]:
        line = next(line for line in lines if f"Kv9.Restore.{name}'" in line)
        if "does not depend on any axioms" not in line:
            names = line.split("[", 1)[1].split("]", 1)[0].split(", ")
            assert set(names) <= {"propext", "Classical.choice", "Quot.sound"}, line
    # Semantic mutation controls: each removes one guard of the step table and must
    # break the theorem that depends on it.
    controls = [
        ('evidence-after-abort-or-publication',
         "  | evidence {s} (isLive : live s = true) (left : s.evidenced < s.targets) (fits : Bool)",
         "  | evidence {s} (isLive : live s = live s) (left : s.evidenced < s.targets) (fits : Bool)"),
        ('evidence-beyond-the-targets',
         "  | evidence {s} (isLive : live s = true) (left : s.evidenced < s.targets) (fits : Bool)",
         "  | evidence {s} (isLive : live s = true) (left : s.evidenced ≤ s.targets) (fits : Bool)"),
        ('evidence-with-a-mismatching-digest',
         "      (ok : fits = true) :",
         "      (ok : fits = fits) :"),
        ('publish-before-every-evidence',
         "  | publish {s} (isLive : live s = true) (all : s.evidenced = s.targets) :",
         "  | publish {s} (isLive : live s = true) (all : s.evidenced ≤ s.targets) :"),
        ('publish-after-abort',
         "  | publish {s} (isLive : live s = true) (all : s.evidenced = s.targets) :",
         "  | publish {s} (isLive : live s = live s) (all : s.evidenced = s.targets) :"),
        ('abort-after-publication',
         "  | abort {s} (open_ : s.published = false) :",
         "  | abort {s} (open_ : s.published = s.published) :"),
        ('retire-the-backup-under-a-live-restore',
         "  | retireBackup {s} (free : live s = false) :",
         "  | retireBackup {s} (free : live s = live s) :"),
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
        "verified_rust_extraction": False, "placement_quality": False,
        "new_chaos_acceptance": False, "new_qps": False,
    }
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"RESTORE_PROOF_ACCEPTED: {len(contract['theorems'])} theorems; {len(controls)} semantic and 2 policy controls")


if __name__ == "__main__":
    main()
