#!/usr/bin/env python3
"""Verify complete persistent-workload artifacts and positive work in 16 Chaos windows."""
import argparse
import copy
import datetime
import hashlib
import importlib.util
import json
from pathlib import Path

from workload_report import PHASES, bounded, require, strict_json, validate

WINDOWS = ["registration-seed-blackhole", "pod-failure-1", "pod-failure-2", "pod-failure-3",
           "partition", "public-admission-overload", "delay"] + [
               f"io-voter-{node}-errno-{errno}" for node in (1, 2, 3) for errno in (5, 28)] + [
                   f"store-loss-voter-{node}-log-missing" for node in (1, 2, 3)]

spec = importlib.util.spec_from_file_location('store_loss_audit', Path(__file__).with_name('check-store-loss-chaos.py'))
store_loss = importlib.util.module_from_spec(spec)
spec.loader.exec_module(store_loss)


def window_check(phase, progress, faults, records, configuration, elapsed, victim_pod=None):
    require(progress["version"] == 1 and progress["stage"] == "measure", "fault phase lacks live measured progress")
    cutoff = progress["monotonic_ns"]
    require(type(cutoff) is int and 0 < cutoff < elapsed, "fault progress timestamp is invalid")
    p = progress["progress"]
    require(0 < p["terminal"] <= p["issued"] <= configuration["max_operations"] and
            p["issued"] - p["terminal"] == p["in_flight"] <= configuration["workers"] and
            0 < p["peak_in_flight"] <= configuration["workers"], "fault progress accounting is invalid")
    rows = p["phase_successes"]
    require([row["phase"] for row in rows] == PHASES, "fault progress vocabulary differs")
    counts = next(row for row in rows if row["phase"] == phase)
    require(counts["put"] > 0 and counts["get"] > 0, "fault phase lacks acknowledged put/get progress")
    calls = {record["id"]: record for record in records if record["type"] == "invoke"}
    observed = {kind: 0 for kind in ("get", "put", "delete")}
    for record in records:
        if record["type"] == "return" and record["outcome"] == "ok" and record["monotonic_ns"] <= cutoff:
            call = calls[record["id"]]
            if call["phase"] == phase:
                observed[call["op"]] += 1
    require(all(type(counts[kind]) is int and 0 <= counts[kind] <= observed[kind] for kind in observed),
            "fault progress is not supported by the complete history at its cutoff")
    if phase.startswith("pod-failure"):
        kind, name, action = "PodChaos", "fail-leader", "pod-failure"
    elif phase.startswith("io-voter"):
        kind, name, action = "IOChaos", "raft-io-fault", "fault"
    elif phase.startswith("store-loss-voter"):
        kind, name, action = "PodChaos", "store-loss-kill", "pod-kill"
    else:
        kind = "NetworkChaos"
        name, action = {"registration-seed-blackhole": ("registration-seed-blackhole", "partition"),
                        "partition": ("isolate-leader", "partition"),
                        "public-admission-overload": ("isolate-leader", "partition"),
                        "delay": ("delay-follower", "delay")}[phase]
    matches = [item for item in faults["items"] if item["kind"] == kind and item["metadata"]["name"] == name]
    require(len(matches) == 1, "fault window lacks its exact Chaos resource")
    fault = matches[0]
    require(fault["spec"]["action"] == action and not fault["metadata"].get("deletionTimestamp"), "fault was removed or has the wrong action")
    require(any(c["type"] == "AllInjected" and c["status"] == "True" for c in fault["status"]["conditions"]),
            "fault window does not contain an injected Chaos resource")
    selector = fault["spec"]["selector"]
    require(selector["namespaces"] == [fault["metadata"]["namespace"]],
            "fault selector is not restricted to the owned database namespace")
    if phase.startswith(("io-voter", "store-loss-voter")):
        require(victim_pod is not None and victim_pod["metadata"]["namespace"] == fault["metadata"]["namespace"] and
                selector["pods"] == {fault["metadata"]["namespace"]: [victim_pod["metadata"]["name"]]},
                "fault does not select the retained victim Pod")
        labels = victim_pod["metadata"]["labels"]
        if phase.startswith("io-voter"):
            require(fault["spec"]["volumePath"] == "/data" and fault["spec"]["path"] == "/data/raft/raft.log" and
                    fault["spec"]["methods"] == ["WRITE"] and fault["spec"]["percent"] == 100,
                    "I/O fault does not target the actual Raft log writes")
        else:
            require(labels["kv9-node"] == phase.split("-")[3] and
                    any(r['id'] == fault['metadata']['namespace'] + '/' + victim_pod['metadata']['name'] and
                        r['phase'] == 'Injected' and r['injectedCount'] > 0 and
                        any(e['operation'] == 'Apply' and e['type'] == 'Succeeded' for e in r['events'])
                        for r in fault['status']['experiment']['containerRecords']),
                    'store-loss fault did not kill the original voter')
    else:
        labels = selector["labelSelectors"]
    require(labels["app"] == "kv9", "fault does not select a database Pod")
    victim = labels["kv9-node"]
    require(victim in ("1", "2", "3"), "fault did not select a database voter")
    if phase.startswith("pod-failure"):
        require(victim == phase[-1], "Pod fault selected the wrong voter")
    if phase.startswith("io-voter"):
        require(victim == phase.split("-")[2] and fault["spec"]["errno"] == int(phase.split("-")[-1]),
                "I/O fault selected the wrong voter/errno")
    return dict(phase=phase, cutoff_ns=cutoff, acknowledged_at_cutoff=observed,
                retained_progress={kind: counts[kind] for kind in observed},
                kind=kind, resource=name, victim=victim, namespace=fault["metadata"]["namespace"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifacts", type=Path)
    parser.add_argument("--expected-revision")
    args = parser.parse_args()
    root = args.artifacts
    checked = validate(root / "persistent-run", root / "persistent-build", args.expected_revision, seconds=60)
    require(checked["full_history_independently_checked"], "persistent Chaos requires a full independently checked history")
    report = strict_json(bounded(root / "persistent-run/report.json", 2 * 1024 * 1024))
    require(report["stop"]["reason"] == "stop_file", "persistent workload ended before the matrix coordinator stopped it")
    config = report["configuration"]
    require(config == strict_json(bounded(root / "persistent-config.json", 65536)), "Chaos launched a different workload configuration")
    records = [strict_json(line) for line in bounded(root / "persistent-run/history.jsonl", config["history_bytes"]).splitlines()][1:]
    pod = strict_json(bounded(root / "persistent-client-pod.json", 2 * 1024 * 1024))
    require(pod["metadata"]["labels"]["app"] == "kv9-persistent-client" and
            "kv9-node" not in pod["metadata"]["labels"], "persistent client is inside the database fault selector")
    require(pod["spec"]["restartPolicy"] == "Never" and len(pod["spec"]["containers"]) == 1,
            "unexpected workload Pod lifecycle")
    require(pod["spec"]["containers"][0]["resources"]["limits"] == {"cpu": "2", "memory": "256Mi"},
            "workload Pod resource limits changed")
    ticks = int(bounded(root / "persistent-clock-ticks.txt", 64))
    require(0 < ticks <= 1_000_000, "invalid workload runtime clock-tick rate")
    evidence, windows = {}, []
    previous = datetime.datetime.min.replace(tzinfo=datetime.timezone.utc)
    for phase in WINDOWS:
        progress = strict_json(bounded(root / f"{phase}-persistent-progress.json", 65536))
        faults = strict_json(bounded(root / f"{phase}-persistent-faults.json", 2 * 1024 * 1024))
        when = datetime.datetime.fromisoformat(bounded(root / f"{phase}-persistent-observed-at.txt", 128).decode().strip())
        require(when.tzinfo is not None and when > previous, "fault observations are missing or reordered")
        previous = when
        victim_pod = None
        if phase.startswith("io-voter"):
            victim_pod = strict_json(bounded(root / f"{phase}-persistent-victim.json", 2 * 1024 * 1024))
            def fields(suffix):
                return dict(line.split("=", 1) for line in bounded(root / (phase + suffix), 65536).decode().splitlines())
            exited, status = fields("-exit.txt"), fields("-status.txt")
            recovered, receipt = fields("-recovered-status.txt"), fields("-majority-put.out")
            process_log = bounded(root / f"{phase}-process.log", 2 * 1024 * 1024).decode()
            require(exited["exit_code"] == "1" and exited["pod"] == victim_pod["metadata"]["name"] and
                    exited["uid"] == victim_pod["metadata"]["uid"] and f'os error {phase.split("-")[-1]}' in status["fatal"] and
                    "node runtime failed:" in process_log and "panicked" not in process_log,
                    "I/O fault lacks the actual victim's fail-stop evidence")
            require(int(receipt["applied_term"]) > 0 and int(recovered["driver_applied_index"]) >= int(receipt["applied_index"]) > 0,
                    "I/O recovery lacks an exact majority receipt and caught-up replica")
        elif phase.startswith("store-loss-voter"):
            victim_pod = strict_json(bounded(root / f"{phase}-persistent-victim.json", 2 * 1024 * 1024))
            require(victim_pod == strict_json(bounded(root / phase / "before/pod.json", 2 * 1024 * 1024)),
                    'store-loss snapshot differs from the original victim')
            store_loss.audit_cell(root, phase)
        window = window_check(phase, progress, faults, records, config, report["elapsed_ns"], victim_pod)
        require(window["namespace"] == pod["metadata"]["namespace"], "fault snapshot belongs to another namespace")
        window["observed_at"] = when.isoformat()
        windows.append(window)
        evidence[phase] = (progress, faults, victim_pod)
    controls = []
    phase = "io-voter-2-errno-28"
    original_progress, original_faults, victim_pod = evidence[phase]
    for name, expected in (("empty-fault-progress", "fault phase lacks acknowledged put/get progress"),
                           ("stale-progress-cutoff", "fault progress is not supported by the complete history at its cutoff"),
                           ("unapplied-fault", "fault window does not contain an injected Chaos resource")):
        progress, faults = copy.deepcopy(original_progress), copy.deepcopy(original_faults)
        if name == "empty-fault-progress":
            next(row for row in progress["progress"]["phase_successes"] if row["phase"] == phase)["put"] = 0
        elif name == "stale-progress-cutoff":
            progress["monotonic_ns"] = 1
        else:
            for item in faults["items"]:
                if item["kind"] == "IOChaos" and item["metadata"]["name"] == "raft-io-fault":
                    for condition in item["status"]["conditions"]:
                        if condition["type"] == "AllInjected": condition["status"] = "False"
        folder = root / "persistent-evidence-controls" / name
        folder.mkdir(parents=True, exist_ok=False)
        (folder / "progress.json").write_text(json.dumps(progress, indent=2) + "\n")
        (folder / "faults.json").write_text(json.dumps(faults, indent=2) + "\n")
        try:
            window_check(phase, progress, faults, records, config, report["elapsed_ns"], victim_pod)
        except ValueError as error:
            require(str(error) == expected, "invalid evidence failed for an unintended reason")
            controls.append(dict(name=name, rejected=True, reason=str(error)))
        else:
            raise ValueError("invalid persistent fault evidence was accepted")
        window_check(phase, original_progress, original_faults, records, config, report["elapsed_ns"], victim_pod)
    result = dict(version=1, accepted=True, build=report["build"], report_sha256=checked["report_sha256"],
                  sources={name: hashlib.sha256((Path(__file__).parent / name).read_bytes()).hexdigest()
                      for name in ("check-persistent-chaos.py", "chaos-mesh-persistent.sh", "chaos-mesh-e2e.sh",
                                   "chaos-mesh-probes.sh", "chaos-mesh-io.sh", "chaos_client.py",
                                   "workload_report.py", "history/checker.py")},
                  topology="single-host Kind", storage="local WAL mode; MinIO is verified by the separate executable E2E",
                  clock_ticks_per_second=ticks, windows=windows, evidence_controls=controls, history=checked["history"])
    with (root / "persistent-history-checker.json").open("x") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print("PASS: persistent full history verified across 16 Chaos windows; three invalid fault evidence controls rejected")


if __name__ == "__main__":
    main()
