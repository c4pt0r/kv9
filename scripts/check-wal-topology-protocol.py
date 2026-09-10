#!/usr/bin/env python3
"""Check the WAL topology publication and retention model and its audited parameterized TLAPS proofs."""
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

from wal_topology_controls import mutations

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "proofs/tla/wal_topology"
PROOFS = ROOT / "proofs/tlaps/wal_topology"
MODULES = ("WalTopology.tla", "WalTopologyMC.tla")
CONFIGS = ("Topology2.cfg", "Topology3.cfg")
ACTIONS = ('WTAck', 'WTUnknown', 'WTRotate', 'WTCreate', 'WTCheckpoint', 'WTTemporarySync', 'WTRename', 'WTParentSync', 'WTInstall', 'WTFail', 'WTCrash', 'WTRead', 'WTRestore', 'WTRefuse', 'WTRecoverySync', 'WTReplay', 'WTUnlink', 'WTCleanupSync', 'WTQuiesce')


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


tlc = module("topology_tlc_gate", "check-tla.py")
proof = module("topology_proof_gate", "check-tlaps.py")
require, digest = tlc.require, tlc.digest


class IsolatedProofProcesses:
    """Keep SANY's extracted standard modules inside each case's own JVM tempdir."""
    def __getattr__(self, name):
        return getattr(subprocess, name)

    def run(self, command, *positional, **keywords):
        if command[0] == "java" and "ProofAudit" in command:
            temporary = Path(keywords["cwd"]) / "sany-temp"
            temporary.mkdir(exist_ok=True)
            command = [command[0], f"-Djava.io.tmpdir={temporary}", *command[1:]]
        return subprocess.run(command, *positional, **keywords)


# The shared validator retains its exact semantic and proof checks. Only its
# subprocess adapter supplies the per-case JVM namespace; it is not a global
# monkeypatch and does not alter other concurrently running tools.
proof.subprocess = IsolatedProofProcesses()


def verdict(output, status, expected=None, coverage=False, temporal=False):
    return tlc.verdict(output, status, "EventuallyDrained" if expected in {"WTProgress", "WTReclaimAll"} else expected,
                       temporal=temporal, coverage=coverage,
                       module="WalTopology", actions=ACTIONS,
                       minimum_distinct=2 if expected in {"WTNoAckWitness", "WTNoUnpositionedWitness"} else 3)


def retained_case(args, name, record, allowed):
    """Reuse only exact copied inputs; verdicts and semantic audits run again."""
    if args.resume_from is None:
        return None
    old = args.resume_from / name
    path = old / "result.json"
    if not path.exists():
        return None
    retained = json.loads(path.read_text())
    if retained.get("verdict") not in allowed or retained.get("sources") != record["sources"]:
        return None
    for key in ("expected", "fingerprint", "temporal"):
        require(retained.get(key) == record.get(key), f"{name}: retained case identity differs")
    for filename, sha in retained["sources"].items():
        require(digest(old / filename) == sha, f"{name}: retained input was modified: {filename}")
    return old, retained


def retained_log(record, old, source, destination):
    data = source.read_bytes()
    destination.write_bytes(data)
    provenance = record.setdefault("retained", dict(directory=str(old),
        result_sha256=digest(old / "result.json"), logs={}))
    provenance["logs"][source.name] = digest(source)


