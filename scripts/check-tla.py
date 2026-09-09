#!/usr/bin/env python3
"""Check the pinned TLA+ inventory, fault controls and reachability witnesses."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "proofs/tla"
JAR_SHA256 = "936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88"
VERSION = "TLC2 Version 2.19 of 08 August 2024 (rev: 5a47802)"
CONFIGS = ("MetadataDistinct.cfg", "MetadataSame.cfg")
MODULES = ("MetadataPlanning.tla", "MetadataMC.tla")
ACTIONS = ("Begin", "Barrier", "Plan", "Submit", "Finish", "Timeout", "ElectAny", "Commit", "Apply")


class Rejected(Exception):
    pass


def require(condition, message):
    if not condition:
        raise Rejected(message)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def replace_once(source, before, after):
    require(source.count(before) == 1, f"mutation target is not unique: {before!r}")
    require(before != after, "mutation is empty")
    return source.replace(before, after)


def messages(output, code, severity=0):
    return [message.strip() for message in re.findall(
        rf"@!@!@STARTMSG {code}:{severity} @!@!@\n(.*?)\n@!@!@ENDMSG {code} @!@!@",
        output, re.S,
    )]


def verdict(output, code, expected=None, temporal=False, coverage=False,
            action_property=False, module="MetadataPlanning", actions=ACTIONS):
    require(messages(output, 2262) == [VERSION], "missing pinned TLC version")
    require(len(messages(output, 2186)) == 1, "missing TLC completion")
    stats = messages(output, 2199)
    require(len(stats) == 1, "missing or duplicate final statistics")
    match = re.fullmatch(
        r"(\d+) states generated, (\d+) distinct states found, (\d+) states left on queue\.", stats[0]
    )
    require(match is not None, "unrecognized final statistics")
    generated, distinct, queued = map(int, match.groups())
    require(generated >= distinct > 2, "empty or trivial state exploration")
    errors = re.findall(r"@!@!@STARTMSG (\d+):1 @!@!@", output)
    if expected is None:
        require(code == 0, "TLC did not exit successfully")
        require(not errors, "TLC reported an error")
        require(queued == 0, "state exploration is incomplete")
        require(len(messages(output, 2193)) == 1, "missing successful model-check verdict")
        if temporal:
            require(len(messages(output, 2192)) == 1 and len(messages(output, 2267)) == 1,
                    "missing complete temporal check")
        if coverage:
            for action in actions:
                hits = re.findall(
                    rf"^<{action} line [^\n]+ of module {module}>: (\d+):(\d+)$",
                    output, re.M,
                )
                require(len(hits) == 1 and int(hits[0][1]) > 0,
                        f"missing action coverage: {action}")
    elif action_property:
        require(code == 13, "control did not produce an action-property violation")
        require(messages(output, 2112, 1) == [f"Action property {expected} is violated."],
                "wrong action-property violation")
        require(set(errors) <= {"2112", "2121"}, "unrelated error in action-property control")
    elif expected == "EventuallyDrained":
        require(code == 13, "liveness control did not produce a temporal violation")
        require(messages(output, 2116, 1) == ["Temporal properties were violated."],
                "missing intended temporal violation")
        require(set(errors) == {"2116", "2264"}, "unrelated error in temporal control")
        require("Back to state" in output or "Stuttering" in output, "missing liveness cycle")
    else:
        require(code == 12, "control did not produce an invariant violation")
        require(messages(output, 2110, 1) == [f"Invariant {expected} is violated."],
                "wrong invariant violation")
        require(set(errors) <= {"2110", "2121"}, "unrelated error in invariant control")
    if expected is not None:
        require(not messages(output, 2193), "contradictory successful verdict")
        require(len(messages(output, 2217, 4)) >= 2, "missing counterexample states")
    return {"generated": generated, "distinct": distinct, "queued": queued}


def config_for(config, invariant=None, temporal=False):
    if temporal:
        return config
    config = replace_once(config, "SPECIFICATION FairSpec", "SPECIFICATION Spec")
    config = replace_once(config, "PROPERTY EventuallyDrained\n", "")
    return replace_once(config, "INVARIANTS TypeOK CatalogSafety LogSafety FreshPlans ReceiptSafety",
                        f"INVARIANTS {invariant}")


def run_case(args, name, config, model, expected=None, fp=0, temporal=False, coverage=False):
    work = args.output / name
    work.mkdir()
    for module in MODULES:
        shutil.copyfile(SOURCE / module, work / module)
    (work / "MetadataPlanning.tla").write_text(model)
    (work / "Run.cfg").write_text(config)
    source_hashes = {p.name: digest(p) for p in sorted(work.iterdir())}
    started = time.monotonic()
    # Fresh storage prevents stale TLC checkpoints from satisfying this check.
    with tempfile.TemporaryDirectory(prefix="kv9-tlc-states.") as states:
        command = [args.java, "-XX:+UseParallelGC", "-Xmx2g", "-cp", str(args.jar),
                   "tlc2.TLC", "-tool", "-workers", "1", "-fp", str(fp),
                   "-lncheck", "final", "-metadir", states, "-config", "Run.cfg"]
        if coverage:
            command += ["-coverage", "999"]
        command += ["MetadataMC.tla"]
        record = {"name": name, "command": command, "sources": source_hashes,
                  "expected": expected, "fingerprint": fp}
        (work / "result.json").write_text(json.dumps(record, indent=2) + "\n")
        with (work / "tlc.log").open("w") as log:
            try:
                result = subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT,
                                        timeout=args.timeout, env=dict(os.environ, LC_ALL="C"))
            except subprocess.TimeoutExpired:
                record.update(verdict="inconclusive", reason="timeout")
                (work / "result.json").write_text(json.dumps(record, indent=2) + "\n")
                raise Rejected(f"{name}: TLC timed out; no obligation discharged") from None
    output = (work / "tlc.log").read_text()
    record.update(exit_code=result.returncode, seconds=time.monotonic() - started)
    try:
        record.update(verdict="accepted", statistics=verdict(
            output, result.returncode, expected, temporal=temporal, coverage=coverage))
    except Rejected as error:
        record.update(verdict="rejected", reason=str(error))
        raise Rejected(f"{name}: {error}; see {work / 'tlc.log'}") from error
    finally:
        (work / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    print(f"PASS: {name}; {record['statistics']['distinct']} distinct states", flush=True)
    return record, output


def output_controls(output):
    """A zero exit status alone, partial logs and missing temporal work fail closed."""
    cases = [
        ("empty output", "", "missing pinned TLC version"),
        ("truncated completion", re.sub(
            r"@!@!@STARTMSG 2186:0 @!@!@.*?@!@!@ENDMSG 2186 @!@!@", "", output, flags=re.S),
         "missing TLC completion"),
        ("unfinished queue", re.sub(r"(\d+) states left on queue\.", "1 states left on queue.", output),
         "state exploration is incomplete"),
        ("skipped temporal check", re.sub(
            r"@!@!@STARTMSG 2267:0 @!@!@.*?@!@!@ENDMSG 2267 @!@!@", "", output, flags=re.S),
         "missing complete temporal check"),
    ]
    for name, mutant, reason in cases:
        verdict(output, 0, temporal=True, coverage=True)
        try:
            verdict(mutant, 0, temporal=True, coverage=True)
        except Rejected as error:
            require(str(error) == reason, f"output control failed elsewhere: {name}: {error}")
        else:
            raise Rejected(f"output control was accepted: {name}")
        verdict(output, 0, temporal=True, coverage=True)
        print(f"PASS: rejected output control: {name}", flush=True)
    return len(cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jar", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--java", default="java")
    parser.add_argument("--timeout", type=int, default=120, help="seconds per TLC process")
    args = parser.parse_args()
    args.jar = args.jar.resolve()
    args.output = args.output.resolve()
    require(args.timeout > 0, "timeout must be positive")
    require(digest(args.jar) == JAR_SHA256, "TLC jar checksum mismatch")
    require(not args.output.exists(), "output directory must be new")
    require({p.name for p in SOURCE.glob('*.tla')} == set(MODULES), "TLA+ module inventory mismatch")
    require({p.name for p in SOURCE.glob('*.cfg')} == set(CONFIGS), "TLC configuration inventory mismatch")
    args.output.mkdir(parents=True)
    records = []
    model = (SOURCE / "MetadataPlanning.tla").read_text()
    configs = {name: (SOURCE / name).read_text() for name in CONFIGS}
    for name, config in configs.items():
        first = None
        for fp in (0, 1):
            record, output = run_case(args, f"{Path(name).stem}-fp{fp}", config, model,
                                      fp=fp, temporal=True, coverage=True)
            records.append(record)
            if first is None:
                first = record["statistics"]
                good_output = output
            else:
                require(first == record["statistics"], "fingerprint runs disagree on exploration counts")

    # Every control changes one protocol mechanism. The baseline and restored
    # runs use exactly the same property/configuration as that control.
    controls = []
    for name, config in configs.items():
        for mechanism, before, after in (
            ("read-index-only", "/\\ applied[host[r]] >= barrierAt[r]",
             "/\\ applied[host[r]] >= committed\n    /\\ committed > 0\n    /\\ log[committed].epoch = term"),
            ("missing-term-fence", "/\\ planningTerm[r] = term\n", "/\\ TRUE\n"),
        ):
            controls.append((f"{Path(name).stem}-{mechanism}", config_for(config, "CatalogSafety"),
                             replace_once(model, before, after), "CatalogSafety"))
    distinct = configs["MetadataDistinct.cfg"]
    controls.append(("index-only-receipt", config_for(distinct, "ReceiptSafety"),
                     replace_once(model, 'Finish(r) ==\n    /\\ phase[r] = "write"\n'
                                  '    /\\ Exact(writeAt[r], "write", r, writeTerm[r])\n',
                                  'Finish(r) ==\n    /\\ phase[r] = "write"\n'
                                  '    /\\ TRUE\n'), "ReceiptSafety"))
    controls.append(("unfair-apply", distinct, replace_once(
        model, '/\\ (\\A n \\in Nodes : WF_vars(Apply(n)))', '/\\ TRUE'), "EventuallyDrained"))
    for name, config, mutant, expected in controls:
        temporal = expected == "EventuallyDrained"
        for suffix, source, failure in (("baseline", model, None), ("mutant", mutant, expected),
                                        ("restored", model, None)):
            record, _ = run_case(args, f"{name}-{suffix}", config, source, failure, temporal=temporal)
            records.append(record)

    witnesses = ("NoTwoSuccesses", "NoUnknownCommit", "NoReplacedWrite")
    for witness in witnesses:
        config = config_for(distinct, f"TypeOK CatalogSafety LogSafety FreshPlans ReceiptSafety {witness}")
        record, _ = run_case(args, f"witness-{witness}", config, model, witness)
        records.append(record)
    rejected_outputs = output_controls(good_output)
    summary = {"jar_sha256": JAR_SHA256, "version": VERSION,
               "sources": {p.name: digest(p) for p in sorted(SOURCE.iterdir()) if p.is_file()},
               "runner_sha256": digest(Path(__file__)), "runs": records,
               "output_controls": rejected_outputs}
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(f"PASS: 4 full model checks; {len(controls)} isolated controls; "
          f"{len(witnesses)} reachability witnesses; {rejected_outputs} output controls", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (Rejected, OSError, subprocess.SubprocessError) as error:
        raise SystemExit(f"FAIL: {error}") from error
