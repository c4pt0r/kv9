#!/usr/bin/env python3
"""Check the first event-driven Raft scheduling boundary and parameterized proofs."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

from raft_schedule_controls import mutations

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "proofs/tla/raft_schedule"
PROOFS = ROOT / "proofs/tlaps/raft_schedule"
SCOPES = {
    "RaftSchedule": dict(wrapper="RaftScheduleMC", configs=("Schedule2.cfg", "Schedule3.cfg"),
        live=("ScheduleService.cfg", "ScheduleStop.cfg"), actions=("RSStartPublish", "RSPublish", "RSRefuse", "RSNotify", "RSHint", "RSSpawn", "RSBegin", "RSDrain", "RSFinish", "RSPark", "RSWake", "RSTimeout", "RSStop", "RSExit", "RSQuiesce"),
        witnesses=("RSNoWorkWitness", "RSNoCoalescingWitness", "RSNoRetainedWitness", "RSNoParkWitness", "RSNoTerminalWitness")),
    "RaftTick": dict(wrapper="RaftTickMC", configs=("Tick.cfg",), live=("TickLive.cfg",),
        actions=("RTAdvance", "RTTraffic", "RTTick", "RTNotDue"), witnesses=("RTNoTickWitness", "RTNoDelayWitness")),
    "RaftInboxBudget": dict(wrapper="RaftInboxBudget", configs=("Inbox.cfg",), live=(),
        actions=("RBAdmit", "RBBegin", "RBPop", "RBFinish", "RBQuiesce"),
        witnesses=("RBNoZeroWeightWitness", "RBNoOvershootWitness", "RBNoRetainedWitness")),
}


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


tlc = module("schedule_tlc_gate", "check-tla.py")
proof = module("schedule_proof_gate", "check-tlaps.py")
require, digest = tlc.require, tlc.digest


class IsolatedProofProcesses:
    """Give each SANY JVM its own extracted standard-module namespace."""
    def __getattr__(self, name):
        return getattr(subprocess, name)

    def run(self, command, *positional, **keywords):
        if command[0] == "java" and "ProofAudit" in command:
            temporary = Path(keywords["cwd"]) / "sany-temp"
            temporary.mkdir(exist_ok=True)
            command = [command[0], f"-Djava.io.tmpdir={temporary}", *command[1:]]
        return subprocess.run(command, *positional, **keywords)


proof.subprocess = IsolatedProofProcesses()


def verdict(output, status, module, expected=None, temporal=False, coverage=False, action=False):
    return tlc.verdict(output, status,
        "EventuallyDrained" if expected in {"RSServiceProgress", "RTProgress"} else expected,
        temporal=temporal, coverage=coverage, action_property=action, module=module,
        actions=SCOPES[module]["actions"], minimum_distinct=2 if expected else 3)


def model_case(args, name, module, config, model, expected=None, fp=0, coverage=False, temporal=False, action=False):
    work = args.output / name
    work.mkdir()
    wrapper = SCOPES[module]["wrapper"]
    if wrapper != module:
        shutil.copyfile(SOURCE / f"{wrapper}.tla", work / f"{wrapper}.tla")
    (work / f"{module}.tla").write_text(model)
    (work / "Run.cfg").write_text(config)
    record = dict(name=name, module=module, expected=expected, fingerprint=fp, temporal=temporal,
        action_property=action, coverage=coverage,
        sources={p.name: digest(p) for p in sorted(work.iterdir())})
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix="kv9-raft-schedule-states.") as states:
            command = ["java", f"-Djava.io.tmpdir={states}", "-XX:+UseParallelGC", "-Xmx512m", "-cp", str(args.jar),
                       "tlc2.TLC", "-tool", "-workers", "1", "-fp", str(fp), "-metadir", states, "-config", "Run.cfg"]
            if coverage:
                command += ["-coverage", "999"]
            command += [f"{wrapper}.tla"]
            record["command"] = command
            with (work / "tlc.log").open("w") as log:
                result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                    timeout=args.timeout, env=dict(os.environ, LC_ALL="C"))
        output = (work / "tlc.log").read_text()
        record.update(exit_code=result.returncode, seconds=time.monotonic() - started)
        record.update(statistics=verdict(output, result.returncode, module, expected, temporal, coverage, action), verdict="accepted")
    except subprocess.TimeoutExpired:
        record.update(verdict="inconclusive", reason="timeout")
        raise tlc.Rejected(f"{name}: TLC timed out") from None
    except tlc.Rejected as error:
        record.update(verdict="rejected", reason=str(error))
        raise tlc.Rejected(f"{name}: {error}; see {work}") from error
    finally:
        (work / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    print(f"PASS: {name}; {record['statistics']['distinct']} distinct states", flush=True)
    return record, output


def scope_inventory(inventory, module):
    """Each independent root imports exactly one model and the pinned standards."""
    return dict(inventory, modules={module + "Proof": inventory["modules"][module + "Proof"]},
                models={module: inventory["models"][module]}, roots=[module + "Proof"])


def proof_case(args, name, module, model, source, expected=None, pattern=None):
    work = args.output / name
    work.mkdir()
    root = module + "Proof"
    (work / f"{module}.tla").write_text(model)
    (work / f"{root}.tla").write_text(source)
    (work / "inventory.json").write_text(json.dumps(scope_inventory(args.inventory, module), indent=2) + "\n")
    record = dict(name=name, module=module, expected=expected, pattern=pattern,
                  sources={p.name: digest(p) for p in sorted(work.iterdir())})
    try:
        proof.check_tree(args, work, record)
    except proof.Rejected as error:
        record.update(verdict="rejected", reason=str(error))
        require(expected is not None and str(error) == expected,
                f"{name}: failed outside the intended gate: {error}")
        if pattern:
            output = (work / f"{root}.log").read_text()
            require(re.search(pattern, output) is not None, f"{name}: missing intended failed obligation")
            counts = re.findall(r"\[ERROR\]: (\d+)/(\d+) obligations failed\.", output)
            require(len(counts) == 1 and 0 < int(counts[0][0]) < int(counts[0][1]),
                    f"{name}: missing nontrivial failed obligation count")
            require("[WARNING]" not in output and "backend errors" in output,
                    f"{name}: unexpected negative proof diagnostics")
        record["verdict"] = "expected rejection"
    else:
        require(expected is None, f"{name}: invalid proof accepted")
        record["verdict"] = "proved"
    finally:
        (work / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    print(f"PASS: {name}; {record['verdict']}", flush=True)
    return record


def check_triple(records, filename):
    before, mutant, restored = records
    require(before["sources"] == restored["sources"], "restored sources differ")
    changed = [name for name, sha in before["sources"].items() if mutant["sources"][name] != sha]
    require(changed == [filename], "control changed unrelated source")
    if "statistics" in before:
        require(before["statistics"] == restored["statistics"], "restored exploration differs")


def output_controls(output, module, *, temporal=False):
    cases = [("empty output", "", "missing pinned TLC version"),
        ("missing completion", re.sub(r"@!@!@STARTMSG 2186:0 @!@!@.*?@!@!@ENDMSG 2186 @!@!@", "", output, flags=re.S), "missing TLC completion"),
        ("unfinished exploration", re.sub(r"(\d+) states left on queue\.", "1 states left on queue.", output), "state exploration is incomplete")]
    if temporal:
        for code in (2192, 2267):
            cases.append((f"missing temporal {code}", re.sub(rf"@!@!@STARTMSG {code}:0 @!@!@.*?@!@!@ENDMSG {code} @!@!@", "", output, flags=re.S), "missing complete temporal check"))
    for name, invalid, reason in cases:
        require(invalid != output, f"missing output control anchor: {name}")
        verdict(output, 0, module, temporal=temporal)
        try:
            verdict(invalid, 0, module, temporal=temporal)
        except tlc.Rejected as error:
            require(str(error) == reason, f"{name}: wrong output rejection")
        else:
            raise tlc.Rejected(f"{name}: malformed output accepted")
        verdict(output, 0, module, temporal=temporal)
    return len(cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", required=True, type=Path)
    parser.add_argument("--tlapm", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--jobs", type=int, default=4)
    parser.add_argument("--model-only", action="store_true", help="explicit finite preflight; never a proof acceptance")
    args = parser.parse_args()
    args.jar, args.tlapm, args.output = args.jar.resolve(), args.tlapm.resolve(), args.output.resolve()
    args.stdlib = args.tlapm.parent.parent / "lib/tlapm/stdlib"
    args.inventory = json.loads((PROOFS / "inventory.json").read_text())
    require(args.timeout > 0 and not args.output.exists(), "positive timeout and new output directory required")
    require(1 <= args.jobs <= 8, "jobs must be between one and eight")
    affinity = sorted(os.sched_getaffinity(0))
    require(affinity and set(affinity) <= set(range(6, 32)), "run with taskset -c 6-31; CPUs 0-5 are reserved")
    require(digest(args.jar) == args.inventory["sany_jar_sha256"] == tlc.JAR_SHA256, "TLC/SANY hash mismatch")
    version = subprocess.check_output([str(args.tlapm), "--version"], text=True, timeout=15).strip()
    require(version == args.inventory["tlapm_version"], "TLAPS version mismatch")
    require(set(args.inventory["models"]) == set(SCOPES), "model inventory mismatch")
    require(set(args.inventory["modules"]) == {m + "Proof" for m in SCOPES} == set(args.inventory["roots"]), "root inventory mismatch")
    require({p.stem for p in PROOFS.glob("*.tla")} == set(args.inventory["modules"]), "proof source inventory mismatch")
    require({p.stem for p in SOURCE.glob("*.tla")} == set(SCOPES) | {s["wrapper"] for s in SCOPES.values()}, "model source inventory mismatch")
    require({p.name for p in SOURCE.glob("*.cfg")} == {c for s in SCOPES.values() for c in s["configs"] + s["live"]}, "configuration inventory mismatch")
    args.output.mkdir(parents=True)
    args.classes = args.output / "auditor"
    args.classes.mkdir()
    shutil.copyfile(ROOT / "scripts/ProofAudit.java", args.classes / "ProofAudit.java")
    subprocess.run(["javac", "-cp", str(args.jar), "-d", str(args.classes), str(args.classes / "ProofAudit.java")], check=True, timeout=30)
    models = {m: (SOURCE / f"{m}.tla").read_text() for m in SCOPES}
    sources = {m: (PROOFS / f"{m}Proof.tla").read_text() for m in SCOPES}
    controls = mutations(models)
    model_records, proof_records, good_outputs = [], [], {}
    # Finish every cheap finite check, fault and witness before expensive proof
    # triples. A model-only summary explicitly carries no proof acceptance.
    for m, scope in SCOPES.items():
        for filename in scope["configs"]:
            pair = []
            for fp in (0, 1):
                record, output = model_case(args, f"{Path(filename).stem}-fp{fp}", m,
                    (SOURCE / filename).read_text(), models[m], fp=fp, coverage=True)
                pair.append(record)
                good_outputs[m] = output
            require(pair[0]["statistics"] == pair[1]["statistics"], "fingerprint exploration differs")
            model_records.extend(pair)
        for filename in scope["live"]:
            record, output = model_case(args, Path(filename).stem, m,
                (SOURCE / filename).read_text(), models[m], temporal=True)
            model_records.append(record)
            good_outputs[m + "Live"] = output
        for witness in scope["witnesses"]:
            config = (SOURCE / scope["configs"][0]).read_text() + f"INVARIANT {witness}\n"
            record, _ = model_case(args, "witness-" + witness, m, config, models[m], witness)
            model_records.append(record)
    for c in controls:
        m = c["module"]
        config = (SOURCE / SCOPES[m]["live"][0]).read_text() if c["live"] else (
            "\n".join(line for line in (SOURCE / SCOPES[m]["configs"][0]).read_text().splitlines()
                if not line.startswith(("INVARIANT", "PROPERTY"))) + "\n" +
            ("PROPERTY " if c["action"] else "INVARIANT ") + c["property"] + "\n")
        group = []
        for suffix, text, invalid in (("baseline", models[m], False), ("mutant", c["model"], True), ("restored", models[m], False)):
            record, _ = model_case(args, f"model-{c['name']}-{suffix}", m, config, text,
                c["property"] if invalid else None, temporal=c["live"], action=c["action"])
            group.append(record)
        check_triple(group, m + ".tla")
        model_records.extend(group)
    if not args.model_only:
        for m in SCOPES:
            proof_records.append(proof_case(args, "proof-baseline-" + m, m, models[m], sources[m]))

        def group(c):
            m = c["module"]
            records = []
            for suffix, text, invalid in (("baseline", models[m], False), ("mutant", c["model"], True), ("restored", models[m], False)):
                records.append(proof_case(args, f"proof-{c['name']}-{suffix}", m, text, sources[m],
                    m + "Proof: TLAPS exit 10" if invalid else None, c["proof_pattern"] if invalid else None))
            check_triple(records, m + ".tla")
            return records

        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = [pool.submit(group, c) for c in controls]
            for future in futures:
                proof_records.extend(future.result())
        for m, source in sources.items():
            first = re.search(r"THEOREM (\w+).*?\n(BY [^\n]+)", source, flags=re.S)
            require(first is not None, "missing first proof control anchor")
            faults = [("omitted-proof", tlc.replace_once(source, first.group(2), "OMITTED"),
                       f"proof hole: {m}Proof.{first.group(1)}"),
                      ("custom-axiom", tlc.replace_once(source, f"EXTENDS {m}, TLAPS", f"EXTENDS {m}, TLAPS\nAXIOM FALSE"),
                       f"unapproved module assumption: {m}Proof")]
            for name, mutant, reason in faults:
                records = []
                for suffix, text, expected in (("baseline", source, None), ("mutant", mutant, reason), ("restored", source, None)):
                    records.append(proof_case(args, f"audit-{m}-{name}-{suffix}", m, models[m], text, expected))
                check_triple(records, m + "Proof.tla")
                proof_records.extend(records)
    model_outputs = sum(output_controls(good_outputs[m], m) for m in SCOPES)
    model_outputs += sum(output_controls(good_outputs[m + "Live"], m, temporal=True) for m in SCOPES if SCOPES[m]["live"])
    proof_outputs = 0
    if not args.model_only:
        for m in SCOPES:
            root = m + "Proof"
            output = (args.output / ("proof-baseline-" + m) / (root + ".log")).read_text()
            proof_outputs += proof.output_controls(output, root, args.inventory["modules"][root]["obligations"])
    summary = dict(verdict="finite preflight only" if args.model_only else "accepted", affinity=affinity,
        jar_sha256=digest(args.jar), tlapm_version=version, tlapm_sha256=digest(args.tlapm), jobs=args.jobs,
        sources={str(p.relative_to(ROOT)): digest(p) for p in [Path(__file__), ROOT / "scripts/check-tla.py",
            ROOT / "scripts/check-tlaps.py", ROOT / "scripts/raft_schedule_controls.py", ROOT / "scripts/ready_controls.py",
            ROOT / "scripts/ProofAudit.java", *sorted(SOURCE.iterdir()), *sorted(PROOFS.iterdir())] if p.is_file()},
        models=model_records, proofs=proof_records, protocol_controls=len(controls),
        model_output_controls=model_outputs, proof_output_controls=proof_outputs)
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"PASS: {summary['verdict']}; {len(model_records)} finite checks; {len(proof_records)} proof/audit cases; "
          f"{len(controls)} protocol controls; {model_outputs + proof_outputs} output controls", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (tlc.Rejected, proof.Rejected, OSError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f"FAIL: {error}") from error
