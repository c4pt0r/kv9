#!/usr/bin/env python3
"""Validate optimized lease-controller source controls and integer timing proofs."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / "crates/raft/src/lease.rs"
TESTS = ROOT / "crates/raft/src/lease/tests.rs"
MATH = ROOT / "proofs/smt/lease_timing"


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def replace(source, old, new):
    require(old != new and source.count(old) == 1, f"non-unique mutation: {old!r}")
    return source.replace(old, new)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--z3", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    args.z3, args.output = args.z3.resolve(), args.output.resolve()
    require(not args.output.exists(), "output must be new")
    inventory = json.loads((MATH / "inventory.json").read_text())
    require(sha(args.z3) == inventory["z3_sha256"], "Z3 digest mismatch")
    version = subprocess.check_output([str(args.z3), "--version"], text=True, timeout=15).strip()
    require(version == inventory["z3_version"], "Z3 version mismatch")
    for name, digest in inventory["sources"].items():
        require(sha(MATH / name) == digest, f"arithmetic source differs: {name}")
    require(inventory["proof_order"] == ["floor", "ceil", "timing"], "wrong dependency order")
    args.output.mkdir(parents=True)
    source = RUST.read_text()
    tests = TESTS.read_text()
    records = []

    def run(work, name, command, timeout):
        record = {"name": name, "command": command}
        save(work / (name + "-invocation.json"), record)
        with (work / (name + ".stdout")).open("x") as stdout, (work / (name + ".stderr")).open("x") as stderr:
            result = subprocess.run(command, cwd=work, stdout=stdout, stderr=stderr, timeout=timeout)
        record["exit_code"] = result.returncode
        save(work / (name + "-result.json"), record)
        return result.returncode, (work / (name + ".stdout")).read_text(), (work / (name + ".stderr")).read_text()

    def rust_case(name, text, test=None, clone=False):
        work = args.output / name
        (work / "src/lease").mkdir(parents=True)
        (work / "src/lib.rs").write_text("pub mod lease;\n")
        (work / "src/lease.rs").write_text(text)
        (work / "src/lease/tests.rs").write_text(tests)
        command = ["rustc", "--edition=2021", "--crate-name", "lease_controller_check", "--test",
                   "-C", "opt-level=3", "src/lib.rs", "-o", "controller-tests"]
        code, stdout, stderr = run(work, "compile", command, 180)
        record = {"name": name, "source_sha256": sha(work / "src/lease.rs"), "test": test}
        if clone:
            require(code != 0 and "error[E0080]" in stderr
                    and "ReadTicket must be neither Clone nor Copy" in stderr, "Clone guard failed elsewhere")
            record["verdict"] = "expected compile rejection"
        else:
            require(code == 0 and "error:" not in stderr, f"control did not compile: {name}")
            binary = work / "controller-tests"
            record["test_binary_sha256"] = sha(binary)
            command = [str(binary), "--test-threads=1"]
            if test:
                command += ["--exact", "lease::tests::" + test]
            code, output, stderr = run(work, "tests", command, 60)
            if test:
                require(code == 101 and f"test lease::tests::{test} ... FAILED" in output
                        and "test result: FAILED. 0 passed; 1 failed;" in output,
                        f"wrong fault-control failure: {name}")
                record["verdict"] = "expected test failure"
            else:
                require(code == 0 and "test result: ok. 25 passed; 0 failed; 0 ignored;" in output,
                        f"incomplete optimized controller suite: {name}")
                record["verdict"] = "passed"
        records.append(record)
        save(work / "result.json", record)
        print(f"PASS: {name}: {record['verdict']}", flush=True)

    rust_case("rust-baseline", source)
    controls = [
        ("lost-quarantine", ".checked_add(timing.recovery_ns)", ".checked_add(0)",
         "recovery_and_repeated_restart_cannot_shorten_prior_obligations"),
        ("vote-before-hold-expiry", "if time < self.hold_until {\n            return Err(Refused::VotingPromise);",
         "if time < self.hold_until.saturating_sub(self.timing.promise_ns) {\n            return Err(Refused::VotingPromise);",
         "fastest_voters_cannot_form_an_election_quorum_during_the_slowest_leader_lease"),
        ("publish-without-quorum", "pending.acknowledgements.len() < self.configuration.quorum()",
         "pending.acknowledgements.len() > self.configuration.voters.len()",
         "duplicate_acks_do_not_form_a_quorum_and_receipt_does_not_publish"),
        ("mix-round-acks", "pending.certificate.renewal != grant.renewal",
         "pending.certificate.renewal.authority != grant.renewal.authority",
         "late_old_round_acks_cannot_complete_the_new_round"),
        ("stale-frontier", "committed: progress.committed,", "committed: 0,",
         "fresh_frontier_and_exact_view_are_required_after_completed_writes"),
        ("early-view", "view_index < ticket.committed", "view_index < ticket.committed.saturating_sub(1)",
         "fresh_frontier_and_exact_view_are_required_after_completed_writes"),
        ("missing-generation", "            || ticket.certificate.renewal.generation != self.generation\n", "",
         "revoke_rearm_and_new_renewal_cannot_resurrect_old_tickets"),
        ("extend-ticket-expiry", "time >= ticket.certificate.end",
         "time >= ticket.certificate.end.saturating_add(self.timing.leader_ns)",
         "an_old_read_retains_its_own_deadline_across_renewal"),
    ]
    for name, old, new, test in controls:
        rust_case(name, replace(source, old, new), test)
    rust_case("clone-ticket", replace(source, "#[derive(Debug)]\npub struct ReadTicket",
                                      "#[derive(Debug, Clone)]\npub struct ReadTicket"), clone=True)
    rust_case("rust-restored", source)
    require(records[0]["source_sha256"] == records[-1]["source_sha256"], "source restoration differs")

    math_records = []

    def math_case(name, source, expected):
        work = args.output / name
        work.mkdir()
        path = work / "proof.smt2"
        path.write_text(source + ("(get-model)\n" if expected == "sat" else ""))
        code, output, stderr = run(work, "solver", [str(args.z3), "-T:5", "-smt2", str(path)], 15)
        require(code == 0 and not stderr and output.splitlines()[0] == expected
                and not re.search(r"error|unknown|timeout", output, re.I), f"incomplete arithmetic proof: {name}")
        if expected == "sat":
            require("define-fun" in output, "missing arithmetic countermodel")
        record = {"name": name, "expected": expected, "source_sha256": sha(path), "verdict": expected}
        math_records.append(record)
        save(work / "result.json", record)
        print(f"PASS: {name}: {expected}", flush=True)

    sources = {name: (MATH / (name + ".smt2")).read_text() for name in inventory["proof_order"]}
    for name in inventory["proof_order"]:
        math_case(name, sources[name], "unsat")
    math_case("floor-rounded-up", replace(sources["floor"], "(div n d)", "(div (+ n (- d 1)) d)"), "sat")
    math_case("ceil-rounded-down", replace(sources["ceil"], "(div (+ n (- d 1)) d)", "(div n d)"), "sat")
    math_case("leader-without-drift-margin", replace(sources["timing"],
        "(define-fun leader () Int (- (div (* E a) b) margin))", "(define-fun leader () Int E)"), "sat")
    math_case("recovery-rounded-down", replace(sources["timing"],
        "(define-fun recovery () Int (+ (div (+ (* E b) (- a 1)) a) margin))",
        "(define-fun recovery () Int (+ (div (* E b) a) margin))"), "sat")
    for name in inventory["proof_order"]:
        math_case(name + "-restored", sources[name], "unsat")
    require([r["source_sha256"] for r in math_records[:3]]
            == [r["source_sha256"] for r in math_records[-3:]], "arithmetic restoration differs")
    save(args.output / "summary.json", {
        "scope": "Isolated optimized Rust component and integer arithmetic; no runtime lease or E2E qualification",
        "rust_tests": 25, "rust_fault_controls": len(controls), "clone_control": 1,
        "math_proofs": 3, "math_countermodels": 4, "records": records, "math_records": math_records,
        "z3_sha256": sha(args.z3), "z3_version": version,
        "rustc": subprocess.check_output(["rustc", "-vV"], text=True, timeout=15),
        "cpu_affinity": sorted(os.sched_getaffinity(0)),
        "sources": {str(p.relative_to(ROOT)): sha(p) for p in [Path(__file__), RUST, TESTS,
                    *sorted(MATH.iterdir())] if p.is_file()},
    })
    print("PASS: optimized source controls and composed integer timing proofs", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        raise SystemExit(f"FAIL: {error}") from error
