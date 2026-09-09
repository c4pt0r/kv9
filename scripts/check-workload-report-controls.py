#!/usr/bin/env python3
"""Reject isolated corruptions of real executable reports and their full histories."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

from workload_report import validate


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--e2e", type=Path, required=True)
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = dict(version=1, sources={name: hashlib.sha256((Path(__file__).parent / name).read_bytes()).hexdigest()
        for name in ("workload_report.py", "check-workload-report-controls.py", "history/checker.py")}, controls=[])
    baselines = {}
    for source in ("correctness", "performance-stop", "dead-seed"):
        verdict = validate(args.e2e / source, args.build)
        baselines[source] = verdict["report_sha256"]
    manifest["baselines"] = baselines

    cases = [
        ("missing-terminal", "correctness", "missing serialized history events"),
        ("duplicate-terminal", "correctness", "missing/duplicate terminal record"),
        ("terminal-accounting", "correctness", "summary lacks terminal records"),
        ("phase-population", "correctness", "latency and outcome counts disagree"),
        ("raw-duration", "correctness", "report histogram differs from full history"),
        ("throughput", "correctness", "reported throughput differs from its cohort"),
        ("drain-denominator", "correctness", "throughput denominator excludes drain or includes setup"),
        ("binary-provenance", "correctness", "report configuration/build provenance differs"),
        ("missing-report", "correctness", "report.json"),
        ("unknown-as-refused", "dead-seed", "reason does not justify history outcome"),
        ("sentinel-loss", "correctness", "immutable sentinel was lost"),
        ("performance-history-claim", "performance-stop", "performance-only run claims a complete history"),
    ]
    for name, source, expected in cases:
        folder = args.output / name
        shutil.copytree(args.e2e / source, folder)
        report_path = folder / "report.json"
        report = json.loads(report_path.read_text())
        records = [json.loads(line) for line in (folder / "history.jsonl").read_text().splitlines()] if source != "performance-stop" else None
        if name == "missing-terminal":
            records.pop()
        elif name == "duplicate-terminal":
            returns = [event for event in records if event["type"] == "return"]
            returns[-1]["id"] = returns[-2]["id"]
        elif name == "terminal-accounting":
            report["history"]["terminal"] -= 1
        elif name == "phase-population":
            report["metrics"]["logical_latency"][1][0], report["metrics"]["logical_latency"][2][0] = report["metrics"]["logical_latency"][2][0], report["metrics"]["logical_latency"][1][0]
        elif name == "raw-duration":
            event = next(e for e in records if e["type"] == "return")
            # Stay inside the observed interval and match the attempt duration;
            # only a comparison with retained samples can reject the old bins.
            event["observation"]["elapsed_ns"] = 0
            for attempt in event["observation"]["attempts"]:
                attempt["elapsed_ns"] = 0
        elif name == "throughput":
            report["cohort_terminal_ops_per_second"] *= 2
        elif name == "drain-denominator":
            report["cohort_elapsed_ns"] -= 1
        elif name == "binary-provenance":
            report["build"]["binary_sha256"] = "0" * 64
        elif name == "unknown-as-refused":
            event = next(e for e in records if e.get("outcome") == "unknown")
            event.update(outcome="refused", result={"proof": "precommit"})
        elif name == "sentinel-loss":
            sentinel = f'{report["configuration"]["run_id"]}:{report["configuration"]["keys"]:016x}'.encode().hex()
            identity = next(e["id"] for e in records if e["type"] == "invoke" and e["phase"] == "verify" and e["args"]["key"] == sentinel)
            next(e for e in records if e["type"] == "return" and e["id"] == identity)["result"] = {"value": None}
        elif name == "performance-history-claim":
            report["history"]["full_history_complete"] = True
        if records is not None:
            data = ("".join(json.dumps(record, separators=(",", ":")) + "\n" for record in records)).encode()
            (folder / "history.jsonl").write_bytes(data)
            report["history_sha256"] = hashlib.sha256(data).hexdigest()
            report["history"]["bytes"] = len(data)
        report_path.write_text(json.dumps(report) + "\n")
        if name == "missing-report":
            report_path.unlink()
        try:
            validate(folder, args.build)
        except (ValueError, OSError) as error:
            if expected not in str(error):
                raise RuntimeError(f"{name}: rejected for the wrong reason: {error}") from error
            manifest["controls"].append(dict(name=name, source=source, rejected=True, reason=str(error)))
        else:
            raise RuntimeError(name + ": corrupt evidence was accepted")
        (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        print("PASS: rejected workload evidence control " + name, flush=True)
    for source, expected_hash in baselines.items():
        restored = validate(args.e2e / source, args.build)
        if restored["report_sha256"] != expected_hash:
            raise RuntimeError("control changed its original evidence")
    print("PASS: 12 corrupt workload artifact controls rejected; three original runs revalidated", flush=True)


if __name__ == "__main__":
    main()
