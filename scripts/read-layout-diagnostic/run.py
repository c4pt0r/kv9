#!/usr/bin/env python3
"""Run the declared three-cell layout intervention without repeating the full grid."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time


def load(path):
    return json.loads(path.read_text())


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--experiment", type=Path, required=True)
    parser.add_argument("--qualified", type=Path, required=True)
    args = parser.parse_args()
    root, qualified = args.experiment.resolve(), args.qualified.resolve()
    plan = load(root / "timing-plan.json")
    build = load(root / "build-summary.json")
    ordinary = load(qualified / "build-summary.json")
    assert build["complete"] and ordinary["complete"]
    assert sha(root / "timing-plan.json") == build["plan_sha256"]
    assert load(root / "codegen-comparison.json")["timing_authorized"]
    assert plan["diagnostic_cases"] == [18, 19, 5]
    assert sorted(os.sched_getaffinity(0)) == list(range(6, 16)) + list(range(22, 32))
    for path, digest in build["sources"].items():
        assert sha(Path(path)) == digest
    binaries = {"ordinary": {arm: ordinary["arms"][arm]["timing"] for arm in ("baseline", "candidate")},
                "aligned64": build["arms"]}
    outputs = root / "runs"
    outputs.mkdir(exist_ok=False)
    summary = {"complete": False, "started_ns": time.time_ns(), "processes": [],
               "promotion_acceptance": False, "plan_sha256": build["plan_sha256"]}

    def execute(name, config, arm, operation, case, order):
        binary = binaries[config][arm]
        assert sha(Path(binary["path"])) == binary["sha256"]
        free = {p: shutil.disk_usage(p).free for p in ("/home/dongxu/kv9", "/mnt/data")}
        assert min(free.values()) >= plan["minimum_free_bytes"]
        argv = ["taskset", "-c", str(plan["cpu"]), binary["path"], str(root),
                str(outputs / (name + ".json")), operation, str(case), str(order)]
        row = {"name": name, "config": config, "arm": arm, "operation": operation,
               "case": case, "order": order, "argv": argv, "complete": False,
               "binary_sha256": binary["sha256"], "started_ns": time.time_ns(),
               "free_bytes": free, "shared_host": True, "loadavg": Path("/proc/loadavg").read_text().strip()}
        process = None
        try:
            with (outputs / (name + ".stdout")).open("x") as out, (outputs / (name + ".stderr")).open("x") as err:
                process = subprocess.Popen(argv, stdout=out, stderr=err)
                row["pid"] = process.pid
                row["start_ticks"] = int(Path(f"/proc/{process.pid}/stat").read_text().rsplit(")", 1)[1].split()[19])
                (outputs / (name + "-launch.json")).write_text(json.dumps(row, indent=2) + "\n")
                row["exit_code"] = process.wait(timeout=plan["timeout_seconds_per_process"])
            assert row["exit_code"] == 0, name
            path = outputs / (name + ".json")
            assert path.stat().st_size <= plan["process_output_max_bytes"]
            payload = load(path)
            assert payload["complete"] and payload["input_plan_sha256"] == build["plan_sha256"]
            assert payload["group_file_sha256"] == plan["corpus_sha256"]
            if operation == "measure":
                assert not payload["counting_build"]
                assert [r["phase"] for r in payload["rows"]] == (["group"] if case == 5 else ["first_touch", "warm"])
                for measured in payload["rows"]:
                    assert measured["case"] == case and measured["order"] == order and measured["backend"] == arm
                    assert measured["passes"] == (144 if measured["phase"] == "warm" else 12)
            assert sha(Path(binary["path"])) == binary["sha256"]
            row.update(complete=True, result_sha256=sha(path), result_bytes=path.stat().st_size)
        finally:
            if process is not None and process.poll() is None:
                process.terminate()
                process.wait(timeout=30)
            row["ended_ns"] = time.time_ns()
            (outputs / (name + "-terminal.json")).write_text(json.dumps(row, indent=2) + "\n")
            summary["processes"].append(row)
            (root / "run-progress.json").write_text(json.dumps(summary, indent=2) + "\n")

    try:
        for order, arm in enumerate(("baseline", "candidate")):
            execute("prepare-" + arm, "aligned64", arm, "prepare", 0, order)
        with (outputs / "input-check.stdout").open("x") as out, (outputs / "input-check.stderr").open("x") as err:
            subprocess.run(["python3", "-B", str(root / "validate-inputs.py")], check=True,
                           stdout=out, stderr=err, timeout=120)
        assert load(outputs / "inputs-accepted.json")["complete"]
        for case in plan["diagnostic_cases"]:
            for sequence, (config, arm, order) in enumerate(plan["diagnostic_sequence"]):
                execute(f"case-{case:02d}-{sequence}-{config}-{arm}", config, arm, "measure", case, order)
                print(json.dumps({"case": case, "sequence": sequence, "config": config, "arm": arm,
                                  "complete": True}), flush=True)
        assert len(summary["processes"]) == 26 and all(r["complete"] for r in summary["processes"])
        for path, digest in build["sources"].items():
            assert sha(Path(path)) == digest
        summary["complete"] = True
    except BaseException as error:
        summary["failure"] = repr(error)
        raise
    finally:
        summary["ended_ns"] = time.time_ns()
        (root / "run-summary.json").write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
