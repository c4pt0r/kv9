#!/usr/bin/env python3
"""Compile the pinned proof inventory freshly and audit transitive axioms."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FOUNDATIONS = {"propext", "Classical.choice", "Quot.sound"}


class Rejected(Exception):
    pass


def check(lean, source, work):
    inventory = json.loads((source / "theorems.json").read_text())
    if not inventory or any(not names for names in inventory.values()):
        raise Rejected("empty theorem inventory")
    if set(inventory) != {p.stem for p in source.glob("*.lean")}:
        raise Rejected("source/inventory mismatch")
    env = dict(os.environ, LEAN_PATH=str(work))
    count = 0
    for module, names in inventory.items():
        if not re.fullmatch(r"[A-Za-z][A-Za-z0-9_]*", module):
            raise Rejected("invalid module name")
        if len(names) != len(set(names)):
            raise Rejected("duplicate theorem inventory")
        original = (source / f"{module}.lean").read_text()
        queries = "\n".join(f"#print axioms {name}" for name in names)
        target = work / f"{module}.lean"
        target.write_text(original + "\n" + queries + "\n")
        result = subprocess.run(
            [lean, "-DwarningAsError=true", "-o", f"{module}.olean", target.name],
            cwd=work, env=env, text=True, capture_output=True, timeout=120,
        )
        if result.returncode:
            raise Rejected("Lean rejected proof:\n" + result.stdout + result.stderr)
        for name in names:
            pattern = re.escape(f"'{name}'") + (
                r" (?:depends on axioms: \[([^]]*)\]|does not depend on any axioms)"
            )
            matches = list(re.finditer(pattern, result.stdout))
            if len(matches) != 1:
                raise Rejected(f"missing or duplicate axiom report: {name}")
            axioms = {
                a.strip() for a in (matches[0].group(1) or "").split(",") if a.strip()
            }
            if axioms - FOUNDATIONS:
                raise Rejected(f"untrusted axioms for {name}: {sorted(axioms - FOUNDATIONS)}")
            count += 1
    return count


def controls(lean, source, root):
    original = (source / "Quorum.lean").read_text()
    proof = """    (2 * faults + 1) / 2 + 1 ≤ (2 * faults + 1) - unavailable := by
  omega"""
    assert original.count(proof) == 1
    cases = [
        ("proof hole", original.replace(proof, proof.rsplit("omega", 1)[0] + "sorry"),
         "declaration uses `sorry`"),
        ("insufficient membership", original.replace("(h : 3 ≤ voters)", "(h : 2 ≤ voters)"),
         "omega could not prove the goal"),
        ("custom axiom", original.replace("import Std", "import Std\naxiom fake : False")
         .replace(proof, proof.rsplit("omega", 1)[0] + "have _ := h\n  exact False.elim fake"),
         "untrusted axioms"),
    ]
    for i, (name, mutant, expected) in enumerate(cases):
        if mutant == original:
            raise Rejected(f"control did not mutate its target: {name}")
        tree = root / f"control-{i}"
        shutil.copytree(source, tree)
        (tree / "Quorum.lean").write_text(mutant)
        work = root / f"control-build-{i}"
        work.mkdir()
        try:
            check(lean, tree, work)
        except Rejected as error:
            if expected not in str(error):
                raise Rejected(f"control failed outside its intended assertion: {name}: {error}")
        else:
            raise Rejected(f"control was accepted: {name}")
        print(f"Rejected control: {name}")
    return len(cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lean", default=os.environ.get("LEAN", "lean"))
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    lean = shutil.which(args.lean)
    if lean is None:
        raise Rejected(f"Lean executable is unavailable: {args.lean}")
    source = ROOT / "proofs/lean"
    version = (source / "lean-toolchain").read_text().strip().split(":v")[1]
    resolved = subprocess.check_output([lean, "--version"], text=True, timeout=15).strip()
    if not resolved.startswith(f"Lean (version {version},"):
        raise Rejected(f"toolchain mismatch: expected {version}; got {resolved}")
    print(resolved)
    with tempfile.TemporaryDirectory(prefix="kv9-proof-check.") as temp:
        root = Path(temp)
        work = root / "build"
        work.mkdir()
        count = check(lean, source, work)
        rejected = controls(lean, source, root) if args.self_test else 0
    print(f"PASS: {count} theorems checked; {rejected} invalid controls rejected")


if __name__ == "__main__":
    try:
        main()
    except (Rejected, subprocess.SubprocessError) as error:
        raise SystemExit(f"FAIL: {error}") from error
