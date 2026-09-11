#!/usr/bin/env python3
"""First v3 point PUT and native BatchPut(64) c1/c64 selection screen.

Root runs smoke/reviews isolation before any diagnostic timing. Default runtime
keeps quorum and WAL/sync behavior; tmpfs mode has no power-loss durability.
"""
import argparse
import base64
import hashlib
import importlib.util
import json
import os
import re
from pathlib import Path
import shutil
import signal
import socket
import stat
import subprocess
import sys
import time

# Fail before source imports, output writes or process launches under -O.
if not __debug__:
    raise RuntimeError("this retained runner requires PYTHONOPTIMIZE=0")
sys.dont_write_bytecode = True

OLD_REVISION = "ca0002c7f8e9ee6f595efcc9f4151085ccce87cb"
NEW_REVISION = "74d241161eadf2121128f6fd9101ead18e562765"
REDIS_REVISION = "0be806d9671e2c50701a64aa7889c8859b7648ba"
SERVER_PINS = {
    "old": dict(revision=OLD_REVISION,
                binary_sha256="b33b5d302f901aac5cf6a95449b9bb5dd68473747e011220dbed7aed2e591d13",
                manifest_sha256="8c2ea115afc1ec8b6c82224dc6449d1d2439f5f27b5758e45090589752d186e8"),
    "new": dict(revision=NEW_REVISION,
                binary_sha256="PENDING_ORIGINAL_CANDIDATE_BINARY_SHA256",
                manifest_sha256="PENDING_ORIGINAL_CANDIDATE_BUILD_SHA256"),
}
REDIS_CLIENT_SHA = "5a8ac274b8cc6548a072f5305b04936a08cc4d1ba84d50150f675bab563049af"
CLIENT_PINS = {
    "client": dict(binary_sha256="8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957",
                   manifest_sha256="eb8b4e1421dd4e64dcca59fcc7829926d5789611f6b3748d07c4090f634f2c4c"),
    "redis": dict(binary_sha256=REDIS_CLIENT_SHA,
                  manifest_sha256="10f9ce2a754a2ef43a6439856c1d5fb00c5a1e5004173e52897a3ea19d6124f0"),
}
PROTOCOL_ID = "kv9-indexed-receipt-write-screen-c1-c64-v1"
WORKLOAD_CELLS = [(name, batch, reads) for name, batch in (("point", 1), ("batch64", 64))
                  for reads in (0,)]
SPACE_GUARDS = {
    "preflight": {"tmpfs": 32 * 1024**3, "retention": 96 * 1024**3},
    "runtime": {"tmpfs": 16 * 1024**3, "retention": 64 * 1024**3},
}
SERVER_COMMAND = ["cargo", "build", "--locked", "--bin", "kv9", "--release",
                  "--message-format=json-render-diagnostics"]
RESP_HELPER_SHA = "008c5bea0640a4a5a092feb6617798a84e9dd4889fd11bf8c6791cfe0852864a"
ZERO = ("public_rpc_in_flight", "public_rpc_queued", "public_rpc_running", "public_rpc_encoded_bytes",
        "raft_async_apply_queued", "raft_async_apply_in_flight", "raft_async_read_queued",
        "raft_async_read_active", "raft_async_read_in_flight", "raft_async_read_active_groups")
BACKGROUND_NAMES = {"redis-server", "dockerd", "containerd", "containerd-shim", "kube-apiserver", "kubelet", "etcd",
                    "chaos-daemon", "chaos-controlle", "minio", "buildkitd", "rust-analyzer"}


def require(value, message):
    if not value:
        raise ValueError(message)


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def save(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + "\n")


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


def placement(smoke):
    return dict(client_cpus=[6, 7] if smoke else [0, 1],
                server_cpus=list(range(8, 16)) + list(range(22, 32)) if smoke else [2, 3, 4, 5],
                separated=True, exclusive_host=False)


def affinity(pid, expected):
    task = Path("/proc", str(pid), "task")
    threads = {}
    for entry in task.iterdir():
        try:
            mask = sorted(os.sched_getaffinity(int(entry.name)))
        except ProcessLookupError:
            continue
        require(mask == expected, f"owned task {pid}/{entry.name} affinity {mask} differs from {expected}")
        threads[entry.name] = mask
    require(threads, "owned process has no observed live threads")
    return dict(pid=pid, expected=expected, process=sorted(os.sched_getaffinity(pid)), threads=threads,
                observed_unix_ns=time.time_ns())


def background_inventory():
    result = []
    for root in Path("/proc").iterdir():
        if not root.name.isdigit():
            continue
        try:
            name = (root / "comm").read_text().strip()
            if name not in BACKGROUND_NAMES:
                continue
            fields = (root / "stat").read_text().rsplit(")", 1)[1].split()
            result.append(dict(pid=int(root.name), name=name, start_ticks=int(fields[19]),
                               user_ticks=int(fields[11]), system_ticks=int(fields[12]),
                               uid=root.stat().st_uid, affinity=sorted(os.sched_getaffinity(int(root.name))),
                               cgroup=(root / "cgroup").read_text()))
        except (OSError, ValueError):
            continue
    return result


def host_sample(background):
    rows = []
    for captured in background:
        try:
            fields = Path("/proc", str(captured["pid"]), "stat").read_text().rsplit(")", 1)[1].split()
            if int(fields[19]) == captured["start_ticks"]:
                rows.append(dict(pid=captured["pid"], start_ticks=int(fields[19]),
                                 user_ticks=int(fields[11]), system_ticks=int(fields[12])))
        except (OSError, ValueError):
            continue
    return dict(unix_ns=time.time_ns(), monotonic_ns=time.monotonic_ns(), proc_stat=Path("/proc/stat").read_text(),
                cpu_pressure=Path("/proc/pressure/cpu").read_text(),
                io_pressure=Path("/proc/pressure/io").read_text(), background=rows)


