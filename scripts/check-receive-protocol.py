#!/usr/bin/env python3
"""Check the replica receive authority model and its audited parameterized TLAPS proofs."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

from receive_controls import mutations

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "proofs/tla/receive"
PROOFS = ROOT / "proofs/tlaps/receive"
MODULES = ("ReceiveAuthority.tla", "ReceiveMC.tla")
CONFIGS = ("Receive2.cfg", "Receive3.cfg")
ACTIONS = ("GRBind", "GRRoute", "GRReply", "GRGrant", "GRStart", "GRReceive", "GRCertify", "GRCrash", "GRRestart", "GRQuiesce")


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


tlc = module("receive_tlc_gate", "check-tla.py")
proof = module("receive_proof_gate", "check-tlaps.py")
require, digest = tlc.require, tlc.digest


def verdict(output, status, expected=None, coverage=False, temporal=False):
    return tlc.verdict(output, status, "EventuallyDrained" if expected == "GRProgress" else expected,
                       temporal=temporal, coverage=coverage,
                       module="ReceiveAuthority", actions=ACTIONS)


def model_case(args, name, config, model, expected=None, fp=0, coverage=False):
    work = args.output / name
    work.mkdir()
    for filename in MODULES:
        shutil.copyfile(SOURCE / filename, work / filename)
    (work / "ReceiveAuthority.tla").write_text(model)
    (work / "Run.cfg").write_text(config)
    record = dict(name=name, expected=expected, fingerprint=fp, temporal="GRFairSpec" in config,
                  sources={p.name: digest(p) for p in sorted(work.iterdir())})
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix="kv9-receive-tlc-states.") as states:
            command = ["java", "-XX:+UseParallelGC", "-Xmx512m", "-cp", str(args.jar),
                       "tlc2.TLC", "-tool", "-workers", "1", "-fp", str(fp),
                       "-metadir", states, "-config", "Run.cfg"]
            if coverage:
                command += ["-coverage", "999"]
            command += ["ReceiveMC.tla"]
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
    print(f"PASS: {name}; {record['statistics']['distinct']} distinct states", flush=True)
    return record, output


def proof_case(args, name, model, source, expected=None, pattern=None):
    work = args.output / name
    work.mkdir()
    (work / "ReceiveAuthority.tla").write_text(model)
    (work / "ReceiveAuthorityProof.tla").write_text(source)
    shutil.copyfile(PROOFS / "inventory.json", work / "inventory.json")
    record = dict(name=name, expected=expected,
                  sources={p.name: digest(p) for p in sorted(work.iterdir())})
    try:
        proof.check_tree(args, work, record)
    except proof.Rejected as error:
        record.update(verdict="rejected", reason=str(error))
        require(expected is not None and str(error) == expected,
                f"{name}: failed outside the intended gate: {error}")
        if pattern:
            output = (work / "ReceiveAuthorityProof.log").read_text()
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
    print(f"PASS: {name}; {record['verdict']}", flush=True)
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
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()
    args.jar, args.tlapm, args.output = args.jar.resolve(), args.tlapm.resolve(), args.output.resolve()
    args.stdlib = args.tlapm.parent.parent / "lib/tlapm/stdlib"
    inventory = json.loads((PROOFS / "inventory.json").read_text())
    require(args.timeout > 0 and not args.output.exists(), "positive timeout and new output directory required")
    require(digest(args.jar) == inventory["sany_jar_sha256"] == tlc.JAR_SHA256, "TLC/SANY hash mismatch")
    version = subprocess.check_output([str(args.tlapm), "--version"], text=True, timeout=15).strip()
    require(version == inventory["tlapm_version"], "TLAPS version mismatch")
    require({p.name for p in SOURCE.glob("*.tla")} == set(MODULES), "receive model inventory mismatch")
    require({p.name for p in SOURCE.glob("*.cfg")} == set(CONFIGS) | {"ReceiveLive.cfg"}, "receive configuration inventory mismatch")
    require({p.stem for p in PROOFS.glob("*.tla")} == set(inventory["modules"]), "receive proof inventory mismatch")
    require(inventory["roots"] == ["ReceiveAuthorityProof"] and set(inventory["models"]) == {"ReceiveAuthority"},
            "receive root/model inventory mismatch")
    args.output.mkdir(parents=True)
    args.classes = args.output / "auditor"
    args.classes.mkdir()
    shutil.copyfile(ROOT / "scripts/ProofAudit.java", args.classes / "ProofAudit.java")
    subprocess.run(["javac", "-cp", str(args.jar), "-d", str(args.classes),
                    str(args.classes / "ProofAudit.java")], check=True, timeout=30)
    model = (SOURCE / "ReceiveAuthority.tla").read_text()
    source = (PROOFS / "ReceiveAuthorityProof.tla").read_text()
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
    for control in controls:
        config = control.get("config", f"SPECIFICATION GRSpec\nCONSTANT GRIncarnations = {{1, 2, 3}}\nINVARIANT {control['property']}\n")
        model_group, proof_group = [], []
        for suffix, text, invalid in (("baseline", model, False), ("mutant", control["model"], True), ("restored", model, False)):
            record, _ = model_case(args, f"model-{control['name']}-{suffix}", config, text,
                                   control["property"] if invalid else None)
            model_group.append(record)
            proof_group.append(proof_case(args, f"proof-{control['name']}-{suffix}", text, source,
                                          "ReceiveAuthorityProof: TLAPS exit 10" if invalid else None,
                                          control["proof_pattern"] if invalid else None))
        check_triple(model_group, "ReceiveAuthority.tla")
        check_triple(proof_group, "ReceiveAuthority.tla")
        models.extend(model_group)
        proofs.extend(proof_group)
    live, live_output = model_case(args, "ReceiveLive", (SOURCE / "ReceiveLive.cfg").read_text(), model)
    models.append(live)
    for witness in ("GRNoReceive", "GRNoRecoveredOwner"):
        config = (SOURCE / CONFIGS[1]).read_text() + f"INVARIANT {witness}\n"
        record, _ = model_case(args, f"witness-{witness}", config, model, witness)
        models.append(record)
    audit_controls = [
        ("omitted-proof", tlc.replace_once(source,
         "BY GRLegalInputs, SMT DEF GRInit, GRInvariant, GRType, GRCredentials,\n   GRPermission, GRRouting, GRSafety",
         "OMITTED"), "proof hole: ReceiveAuthorityProof.GRInvariantInit"),
        ("custom-axiom", tlc.replace_once(source, "EXTENDS ReceiveAuthority, TLAPS",
         "EXTENDS ReceiveAuthority, TLAPS\nAXIOM FALSE"), "unapproved module assumption: ReceiveAuthorityProof"),
    ]
    for name, mutant, reason in audit_controls:
        group = []
        for suffix, text, expected in (("baseline", source, None), ("mutant", mutant, reason), ("restored", source, None)):
            group.append(proof_case(args, f"audit-{name}-{suffix}", model, text, expected))
        check_triple(group, "ReceiveAuthorityProof.tla")
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
    proof_output = (args.output / "proof-baseline/ReceiveAuthorityProof.log").read_text()
    proof_outputs = proof.output_controls(proof_output, "ReceiveAuthorityProof", 117)
    summary = dict(jar_sha256=digest(args.jar), tlapm_version=version, tlapm_sha256=digest(args.tlapm),
                   sources={str(p.relative_to(ROOT)): digest(p) for p in [
                       Path(__file__), ROOT / "scripts/check-tla.py", ROOT / "scripts/check-tlaps.py",
                       ROOT / "scripts/receive_controls.py", ROOT / "scripts/ready_controls.py",
                       ROOT / "scripts/ProofAudit.java", *sorted(SOURCE.iterdir()), *sorted(PROOFS.iterdir())
                   ] if p.is_file()},
                   models=models, proofs=proofs, model_output_controls=model_outputs, proof_output_controls=proof_outputs)
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"PASS: 4 full receive model checks and 1 fair continuation; {len(controls)} protocol controls; 2 witnesses; "
          "13 TLAPS theorems; 117 obligations; 2 audit controls; 8 output controls", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (tlc.Rejected, proof.Rejected, OSError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f"FAIL: {error}") from error
