#!/usr/bin/env python3
"""Fresh native batch TLC/TLAPS checks, audited imports, and isolated counterexamples."""
import argparse
from concurrent.futures import ThreadPoolExecutor, wait, FIRST_COMPLETED
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

from native_batch_controls import mutations, witness_mutations

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "proofs/tla/native_batch"
PROOFS = ROOT / "proofs/tlaps/native_batch"
COPIES = {
    "RawMutation.tla": "proofs/tla/raw_group/RawMutation.tla",
    "RawMutationProof.tla": "proofs/tlaps/raw_group/RawMutationProof.tla",
    "ClientRetry.tla": "proofs/tla/client/ClientRetry.tla",
    "ClientRetryProof.tla": "proofs/tlaps/client/ClientRetryProof.tla",
    "NativeBatch.tla": "proofs/tla/native_batch/NativeBatch.tla",
    "NativeBatchMC.tla": "proofs/tla/native_batch/NativeBatchMC.tla",
    "NativeBatchAlgebraProof.tla": "proofs/tlaps/native_batch/NativeBatchAlgebraProof.tla",
    "NativeBatchProof.tla": "proofs/tlaps/native_batch/NativeBatchProof.tla",
    "inventory.json": "proofs/tlaps/native_batch/inventory.json",
}
ACTIONS = ("NBDispatch", "NBEffect", "NBSuccess", "NBControl", "NBBackground",
           "NBReadStart", "NBReadCapture", "NBReadItem", "NBReadPublish", "NBReadFail", "NBQuiesce")
CONFIGS = ("Batch2.cfg", "Batch3.cfg")
WITNESSES = (("NBNoSuccess", False), ("NBNoLateEffect", True),
             ("NBNoRetriedSuccess", False), ("NBNoCompletedOldView", False),
             ("NBNoCollectionOverlap", True))


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


tlc = module("native_batch_tlc", "check-tla.py")
proof = module("native_batch_tlaps", "check-tlaps.py")
require, digest = tlc.require, tlc.digest


class ProofProcesses:
    def __getattr__(self, name):
        return getattr(subprocess, name)

    def run(self, command, *args, **kwargs):
        if command[0] == "java" and "ProofAudit" in command:
            temporary = Path(kwargs["cwd"]) / "sany-temp"
            temporary.mkdir(exist_ok=True)
            command = [command[0], f"-Djava.io.tmpdir={temporary}", *command[1:]]
        return subprocess.run(command, *args, **kwargs)


proof.subprocess = ProofProcesses()


def save(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def case_tree(args, name, edits=None):
    work = args.output / name
    work.mkdir()
    for filename, source in COPIES.items():
        shutil.copyfile(ROOT / source, work / filename)
    for filename, text in (edits or {}).items():
        require(filename in COPIES, "undeclared edited source")
        (work / filename).write_text(text)
    return work


def model_case(args, name, config, edits=None, expected=None, action=False, fp=0, coverage=False):
    work = case_tree(args, name, edits)
    (work / "Run.cfg").write_text(config)
    record = dict(name=name, expected=expected, action=action, fingerprint=fp,
                  sources={p.name: digest(p) for p in sorted(work.iterdir())})
    command = ["java", f"-Djava.io.tmpdir={work}", "-XX:+UseParallelGC", "-Xmx512m",
               "-cp", str(args.jar), "tlc2.TLC", "-tool", "-workers", "1", "-fp", str(fp),
               "-metadir", str(work / "states"), "-config", "Run.cfg"]
    if coverage:
        command += ["-coverage", "999"]
    command += ["NativeBatchMC.tla"]
    record.update(command=command, started_ns=time.time_ns())
    try:
        with (work / "tlc.log").open("w") as log:
            result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                                    timeout=args.timeout, env=dict(os.environ, LC_ALL="C"))
        output = (work / "tlc.log").read_text()
        record.update(exit_code=result.returncode, statistics=tlc.verdict(
            output, result.returncode, expected, action_property=action, coverage=coverage,
            module="NativeBatch", actions=ACTIONS, minimum_distinct=2 if expected else 3))
        record["verdict"] = "expected counterexample" if expected else "complete"
    except Exception as error:
        record.update(verdict="failed", reason=str(error))
        raise
    finally:
        record["ended_ns"] = time.time_ns()
        save(work / "result.json", record)
    print("PASS:", name, record["verdict"], flush=True)
    return record


