"""Derive event-count ratios from completed, same-process metric snapshots.

These are whole-capture event ratios, not measured QPS, group-size histograms,
or exclusive CPU/time attribution. WAL write events can include segment headers.
"""

import argparse
import hashlib
import json
from pathlib import Path


METRICS = (
    "raft_command_apply",
    "engine_wal_record_write",
    "engine_wal_record_sync",
    "raft_wal_record_write",
    "raft_wal_record_sync",
)


def success(snapshot, name):
    entries = [m for m in snapshot["metrics"] if m["name"] == name]
    assert len(entries) == 1, (name, "missing or duplicate metric")
    latency = entries[0]["latency"]
    assert latency["valid"]
    outcomes = latency["outcomes"]
    assert len({o["outcome"] for o in outcomes}) == len(outcomes)
    rows = [o for o in outcomes if o["outcome"] == "success"]
    assert len(rows) == 1
    for row in outcomes:
        assert not row["count_saturated"] and not row["sum_saturated"]
        assert sum(row["buckets"]) == row["count"]
    return rows[0], {o["outcome"]: o["count"] for o in outcomes}


def derive(root):
    inputs, rows = [], []
    for workload in ("point_put", "batch_put"):
        captures = []
        for name in ("before-metrics.json", "after-metrics.json"):
            path = root / workload / name
            raw = path.read_bytes()
            inputs.append(dict(path=str(path), bytes=len(raw),
                               sha256=hashlib.sha256(raw).hexdigest()))
            captures.append(json.loads(raw))
        before, after = captures
        assert set(before) == set(after) == {"1", "2", "3"}
        for voter in sorted(before):
            first, last = before[voter], after[voter]
            for key in ("process_id", "node_id", "exporter_created_unix_ns",
                        "schema_version", "reset", "snapshot_consistency"):
                assert first[key] == last[key], (workload, voter, key)
            assert first["node_id"] == int(voter)
            assert first["schema_version"] == 2
            assert first["snapshot_consistency"] == "coherent_per_metric_independent_between_metrics"
            assert int(first["captured_unix_ns"]) < int(last["captured_unix_ns"])
            assert first["exporter_uptime_ns"] < last["exporter_uptime_ns"]
            for snapshot in (first, last):
                assert snapshot["export_failures_before_capture"] == 0
                assert not snapshot["export_failures_saturated"]
            counts = {}
            for name in METRICS:
                a, outcomes_a = success(first, name)
                b, outcomes_b = success(last, name)
                assert outcomes_a.keys() == outcomes_b.keys()
                assert all(outcomes_b[k] == outcomes_a[k] for k in outcomes_a if k != "success")
                assert b["count"] >= a["count"] and b["sum_ns"] >= a["sum_ns"]
                counts[name] = b["count"] - a["count"]
            assert counts["engine_wal_record_write"] > 0
            rows.append(dict(
                workload=workload, voter=int(voter), process_id=first["process_id"],
                captured_before_ns=int(first["captured_unix_ns"]),
                captured_after_ns=int(last["captured_unix_ns"]),
                successful_event_deltas=counts,
                command_apply_events_per_engine_write_event=(
                    counts["raft_command_apply"] / counts["engine_wal_record_write"]),
            ))
    return dict(
        schema=1, complete=True,
        scope="Offline arithmetic on existing full-capture metric intervals; not new runtime or nominal timed-window acceptance.",
        limitations=[
            "Metrics are coherent individually, not one simultaneous cross-metric snapshot.",
            "Capture intervals include setup/warmup activity and exceed the nominal measured interval.",
            "Engine write events include segment headers; they are not exactly Raw apply groups.",
            "No group-size distribution, mutation count, duplicate-key rate or causal speedup is inferred.",
            "Per-command apply intervals overlap inside a group; their duration sums are not exclusive CPU or wall time.",
        ], inputs=inputs, rows=rows,
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile-root", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = derive(args.profile_root)
    with args.output.open("x") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"complete": True, "rows": len(result["rows"]),
                      "output": str(args.output)}))
