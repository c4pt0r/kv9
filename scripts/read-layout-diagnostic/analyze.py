#!/usr/bin/env python3
"""Independently validate the layout experiment and retain every sample."""
import argparse
import hashlib
import importlib.util
import json
import math
from pathlib import Path


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def percentile(values, quantile):
    return sorted(values)[math.ceil(len(values) * quantile / 100) - 1]


def main():
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--experiment", type=Path, required=True)
    args = parser.parse_args()
    root = args.experiment.resolve()
    inputs_used = {}

    def read(path):
        data = path.read_bytes()
        inputs_used[str(path)] = {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}
        return json.loads(data)

    plan = read(root / "timing-plan.json")
    build = read(root / "build-summary.json")
    run = read(root / "run-summary.json")
    assert build["complete"] and run["complete"] and len(run["processes"]) == 26
    assert run["plan_sha256"] == build["plan_sha256"] == sha(root / "timing-plan.json")
    assert all(p["complete"] and p["exit_code"] == 0 for p in run["processes"])
    binding_file = root / "retained-tool-bindings.json"
    bindings = read(binding_file)["bindings"] if binding_file.exists() else {}
    for path, digest in build["sources"].items():
        original = Path(path)
        if path in bindings:
            binding = bindings[path]
            assert original.name == "build.py" and original.parent.name == "read-layout-diagnostic"
            retained = Path(binding["path"])
            assert retained.resolve().is_relative_to(root / "original-tools")
            assert sha(retained) == binding["sha256"] == digest
        else:
            assert sha(original) == digest
    for arm in ("baseline", "candidate"):
        cache = read(root / ("build-" + arm) / "cache-safety.json")
        sources = read(root / ("build-" + arm) / "sources.json")
        assert cache["complete"] and cache["invalidation_complete"]
        assert cache["source_identity_sha256"] == hashlib.sha256(
            json.dumps(sources, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    binaries = {}
    for config in plan["configurations"]:
        for arm in ("baseline", "candidate"):
            inspection = read(root / f"codegen-{config}-{arm}/result.json")
            assert inspection["complete"] and sha(Path(inspection["binary"])) == inspection["sha256"]
            binaries[config, arm] = inspection
    spec = importlib.util.spec_from_file_location("inputs", root / "validate-inputs.py")
    inputs = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(inputs)
    models, metadata = inputs.reconstruct()
    accepted = read(root / "runs/inputs-accepted.json")
    assert accepted["complete"] and accepted["independent_reconstruction"] and accepted["models"] == metadata
    for arm in ("baseline", "candidate"):
        path = root / "runs" / ("prepare-" + arm + ".json")
        prepared = read(path)
        assert accepted["prepared_sha256"][arm] == sha(path)
        assert prepared["workloads"] == metadata and prepared["correctness_live_prefixes"] == 1908
        assert prepared["correctness_old_views"] == 954
    expected = [("prepare-" + arm, "aligned64", arm, "prepare", 0, order)
                for order, arm in enumerate(("baseline", "candidate"))]
    expected += [(f"case-{case:02d}-{sequence}-{config}-{arm}", config, arm, "measure", case, order)
                 for case in plan["diagnostic_cases"]
                 for sequence, (config, arm, order) in enumerate(plan["diagnostic_sequence"])]
    measured = []
    for recorded, (name, config, arm, operation, case, order) in zip(run["processes"], expected):
        terminal = read(root / "runs" / (name + "-terminal.json"))
        assert terminal == recorded and terminal["complete"] and terminal["exit_code"] == 0
        pin = binaries[config, arm]
        path = root / "runs" / (name + ".json")
        assert terminal["result_sha256"] == sha(path) and terminal["binary_sha256"] == pin["sha256"]
        assert terminal["argv"] == ["taskset", "-c", str(plan["cpu"]), pin["binary"], str(root),
                                    str(path), operation, str(case), str(order)]
        payload = read(path)
        assert payload["complete"] and payload["input_plan_sha256"] == build["plan_sha256"]
        assert payload["group_file_sha256"] == plan["corpus_sha256"] and payload["workloads"] == metadata
        if operation == "prepare":
            continue
        assert not payload["counting_build"]
        assert [r["phase"] for r in payload["rows"]] == (["group"] if case == 5 else ["first_touch", "warm"])
        for row in payload["rows"]:
            assert row["case"] == case and row["order"] == order and row["backend"] == arm
            assert row["dataset"] == "original24"
            metrics = row["metrics"]
            samples = metrics["samples_ns"]
            passes = 144 if row["phase"] == "warm" else 12
            assert row["passes"] == passes
            assert len(samples) == metrics["windows"] == passes * (106 if case == 5 else 512)
            assert all(type(value) is int and value > 0 for value in samples)
            assert metrics["sum_ns"] == sum(samples) and metrics["mean_ns"] == sum(samples) / len(samples)
            for quantile in (50, 95, 99):
                assert metrics[f"p{quantile}_ns"] == percentile(samples, quantile)
            if case == 5:
                assert row["operation"] == "write" and row["workload"] == "unique_insert" and row["pinned"]
                assert row["mutations"] == 100096 * passes
                model = models[row["dataset"], row["workload"]]
                assert row["final_state_sha256"] == inputs.digest(model) and row["final_keys"] == len(model)
            else:
                assert row["operation"] == ("get_hit" if case == 18 else "get_miss")
                assert row["workload"] == "overwrite" and not row["pinned"]
                assert row["epochs"] == 12 and row["warmup_passes"] == 8
                assert row["passes_per_epoch"] == (12 if row["phase"] == "warm" else 1)
                assert row["queries"] == 512 and row["all_query_outputs_checked"]
                query = next(q for q in accepted["query_identities"]
                             if (q["dataset"], q["workload"], q["operation"]) ==
                             (row["dataset"], row["workload"], row["operation"]))
                assert row["query_sha256"] == query["query_sha256"]
            measured.append(dict(row, config=config))
    assert len(measured) == 40
    cells = []
    for case in plan["diagnostic_cases"]:
        for phase in (["group"] if case == 5 else ["first_touch", "warm"]):
            for config in plan["configurations"]:
                rows = [r for r in measured if (r["case"], r["phase"], r["config"]) == (case, phase, config)]
                assert len(rows) == 4 and sorted(r["order"] for r in rows) == [0, 1, 2, 3]
                pooled = {}
                for arm in ("baseline", "candidate"):
                    samples = [n for r in rows if r["backend"] == arm for n in r["metrics"]["samples_ns"]]
                    pooled[arm] = {"samples": len(samples), "mean_ns": sum(samples) / len(samples),
                                   "p99_ns": percentile(samples, 99)}
                orders = []
                for old, new in ((0, 1), (3, 2)):
                    base = next(r["metrics"] for r in rows if r["order"] == old)
                    candidate = next(r["metrics"] for r in rows if r["order"] == new)
                    orders.append({"baseline_order": old, "candidate_order": new,
                                   "baseline_mean_ns": base["mean_ns"], "candidate_mean_ns": candidate["mean_ns"],
                                   "baseline_p99_ns": base["p99_ns"], "candidate_p99_ns": candidate["p99_ns"],
                                   "mean_change_percent": (candidate["mean_ns"] / base["mean_ns"] - 1) * 100,
                                   "p99_change_percent": (candidate["p99_ns"] / base["p99_ns"] - 1) * 100})
                cells.append({"case": case, "phase": phase, "config": config, "pooled": pooled, "orders": orders,
                              "mean_change_percent": (pooled["candidate"]["mean_ns"] / pooled["baseline"]["mean_ns"] - 1) * 100,
                              "p99_change_percent": (pooled["candidate"]["p99_ns"] / pooled["baseline"]["p99_ns"] - 1) * 100})
    hit = next(c for c in cells if (c["case"], c["phase"], c["config"]) == (18, "warm", "ordinary"))
    result = {"complete": True, "terminal_processes": 26, "rows": 40, "comparison_cells": 10,
              "new_preparation_prefix_states": 3816, "new_preparation_old_views": 1908,
              "ordinary_warm_hit_regression_reproduced": all(o["mean_change_percent"] > 5 for o in hit["orders"]),
              "cells": cells, "production_changed": False, "new_database_qps": False,
              "original_selection_gate_unchanged": True, "promotion_acceptance": False,
              "scope": "All samples retained. A global alignment intervention is not a single-function/cache-cause proof or a production optimization. No allocation-count or profiler-overhead claim is added."}
    (root / "analysis.json").write_text(json.dumps(result, indent=2) + "\n")
    (root / "analysis-inputs.json").write_text(json.dumps(inputs_used, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
