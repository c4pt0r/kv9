#!/usr/bin/env python3
"""Check conditional lease lemmas and real-clock containment locally; no Cargo."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "proofs/tlaps/leader_lease"
MODULE = "LeaderLeaseProof"
spec = importlib.util.spec_from_file_location("existing_tlaps_gate", ROOT / "scripts/check-tlaps.py")
proof = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proof)
require = proof.require


def save(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tlapm", required=True, type=Path)
    parser.add_argument("--jar", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    args.tlapm, args.jar, args.output = (p.resolve() for p in (args.tlapm, args.jar, args.output))
    args.stdlib = args.tlapm.parent.parent / "lib/tlapm/stdlib"
    args.timeout = 180
    inventory = json.loads((SOURCE / "inventory.json").read_text())
    require(proof.sha(args.tlapm) == inventory["tlapm_sha256"], "TLAPM digest mismatch")
    require(proof.sha(args.jar) == inventory["sany_jar_sha256"], "SANY digest mismatch")
    version = subprocess.check_output([str(args.tlapm), "--version"], text=True, timeout=15).strip()
    require(version == inventory["tlapm_version"], "TLAPM version mismatch")
    require(not args.output.exists(), "output must be new")
    args.output.mkdir(parents=True)
    args.classes = args.output / "auditor"
    args.classes.mkdir()
    shutil.copyfile(ROOT / "scripts/ProofAudit.java", args.classes / "ProofAudit.java")
    subprocess.run(["javac", "-cp", str(args.jar), "-d", str(args.classes),
                    str(args.classes / "ProofAudit.java")], check=True, timeout=30)
    source = (SOURCE / f"{MODULE}.tla").read_text()
    records = []

    def case(name, text, expected=None):
        work = args.output / name
        work.mkdir()
        (work / f"{MODULE}.tla").write_text(text)
        save(work / "inventory.json", inventory)
        record = {"name": name, "source_sha256": proof.sha(work / f"{MODULE}.tla"),
                  "expected": expected}
        try:
            proof.check_tree(args, work, record)
        except proof.Rejected as error:
            record.update(reason=str(error), verdict="rejected")
            require(expected is not None, str(error))
            require(str(error) == expected["reason"], f"wrong rejection: {error}")
            if "formula" in expected:
                output = (work / f"{MODULE}.log").read_text()
                require(expected["formula"] in output, "missing intended failed formula")
                require(re.search(r"\[ERROR\]: [1-9][0-9]*/23 obligations failed\.", output),
                        "missing actual failed obligations")
            record["verdict"] = "expected rejection"
        else:
            require(expected is None, "invalid proof accepted")
            record["verdict"] = "proved"
        finally:
            save(work / "result.json", record)
            records.append(record)
        print(f"PASS: {name}: {record['verdict']}", flush=True)

    case("baseline", source)
    # False variants must fail in the intended theorem. A failed proof alone
    # is not claimed to be a generated execution counterexample.
    controls = [
        ("missing-send-order", "s <= g, g <= t,", "TRUE, g <= t,", "t - g < H"),
        ("missing-prior-vote-fence", "v < g => FALSE,", "v < g => TRUE,", "v =< t => FALSE"),
        ("missing-promise", "g <= v /\\ v < expiry => FALSE", "g <= v /\\ v < expiry => TRUE", "v =< t => FALSE"),
        ("disjoint-quorums", "Q \\cap V # {},", "TRUE,", "PROVE  ~(\\A q \\in V"),
        ("missing-apply-fence", "c <= j,", "TRUE,", "index[w] =< j"),
        ("missing-expiry-check", "covered /\\ now < end", "covered /\\ TRUE", "PROVE  ~LLFinalGuard"),
    ]
    for name, old, new, formula in controls:
        case(name, proof.replace_once(source, old, new),
             {"reason": f"{MODULE}: TLAPS exit 10", "formula": formula})
    case("omitted-proof", proof.replace_once(source, "BY SMT DEF LLFinalGuard", "PROOF OMITTED"),
         {"reason": f"proof hole: {MODULE}.LLExpiredFinalCheckRejects"})
    case("false-module-assumption", proof.replace_once(source, "EXTENDS Integers, TLAPS",
         "EXTENDS Integers, TLAPS\nASSUME FALSE"),
         {"reason": f"unapproved module assumption: {MODULE}"})
    case("restored", source)
    require(records[0]["source_sha256"] == records[-1]["source_sha256"], "restoration differs")
    output = (args.output / "baseline" / f"{MODULE}.log").read_text()
    output_controls = proof.output_controls(output, MODULE, 23)

    z3 = args.tlapm.parent.parent / "lib/tlapm/backends/bin/z3"
    real_source = (SOURCE / "clock-containment.smt2").read_text()
    real_cases = [("baseline", real_source, "unsat")]
    real_controls = [
        ("late-anchor", "(assert (<= s g))", "(assert true)"),
        ("paused-clock", "(assert (>= leader_elapsed (* a (- t s))))", "(assert true)"),
        ("missing-drift-margin", "(assert (<= (* D b) (* E a)))", "(assert (<= D E))"),
    ]
    for name, old, new in real_controls:
        real_cases.append((name, proof.replace_once(real_source, old, new) + "(get-model)\n", "sat"))
    real_cases.append(("restored", real_source, "unsat"))
    real_records = []
    for name, text, verdict in real_cases:
        work = args.output / ("real-" + name)
        work.mkdir()
        model = work / "clock.smt2"
        model.write_text(text)
        command = [str(z3), "-T:5", "-smt2", str(model)]
        result = subprocess.run(command, text=True, capture_output=True, timeout=15)
        (work / "stdout.log").write_text(result.stdout)
        (work / "stderr.log").write_text(result.stderr)
        record = {"name": name, "command": command, "exit_code": result.returncode,
                  "source_sha256": proof.sha(model), "expected": verdict}
        save(work / "result.json", record)
        require(result.returncode == 0 and not result.stderr, "real arithmetic backend failed")
        require(result.stdout.splitlines()[0] == verdict, "wrong real arithmetic verdict")
        require("error" not in result.stdout.lower() and "unknown" not in result.stdout.lower(),
                "incomplete real arithmetic result")
        if verdict == "sat":
            require("define-fun" in result.stdout, "missing real countermodel")
        real_records.append(record)
        print(f"PASS: real-{name}: {verdict}", flush=True)
    require(real_records[0]["source_sha256"] == real_records[-1]["source_sha256"], "real restore differs")
    save(args.output / "summary.json", {
        "scope": "Conditional lease lemmas; no Rust refinement or E2E acceptance",
        "theorems": 8, "obligations": 23, "proof_controls": 8,
        "output_controls": output_controls, "real_countermodels": 3,
        "proof_runs": records, "real_runs": real_records,
        "cpu_affinity": sorted(os.sched_getaffinity(0)),
        "tlapm_version": version, "tlapm_sha256": proof.sha(args.tlapm),
        "sany_sha256": proof.sha(args.jar), "z3_sha256": proof.sha(z3),
        "z3_version": subprocess.check_output([str(z3), "--version"], text=True, timeout=15).strip(),
        "sources": {str(p.relative_to(ROOT)): proof.sha(p) for p in [
            Path(__file__), ROOT / "scripts/check-tlaps.py", ROOT / "scripts/ProofAudit.java",
            *sorted(SOURCE.iterdir())] if p.is_file()},
    })
    print("PASS: 8 TLAPS theorems / 23 obligations; real-clock theorem and 3 countermodels", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (proof.Rejected, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        raise SystemExit(f"FAIL: {error}") from error
