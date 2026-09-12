#!/usr/bin/env python3
"""Check the fixed-configuration lease transition proof and bounded fault model."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
MODEL = ROOT / "proofs/tla/leader_lease"
PROOF = ROOT / "proofs/tlaps/lease_authority"
MODULE = "LeaseAuthorityProof"


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


proof = load("lease_proof_gate", ROOT / "scripts/check-tlaps.py")
tlc = load("lease_tlc_gate", ROOT / "scripts/check-tla.py")
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
    inventory = json.loads((PROOF / "inventory.json").read_text())
    require(proof.sha(args.tlapm) == inventory["tlapm_sha256"], "TLAPM digest mismatch")
    require(proof.sha(args.jar) == inventory["sany_jar_sha256"], "TLC/SANY digest mismatch")
    version = subprocess.check_output([str(args.tlapm), "--version"], text=True, timeout=15).strip()
    require(version == inventory["tlapm_version"], "TLAPM version mismatch")
    require(not args.output.exists(), "output must be new")
    args.output.mkdir(parents=True)
    args.classes = args.output / "auditor"
    args.classes.mkdir()
    shutil.copyfile(ROOT / "scripts/ProofAudit.java", args.classes / "ProofAudit.java")
    subprocess.run(["javac", "-cp", str(args.jar), "-d", str(args.classes),
                    str(args.classes / "ProofAudit.java")], check=True, timeout=30)
    model = (MODEL / "LeaseAuthority.tla").read_text()
    theorem = (PROOF / f"{MODULE}.tla").read_text()
    wrapper = (MODEL / "LeaseAuthorityMC.tla").read_text()
    config = (MODEL / "Lease3Symmetric.cfg").read_text()
    records = []

    def proof_case(name, source, expected=None):
        work = args.output / name
        work.mkdir()
        (work / "LeaseAuthority.tla").write_text(model)
        (work / f"{MODULE}.tla").write_text(source)
        save(work / "inventory.json", inventory)
        record = {"name": name, "expected": expected,
                  "sources": {p.name: proof.sha(p) for p in work.iterdir()}}
        try:
            proof.check_tree(args, work, record)
        except proof.Rejected as error:
            record.update(verdict="rejected", reason=str(error))
            require(expected is not None and str(error) == expected, f"{name}: {error}")
            record["verdict"] = "expected rejection"
        else:
            require(expected is None, "invalid proof accepted")
            record["verdict"] = "proved"
        finally:
            save(work / "result.json", record)
            records.append(record)
        print(f"PASS: {name}: {record['verdict']}", flush=True)

    def model_case(name, source, cfg, expected=None, renewal=False):
        work = args.output / name
        work.mkdir()
        (work / "LeaseAuthority.tla").write_text(source)
        (work / "LeaseAuthorityMC.tla").write_text(wrapper)
        if renewal:
            shutil.copyfile(MODEL / "LeaseAuthorityRenewal.tla", work / "LeaseAuthorityRenewal.tla")
        (work / "Run.cfg").write_text(cfg)
        command = ["java", "-XX:+UseParallelGC", "-Xmx512m", "-cp", str(args.jar),
                   "tlc2.TLC", "-tool", "-workers", "1", "-fp", "0",
                   "-metadir", str(work / "states"), "-config", "Run.cfg",
                   "LeaseAuthorityRenewal.tla" if renewal else "LeaseAuthorityMC.tla"]
        record = {"name": name, "expected": expected, "command": command,
                  "sources": {p.name: proof.sha(p) for p in work.iterdir()}}
        save(work / "result.json", record)
        started = time.monotonic()
        try:
            with (work / "tlc.log").open("w") as log:
                result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                                        timeout=args.timeout, env=dict(os.environ, LC_ALL="C"))
            record.update(exit_code=result.returncode, seconds=time.monotonic() - started)
            record["statistics"] = tlc.verdict((work / "tlc.log").read_text(), result.returncode,
                                               expected, minimum_distinct=3 if expected is None else 2)
            record["verdict"] = "exhausted" if expected is None else "expected counterexample"
        except subprocess.TimeoutExpired:
            record.update(verdict="inconclusive", reason="180-second timeout")
            raise proof.Rejected(f"{name}: timeout; exploration is incomplete") from None
        except tlc.Rejected as error:
            record.update(verdict="rejected", reason=str(error))
            raise proof.Rejected(f"{name}: {error}") from error
        finally:
            save(work / "result.json", record)
            records.append(record)
        print(f"PASS: {name}: {record['verdict']}; {record['statistics']}", flush=True)

    proof_case("proof-baseline", theorem)
    proof_case("omitted-proof", proof.replace_once(theorem,
        "BY LAInvariantInit, LAInvariantStep, PTL DEF LASpec", "PROOF OMITTED"),
        f"proof hole: {MODULE}.LAInvariantAlways")
    proof_case("false-assumption", proof.replace_once(theorem, "EXTENDS LeaseAuthority, TLAPS",
        "EXTENDS LeaseAuthority, TLAPS\nASSUME FALSE"), f"unapproved module assumption: {MODULE}")
    model_case("full-three-voters", model, config)
    controls = [
        ("lost-restart-quarantine", "laNow + LARecoveryWindow", "laNow", "LAPromiseProtection"),
        ("grant-after-higher-vote", "q \\notin laHigherVotes /\\ laNow >= laRecovery[q]",
         "TRUE /\\ laNow >= laRecovery[q]", "LAVoteHistory"),
        ("vote-during-promise", "laNow >= laHold[q] /\\ laNow >= laRecovery[q]",
         "TRUE /\\ laNow >= laRecovery[q]", "LAVoteHistory"),
        ("publish-without-quorum", "LALiveRound(r) /\\ LAHasQuorum(laDelivered[r])",
         "LALiveRound(r) /\\ TRUE", "LAAuthority"),
        ("stale-read-frontier", "laReadTarget' = [laReadTarget EXCEPT ![r] = laKnown]",
         "laReadTarget' = [laReadTarget EXCEPT ![r] = laApplied]", "LAReadCoverage"),
        ("read-before-apply", 'laReadPhase[r] = "wait" /\\ laApplied >= laReadTarget[r]',
         'laReadPhase[r] = "wait" /\\ TRUE', "LAReadCoverage"),
    ]
    for name, old, new, invariant in controls:
        model_case(name, proof.replace_once(model, old, new),
                   proof.replace_once(config, "INVARIANT LAInvariant", f"INVARIANT {invariant}"), invariant)
    # Reachability witnesses are deliberate violations of a negated event,
    # not safety failures or exhaustive acceptance of the larger round domain.
    witnesses = ["LANoSuccessfulReadWitness", "LANoReplacementWriteWitness",
                 "LANoExpiredCertificateWitness", "LANoRestartPromiseWitness",
                 "LANoApplyGapWitness", "LANoOldGenerationWitness", "LANoRenewalWitness"]
    for witness in witnesses:
        cfg = proof.replace_once(config, "INVARIANT LAInvariant", f"INVARIANTS LAInvariant {witness}")
        if witness == "LANoRenewalWitness":
            cfg = proof.replace_once(cfg, "LARounds = {1}", "LARounds = {1, 2}")
            cfg = proof.replace_once(cfg, "SPECIFICATION LAMCSpec", "SPECIFICATION LARenewalSpec")
            # The witness distinguishes a follower; do not permute it away.
            cfg = proof.replace_once(cfg, "SYMMETRY LASymmetry", "CONSTANT LAFollower = n2")
        model_case(witness, model, cfg, witness, renewal=witness == "LANoRenewalWitness")
    proof_case("proof-restored", theorem)
    require(records[0]["sources"] == records[-1]["sources"], "restoration differs")
    count = inventory["modules"][MODULE]["obligations"]
    output_controls = proof.output_controls(
        (args.output / "proof-baseline" / f"{MODULE}.log").read_text(), MODULE, count)
    save(args.output / "summary.json", {
        "scope": "Fixed-configuration lease transition safety; no Rust/clock refinement or E2E acceptance",
        "theorems": len(inventory["modules"][MODULE]["theorems"]), "obligations": count,
        "proof_integrity_controls": 2, "output_controls": output_controls,
        "model_fault_controls": len(controls), "reachability_witnesses": len(witnesses),
        "records": records, "cpu_affinity": sorted(os.sched_getaffinity(0)),
        "tlapm_version": version, "tlapm_sha256": proof.sha(args.tlapm),
        "jar_sha256": proof.sha(args.jar),
        "sources": {str(p.relative_to(ROOT)): proof.sha(p) for p in [Path(__file__),
            ROOT / "scripts/check-tlaps.py", ROOT / "scripts/check-tla.py", ROOT / "scripts/ProofAudit.java",
            *sorted(MODEL.iterdir()), *sorted(PROOF.iterdir())] if p.is_file()},
    })
    print("PASS: lease transition proof, bounded model, fault controls and witnesses", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (proof.Rejected, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        raise SystemExit(f"FAIL: {error}") from error
