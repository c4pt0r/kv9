#!/usr/bin/env python3
"""Audit and freshly check the parameterized TLAPS proof inventory."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "proofs/tlaps"
MODEL = ROOT / "proofs/tla/MetadataPlanning.tla"


class Rejected(Exception):
    pass


def require(condition, message):
    if not condition:
        raise Rejected(message)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def replace_once(text, old, new):
    require(old != new and text.count(old) == 1, f"non-unique mutation target: {old!r}")
    return text.replace(old, new)


def proof_verdict(output, status, module, expected_count):
    require(status == 0, f"{module}: TLAPS exit {status}")
    require("[ERROR]" not in output and "[WARNING]" not in output,
            f"{module}: proof diagnostics are not clean")
    counts = re.findall(r"\[INFO\]: All (\d+) obligations? proved\.", output)
    require(counts == [str(expected_count)] and expected_count > 0,
            f"{module}: missing or unexpected obligation count")
    return expected_count


def audit(args, work, inventory):
    roots = inventory["roots"]
    require(roots and len(roots) == len(set(roots)), "empty or duplicate root inventory")
    require(set(roots) <= set(inventory["modules"]), "unlisted root module")
    merged = {}
    for root in roots:
        command = ["java", "-cp", f"{args.jar}:{args.classes}", "ProofAudit",
                   str(work / f"{root}.tla"), str(work), str(work), str(args.stdlib)]
        result = subprocess.run(command, cwd=work, text=True, capture_output=True, timeout=30)
        with (work / "sany.log").open("a") as log:
            log.write(result.stdout + result.stderr)
        require(result.returncode == 0, "SANY rejected the proof inventory")
        lines = [line.removeprefix("AUDIT_JSON=") for line in result.stdout.splitlines()
                 if line.startswith("AUDIT_JSON=")]
        require(len(lines) == 1, "missing semantic audit result")
        records = json.loads(lines[0])
        require(len(records) == len({r["module"] for r in records}), "duplicate semantic module")
        for record in records:
            name = record["module"]
            require(name not in merged or merged[name] == record, "inconsistent shared module audit")
            merged[name] = record
    records = [merged[name] for name in sorted(merged)]
    owned = set(inventory["modules"]) | set(inventory["models"])
    found = set()
    for record in records:
        module = record["module"]
        require(module not in found, "duplicate semantic module")
        found.add(module)
        if module in inventory["standard_modules"]:
            require(record["sha256"] == inventory["standard_modules"][module],
                    f"untrusted standard module: {module}")
            continue
        require(module in owned, f"unlisted imported module: {module}")
        require(Path(record["file"]).resolve() == work / f"{module}.tla",
                f"module resolved outside the copied source: {module}")
        require(record["inner_modules"] == record["instances"] == 0,
                f"unlisted nested module or instance: {module}")
        expected_assumptions = inventory["models"][module]["assumptions"] if module in inventory["models"] else []
        require(record["assumptions"] == expected_assumptions,
                f"unapproved module assumption: {module}")
        expected = [] if module in inventory["models"] else inventory["modules"][module]["theorems"]
        require(sorted(t["name"] for t in record["theorems"]) == sorted(expected),
                f"theorem inventory mismatch: {module}")
        for theorem in record["theorems"]:
            require(not theorem["hole"], f"proof hole: {module}.{theorem['name']}")
    require(found == owned | set(inventory["standard_modules"]), "incomplete module dependency audit")
    (work / "semantic-audit.json").write_text(json.dumps(records, indent=2) + "\n")


def check_tree(args, work, record):
    inventory = json.loads((work / "inventory.json").read_text())
    require(inventory["modules"] and all(item["theorems"] for item in inventory["modules"].values()),
            "empty theorem inventory")
    audit(args, work, inventory)
    record["modules"] = []
    (work / "cache").mkdir()
    for module, item in inventory["modules"].items():
        command = [str(args.tlapm), "--strict", "--nofp", "--threads", "1",
                   "--cache-dir", str(work / "cache" / module), str(work / f"{module}.tla")]
        module_record = {"module": module, "command": command}
        record["modules"].append(module_record)
        with (work / f"{module}.log").open("w") as log:
            started = time.monotonic()
            try:
                result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                                        timeout=args.timeout)
            except subprocess.TimeoutExpired:
                module_record.update(verdict="inconclusive", reason="timeout")
                raise Rejected(f"{module}: proof timeout; no obligation discharged") from None
        output = (work / f"{module}.log").read_text()
        module_record.update(exit_code=result.returncode, seconds=time.monotonic() - started)
        count = proof_verdict(output, result.returncode, module, item["obligations"])
        module_record.update(verdict="proved", obligations=count)


def run_case(args, name, mutation=None, expected=None):
    work = args.output / name
    work.mkdir()
    for path in SOURCE.glob("*.tla"):
        shutil.copyfile(path, work / path.name)
    inventory = json.loads((SOURCE / "inventory.json").read_text())
    for module, item in inventory["models"].items():
        path = ROOT / item["source"]
        require(path.resolve().is_relative_to(ROOT / "proofs/tla") and path.name == f"{module}.tla",
                "unlisted model source path")
        shutil.copyfile(path, work / path.name)
    shutil.copyfile(SOURCE / "inventory.json", work / "inventory.json")
    if mutation is not None:
        filename, transform = mutation
        path = work / filename
        before = path.read_text()
        after = transform(before)
        require(after != before, f"empty source mutation: {name}")
        path.write_text(after)
    record = {"name": name, "expected": expected,
              "sources": {p.name: sha(p) for p in sorted(work.iterdir()) if p.is_file()}}
    try:
        check_tree(args, work, record)
    except Rejected as error:
        record.update(verdict="rejected", reason=str(error))
        if expected is None:
            raise Rejected(f"{name}: {error}; see {work}") from error
        require(str(error) == expected["reason"], f"{name}: failed outside intended gate: {error}")
        if "log" in expected:
            output = (work / expected["log"]).read_text()
            require(re.search(expected["pattern"], output) is not None,
                    f"{name}: missing intended failed obligation")
            if "failed" in expected:
                failures = re.findall(r"\[ERROR\]: (\d+)/(\d+) obligations failed\.", output)
                require(len(failures) == 1 and int(failures[0][0]) == expected["failed"],
                        f"{name}: unexpected failed obligation count")
        record["verdict"] = "expected rejection"
    else:
        require(expected is None, f"{name}: invalid control was accepted")
        record["verdict"] = "proved"
    finally:
        (work / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    print(f"PASS: {name}: {record['verdict']}", flush=True)
    return record


def output_controls(output, module, count):
    cases = [("empty output", ""),
             ("zero obligations", re.sub(r"All \d+ obligations proved", "All 0 obligation proved", output)),
             ("missing summary", re.sub(r"\[INFO\]: All \d+ obligations proved\.", "", output))]
    for name, mutant in cases:
        proof_verdict(output, 0, module, count)
        try:
            proof_verdict(mutant, 0, module, count)
        except Rejected as error:
            require(str(error) == f"{module}: missing or unexpected obligation count",
                    f"output control failed elsewhere: {name}")
        else:
            raise Rejected(f"output control was accepted: {name}")
        proof_verdict(output, 0, module, count)
        print(f"PASS: rejected output control: {name}", flush=True)
    return len(cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tlapm", required=True, type=Path)
    parser.add_argument("--jar", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    args.tlapm, args.jar, args.output = args.tlapm.resolve(), args.jar.resolve(), args.output.resolve()
    args.stdlib = args.tlapm.parent.parent / "lib/tlapm/stdlib"
    inventory = json.loads((SOURCE / "inventory.json").read_text())
    require(args.timeout > 0, "timeout must be positive")
    require(sha(args.jar) == inventory["sany_jar_sha256"], "SANY jar checksum mismatch")
    version = subprocess.check_output([str(args.tlapm), "--version"], text=True, timeout=15).strip()
    require(version == inventory["tlapm_version"], "TLAPS version mismatch")
    require(set(inventory["modules"]) == {p.stem for p in SOURCE.glob("*.tla")},
            "module inventory mismatch")
    require(not args.output.exists(), "output directory must be new")
    args.output.mkdir(parents=True)
    args.classes = args.output / "auditor"
    args.classes.mkdir()
    shutil.copyfile(ROOT / "scripts/ProofAudit.java", args.classes / "ProofAudit.java")
    subprocess.run(["javac", "-cp", str(args.jar), "-d", str(args.classes),
                    str(args.classes / "ProofAudit.java")], check=True, timeout=30)
    records = [run_case(args, "baseline")]
    finish_exact = 'Finish(r) ==\n    /\\ phase[r] = "write"\n    /\\ Exact(writeAt[r], "write", r, writeTerm[r])'
    finish_apply = '/\\ applied[host[r]] >= writeAt[r]'
    receipt_failure = {"reason": "MetadataReceipt: TLAPS exit 10", "log": "MetadataReceipt.log",
                       "pattern": r"PROVE\s+\\A r \\in Requests : Finish\(r\) => ReceiptSafety'", "failed": 1}
    controls = [
        ("index-only-receipt", (MODEL.name, lambda text: replace_once(text, finish_exact,
         'Finish(r) ==\n    /\\ phase[r] = "write"\n    /\\ TRUE')), receipt_failure),
        ("early-receipt", (MODEL.name, lambda text: replace_once(text, finish_apply, '/\\ TRUE')),
         receipt_failure),
        ("discard-committed-entry", (MODEL.name, lambda text: replace_once(
            text, 'SubSeq(log, 1, keep)', 'SubSeq(log, 1, keep - 1)')),
         {"reason": "MetadataPrefix: TLAPS exit 10", "log": "MetadataPrefix.log",
          "pattern": r"PROVE\s+\\A n \\in Nodes, keep \\in committed\.\.Len\(log\) :\s+Elect\(n, keep\) => PrefixStable", "failed": 1}),
        ("omitted-proof", ("MetadataReceipt.tla", lambda text: replace_once(
            text, '<1> QED BY <1>1, <1>2, PTL\n', '<1> QED OMITTED\n')),
         {"reason": "proof hole: MetadataReceipt.ReceiptAlways"}),
        ("custom-axiom", ("Collections.tla", lambda text: replace_once(
            text, 'EXTENDS Naturals, Sequences, TLAPS',
            'EXTENDS Naturals, Sequences, TLAPS\n  AXIOM Fake == FALSE')),
         {"reason": "unapproved module assumption: Collections"}),
        ("empty-inventory", ("inventory.json", lambda text: json.dumps(
            dict(json.loads(text), modules={}), indent=2) + '\n'),
         {"reason": "empty theorem inventory"}),
        ("invalid-action-property", ("MetadataPrefix.tla", lambda text: replace_once(
            text, 'Spec => [][PrefixStable]_vars', 'Spec => []PrefixStable')),
         {"reason": "SANY rejected the proof inventory", "log": "sany.log",
          "pattern": r"\[\] followed by action not of form \[A\]_v"}),
        ("no-planner-mutex", (MODEL.name, lambda text: replace_once(
            text, '/\\ \\A other \\in Requests :\n           host[other] = leader => phase[other] \\notin Active',
            '/\\ TRUE')),
         {"reason": "MetadataPlanningControl: TLAPS exit 10", "log": "MetadataPlanningControl.log",
          "pattern": r"PROVE\s+\\A r \\in Requests : Begin\(r\) => PlanningControl'", "failed": 1}),
        ("read-index-only", (MODEL.name, lambda text: replace_once(
            text, '/\\ applied[host[r]] >= barrierAt[r]',
            '/\\ applied[host[r]] >= committed\n    /\\ committed > 0\n    /\\ log[committed].epoch = term')),
         {"reason": "MetadataPlanningBarrier: TLAPS exit 10", "log": "MetadataPlanningBarrier.log",
          "pattern": r"PROVE\s+\\A r \\in Requests : Barrier\(r\) => Barriers'", "failed": 1}),
        ("no-term-fence", (MODEL.name, lambda text: replace_once(
            text, '/\\ host[r] = leader\n    /\\ planningTerm[r] = term',
            '/\\ host[r] = leader\n    /\\ TRUE')),
         {"reason": "MetadataUniqueness: TLAPS exit 10", "log": "MetadataUniqueness.log",
          "pattern": r"PROVE\s+\\A r \\in Requests : Submit\(r\) => UniqueCatalog'", "failed": 1}),
        ("allocator-no-advance", (MODEL.name, lambda text: replace_once(
            text, 'ELSE log[LastWrite(cut)].id + 1', 'ELSE log[LastWrite(cut)].id')),
         {"reason": "MetadataUniqueness: TLAPS exit 10", "log": "MetadataUniqueness.log",
          "pattern": r"PROVE\s+/\\ NextId\(Len\(log\)\) \\in Nat \\ \{0\}", "failed": 1}),
        ("incomplete-root", ("inventory.json", lambda text: json.dumps(
            dict(json.loads(text), roots=["MetadataReceipt"]), indent=2) + '\n'),
         {"reason": "incomplete module dependency audit"}),
    ]
    from ready_controls import mutations
    ready_model = (ROOT / inventory["models"]["ReadyPublication"]["source"]).read_text()
    for control in mutations(ready_model):
        controls.append((control["name"],
                         ("ReadyPublication.tla", lambda text, source=control["model"]: source),
                         {"reason": "ReadyPublicationProof: TLAPS exit 10",
                          "log": "ReadyPublicationProof.log", "failed": 1,
                          "pattern": control["proof_pattern"]}))
    for name, mutation, expected in controls:
        before = run_case(args, name + "-baseline")
        mutant = run_case(args, name + "-mutant", mutation, expected)
        after = run_case(args, name + "-restored")
        require(before["sources"] == after["sources"], "restored sources differ from baseline")
        changed = [f for f, checksum in before["sources"].items() if mutant["sources"][f] != checksum]
        require(changed == [mutation[0]], "control changed more than its owned source")
        records.extend((before, mutant, after))
    root = inventory["roots"][-1]
    output = (args.output / f"baseline/{root}.log").read_text()
    output_count = output_controls(output, root, inventory["modules"][root]["obligations"])
    count = sum(len(item["theorems"]) for item in inventory["modules"].values())
    obligations = sum(item["obligations"] for item in inventory["modules"].values())
    summary = {"version": version, "tlapm_sha256": sha(args.tlapm),
               "sany_jar_sha256": sha(args.jar), "runner_sha256": sha(Path(__file__)),
               "controls_sha256": sha(ROOT / "scripts/ready_controls.py"),
               "auditor_sha256": sha(ROOT / "scripts/ProofAudit.java"), "runs": records,
               "theorems": count, "obligations": obligations, "output_controls": output_count}
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"PASS: {count} TLAPS theorems; {obligations} proof obligations; "
          f"{len(controls)} isolated controls; {output_count} output controls", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (Rejected, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        raise SystemExit(f"FAIL: {error}") from error
