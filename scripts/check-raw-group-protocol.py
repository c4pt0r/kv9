#!/usr/bin/env python3
"""Check the Raw group composition boundary and parameterized proofs."""
import argparse
from concurrent.futures import ThreadPoolExecutor, wait, FIRST_COMPLETED
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

from raw_group_controls import mutations

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "proofs/tla/raw_group"
PROOFS = ROOT / "proofs/tlaps/raw_group"
SCOPES = {
    "RawGroup": dict(wrapper="RawGroupMC", configs=(
        "Ordered.cfg", "Stale.cfg", "Empty.cfg", "ReadError.cfg", "BadPosition.cfg", "OptOut.cfg", "Oversized.cfg",
        "CatalogBarrier.cfg", "ManifestBarrier.cfg", "ConfBarrier.cfg", "NoopBarrier.cfg", "MalformedBarrier.cfg",
        "HiddenSystem.cfg", "CountBound.cfg", "ByteBound.cfg"),
        live=("OrderedLive.cfg", "MalformedBarrierLive.cfg", "OversizedLive.cfg"),
        actions=("RGGrow", "RGChooseDone", "RGDelegate", "RGLoad", "RGPlan", "RGPlanDone", "RGEffect",
            "RGEngineSuccess", "RGPublish", "RGTailPass", "RGReport", "RGFail", "RGQuiesce"),
        witnesses=(("RGNoSuccessWitness", "Ordered.cfg"), ("RGNoUnknownWitness", "Ordered.cfg"),
            ("RGNoLaterFailureWitness", "MalformedBarrier.cfg"), ("RGNoStaleWitness", "Stale.cfg"),
            ("RGNoSingletonWitness", "Oversized.cfg"), ("RGNoEmptyEffectWitness", "Empty.cfg"))),
}



def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


tlc = module("raw_group_tlc_gate", "check-tla.py")
proof = module("raw_group_proof_gate", "check-tlaps.py")
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


def verdict(output, status, module, expected=None, temporal=False, coverage=False, action=False, actions=None):
    return tlc.verdict(output, status,
        "EventuallyDrained" if expected in {"RGProgress"} else expected,
        temporal=temporal, coverage=coverage, action_property=action, module=module,
        actions=actions if actions is not None else SCOPES[module]["actions"], minimum_distinct=2 if expected else 3)


def model_case(args, name, module, config, model, expected=None, fp=0, coverage=False, temporal=False, action=False):
    work = args.output / name
    work.mkdir()
    wrapper = SCOPES[module]["wrapper"]
    if wrapper != module:
        shutil.copyfile(SOURCE / f"{wrapper}.tla", work / f"{wrapper}.tla")
    shutil.copyfile(SOURCE / "RawMutation.tla", work / "RawMutation.tla")
    (work / f"{module}.tla").write_text(model)
    (work / "Run.cfg").write_text(config)
    record = dict(name=name, module=module, expected=expected, fingerprint=fp, temporal=temporal,
        action_property=action, coverage=coverage,
        sources={p.name: digest(p) for p in sorted(work.iterdir())})
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix="kv9-raw-group-states.") as states:
            command = ["java", f"-Djava.io.tmpdir={states}", "-XX:+UseParallelGC", "-Xmx512m", "-cp", str(args.jar),
                       "tlc2.TLC", "-tool", "-lncheck", "final", "-workers", "1", "-fp", str(fp), "-metadir", states, "-config", "Run.cfg"]
            if coverage:
                command += ["-coverage", "999"]
            command += [f"{wrapper}.tla"]
            record["command"] = command
            with (work / "tlc.log").open("w") as log:
                result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                    timeout=args.timeout, env=dict(os.environ, LC_ALL="C"))
        output = (work / "tlc.log").read_text()
        record.update(exit_code=result.returncode, seconds=time.monotonic() - started)
        if coverage:
            filename = next(c for c in SCOPES[module]["configs"] if (SOURCE / c).read_text() == config)
            needed = {"RGChooseDone", "RGQuiesce"}
            if filename in {"OptOut.cfg", "Oversized.cfg"}:
                needed.add("RGDelegate")
            else:
                needed |= {"RGGrow", "RGLoad", "RGPlan", "RGFail"}
                if filename not in {"ReadError.cfg", "BadPosition.cfg"}:
                    needed |= {"RGPlanDone", "RGEffect", "RGEngineSuccess", "RGPublish"}
                    if filename != "MalformedBarrier.cfg":
                        needed.add("RGReport")
                    if filename in {"CatalogBarrier.cfg", "ManifestBarrier.cfg", "ConfBarrier.cfg", "NoopBarrier.cfg", "HiddenSystem.cfg", "CountBound.cfg", "ByteBound.cfg"}:
                        needed.add("RGTailPass")
            record["covered_actions"] = sorted(needed)
        record.update(statistics=verdict(output, result.returncode, module, expected, temporal, coverage, action,
            record.get("covered_actions")), verdict="accepted")
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
    """The transition proof imports and freshly rechecks the mutation algebra."""
    require(module == "RawGroup", "unexpected proof root")
    return inventory