def failed_theorems(output, source, filename):
    positions = re.findall(r'File "[^"\n]+/' + re.escape(filename) +
                           r'", line (\d+),[^\n]*:\n\[ERROR\]: Could not prove or check:', output)
    declarations = [(source.count("\n", 0, m.start()) + 1, m.group(1))
                    for m in re.finditer(r"^THEOREM (\w+)", source, re.M)]
    return {next(name for line, name in reversed(declarations) if line <= int(position))
            for position in positions}


def proof_case(args, name, edits=None, expected=None, expected_module=None, theorem=None):
    work = case_tree(args, name, edits)
    record = dict(name=name, expected=expected,
                  sources={p.name: digest(p) for p in sorted(work.iterdir())})
    try:
        proof.check_tree(args, work, record)
        require(expected is None, name + ": invalid proof accepted")
        record["verdict"] = "proved"
    except proof.Rejected as error:
        record.update(verdict="rejected", reason=str(error))
        require(expected is not None and str(error) == expected,
                name + ": rejection outside intended gate: " + str(error))
        if theorem:
            output = (work / (expected_module + ".log")).read_text()
            names = failed_theorems(output, (work / (expected_module + ".tla")).read_text(),
                                    expected_module + ".tla")
            require(theorem in names, name + ": intended theorem did not fail")
            counts = re.findall(r"\[ERROR\]: (\d+)/(\d+) obligations failed\.", output)
            require(len(counts) == 1 and 0 < int(counts[0][0]) < int(counts[0][1]),
                    name + ": missing nontrivial obligation failure")
            require("[WARNING]" not in output and "backend errors" in output,
                    name + ": unexpected negative proof diagnostic")
            record["failed_theorems"] = sorted(names)
        record.update(verdict="expected rejection", reason=str(error))
    except Exception as error:
        record.update(verdict="failed", reason=str(error))
        raise
    finally:
        save(work / "result.json", record)
    print("PASS:", name, record["verdict"], flush=True)
    return record


def triple(records, filename):
    before, mutated, after = records
    require(before["sources"] == after["sources"], "restored sources differ")
    require([n for n in before["sources"] if before["sources"][n] != mutated["sources"][n]] == [filename],
            "mutation changed unrelated inputs")
    if "statistics" in before:
        require(before["statistics"] == after["statistics"], "restored exploration differs")