def model_case(args, name, config, model, expected=None, fp=0, coverage=False):
    work = args.output / name
    work.mkdir()
    for filename in MODULES:
        shutil.copyfile(SOURCE / filename, work / filename)
    (work / "WalTopology.tla").write_text(model)
    (work / "Run.cfg").write_text(config)
    record = dict(name=name, expected=expected, fingerprint=fp, temporal=any(x in config for x in ("WTMCLiveSpec", "WTCleanupLiveSpec")),
                  sources={p.name: digest(p) for p in sorted(work.iterdir())})
    started = time.monotonic()
    try:
        previous = retained_case(args, name, record, {"accepted"})
        if previous:
            old, retained = previous
            command = retained["command"]
            normalized = [value for value in command if not value.startswith("-Djava.io.tmpdir=")]
            require(len(normalized) > 12 and normalized[11] == "-metadir",
                    f"{name}: retained TLC command differs")
            expected_command = ["java", "-XX:+UseParallelGC", "-Xmx512m", "-cp", str(args.jar),
                                "tlc2.TLC", "-tool", "-workers", "1", "-fp", str(fp),
                                "-metadir", "<states>", "-config", "Run.cfg"]
            # The metadir is deliberately unique per original execution.
            normalized[12] = "<states>"
            if coverage:
                expected_command += ["-coverage", "999"]
            expected_command += ["WalTopologyMC.tla"]
            require(normalized == expected_command, f"{name}: retained TLC command differs")
            retained_log(record, old, old / "tlc.log", work / "tlc.log")
            output = (work / "tlc.log").read_text()
            record.update(command=retained["command"], exit_code=retained["exit_code"],
                          seconds=retained["seconds"], validation_seconds=time.monotonic() - started,
                          statistics=verdict(output, retained["exit_code"], expected, coverage, record["temporal"]),
                          verdict="accepted")
        else:
            with tempfile.TemporaryDirectory(prefix="kv9-wal-topology-tlc-states.") as states:
                command = ["java", f"-Djava.io.tmpdir={states}", "-XX:+UseParallelGC", "-Xmx512m", "-cp", str(args.jar),
                           "tlc2.TLC", "-tool", "-workers", "1", "-fp", str(fp),
                           "-metadir", states, "-config", "Run.cfg"]
                if coverage:
                    command += ["-coverage", "999"]
                command += ["WalTopologyMC.tla"]
                record["command"] = command
                with (work / "tlc.log").open("w") as log:
                    result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                                            timeout=args.timeout, env=dict(os.environ, LC_ALL="C"))
            output = (work / "tlc.log").read_text()
            record.update(exit_code=result.returncode, seconds=time.monotonic() - started,
                          statistics=verdict(output, result.returncode, expected, coverage, record["temporal"]), verdict="accepted")
    except subprocess.TimeoutExpired:
        record.update(verdict="inconclusive", reason="timeout")
        raise tlc.Rejected(f"{name}: TLC timed out") from None
    except tlc.Rejected as error:
        record.update(verdict="rejected", reason=str(error))
        raise tlc.Rejected(f"{name}: {error}; see {work}") from error
    finally:
        (work / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    print(f"PASS: {name}; {record['statistics']['distinct']} distinct states" +
          ("; retained execution revalidated" if "retained" in record else ""), flush=True)
    return record, output


def proof_case(args, name, model, source, expected=None, pattern=None):
    work = args.output / name
    work.mkdir()
    (work / "WalTopology.tla").write_text(model)
    (work / "WalTopologyProof.tla").write_text(source)
    shutil.copyfile(PROOFS / "inventory.json", work / "inventory.json")
    record = dict(name=name, expected=expected,
                  sources={p.name: digest(p) for p in sorted(work.iterdir())})
    try:
        previous = retained_case(args, name, record, {"proved", "expected rejection"})
        if previous and previous[1].get("modules"):
            old, retained = previous
            inventory = json.loads((work / "inventory.json").read_text())
            proof.audit(args, work, inventory)
            record["modules"] = retained["modules"]
            require({r["module"] for r in record["modules"]} == set(inventory["modules"]),
                    f"{name}: retained proof module inventory differs")
            for checked in record["modules"]:
                module = checked["module"]
                require(checked["command"] == [str(args.tlapm), "--strict", "--nofp", "--threads", "1",
                        "--cache-dir", str(old / "cache" / module), str(old / f"{module}.tla")],
                        f"{name}: retained proof command differs")
                retained_log(record, old, old / f"{module}.log", work / f"{module}.log")
                proof.proof_verdict((work / f"{module}.log").read_text(), checked["exit_code"], module,
                                    inventory["modules"][module]["obligations"])
        else:
            proof.check_tree(args, work, record)
    except proof.Rejected as error:
        record.update(verdict="rejected", reason=str(error))
        require(expected is not None and str(error) == expected,
                f"{name}: failed outside the intended gate: {error}")
        if pattern:
            output = (work / "WalTopologyProof.log").read_text()
            require(re.search(pattern, output) is not None, f"{name}: missing intended failed obligation")
            failures = re.findall(r"\[ERROR\]: (\d+)/(\d+) obligations failed\.", output)
            require(len(failures) == 1 and 0 < int(failures[0][0]) < int(failures[0][1]),
                    f"{name}: missing nontrivial failed obligation count")
            require("[WARNING]" not in output, f"{name}: unexpected proof warning")
        record["verdict"] = "expected rejection"
    else:
        require(expected is None, f"{name}: invalid proof accepted")
        record["verdict"] = "proved"
    finally:
        (work / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    print(f"PASS: {name}; {record['verdict']}" +
          ("; retained execution revalidated" if "retained" in record else ""), flush=True)
    return record


def check_triple(records, source):
    before, mutant, restored = records
    require(before["sources"] == restored["sources"], "restored sources differ")
    changed = [name for name, sha in before["sources"].items() if mutant["sources"][name] != sha]
    require(changed == [source], "control changed unrelated source")
    if "statistics" in before:
        require(before["statistics"] == restored["statistics"], "restored exploration differs")


def model_output_controls(output):
    cases = [
        ("empty output", "", "missing pinned TLC version"),
        ("missing completion", re.sub(
            r"@!@!@STARTMSG 2186:0 @!@!@.*?@!@!@ENDMSG 2186 @!@!@", "", output, flags=re.S),
         "missing TLC completion"),
        ("unfinished exploration", re.sub(r"(\d+) states left on queue\.", "1 states left on queue.", output),
         "state exploration is incomplete"),
    ]
    for name, mutant, reason in cases:
        verdict(output, 0, coverage=True)
        try:
            verdict(mutant, 0, coverage=True)
        except tlc.Rejected as error:
            require(str(error) == reason, f"{name}: wrong output rejection")
        else:
            raise tlc.Rejected(f"{name}: invalid model output accepted")
        verdict(output, 0, coverage=True)
    return len(cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", required=True, type=Path)
    parser.add_argument("--tlapm", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--resume-from", type=Path, help="revalidate exact retained case inputs and verdicts; execute missing or rejected cases fresh")
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--jobs", type=int, default=4, help="independent isolated control groups (1-8)")
    args = parser.parse_args()
    args.jar, args.tlapm, args.output = args.jar.resolve(), args.tlapm.resolve(), args.output.resolve()
    args.resume_from = args.resume_from.resolve() if args.resume_from else None
    require(args.resume_from is None or (args.resume_from.is_dir() and args.resume_from != args.output),
            "resume source must be a separate existing directory")
    args.stdlib = args.tlapm.parent.parent / "lib/tlapm/stdlib"
    inventory = json.loads((PROOFS / "inventory.json").read_text())
    require(args.timeout > 0 and not args.output.exists(), "positive timeout and new output directory required")
    require(1 <= args.jobs <= 8, "jobs must be between one and eight")
    require(digest(args.jar) == inventory["sany_jar_sha256"] == tlc.JAR_SHA256, "TLC/SANY hash mismatch")
    version = subprocess.check_output([str(args.tlapm), "--version"], text=True, timeout=15).strip()
    require(version == inventory["tlapm_version"], "TLAPS version mismatch")
    require({p.name for p in SOURCE.glob("*.tla")} == set(MODULES), "topology model inventory mismatch")
    require({p.name for p in SOURCE.glob("*.cfg")} == set(CONFIGS) | {"TopologyLive.cfg", "TopologyCleanup.cfg"}, "topology configuration inventory mismatch")
    require({p.stem for p in PROOFS.glob("*.tla")} == set(inventory["modules"]), "topology proof inventory mismatch")
    require(inventory["roots"] == ["WalTopologyProof"] and set(inventory["models"]) == {"WalTopology"},
            "topology root/model inventory mismatch")
    args.output.mkdir(parents=True)
    args.classes = args.output / "auditor"
    args.classes.mkdir()
    shutil.copyfile(ROOT / "scripts/ProofAudit.java", args.classes / "ProofAudit.java")
    subprocess.run(["javac", "-cp", str(args.jar), "-d", str(args.classes),
                    str(args.classes / "ProofAudit.java")], check=True, timeout=30)
    model = (SOURCE / "WalTopology.tla").read_text()
    source = (PROOFS / "WalTopologyProof.tla").read_text()
    models, proofs = [], []
    for filename in CONFIGS:
        config = (SOURCE / filename).read_text()
        pair = []
        for fp in (0, 1):
            record, good_output = model_case(args, f"{Path(filename).stem}-fp{fp}", config, model, fp=fp, coverage=True)
            pair.append(record)
        require(pair[0]["statistics"] == pair[1]["statistics"], "fingerprint exploration differs")
        models.extend(pair)
    proofs.append(proof_case(args, "proof-baseline", model, source))
    controls = mutations(model)
    def control_group(control):
        config = (SOURCE / control["config"]).read_text() if control.get("live") else (
            (SOURCE / (CONFIGS[1] if control["name"] == "lose-unpositioned-pin" else CONFIGS[0])).read_text().split("INVARIANT", 1)[0] + f"INVARIANT {control['property']}\n")
        model_group, proof_group = [], []
        for suffix, text, invalid in (("baseline", model, False), ("mutant", control["model"], True), ("restored", model, False)):
            record, _ = model_case(args, f"model-{control['name']}-{suffix}", config, text,
                                   control["property"] if invalid else None)
            model_group.append(record)
            proof_group.append(proof_case(args, f"proof-{control['name']}-{suffix}", text, source,
                                          "WalTopologyProof: TLAPS exit 10" if invalid else None,
                                          control["proof_pattern"] if invalid else None))
        check_triple(model_group, "WalTopology.tla")
        check_triple(proof_group, "WalTopology.tla")
        return model_group, proof_group
    # Groups share only immutable sources/tools. Every subprocess retains its
    # own copied tree, fresh cache, log and verdict; phases within a control
    # remain baseline -> mutant -> restored. Parallelism changes no gate.
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(control_group, control) for control in controls]
        for future in futures:
            model_group, proof_group = future.result()
            models.extend(model_group)
            proofs.extend(proof_group)
    live, live_output = model_case(args, "TopologyLive", (SOURCE / "TopologyLive.cfg").read_text(), model)
    models.append(live)
    cleanup, _ = model_case(args, "TopologyCleanup", (SOURCE / "TopologyCleanup.cfg").read_text(), model)
    models.append(cleanup)
    for witness in ("WTNoAckWitness", "WTNoAmbiguityWitness", "WTNoReclaimWitness", "WTNoOrphanWitness", "WTNoUnpositionedWitness"):
        config = (SOURCE / CONFIGS[1]).read_text() + f"INVARIANT {witness}\n"
        record, _ = model_case(args, f"witness-{witness}", config, model, witness)
        models.append(record)
    audit_controls = [
        ("omitted-proof", tlc.replace_once(source,
         "BY WTLegalInputs, SMT DEF WTInit, WTInitialTopology, WTInvariant, WTType, WTLayouts, WTAckRecoverable, WTCandidateSafe, WTAuthority, WTPlanAuthority, WTStage, WTValid, WTAvailable, WTContents, WTSelected, WTTopologies, WTPhases, WTSegments",
         "OMITTED"), "proof hole: WalTopologyProof.WTInvariantInit"),
        ("custom-axiom", tlc.replace_once(source, "EXTENDS WalTopology, TLAPS",
         "EXTENDS WalTopology, TLAPS\nAXIOM FALSE"), "unapproved module assumption: WalTopologyProof"),
    ]
    for name, mutant, reason in audit_controls:
        group = []
        for suffix, text, expected in (("baseline", source, None), ("mutant", mutant, reason), ("restored", source, None)):
            group.append(proof_case(args, f"audit-{name}-{suffix}", model, text, expected))
        check_triple(group, "WalTopologyProof.tla")
        proofs.extend(group)
    model_outputs = model_output_controls(good_output)
    for code in (2192, 2267):
        invalid = re.sub(rf"@!@!@STARTMSG {code}:0 @!@!@.*?@!@!@ENDMSG {code} @!@!@", "", live_output, flags=re.S)
        require(invalid != live_output, "missing temporal output control anchor")
        verdict(live_output, 0, temporal=True)
        try:
            verdict(invalid, 0, temporal=True)
        except tlc.Rejected as error:
            require(str(error) == "missing complete temporal check", "wrong temporal output rejection")
        else:
            raise tlc.Rejected("incomplete temporal output accepted")
        verdict(live_output, 0, temporal=True)
        model_outputs += 1
    proof_output = (args.output / "proof-baseline/WalTopologyProof.log").read_text()
    proof_outputs = proof.output_controls(proof_output, "WalTopologyProof", inventory["modules"]["WalTopologyProof"]["obligations"])
    summary = dict(jar_sha256=digest(args.jar), tlapm_version=version, tlapm_sha256=digest(args.tlapm), jobs=args.jobs,
                   sources={str(p.relative_to(ROOT)): digest(p) for p in [
                       Path(__file__), ROOT / "scripts/check-tla.py", ROOT / "scripts/check-tlaps.py",
                       ROOT / "scripts/wal_topology_controls.py", ROOT / "scripts/ready_controls.py",
                       ROOT / "scripts/ProofAudit.java", *sorted(SOURCE.iterdir()), *sorted(PROOFS.iterdir())
                   ] if p.is_file()},
                   models=models, proofs=proofs, model_output_controls=model_outputs, proof_output_controls=proof_outputs,
                   resume_from=str(args.resume_from) if args.resume_from else None,
                   retained_cases=sum("retained" in r for r in models + proofs))
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    count = len(inventory["modules"]["WalTopologyProof"]["theorems"])
    obligations = inventory["modules"]["WalTopologyProof"]["obligations"]
    print(f"PASS: 4 full topology model checks and 2 fair continuations; {len(controls)} protocol controls; "
          f"5 witnesses; {count} TLAPS declarations; {obligations} obligations; 2 audit controls; "
          f"{model_outputs + proof_outputs} output controls", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (tlc.Rejected, proof.Rejected, OSError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f"FAIL: {error}") from error