def proof_case(args, name, module, model, source, expected=None, pattern=None, source_edits=None, theorem=None):
    work = args.output / name
    work.mkdir()
    root = module + "Proof"
    shutil.copyfile(PROOFS / "RawMutationProof.tla", work / "RawMutationProof.tla")
    shutil.copyfile(SOURCE / "RawMutation.tla", work / "RawMutation.tla")
    (work / f"{module}.tla").write_text(model)
    (work / f"{root}.tla").write_text(source)
    for filename, text in (source_edits or {}).items():
        (work / filename).write_text(text)
    (work / "inventory.json").write_text(json.dumps(scope_inventory(args.inventory, module), indent=2) + "\n")
    record = dict(name=name, module=module, expected=expected, pattern=pattern, theorem=theorem,
                  sources={p.name: digest(p) for p in sorted(work.iterdir())})
    try:
        proof.check_tree(args, work, record)
    except proof.Rejected as error:
        record.update(verdict="rejected", reason=str(error))
        require(expected is not None and str(error) == expected,
                f"{name}: failed outside the intended gate: {error}")
        if pattern:
            output = (work / f"{root}.log").read_text()
            if theorem:
                locations = re.findall(r'File "[^"\n]+/RawGroupProof\.tla", line (\d+),[^\n]*:\n\[ERROR\]: Could not prove or check:', output)
                declarations = [(source.count("\n", 0, match.start()) + 1, match.group(1))
                    for match in re.finditer(r"^THEOREM (\w+)", source, re.M)]
                failed_theorems = {next(name for line, name in reversed(declarations) if line <= int(location))
                    for location in locations}
                record["failed_theorems"] = sorted(failed_theorems)
                require(theorem in failed_theorems, f"{name}: intended theorem did not fail: {theorem}")
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