def groups(function, jobs, count):
    iterator = iter(jobs)
    pool = ThreadPoolExecutor(max_workers=count)
    pending = set()
    try:
        for _ in range(count):
            job = next(iterator, None)
            if job is not None:
                pending.add(pool.submit(function, job))
        while pending:
            done, pending = wait(pending, return_when=FIRST_COMPLETED)
            results = [future.result() for future in done]
            yield from results
            for _ in done:
                job = next(iterator, None)
                if job is not None:
                    pending.add(pool.submit(function, job))
    finally:
        for future in pending:
            future.cancel()
        pool.shutdown(wait=True, cancel_futures=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", type=Path, required=True)
    parser.add_argument("--tlapm", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=300)
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--model-only", action="store_true")
    args = parser.parse_args()
    args.jar, args.tlapm, args.output = args.jar.resolve(), args.tlapm.resolve(), args.output.resolve()
    require(args.timeout > 0 and 1 <= args.jobs <= 4, "invalid proof resource bounds")
    require(not args.output.exists() and not args.output.is_relative_to(ROOT), "new outside-source output required")
    affinity = sorted(os.sched_getaffinity(0))
    require(affinity and set(affinity) <= set(range(6, 32)), "run with CPUs 6-31")
    inv = json.loads((PROOFS / "inventory.json").read_text())
    require(digest(args.jar) == inv["sany_jar_sha256"] == tlc.JAR_SHA256, "TLC/SANY hash mismatch")
    version = subprocess.check_output([str(args.tlapm), "--version"], text=True, timeout=15).strip()
    require(version == inv["tlapm_version"], "TLAPS version mismatch")
    require(set(inv["modules"]) == {"RawMutationProof", "ClientRetryProof", "NativeBatchAlgebraProof", "NativeBatchProof"}
            and inv["roots"] == ["NativeBatchProof"], "proof module inventory differs")
    require(set(inv["models"]) == {"RawMutation", "ClientRetry", "NativeBatch"}, "model inventory differs")
    require({p.name for p in SOURCE.iterdir()} == {"NativeBatch.tla", "NativeBatchMC.tla", *CONFIGS},
            "native model/config inventory differs")
    require({p.name for p in PROOFS.iterdir()} == {"NativeBatchAlgebraProof.tla", "NativeBatchProof.tla", "inventory.json"},
            "native proof inventory differs")
    bound = {p: digest(ROOT / p) for p in sorted(set(COPIES.values()) | {
        "scripts/check-native-batch-protocol.py", "scripts/native_batch_controls.py",
        "scripts/check-tla.py", "scripts/check-tlaps.py", "scripts/ready_controls.py", "scripts/ProofAudit.java",
        *(str((SOURCE / c).relative_to(ROOT)) for c in CONFIGS)})}
    args.output.mkdir()
    for name, expected in bound.items():
        path = args.output / "source-snapshot" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / name, path)
        require(digest(path) == expected, "source changed during snapshot")
    args.stdlib = args.tlapm.parent.parent / "lib/tlapm/stdlib"
    args.classes = args.output / "auditor"
    args.classes.mkdir()
    shutil.copyfile(ROOT / "scripts/ProofAudit.java", args.classes / "ProofAudit.java")
    subprocess.run(["javac", "-cp", str(args.jar), "-d", str(args.classes),
                    str(args.classes / "ProofAudit.java")], check=True, timeout=30)
    models = {n: (ROOT / path).read_text() for n, path in COPIES.items() if n.endswith(".tla")}
    controls = mutations(models)
    records, proofs = [], []
    config = (SOURCE / CONFIGS[0]).read_text()
    plain = "\n".join(line for line in config.splitlines() if not line.startswith(("INVARIANT", "PROPERTY"))) + "\n"
    for cfg in CONFIGS:
        runs = [model_case(args, Path(cfg).stem + "-fp" + str(fp), (SOURCE / cfg).read_text(),
                           fp=fp, coverage=True) for fp in (0, 1)]
        require(runs[0]["statistics"] == runs[1]["statistics"], "fingerprint exploration differs")
        records += runs
    for witness, action in WITNESSES:
        selected = plain + ("PROPERTY " if action else "INVARIANT ") + witness + "\n"
        records.append(model_case(args, "witness-" + witness, selected,
                                  expected=witness, action=action))
    witness_controls = witness_mutations(models)
    for c in witness_controls:
        selected = plain + "PROPERTY " + c["property"] + "\n"
        runs = []
        for suffix, restricted in (("baseline", False), ("restricted", True), ("restored", False)):
            runs.append(model_case(args, "witness-control-" + c["name"] + "-" + suffix, selected,
                {c["filename"]: c["text"]} if restricted else None,
                expected=None if restricted else c["property"], action=c["action"]))
        triple(runs, c["filename"])
        records += runs
    for c in controls:
        selected = plain + ("PROPERTY " if c["action"] else "INVARIANT ") + c["property"] + "\n"
        runs = []
        for suffix, invalid in (("baseline", False), ("mutant", True), ("restored", False)):
            runs.append(model_case(args, "model-" + c["name"] + "-" + suffix, selected,
                {c["filename"]: c["text"]} if invalid else None,
                c["property"] if invalid else None, action=c["action"]))
        triple(runs, c["filename"])
        records += runs
    if not args.model_only:
        proofs.append(proof_case(args, "proof-baseline"))
        def protocol_control(c):
            runs = []
            for suffix, invalid in (("baseline", False), ("mutant", True), ("restored", False)):
                runs.append(proof_case(args, "proof-" + c["name"] + "-" + suffix,
                    {c["filename"]: c["text"]} if invalid else None,
                    c["proof_module"] + ": TLAPS exit 10" if invalid else None,
                    c["proof_module"] if invalid else None, c["proof_theorem"] if invalid else None))
            triple(runs, c["filename"])
            return runs
        for runs in groups(protocol_control, controls, args.jobs):
            proofs += runs
        # Audit every owned proof, including imported algebra/retry modules.
        for name in inv["modules"]:
            filename = name + ".tla"
            source = models[filename]
            first = re.search(r"^THEOREM (\w+)", source, re.M)
            start = re.search(r"^(?:BY |<1>)", source[first.end():], re.M)
            require(start is not None, "missing first proof body")
            begin = first.end() + start.start()
            end = re.search(r"^(?:THEOREM |={5})", source[begin:], re.M)
            require(end is not None, "missing first proof terminator")
            hole = source[:begin] + "OMITTED\n\n" + source[begin + end.start():]
            extends = re.search(r"^EXTENDS [^\n]+", source, re.M).group(0)
            axiom = tlc.replace_once(source, extends, extends + "\nAXIOM FALSE")
            for label, mutant, expected in (
                ("hole", hole, f"proof hole: {name}.{first.group(1)}"),
                ("axiom", axiom, f"unapproved module assumption: {name}")):
                runs = []
                for suffix, changed in (("baseline", False), ("mutant", True), ("restored", False)):
                    runs.append(proof_case(args, f"audit-{name}-{label}-{suffix}",
                        {filename: mutant} if changed else None, expected if changed else None))
                triple(runs, filename)
                proofs += runs
    proof_outputs = 0
    if not args.model_only:
        for name, item in inv["modules"].items():
            text = (args.output / "proof-baseline" / (name + ".log")).read_text()
            proof_outputs += proof.output_controls(text, name, item["obligations"])
    good = (args.output / "Batch2-fp0/tlc.log").read_text()
    output_controls = [
        ("", "missing pinned TLC version"),
        (re.sub(r"@!@!@STARTMSG 2186:0 @!@!@.*?@!@!@ENDMSG 2186 @!@!@", "", good, flags=re.S), "missing TLC completion"),
        (re.sub(r"(\d+) states left on queue\.", "1 states left on queue.", good), "state exploration is incomplete")]
    for invalid, expected in output_controls:
        require(invalid != good, "missing log-control mutation")
        try:
            tlc.verdict(invalid, 0, module="NativeBatch", actions=ACTIONS)
        except tlc.Rejected as error:
            require(str(error) == expected, "log rejected outside intended check")
        else:
            raise tlc.Rejected("corrupted model output accepted")
    require(bound == {n: digest(ROOT / n) for n in bound}, "source changed during acceptance")
    summary = dict(verdict="finite preflight only" if args.model_only else "accepted",
        affinity=affinity, jar_sha256=digest(args.jar), tlapm_sha256=digest(args.tlapm),
        tlapm_version=version, sources=bound, models=records, proofs=proofs,
        protocol_controls=len(controls), witness_controls=len(witness_controls),
        model_output_controls=len(output_controls),
        proof_output_controls=proof_outputs)
    save(args.output / "summary.json", summary)
    print("PASS:", summary["verdict"], len(records), "finite cases;", len(proofs), "proof/audit cases", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (tlc.Rejected, proof.Rejected, OSError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit("FAIL: " + str(error)) from error
