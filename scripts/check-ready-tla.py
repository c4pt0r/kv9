#!/usr/bin/env python3
"""Check Ready publication, crash cuts and fail-stop controls with the pinned TLC."""

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

from ready_controls import mutations

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "proofs/tla/ready"
MODULES = ("ReadyPublication.tla", "ReadyMC.tla")
CONFIGS = ("Ready2.cfg", "Ready3.cfg")
ACTIONS = ("RPCollect", "RPPersistEntries", "RPPersistHardState", "RPAdvance",
           "RPPersistLight", "RPPublish", "RPApply", "RPAcknowledge",
           "RPFailBefore", "RPFailAfter", "RPCrash")
spec = importlib.util.spec_from_file_location("tlc_gate", ROOT / "scripts/check-tla.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
require, digest = gate.require, gate.digest


def verdict(output, status, expected=None, coverage=False):
    return gate.verdict(output, status, expected, coverage=coverage,
                        action_property=expected == "RPFatalFreeze",
                        module="ReadyPublication", actions=ACTIONS)


def run_case(args, name, config, model, expected=None, fp=0, coverage=False):
    work = args.output / name
    work.mkdir()
    for module in MODULES:
        shutil.copyfile(SOURCE / module, work / module)
    (work / "ReadyPublication.tla").write_text(model)
    (work / "Run.cfg").write_text(config)
    record = {"name": name, "expected": expected, "fingerprint": fp,
              "sources": {p.name: digest(p) for p in sorted(work.iterdir())}}
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="kv9-ready-tlc-states.") as states:
        command = ["java", "-XX:+UseParallelGC", "-Xmx1g", "-cp", str(args.jar),
                   "tlc2.TLC", "-tool", "-workers", "1", "-fp", str(fp),
                   "-lncheck", "final", "-metadir", states, "-config", "Run.cfg"]
        if coverage:
            command += ["-coverage", "999"]
        command += ["ReadyMC.tla"]
        record["command"] = command
        try:
            with (work / "tlc.log").open("w") as log:
                result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                                        timeout=args.timeout, env=dict(os.environ, LC_ALL="C"))
            output = (work / "tlc.log").read_text()
            record.update(exit_code=result.returncode, seconds=time.monotonic() - started)
            record["statistics"] = verdict(output, result.returncode, expected, coverage)
            record["verdict"] = "accepted"
        except subprocess.TimeoutExpired:
            record.update(verdict="inconclusive", reason="timeout")
            raise gate.Rejected(f"{name}: TLC timed out; no obligation discharged") from None
        except gate.Rejected as error:
            record.update(verdict="rejected", reason=str(error))
            raise gate.Rejected(f"{name}: {error}; see {work / 'tlc.log'}") from error
        finally:
            (work / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    print(f"PASS: {name}; {record['statistics']['distinct']} distinct states", flush=True)
    return record, output


def output_controls(output):
    cases = [
        ("empty output", "", "missing pinned TLC version"),
        ("truncated completion", re.sub(
            r"@!@!@STARTMSG 2186:0 @!@!@.*?@!@!@ENDMSG 2186 @!@!@", "", output, flags=re.S),
         "missing TLC completion"),
        ("unfinished queue", re.sub(r"(\d+) states left on queue\.", "1 states left on queue.", output),
         "state exploration is incomplete"),
    ]
    for name, mutant, reason in cases:
        verdict(output, 0, coverage=True)
        try:
            verdict(mutant, 0, coverage=True)
        except gate.Rejected as error:
            require(str(error) == reason, f"output control failed elsewhere: {name}")
        else:
            raise gate.Rejected(f"invalid output was accepted: {name}")
        verdict(output, 0, coverage=True)
        print(f"PASS: rejected Ready output control: {name}", flush=True)
    return len(cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()
    args.jar, args.output = args.jar.resolve(), args.output.resolve()
    require(args.timeout > 0, "timeout must be positive")
    require(digest(args.jar) == gate.JAR_SHA256, "TLC jar checksum mismatch")
    require(not args.output.exists(), "output directory must be new")
    require({p.name for p in SOURCE.glob("*.tla")} == set(MODULES), "Ready module inventory mismatch")
    require({p.name for p in SOURCE.glob("*.cfg")} == set(CONFIGS), "Ready configuration inventory mismatch")
    args.output.mkdir(parents=True)
    model = (SOURCE / "ReadyPublication.tla").read_text()
    records = []
    for name in CONFIGS:
        config = (SOURCE / name).read_text()
        first = None
        for fp in (0, 1):
            record, output = run_case(args, f"{Path(name).stem}-fp{fp}", config, model,
                                      fp=fp, coverage=True)
            records.append(record)
            if first is None:
                first, good_output = record["statistics"], output
            else:
                require(first == record["statistics"], "Ready fingerprint exploration counts differ")
    controls = mutations(model)
    for control in controls:
        kind = "PROPERTY" if control["property"] == "RPFatalFreeze" else "INVARIANT"
        config = f"SPECIFICATION RPSpec\nCONSTANT RPMaxIndex = 2\n{kind} {control['property']}\n"
        group = []
        for suffix, source, failure in (("baseline", model, None),
                                        ("mutant", control["model"], control["property"]),
                                        ("restored", model, None)):
            record, _ = run_case(args, f"{control['name']}-{suffix}", config, source, failure)
            group.append(record)
        require(group[0]["sources"] == group[2]["sources"], "restored Ready source differs")
        require(group[0]["statistics"] == group[2]["statistics"], "restored Ready exploration differs")
        changed = [f for f, h in group[0]["sources"].items() if group[1]["sources"][f] != h]
        require(changed == ["ReadyPublication.tla"], "Ready control changed an unrelated source")
        records.extend(group)
    witnesses = ("RPNoLateCommit", "RPNoAckedRestart")
    for witness in witnesses:
        config = (SOURCE / CONFIGS[0]).read_text() + f"INVARIANT {witness}\n"
        record, _ = run_case(args, f"witness-{witness}", config, model, witness)
        records.append(record)
    count = output_controls(good_output)
    summary = {"jar_sha256": gate.JAR_SHA256, "version": gate.VERSION,
               "runner_sha256": digest(Path(__file__)),
               "shared_checker_sha256": digest(ROOT / "scripts/check-tla.py"),
               "controls_sha256": digest(ROOT / "scripts/ready_controls.py"),
               "sources": {p.name: digest(p) for p in sorted(SOURCE.iterdir()) if p.is_file()},
               "runs": records, "output_controls": count}
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"PASS: 4 full Ready model checks; {len(controls)} isolated controls; "
          f"{len(witnesses)} reachability witnesses; {count} output controls", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (gate.Rejected, OSError, ValueError, subprocess.SubprocessError) as error:
        raise SystemExit(f"FAIL: {error}") from error