def source_binding(source, manifest, expected_revision):
    require(re.fullmatch(r"[0-9a-f]{40}", expected_revision) is not None, "invalid expected source revision")
    require(manifest["revision"] == expected_revision and manifest["dirty"] is False and
            manifest["profile"] == "release", "role is not the expected clean release")
    sources = manifest["sources"]
    require(isinstance(sources, dict) and sources, "empty source inventory")
    expected = hashlib.sha256(json.dumps(sources, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    require(expected == manifest["source_tree_sha256"], "source inventory digest differs")
    require(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, text=True).strip() == expected_revision,
            "source checkout revision differs")
    require(not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=all"], cwd=source),
            "source checkout is dirty")
    for path, expected in sources.items():
        relative = Path(path)
        require(not relative.is_absolute() and ".." not in relative.parts and expected is not None,
                "invalid clean source path/digest")
        require(digest(source / relative) == expected, "source bytes differ: " + path)
    return dict(path=str(source), revision=expected_revision, source_tree_sha256=manifest["source_tree_sha256"],
                sources=sources)


def cargo_server(path, retained=None):
    records = [json.loads(line) for line in path.read_text().splitlines()]
    require(records and records[-1].get("reason") == "build-finished" and records[-1].get("success") is True,
            "server Cargo gate not successful")
    require(sum(row.get("reason") == "build-finished" for row in records) == 1, "ambiguous Cargo completion")
    selected = {}
    for name in ("kv9", "kv9_engine", "kv9_raft", "kv9_server"):
        rows = [r for r in records if r.get("reason") == "compiler-artifact" and r.get("target", {}).get("name") == name]
        require(len(rows) == 1 and rows[0]["features"] == [] and rows[0]["profile"]["opt_level"] == "3" and
                rows[0]["profile"]["test"] is False, "ambiguous/nondefault/nonrelease server artifact: " + name)
        if retained is not None:
            require(rows[0] == retained[name], "original retained server artifact differs")
        selected[name] = rows[0]
    require(selected["kv9"]["target"]["kind"] == ["bin"] and selected["kv9"]["executable"],
            "server artifact is not executable")
    return selected


def verify_inputs(args):
    """Separate original role attestations; never fabricate a mixed-revision build."""
    roles = {}
    require(args.expected_client_revision == REDIS_REVISION, "native client must use the frozen v3 revision")
    for name, source, build in (("old", args.old_server_source, args.old_server_build),
                                ("new", args.new_server_source, args.new_server_build)):
        pin = SERVER_PINS[name]
        manifest = read(build / "build.json")
        require(digest(build / "build.json") == pin["manifest_sha256"], "frozen server manifest differs: " + name)
        bound = source_binding(source, manifest, pin["revision"])
        files = [build / "build.json"]
        declaration = manifest["binaries"]["kv9"]
        require(declaration["command"] == SERVER_COMMAND, "server command differs: " + name)
        declared = declaration["sha256"]
        cargo = build / "kv9-cargo.jsonl"
        selected = cargo_server(cargo)
        files += [cargo]
        require(digest(build / "kv9") == declared == pin["binary_sha256"], "frozen server binary differs: " + name)
        roles[name] = dict(source=bound, build_directory=str(build), binary=str(build / "kv9"),
                           binary_sha256=declared, rustc=manifest["rustc"], command=SERVER_COMMAND,
                           cargo_artifacts=selected, evidence={str(p): digest(p) for p in files})
    for name, source, build, revision, binary in (
        ("client", args.client_source, args.client_build, args.expected_client_revision, "kv9-batch-benchmark"),
        ("redis", args.redis_client_source, args.redis_client_build, REDIS_REVISION, "kv9-redis-batch-reference")):
        manifest = read(build / "build.json")
        inventory = read(build / "sources.json")
        require(digest(build / "build.json") == CLIENT_PINS[name]["manifest_sha256"] and
                manifest["binary_sha256"] == CLIENT_PINS[name]["binary_sha256"], "frozen v3 client pin differs")
        require(all(inventory[k] == manifest[k] for k in
                    ("revision", "dirty", "source_tree_sha256", "binary_sha256")), "client source/build mismatch")
        bound = source_binding(source, dict(manifest, sources=inventory["sources"]), revision)
        require(digest(build / binary) == manifest["binary_sha256"], "role client executable differs")
        if name == "redis":
            require(manifest["binary_sha256"] == REDIS_CLIENT_SHA, "original Redis client changed")
        roles[name] = dict(source=bound, build_directory=str(build), binary=str(build / binary),
                           binary_sha256=manifest["binary_sha256"], rustc=manifest["rustc"],
                           evidence={str(build / p): digest(build / p) for p in ("build.json", "sources.json", "cargo.jsonl")})
    common = "crates/server/src/bin/kv9-batch-benchmark/common.rs"
    require(roles["client"]["source"]["sources"][common] == roles["redis"]["source"]["sources"][common],
            "native/Redis deterministic payload or scheduling source differs")
    require(roles["client"]["source"]["sources"][common] ==
            "b98d4a49f9ee2199e183204b72fd52813907574a03c1cf3376cf5d281d7902fd", "fixed shared source changed")
    require(len({r["rustc"] for r in roles.values()}) == 1, "role compiler identities differ")
    return roles


def verify_client_cargo(args, validator):
    results = {}
    for role, build, revision, reference in (("client", args.client_build, args.expected_client_revision, False),
                                            ("redis", args.redis_client_build, REDIS_REVISION, True)):
        _, manifest, features = validator.build_check(build, build, revision, reference=reference)
        require(features == [] and manifest["profile"] == "release", "client is not a default-feature release")
        results[role] = dict(binary_sha256=manifest["binary_sha256"], features=features, revision=revision)
    return results


def make_plan(smoke):
    first = [(concurrency, name, batch, reads, role) for concurrency in (1, 64)
             for name, batch, reads in WORKLOAD_CELLS for role in ("old", "new", "redis")]
    rows = []
    for repeat in range(1 if smoke else 2):
        for concurrency, name, batch, reads, role in (first if repeat == 0 else reversed(first)):
            server = None if role == "redis" else role
            read_api = ("get" if name == "point" else "mget") if server is None else (
                "point_get" if name == "point" else "batch_get")
            write_api = ("set" if name == "point" else "mset") if server is None else (
                "point_put" if name == "point" else "batch_put")
            workload = f"{name}-r{reads:03d}"
            shared = dict(run_id=f"p{repeat}{concurrency:04d}", seed=71,
                          workers=concurrency, keys=4096, batch_size=batch,
                          value_bytes=128, read_percent=reads, warmup_calls=128,
                          measure_ms=2000 if smoke else 10000, max_calls=10_000_000,
                          load=dict(kind="closed_loop"))
            rows.append(dict(repeat=repeat, arm=f"{role}-{workload}", server_role=server, workload=workload,
                             read_api=read_api, write_api=write_api, mix={0:"write",50:"mixed",100:"read"}[reads],
                             target="kv9" if server else "redis-memory", shared=shared))
    require(len(rows) == (12 if smoke else 24), "matched v3 c1/c64 cohort inventory differs")
    return rows


def configs(row, peers, keyspace, redis_address):
    shared = row["shared"]
    native_api = row["read_api"] if row["server_role"] else ("point_get" if row["read_api"] == "get" else "batch_get")
    redis_api = row["read_api"] if not row["server_role"] else ("get" if row["read_api"] == "point_get" else "mget")
    native_write = row["write_api"] if row["server_role"] else ("point_put" if row["write_api"] == "set" else "batch_put")
    redis_write = row["write_api"] if not row["server_role"] else ("set" if row["write_api"] == "point_put" else "mset")
    native = dict(version=3, rpc_transport="tonic_stream", read_api=native_api, write_api=native_write, **shared,
                  client=dict(version=1, peers=peers, keyspace_id=keyspace, epoch_conf_ver=1, epoch_version=1,
                              max_in_flight=shared["workers"], max_attempts=6, deadline_ms=1500, retry_backoff_ms=5))
    redis = dict(version=3, read_api=redis_api, write_api=redis_write, address=redis_address, deadline_ms=1500, **shared)
    return native, redis


def final_dataset(configuration, dataset, resp):
    result = resp.valid_dataset(configuration, dataset)
    require(len(dataset) == configuration["keys"] + 1, "final dataset count differs")
    changed = 0
    for index in range(configuration["keys"]):
        value = dataset[f'{configuration["run_id"]}:{index:016x}'.encode()]
        nonce = int.from_bytes(value[8:16], "big")
        if nonce:
            require(resp.mix(configuration["seed"] ^ nonce) % 100 >= configuration["read_percent"],
                    "final value contains a read-only nonce")
            first = resp.mix(configuration["seed"] ^ nonce ^ 0xc6275c213842315b) % configuration["keys"]
            require((index - first) % configuration["keys"] < configuration["batch_size"],
                    "final nonce does not write this key")
            changed += 1
    require(changed > 0 if configuration["read_percent"] < 100 else changed == 0,
            "writes absent or a read-only workload changed data")
    return dict(result, changed_keys=changed, maximum_configured_nonce=configuration["warmup_calls"]+configuration["max_calls"],
                deterministic_write_key_membership_checked=True, exact_issued_nonce_set_checked=False,
                full_linearizability_checked=False)


def population_observations(report, configuration, reference):
    """Keep validity separate from healthy, single-attempt comparison suitability."""
    result = {}
    for phase, metrics in report["metrics"].items():
        counts = [sum(p["calls"] for p in op["populations"]) for op in metrics["statistics"]]
        success = [op["populations"][0]["calls"] for op in metrics["statistics"]]
        attempts = [op["command_attempts"] if reference else sum(h["raw"]["count"] for h in op["attempts"])
                    for op in metrics["statistics"]]
        if phase in ("warmup", "measurement"):
            expected = [configuration["read_api"].removeprefix("point_"), configuration["write_api"].removeprefix("point_")]
            require(metrics["operations"] == expected, "selected traffic API label differs")
            if phase == "measurement":
                require(sum(counts) == report["measured_issued"] > 0, "measured population missing")
                require(all(counts[i] > 0 for i in range(2) if
                        (i == 0 and configuration["read_percent"] > 0) or
                        (i == 1 and configuration["read_percent"] < 100)), "requested operation has no measured calls")
                if configuration["read_percent"] in (0, 100):
                    require(counts[0 if configuration["read_percent"] == 0 else 1] == 0, "pure measurement mix differs")
        result[phase] = dict(calls=counts, successes=success, attempts=attempts,
                            all_success=counts == success, all_success_single_attempt=counts == success == attempts)
    # Failures remain in the raw report and summaries, including an accounting-
    # valid cohort that does not qualify for healthy single-attempt comparison.
    return result


def storage_observation(directory):
    result = dict(started_unix_ns=time.time_ns(), filesystems={})
    for name, path in (("tmpfs", Path("/dev/shm")), ("retention", directory)):
        try:
            value = os.statvfs(path)
            device = path.stat().st_dev
        except OSError as error:
            # The inherited resource sampler tolerates process-exit OSErrors;
            # missing filesystem evidence must instead stop this cohort.
            raise ValueError("storage observation failed: " + str(error)) from error
        result["filesystems"][name] = dict(path=str(path), device=device,
            fragment_bytes=value.f_frsize, blocks=value.f_blocks, free_blocks=value.f_bfree,
            available_blocks=value.f_bavail, available_bytes=value.f_bavail * value.f_frsize)
    result["completed_unix_ns"] = time.time_ns()
    return result


def require_storage(observation, phase):
    require(all(observation["filesystems"][name]["available_bytes"] >= minimum
                for name, minimum in SPACE_GUARDS[phase].items()),
            "storage guard intervention: retained incomplete cohort; no rerun")


def fresh_drain(fixture, directory, label):
    started = time.time_ns()
    baseline = {str(n): fixture.state(n) for n in fixture.nodes}
    require(all(baseline.values()), "drain baseline is missing a bound status")
    first = {}
    def qualifies(state):
        return (state.get("bootstrap_state") == "Serving" and state.get("fatal") == "" and
                all(state.get(k) == "0" for k in ZERO) and state.get("raft_async_apply_stopped") ==
                state.get("raft_async_read_stopped") == "false" and
                state.get("applied_term") == state.get("driver_applied_term") and
                state.get("applied_index") == state.get("driver_applied_index"))
    def drained():
        states = {str(n): fixture.state(n) for n in fixture.nodes}
        for node, state in states.items():
            require(state and all(state.get(k) == baseline[node][k] for k in ("pid", "process_start_ticks", "process_boot_id")), "status writer changed during drain")
            if (int(state["metrics_export_successes"]) > int(baseline[node]["metrics_export_successes"])
                    and node not in first and qualifies(state)):
                first[node] = dict(status=state, observed_unix_ns=time.time_ns())
        if any(n not in first or int(s["metrics_export_successes"]) <= int(first[n]["status"]["metrics_export_successes"]) for n, s in states.items()):
            return None
        empty = all(qualifies(s) for s in states.values())
        positions = {(s.get("applied_term"), s.get("applied_index")) for s in states.values()}
        return states if empty and len(positions) == 1 and fixture.leader() else None
    final = fixture.wait("fresh two-export empty public/apply/read drain", drained, 20)
    record = dict(started_unix_ns=started, baseline=baseline, first_advances=first, final=final, completed_unix_ns=time.time_ns())
    require(record["completed_unix_ns"] - started <= 20_000_000_000, "drain freshness exceeded 20 seconds")
    save(directory / (label + "-fresh-drain.json"), record)
    return record


def native_readback(fixture, configuration, directory, resp):
    prefix = configuration["run_id"].encode() + b":"
    end = configuration["run_id"].encode() + b";"
    start = prefix
    dataset = {}
    for page in range(20):
        leader = fixture.wait("leader for independent final scan", fixture.leader)
        text = fixture.command([fixture.build / "kv9", "client", "raw-scan", "--addr", fixture.addresses[leader],
                                "--keyspace", configuration["client"]["keyspace_id"], "--start-hex", start.hex(),
                                "--end-hex", end.hex(), "--limit", "256"])
        (directory / f"readback-{page:02d}.txt").write_text(text + "\n")
        lines = text.splitlines()
        require(lines and lines[-1].startswith("count="), "independent scan count missing")
        pairs = []
        for line in lines[:-1]:
            parts = dict(piece.split("=", 1) for piece in line.split())
            require(set(parts) == {"key_hex", "value_hex"}, "unexpected scan result")
            pairs.append((bytes.fromhex(parts["key_hex"]), bytes.fromhex(parts["value_hex"])))
        require(len(pairs) == int(lines[-1][6:]) and len(pairs) <= 256, "independent scan count differs")
        require([k for k, _ in pairs] == sorted(k for k, _ in pairs), "scan order differs")
        for key, value in pairs:
            require(start <= key < end and key not in dataset, "scan duplicates/escapes range")
            dataset[key] = value
        if len(pairs) < 256:
            break
        start = pairs[-1][0] + b"\x00"
    else:
        raise ValueError("independent final scan exceeded its page bound")
    require(len(dataset) == configuration["keys"] + 1, "independent final key count differs")
    return final_dataset(configuration, dataset, resp)


def summary_from_report(report):
    metrics = report["metrics"]["measurement"]
    elapsed = report["cohort_elapsed_ns"] / 1e9
    operations = []
    def histogram(h):
        # The exact raw buckets stay in report.json. Avoid copying every large
        # bucket array into matrix.json again after each completed cohort.
        return {k: v for k, v in h.items() if k != "raw"} | {
            "population": {k: v for k, v in h["raw"].items() if k != "buckets"}}
    for name, op in zip(metrics["operations"], metrics["statistics"]):
        operations.append(dict(operation=name, populations=[dict(outcome=outcome, calls=p["calls"], input_items=p["input_items"],
            whole_call_latency=histogram(p["whole_call"]), scheduled_to_completion=histogram(p["scheduled_to_completion"]),
            client_or_sdk_latency=histogram(p.get("client_call", p.get("sdk_call")))) for outcome, p in zip(metrics["outcomes"], op["populations"])],
            reasons=dict(zip(metrics["reasons"], op["reasons"])),
            attempts={k: [histogram(h) for h in v] if k == "attempts" else v for k, v in op.items() if "attempt" in k or "failures" in k or k == "rpc_codes"},
            dispatch_lateness=histogram(op["dispatch_lateness"])))
    outcomes = {outcome: sum(op["populations"][i]["calls"] for op in metrics["statistics"]) for i, outcome in enumerate(metrics["outcomes"])}
    return dict(calls=report["measured_completed"], issued=report["measured_issued"], dropped_slots=report["dropped_slots"],
                outcomes=outcomes, outcome_rates_per_second={k: v / elapsed for k, v in outcomes.items()},
                successful_calls_per_second=report["successful_batches_per_second"],
                successful_input_items_per_second=report["successful_input_items_per_second"],
                cohort_elapsed_ns=report["cohort_elapsed_ns"], operations=operations,
                latency_scope="whole logical point/batch call; never divide latency by item count")


def observe_client(comparison, process, servers, directory, masks, background, benchmark):
    original = comparison.process_sample
    host_samples = []
    sample_calls = []
    storage_samples = []
    def sampled(pid):
        item = original(pid)
        item["thread_placement"] = affinity(pid, masks[pid])
        sample_calls.append(item)
        if pid == process.pid:
            host_samples.append(host_sample(background))
            space = storage_observation(directory)
            storage_samples.append(space)
            try:
                require_storage(space, "runtime")
            except ValueError:
                save(directory / "storage-guard-intervention.json", dict(phase="runtime", observation=space,
                     thresholds=SPACE_GUARDS["runtime"], timing_complete=False))
                raise
        return item
    comparison.process_sample = sampled
    try:
        # Reuse the unchanged resource sampler; only add task-affinity and
        # background/per-core observations to its ordinary sample records.
        code = comparison.observe(process, servers, directory, 90)
    finally:
        comparison.process_sample = original
        save(directory / "host-resource-samples.json", host_samples)
        save(directory / "sample-calls.json", sample_calls)
        save(directory / "storage-resource-samples.json", storage_samples)
    save(directory / "client-exit.json", dict(exit_code=code, observed_unix_ns=time.time_ns()))
    require(code == 0, "client incomplete/failed; original report retained")
    report = read(directory / "run/report.json")
    require(report["stop_reason"] == "duration" and report["complete"] is True, "cohort capped or incomplete")
    samples = read(directory / "resource-samples.json")
    start = report["measurement_start_unix_ns"]
    end = start + report["cohort_elapsed_ns"]
    inside = [s for s in samples if start <= s["unix_ns"] <= end]
    require(len(inside) >= 10, "insufficient measured resource samples")
    require(inside[0]["unix_ns"] - start <= 150_000_000 and end - inside[-1]["unix_ns"] <= 150_000_000,
            "resource samples do not adequately cover measurement edges")
    expected = {"client", *servers}
    require(all(set(s["processes"]) == expected for s in inside), "missing owned resource process inside measured cohort")
    save(directory / "resource-coverage.json", dict(samples=len(inside), first_gap_ns=inside[0]["unix_ns"] - start,
         last_gap_ns=end - inside[-1]["unix_ns"], actual_start_unix_ns=start, actual_end_unix_ns=end,
         cpu=comparison.cpu_summary(samples, start, end)))
    return report


def allocator_provenance(fixture, directory, label):
    """Endpoint-only provenance outside timing; never retain unrelated env values."""
    forbidden = ("_RJEM_MALLOC_CONF", "MALLOC_CONF", "LD_PRELOAD", "LD_AUDIT", "TOKIO_WORKER_THREADS")
    record = dict(started_unix_ns=time.time_ns(), label=label, voters={})
    for node, child in fixture.nodes.items():
        root = Path("/proc", str(child.pid))
        expected = fixture.identities[child.pid]
        before = (root / "stat").read_text()
        boot_before = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
        environ = (root / "environ").read_bytes()
        names = {entry.partition(b"=")[0] for entry in environ.split(b"\0") if entry}
        selected = sorted(name for name in forbidden if name.encode() in names)
        conf = root / "root/etc/_rjem_malloc.conf"
        try:
            info = conf.lstat()
            config = dict(path=str(conf), exists=True, symlink=stat.S_ISLNK(info.st_mode),
                          mode=info.st_mode, device=info.st_dev, inode=info.st_ino)
        except FileNotFoundError:
            config = dict(path=str(conf), exists=False, symlink=False)
        after = (root / "stat").read_text()
        boot_after = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
        for captured, boot in ((before, boot_before), (after, boot_after)):
            require(int(captured.split(" ", 1)[0]) == child.pid and
                    int(captured.rsplit(")", 1)[1].split()[19]) == expected["start_ticks"] and
                    boot == expected["boot_id"], "allocator observation lifetime changed")
        record["voters"][str(node)] = dict(pid=child.pid, start_ticks=expected["start_ticks"],
             boot_id=boot_before, stat_before=before, stat_after=after, boot_after=boot_after,
             checked_environment_names=list(forbidden), present_environment_names=selected,
             environ_sha256=hashlib.sha256(environ).hexdigest(), configuration_file=config)
        # Preserve the offending bounded record before rejecting an override.
        record["ended_unix_ns"] = time.time_ns()
        save(directory / (label + "-allocator-provenance.json"), record)
        require(not selected and not config["symlink"], "allocator runtime tuning/interposition override present")
    record["complete"] = True
    record["ended_unix_ns"] = time.time_ns()
    save(directory / (label + "-allocator-provenance.json"), record)


def native_trial(row, directory, args, modules, roles):
    benchmark, support, tmpfs, comparison, resp, native_validator, redis_validator = modules
    place = placement(args.smoke)
    server_role = roles[row["server_role"]]
    server_build = Path(server_role["build_directory"])
    expected_hashes = {True: roles["client"]["binary_sha256"], False: server_role["binary_sha256"]}
    class Guarded:
        def __init__(self, out, build):
            super().__init__(out, build)
            self.placement.update(place)
            self.record["placement"] = dict(place)
        def launch(self, command, logfile, client=False):
            child = super().launch(command, logfile, client)
            identity = self.identities[child.pid]
            expected = place["client_cpus" if client else "server_cpus"]
            require(identity["cpu_affinity"] == expected, "inherited process placement differs")
            require(identity["executable_sha256"] == expected_hashes[client], "executing native binary differs")
            identity["boot_id"] = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
            identity["thread_placement"] = affinity(child.pid, expected)
            return child
    class Volatile(Guarded, support.StreamingFixture, tmpfs.TmpfsFixture):
        pass
    class Disk(Guarded, support.StreamingFixture):
        pass
    fixture = (Volatile if args.storage == "tmpfs" else Disk)(directory / "fixture", server_build)
    result = dict(complete=False, server_role=row["server_role"], server_binary_sha256=server_role["binary_sha256"],
                  client_binary_sha256=roles["client"]["binary_sha256"], read_api=row["read_api"],
                  write_api=row["write_api"], workload=row["workload"])
    try:
        fixture.start()
        mounts = {}
        for node in fixture.nodes:
            physical = (fixture.out / "data" / f"n{node}").resolve()
            observed = json.loads(subprocess.check_output(["findmnt", "--json", "--target", str(physical),
                "--output", "TARGET,FSTYPE,SOURCE,OPTIONS"], text=True, timeout=10))
            require(len(observed["filesystems"]) == 1, "ambiguous voter data mount")
            kind = observed["filesystems"][0]["fstype"]
            require((kind == "tmpfs") if args.storage == "tmpfs" else kind not in ("tmpfs", "ramfs"), "declared voter storage mode differs from actual mount")
            mounts[str(node)] = dict(path=str(physical), mount=observed)
        save(directory / "storage-mounts.json", mounts)
        save(directory / "listener-before.json", {str(n): fixture.listener_evidence(n) for n in fixture.nodes})
        leader = fixture.wait("leader before fresh native keyspace", fixture.leader)
        receipt = fixture.command([server_build / "kv9", "client", "create-keyspace", "--addr", fixture.addresses[leader],
                                   "--name", row["shared"]["run_id"], "--api-type", "raw"])
        keyspace = int(dict(line.split("=", 1) for line in receipt.splitlines())["keyspace_id"])
        config, reference = configs(row, [dict(node_id=n, address=a) for n, a in fixture.addresses.items()], keyspace, "127.0.0.1:6379")
        redis_validator.paired_configuration(reference, config)
        save(directory / "requested-config.json", config)
        fresh_drain(fixture, directory, "before")
        fixture.snapshot(directory, "before")
        background = background_inventory()
        save(directory / "background-before.json", background)
        save(directory / "host-before.json", host_sample(background))
        allocator_provenance(fixture, directory, "before")
        child = fixture.launch([args.client_build / "kv9-batch-benchmark", "--config", directory / "requested-config.json",
                                "--build-manifest", args.client_build / "build.json", "--output", directory / "run"], directory / "client.log", client=True)
        fixture.workloads.append(child)
        save(directory / "client-identity.json", fixture.identities[child.pid])
        masks = {p.pid: place["server_cpus"] for p in fixture.nodes.values()}
        masks[child.pid] = place["client_cpus"]
        report = observe_client(comparison, child, {f"voter-{n}": p.pid for n, p in fixture.nodes.items()}, directory, masks, background, benchmark)
        result["validated"] = native_validator.validate(directory / "run", args.client_build, directory / "requested-config.json", args.expected_client_revision, require_timing=not args.smoke)
        result["population_observations"] = population_observations(report, config, False)
        fresh_drain(fixture, directory, "post-client")
        allocator_provenance(fixture, directory, "post-client")
        result["readback"] = native_readback(fixture, config, directory, resp)
        fresh_drain(fixture, directory, "post-readback")
        fixture.snapshot(directory, "after")
        save(directory / "listener-after.json", {str(n): fixture.listener_evidence(n) for n in fixture.nodes})
        save(directory / "host-after.json", host_sample(background))
        result["summary"] = summary_from_report(report)
        result["report_sha256"] = digest(directory / "run/report.json")
        fixture.finish()
        result["workload_complete"] = True
    except BaseException as error:
        result["failure"] = repr(error)
    finally:
        save(directory / "pre-cleanup.json", result)
        try:
            fixture.close()
            children = [dict(pid=p.pid, identity=fixture.identities.get(p.pid), exit_code=p.poll(),
                             absent=not Path("/proc", str(p.pid)).exists()) for p in fixture.children]
            require(all(p["exit_code"] is not None and p["absent"] for p in children), "native owned lifetime did not exit")
            save(directory / "cleanup.json", dict(complete=True, children=children, storage=args.storage))
            result["cleanup_complete"] = True
        except BaseException as error:
            result["cleanup_failure"] = repr(error)
    result["complete"] = result.get("workload_complete", False) and result.get("cleanup_complete", False)
    return result


def redis_trial(row, directory, args, modules, roles):
    benchmark, support, tmpfs, comparison, resp, native_validator, redis_validator = modules
    place = placement(args.smoke)
    class PlacedChildren(resp.Children):
        def launch(self, label, command, executable):
            old = os.sched_getaffinity(0)
            expected_hash = roles["redis"]["binary_sha256"] if label == "client" else args.redis_server_sha256
            require(digest(executable) == expected_hash, "Redis role executable changed before launch")
            expected = place["client_cpus" if label == "client" else "server_cpus"]
            try:
                os.sched_setaffinity(0, expected)
                child = super().launch(label, command, executable)
            finally:
                os.sched_setaffinity(0, old)
            require(digest(Path("/proc", str(child.pid), "exe")) == expected_hash,
                    "executing Redis role binary differs")
            save(directory / (label + "-thread-placement.json"), affinity(child.pid, expected))
            return child
    children = PlacedChildren(directory)
    reservation = socket.socket()
    result = dict(complete=False, client_binary_sha256=roles["redis"]["binary_sha256"],
                  server_binary_sha256=args.redis_server_sha256, operation=row["read_api"],
                  write_operation=row["write_api"], workload=row["workload"], batch_size=row["shared"]["batch_size"])
    try:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
        config, reference = configs(row, [dict(node_id=1, address="127.0.0.1:20160")], 1, f"127.0.0.1:{port}")
        redis_validator.paired_configuration(reference, config)
        save(directory / "requested-config.json", reference)
        (directory / "redis.conf").write_text(f'bind 127.0.0.1\nport {port}\nprotected-mode yes\nsave ""\nappendonly no\ndir {directory}\ndaemonize no\nlogfile ""\nio-threads 1\nmaxclients 512\n')
        reservation.close()
        server = children.launch("redis", [args.redis_server, directory / "redis.conf"], args.redis_server)
        for _ in range(150):
            require(server.poll() is None, "owned Redis exited during startup")
            try:
                if resp.request(reference["address"], [b"PING"]) == b"PONG":
                    break
            except OSError:
                pass
            time.sleep(.02)
        else:
            raise TimeoutError("owned Redis readiness timeout")
        save(directory / "redis-listener.json", resp.listener_identity(server.pid, port))
        def settings(label):
            values = resp.request(reference["address"], [b"CONFIG", b"GET", b"save", b"appendonly", b"io-threads"])
            observed = {values[i].decode(): values[i + 1].decode() for i in range(0, len(values), 2)}
            replication = resp.request(reference["address"], [b"INFO", b"replication"]).decode()
            require(observed == {"save": "", "appendonly": "no", "io-threads": "1"} and
                    "role:master\r\n" in replication and "connected_slaves:0\r\n" in replication, "Redis mode differs")
            save(directory / (label + "-redis-config.json"), dict(configuration=observed, replication=replication,
                 info_server=resp.request(reference["address"], [b"INFO", b"server"]).decode()))
        settings("before")
        background = [r for r in background_inventory() if r["pid"] != server.pid]
        save(directory / "background-before.json", background)
        save(directory / "host-before.json", host_sample(background))
        child = children.launch("client", [args.redis_client_build / "kv9-redis-batch-reference", "--config", directory / "requested-config.json",
                                 "--build-manifest", args.redis_client_build / "build.json", "--output", directory / "run"], args.redis_client_build / "kv9-redis-batch-reference")
        report = observe_client(comparison, child, {"redis": server.pid}, directory,
                                {child.pid: place["client_cpus"], server.pid: place["server_cpus"]}, background, benchmark)
        result["validated"] = redis_validator.validate(directory / "run", args.redis_client_build, directory / "requested-config.json", REDIS_REVISION, require_timing=not args.smoke)
        result["population_observations"] = population_observations(report, reference, True)
        dataset = {}
        for first in range(0, reference["keys"] + 1, 128):
            keys = [f'{reference["run_id"]}:{i:016x}'.encode() for i in range(first, min(reference["keys"] + 1, first + 128))]
            values = resp.request(reference["address"], [b"MGET", *keys])
            require(isinstance(values, list) and len(values) == len(keys), "Redis final MGET count differs")
            dataset.update(zip(keys, values))
        save(directory / "final-dataset.json", {base64.b64encode(k).decode(): base64.b64encode(v).decode() if v is not None else None for k, v in dataset.items()})
        result["readback"] = final_dataset(reference, dataset, resp)
        settings("after")
        save(directory / "redis-thread-placement-after.json", affinity(server.pid, place["server_cpus"]))
        save(directory / "host-after.json", host_sample(background))
        result["summary"] = summary_from_report(report)
        result["report_sha256"] = digest(directory / "run/report.json")
        result["workload_complete"] = True
    except BaseException as error:
        result["failure"] = repr(error)
    finally:
        save(directory / "pre-cleanup.json", result)
        reservation.close()
        result["cleanup_errors"] = children.cleanup()
    result["complete"] = result.get("workload_complete", False) and not result["cleanup_errors"]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--client-source", type=Path, required=True)
    parser.add_argument("--client-build", type=Path, required=True,
                        help="standalone build-workload output containing build.json and kv9-batch-benchmark")
    parser.add_argument("--expected-client-revision", required=True)
    parser.add_argument("--old-server-source", type=Path, default=Path("/tmp/kv9-wal-crc32-table"))
    parser.add_argument("--old-server-build", type=Path, default=Path("/tmp/kv9-wal-crc32-release-first"))
    parser.add_argument("--new-server-source", type=Path, default=Path("/tmp/kv9-indexed-apply-receipts"))
    parser.add_argument("--new-server-build", type=Path, default=Path("/tmp/kv9-indexed-receipt-release-first"))
    parser.add_argument("--redis-client-source", type=Path, default=Path("/tmp/kv9-point-write-measurement-v3"))
    parser.add_argument("--redis-client-build", type=Path, default=Path("/tmp/kv9-point-write-v3-release-first/redis"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--smoke", action="store_true")
    parser.add_argument("--storage", choices=("tmpfs", "wal"), default="tmpfs")
    parser.add_argument("--isolation-snapshot", type=Path)
    parser.add_argument("--redis-server", type=Path, default=Path("/usr/bin/redis-server"))
    parser.add_argument("--resp-helper", type=Path, default=Path("/tmp/kv9-redis-batch-reference-preparation/run_fixture.py"))
    args = parser.parse_args()
    def interrupted(signum, _frame):
        raise RuntimeError("matched driver interrupted by signal " + str(signum))
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    for name in ("client_source", "client_build", "old_server_source", "old_server_build", "new_server_source",
                 "new_server_build", "redis_client_source", "redis_client_build", "output", "redis_server", "resp_helper"):
        path = getattr(args, name)
        # Redis is a multicall binary: argv[0] must remain redis-server even
        # when the installed symlink points to redis-check-rdb.
        setattr(args, name, Path(os.path.abspath(path)) if name == "redis_server" else path.resolve())
    require(not args.output.exists(), "output directory must be new; original attempts are never overwritten")
    for source in (args.client_source, args.old_server_source, args.new_server_source, args.redis_client_source):
        require(not args.output.is_relative_to(source), "output must not mutate a source worktree")
    for retained in (args.client_build, args.old_server_build, args.new_server_build, args.redis_client_build):
        require(not args.output.is_relative_to(retained), "output must not mutate retained build artifacts")
    require(digest(args.resp_helper) == RESP_HELPER_SHA, "proven RESP/data helper source differs")
    isolation = None
    if not args.smoke:
        require(args.isolation_snapshot is not None, "timed diagnostic requires supplied isolation observations")
    if args.isolation_snapshot is not None:
        isolation = read(args.isolation_snapshot)
        require(isolation.get("complete") is True and isolation.get("measured_client_cpus") == [0, 1] and
                isolation.get("measured_server_cpus") == [2, 3, 4, 5], "supplied isolation snapshot is incomplete or has different timing placement")
        if "driver_sha256" in isolation:
            require(isolation["driver_sha256"] == digest(__file__), "isolation wrapper bound a different driver")
    roles = verify_inputs(args)
    # Every helper source is already present in the verified clean client
    # inventory before any import. The retained RESP module is used only for
    # its generic lifecycle/protocol helpers; its development main is not called.
    sys.path.insert(0, str(args.client_source / "scripts"))
    import benchmark
    import batch_benchmark_report as native_validator
    redis_validator = module("redis_point_report", args.redis_client_source / "scripts/redis_batch_report.py")
    support = module("native_process_support", args.client_source / "scripts/native-batch-e2e.py")
    tmpfs = module("tmpfs_support", args.client_source / "scripts/tmpfs-redis-diagnostic.py")
    comparison = module("reference_resource_support", args.client_source / "scripts/redis-comparison.py")
    resp = module("retained_resp_support", args.resp_helper)
    modules = benchmark, support, tmpfs, comparison, resp, native_validator, redis_validator
    cargo = verify_client_cargo(args, native_validator)
    args.redis_server_sha256 = digest(args.redis_server)
    helper_hashes = {str(p): digest(p) for p in (Path(__file__), args.resp_helper)}
    for loaded in list(sys.modules.values()) + [support, tmpfs, comparison, redis_validator]:
        filename = getattr(loaded, "__file__", None)
        if not filename:
            continue
        path = Path(filename).resolve()
        for role, source in (("client", args.client_source), ("redis", args.redis_client_source)):
            if path.is_relative_to(source):
                relative = str(path.relative_to(source))
                require(relative in roles[role]["source"]["sources"], "imported helper is outside source inventory")
                require(digest(path) == roles[role]["source"]["sources"][relative], "imported helper bytes differ")
                helper_hashes[str(path)] = digest(path)
    rows = make_plan(args.smoke)
    args.output.mkdir(parents=True)
    plan = dict(version=2, protocol_id=PROTOCOL_ID, workload_cells=WORKLOAD_CELLS, storage_guards=SPACE_GUARDS,
                concurrency_points=[1, 64], complete=False, client_revision=args.expected_client_revision,
                smoke=args.smoke, placement=placement(args.smoke), role_bindings=roles, client_cargo=cargo,
                scope="12-cohort write-only v3 c1/c64 correctness smoke only; no timing acceptance" if args.smoke else
                      "24-cohort shared-host v3 point PUT/BatchPut(64) c1/c64 selection screen; not full-workload acceptance, no exclusive host or equivalent durability",
                storage=args.storage, kv9_durability=tmpfs.VOLATILE if args.storage == "tmpfs" else "Normal disk WAL and quorum; no Redis durability equivalence",
                redis_durability="Standalone memory; save/AOF disabled; no replicas", comparable_durability=False,
                isolation_snapshot=isolation, isolation_snapshot_sha256=digest(args.isolation_snapshot) if args.isolation_snapshot else None,
                redis_server_sha256=args.redis_server_sha256, helper_hashes=helper_hashes,
                fixed_rates=None, cohort_inventory=rows, attempts=[])
    (args.output / "driver.py").write_bytes(Path(__file__).read_bytes())
    save(args.output / "plan.json", plan)
    save(args.output / "matrix.json", plan)
    try:
        retained = args.output / "retained-inputs"
        retained.mkdir()
        for role, binding in roles.items():
            directory = retained / role
            directory.mkdir()
            for number, (path, expected) in enumerate(binding["evidence"].items()):
                destination = directory / f"{number:02d}-{Path(path).name}"
                destination.write_bytes(Path(path).read_bytes())
                require(digest(destination) == expected, "retained role input copy differs")
        helpers = retained / "helpers"
        helpers.mkdir()
        for number, (path, expected) in enumerate(helper_hashes.items()):
            destination = helpers / f"{number:02d}-{Path(path).name}"
            destination.write_bytes(Path(path).read_bytes())
            require(digest(destination) == expected, "retained helper copy differs")
        benchmark.host(args.output, False)
        if args.isolation_snapshot:
            (args.output / "isolation-snapshot.json").write_bytes(args.isolation_snapshot.read_bytes())
        for index, row in enumerate(rows):
            directory = args.output / f'{index:03d}-{row["arm"]}-{row["shared"]["run_id"]}'
            directory.mkdir()
            entry = dict(index=index, descriptor=row, directory=str(directory), complete=False)
            plan["attempts"].append(entry)
            save(args.output / "matrix.json", plan)
            space = storage_observation(directory)
            save(directory / "storage-preflight.json", space)
            try:
                require_storage(space, "preflight")
            except ValueError:
                save(directory / "storage-guard-intervention.json", dict(phase="preflight", observation=space,
                     thresholds=SPACE_GUARDS["preflight"], timing_complete=False))
                raise
            result = (native_trial if row["target"] == "kv9" else redis_trial)(row, directory, args, modules, roles)
            entry.update(result)
            save(directory / "summary.json", result)
            save(args.output / "matrix.json", plan)
            require(result["complete"], "retained matched cohort failed: " + directory.name)
            print("PASS: matched cohort", directory.name, flush=True)
        require(verify_inputs(args) == roles, "role source/build identity changed during matrix")
        require(verify_client_cargo(args, native_validator) == cargo, "client Cargo identity changed during matrix")
        require(all(digest(p) == h for p, h in helper_hashes.items()), "helper changed during matrix")
        require(digest(args.redis_server) == args.redis_server_sha256, "Redis server executable changed during matrix")
        plan["complete"] = True
    except BaseException as error:
        plan["failure"] = repr(error)
        raise
    finally:
        plan["ended_unix_ns"] = time.time_ns()
        save(args.output / "matrix.json", plan)
    print("PASS: all matched cohorts complete; independent final audit remains required", flush=True)


if __name__ == "__main__":
    main()
