#!/usr/bin/env python3
"""Check Scoped routing and bounded retry composition and explicit defect controls."""
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
    contract = json.loads((root / "proofs/lean/routed-client/source-contract.json").read_text())
    assert contract["schema"] == 1
    for name, digest in contract["source_sha256"].items():
        assert hashlib.sha256((root / name).read_bytes()).hexdigest() == digest, name
    source = (root / "proofs/lean/routed-client/Routing.lean").read_text()

    def clean(text):
        assert not re.search(r"\b(sorry|admit|axiom|unsafe)\b", text), "proof hole or custom axiom"

    clean(source)
    commands = []

    def compile_source(label, text, success):
        directory = output / label
        directory.mkdir()
        path = directory / "Routing.lean"
        path.write_text(text)
        command = [args.lean, "-DwarningAsError=true", "-o", str(directory / "Routing.olean"), str(path)]
        result = subprocess.run(command, cwd=directory, text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, timeout=60)
        (directory / "compiler.log").write_text(result.stdout)
        assert (result.returncode == 0) == success, (label, result.stdout)
        if not success:
            assert any(marker in result.stdout for marker in
                       ("error: unsolved goals", "error: Type mismatch", "error: Tactic", "error: omega")), result.stdout
            assert not any(marker in result.stdout for marker in
                           ("unexpected token", "Unknown identifier", "unknown constant", "unknown namespace")), result.stdout
        commands.append({"label": label, "command": command, "exit_code": result.returncode})

    compile_source("positive", source, True)
    audit = output / "positive/Audit.lean"
    audit.write_text("import Routing\n" + "\n".join(
        f"#print axioms Kv9.RoutedClient.{name}" for name in contract["theorems"]) + "\n")
    env = dict(os.environ, LEAN_PATH=str(audit.parent))
    result = subprocess.run([args.lean, "-DwarningAsError=true", str(audit)], cwd=audit.parent,
                            env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
    (output / "axioms.log").write_text(result.stdout)
    assert result.returncode == 0, result.stdout
    assert len(result.stdout.splitlines()) == len(contract["theorems"])
    for name in contract["theorems"]:
        line = next(line for line in result.stdout.splitlines() if f"Kv9.RoutedClient.{name}'" in line)
        if "does not depend on any axioms" not in line:
            names = line.split("[", 1)[1].split("]", 1)[0].split(", ")
            assert set(names) <= {"propext", "Classical.choice", "Quot.sound"}, line
    controls = [
        ('lookup-free-budget',
         'attempts := s.attempts + 1, phase := if ready',
         'attempts := s.attempts, phase := if ready'),
        ('dispatch-free-budget',
         'attempts := s.attempts + 1, sent := s.sent + 1',
         'attempts := s.attempts, sent := s.sent + 1'),
        ('dispatch-without-budget',
         '| dispatch (s) (phase : s.phase = .ready)\n      (room : s.attempts < budget)',
         '| dispatch (s) (phase : s.phase = .ready)\n      (room : True)'),
        ('lookup-after-deadline',
         '| lookup (s) (ready : Bool) (phase : s.phase = .lookup)\n      (room : s.attempts < budget) (time : s.now < deadline)',
         '| lookup (s) (ready : Bool) (phase : s.phase = .lookup)\n      (room : s.attempts < budget) (time : True)'),
        ('replay-unknown',
         'Step budget deadline s {s with phase := .unknown}',
         'Step budget deadline s {s with phase := .ready}'),
        ('refuse-after-effect', '(noEffects : s.effects = [])', '(noEffects : True)'),
        ('forget-refused-dispatch', 'closed := s.sent, phase := if refresh',
         'closed := s.closed, phase := if refresh'),
        ('effect-from-closed-dispatch', '(live : s.closed < i)', '(live : s.closed ≤ i)'),
        ('partial-scope-only', 'decide (owner = request)', 'decide (owner.region = request.region)'),
        ('crossing-batch', '&& keys.all (contains owner)', '&& true'),
        ('foreign-root', 'decide (scope.root = root) &&', 'true &&'),
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
        "verified_rust_extraction": False, "complete_public_routing_gate": False,
        "new_chaos_acceptance": False, "new_qps": False,
    }
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"ROUTED_CLIENT_PROOF_ACCEPTED: {len(contract['theorems'])} theorems; {len(controls)} semantic and 2 policy controls")


if __name__ == "__main__":
    main()