def bounded_groups(function, jobs, parallelism):
    """Keep at most parallelism groups active; stop scheduling after failure."""
    iterator = iter(jobs)
    pool = ThreadPoolExecutor(max_workers=parallelism)
    pending = set()
    try:
        for _ in range(parallelism):
            item = next(iterator, None)
            if item is None:
                break
            pending.add(pool.submit(function, item))
        while pending:
            finished, pending = wait(pending, return_when=FIRST_COMPLETED)
            # Read every completed verdict before scheduling replacements.
            batches = [future.result() for future in finished]
            for batch in batches:
                yield batch
            for _ in finished:
                item = next(iterator, None)
                if item is not None:
                    pending.add(pool.submit(function, item))
    finally:
        for future in pending:
            future.cancel()
        pool.shutdown(wait=True, cancel_futures=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", required=True, type=Path)
    parser.add_argument("--tlapm", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--timeout", type=int, default=900)
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
    require(set(args.inventory["models"]) == {"RawGroup", "RawMutation"}, "model inventory mismatch")
    require(set(args.inventory["modules"]) == {"RawGroupProof", "RawMutationProof"} and args.inventory["roots"] == ["RawGroupProof"], "root inventory mismatch")
    require({p.stem for p in PROOFS.glob("*.tla")} == set(args.inventory["modules"]), "proof source inventory mismatch")
    require({p.stem for p in SOURCE.glob("*.tla")} == set(SCOPES) | {"RawMutation"} | {s["wrapper"] for s in SCOPES.values()}, "model source inventory mismatch")
    require({p.name for p in SOURCE.glob("*.cfg")} == {c for s in SCOPES.values() for c in s["configs"] + s["live"]}, "configuration inventory mismatch")
    bound_sources = {str(p.relative_to(ROOT)): digest(p) for p in [Path(__file__), ROOT / "scripts/check-tla.py",
            ROOT / "scripts/check-tlaps.py", ROOT / "scripts/raw_group_controls.py", ROOT / "scripts/ready_controls.py",
            ROOT / "scripts/ProofAudit.java", *sorted(SOURCE.iterdir()), *sorted(PROOFS.iterdir())] if p.is_file()}
    args.output.mkdir(parents=True)
    snapshot = args.output / "source-snapshot"
    for relative, expected_sha in bound_sources.items():
        target = snapshot / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / relative, target)
        require(digest(target) == expected_sha, "source changed while copying the gate snapshot")
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
        for witness, filename in scope["witnesses"]:
            config = (SOURCE / filename).read_text() + f"INVARIANT {witness}\n"
            record, _ = model_case(args, "witness-" + witness, m, config, models[m], witness)
            model_records.append(record)
    require(set().union(*(set(r.get("covered_actions", ())) for r in model_records)) == set(SCOPES["RawGroup"]["actions"]),
            "the positive corpus does not exercise every service action")
    for c in controls:
        m = c["module"]
        config = (SOURCE / (c["config"] if c["config"].endswith("Live.cfg") else SCOPES[m]["live"][0])).read_text() if c["live"] else (
            "\n".join(line for line in (SOURCE / (c["config"] or SCOPES[m]["configs"][0])).read_text().splitlines()
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
                    m + "Proof: TLAPS exit 10" if invalid else None, c["proof_pattern"] if invalid else None, theorem=c["proof_theorem"] if invalid else None))
            check_triple(records, m + ".tla")
            return records

        for records in bounded_groups(group, controls, args.jobs):
            proof_records.extend(records)
        semantic_jobs = []
        for filename in ("RawMutationProof.tla", "RawGroupProof.tla"):
            m = "RawGroup"
            source = (PROOFS / filename).read_text()
            first = re.search(r"THEOREM (\w+)[^\n]*\n(?:[^\n]*\n){0,3}?(BY [^\n]+)", source)
            require(first is not None, "missing leaf proof control anchor")
            extends = re.search(r"^EXTENDS [^\n]+", source, flags=re.M).group(0)
            faults = [("omitted-proof", tlc.replace_once(source, first.group(0), first.group(0).removesuffix(first.group(2)) + "OMITTED"),
                       f"proof hole: {Path(filename).stem}.{first.group(1)}"),
                      ("custom-axiom", tlc.replace_once(source, extends, extends + "\nAXIOM FALSE"),
                       f"unapproved module assumption: {Path(filename).stem}")]
            semantic_jobs.extend((filename, name, source, mutant, reason) for name, mutant, reason in faults)

        def semantic_group(job):
            filename, name, source, mutant, reason = job
            records = []
            for suffix, text, expected in (("baseline", source, None), ("mutant", mutant, reason), ("restored", source, None)):
                records.append(proof_case(args, f"audit-{Path(filename).stem}-{name}-{suffix}", "RawGroup",
                    models["RawGroup"], sources["RawGroup"], expected, source_edits={filename: text}))
            check_triple(records, filename)
            return records

        for records in bounded_groups(semantic_group, semantic_jobs, args.jobs):
            proof_records.extend(records)
    model_outputs = sum(output_controls(good_outputs[m], m) for m in SCOPES)
    model_outputs += sum(output_controls(good_outputs[m + "Live"], m, temporal=True) for m in SCOPES if SCOPES[m]["live"])
    proof_outputs = 0
    if not args.model_only:
        for root in args.inventory["modules"]:
            output = (args.output / "proof-baseline-RawGroup" / (root + ".log")).read_text()
            proof_outputs += proof.output_controls(output, root, args.inventory["modules"][root]["obligations"])
    require(bound_sources == {name: digest(ROOT / name) for name in bound_sources},
            "protocol source changed during the gate; retained results are not accepted")
    summary = dict(verdict="finite preflight only" if args.model_only else "accepted", affinity=affinity,
        jar_sha256=digest(args.jar), tlapm_version=version, tlapm_sha256=digest(args.tlapm), jobs=args.jobs,
        sources=bound_sources,
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
