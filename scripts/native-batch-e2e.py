#!/usr/bin/env python3
"""Check native atomic batch histories across leader loss and restart with three WAL voters."""
import argparse
import hashlib
import json
from pathlib import Path
import time

from benchmark import Fixture, process_sample, read, save
from batch_workload_report import validate


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


class StreamingFixture(Fixture):
    def __init__(self, out, build):
        super().__init__(out, "wal", build, dict(client_cpus=[6, 7], server_cpus=list(range(8, 32)),
                                               separated=True, exclusive_host=False))
        self.boot_id = Path("/proc/sys/kernel/random/boot_id").read_text().strip()

    def state(self, node):
        state = super().state(node)
        if not state:
            return {}
        try:
            ticks = Path("/proc", str(self.nodes[node].pid), "stat").read_text().rsplit(")", 1)[1].split()[19]
        except (OSError, IndexError):
            return {}
        if state.get("process_start_ticks") != ticks or state.get("process_boot_id") != self.boot_id:
            return {}
        return state

    def restart(self, node):
        self.nodes[node] = self.launch(
            [self.build / "kv9", "start", "--node-id", node, "--addr", self.addresses[node],
             "--data-dir", self.out / "data" / f"n{node}"], self.out / f"n{node}-restart.log")
        self.wait("restarted original directory", self.leader)

    def listener_evidence(self, node):
        """The production executable owns the ordinary advertised listening socket."""
        process = self.nodes[node]
        forbidden = {b"KV9_RPC_EXPERIMENT_ADDR", b"KV9_GRPC_STREAM_EXPERIMENT_ADDR"}
        keys = {entry.partition(b"=")[0] for entry in
                Path("/proc", str(process.pid), "environ").read_bytes().split(b"\0")}
        if keys & forbidden:
            raise ValueError("normal endpoint fixture inherited an experimental listener override")
        owned_inodes = set()
        for fd in Path("/proc", str(process.pid), "fd").iterdir():
            try:
                target = str(fd.readlink())
            except FileNotFoundError:
                continue
            if target.startswith("socket:[") and target.endswith("]"):
                owned_inodes.add(target[8:-1])
        listeners = []
        for table in ("tcp", "tcp6"):
            for line in Path("/proc", str(process.pid), "net", table).read_text().splitlines()[1:]:
                fields = line.split()
                if fields[3] == "0A" and fields[9] in owned_inodes:
                    listeners.append(dict(table=table, local_address=fields[1], inode=fields[9]))
        port = int(self.addresses[node].rsplit(":", 1)[1])
        if (len(listeners) != 1 or listeners[0]["table"] != "tcp" or
            listeners[0]["local_address"] != f"0100007F:{port:04X}"):
            raise ValueError("normal voter must own exactly its ordinary loopback listener")
        return dict(advertised_endpoint=self.addresses[node], listener_inodes=listeners,
                    experimental_environment_absent=True, observed_unix_ns=time.time_ns(), pid=process.pid)


def normal_build(build):
    """Use the existing standalone build-benchmark layout, never its calibration server."""
    manifest = read(build / "build.json")
    workload = read(build / "workload/build.json")
    sources = read(build / "workload/sources.json")
    if manifest["dirty"] is not False or workload["dirty"] is not False:
        raise ValueError("normal transport acceptance requires a clean frozen source")
    if (digest(build / "kv9") != manifest["binaries"]["kv9"]["sha256"] or
        digest(build / "workload/kv9-batch-workload") != workload["binary_sha256"] or
        digest(build / "workload/build.json") != manifest["workload_build_sha256"] or
        any(workload[key] != manifest[key] for key in ("revision", "dirty", "source_tree_sha256", "profile", "rustc")) or
        sources["sources"] != manifest["sources"] or sources["revision"] != manifest["revision"] or
        sources["dirty"] is not False or sources["source_tree_sha256"] != manifest["source_tree_sha256"] or
        hashlib.sha256(json.dumps(sources["sources"], sort_keys=True, separators=(",", ":")).encode()).hexdigest()
            != manifest["source_tree_sha256"]):
        raise ValueError("standalone server/client source and executable provenance differs")
    attestation = {}
    for name, path, command, wanted in (
        ("server", build / "kv9-cargo.jsonl", manifest["binaries"]["kv9"]["command"],
         ("kv9", "kv9_engine", "kv9_raft", "kv9_server")),
        ("workload", build / "workload/cargo.jsonl", sources["command"],
         ("kv9-batch-workload", "kv9_engine", "kv9_raft", "kv9_server")),
    ):
        if any(flag in command for flag in ("--features", "--all-features", "--no-default-features")):
            raise ValueError("normal RPC acceptance requires the standalone default-feature build")
        records = [json.loads(line) for line in path.read_text().splitlines()]
        selected = {}
        for target in wanted:
            rows = [r for r in records if r.get("reason") == "compiler-artifact" and
                    r.get("target", {}).get("name") == target]
            if len(rows) != 1 or rows[0]["features"] != []:
                raise ValueError("missing, ambiguous or non-default Cargo artifact: " + target)
            selected[target] = rows[0]
        executable = selected[wanted[0]]
        if not executable.get("executable") or "bin" not in executable["target"]["kind"]:
            raise ValueError("selected Cargo artifact is not the standalone executable")
        attestation[name] = dict(command=command, artifacts=selected, cargo_sha256=digest(path))
    return manifest, workload, attestation


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--build", type=Path, required=True)
    args = parser.parse_args()
    out, build = args.output.resolve(), args.build.resolve()
    out.mkdir(parents=True, exist_ok=False)
    helpers = ("native-batch-e2e.py", "benchmark.py", "batch_workload_report.py",
               "workload_report.py", "root_provision.py", "wal_layout.py", "history/checker.py")
    try:
        manifest, workload_manifest, feature_attestation = normal_build(build)
        helper_sources = {name: digest(Path(__file__).parent / name) for name in helpers}
        if any(manifest["sources"].get("scripts/" + name) != value
               for name, value in helper_sources.items()):
            raise ValueError("executing helper differs from the frozen build source")
    except Exception as error:
        save(out / "summary.json", dict(complete=False, stage="build-attestation", failure=str(error)))
        raise
    server_hash = digest(build / "kv9")
    bound_paths = [build / name for name in ("build.json", "kv9", "kv9-cargo.jsonl",
        "workload/build.json", "workload/kv9-batch-workload", "workload/cargo.jsonl", "workload/sources.json")]
    bound_hashes = {str(path): digest(path) for path in bound_paths}
    fixture = StreamingFixture(out / "fixture", build)
    summary = dict(complete=False, cases=[], lifetimes=[], server_sha256=server_hash,
        build_sha256=digest(build / "build.json"), feature_attestation=feature_attestation,
        bound_artifacts=bound_hashes, source_revision=manifest["revision"],
        helper_sources=helper_sources,
        scope="Atomic native BatchGet/BatchPut with overlapping point operations: normal standalone build and shared endpoint, default streaming and explicit unary. Local three-process WAL correctness with leader loss/restart; no throughput, actual Chaos Mesh, cross-host or power-loss claim")
    save(out / "summary.json", summary)

    def capture():
        for node, process in fixture.nodes.items():
            state = fixture.state(node)
            if not state:
                raise ValueError("status writer identity unavailable")
            identity = process_sample(process.pid)
            actual = digest(Path("/proc", str(process.pid), "exe"))
            if actual != server_hash or str(identity["start_ticks"]) != state["process_start_ticks"]:
                raise ValueError("executing voter differs from bound artifact/lifetime")
            if not any((row["pid"], row["start_ticks"]) == (process.pid, identity["start_ticks"])
                       for row in summary["lifetimes"]):
                summary["lifetimes"].append(dict(node_id=node, **identity, executable_sha256=actual,
                    boot_id=fixture.boot_id, status=state,
                    ordinary_listener=fixture.listener_evidence(node)))

    def success_progress(folder):
        zero = {kind: 0 for kind in ("get", "put", "delete", "batch_get", "batch_put")}
        try:
            for row in read(folder / "progress.json")["successes"]:
                if row["phase"] == "measure":
                    zero[row["operation"]] += row["calls"]
        except (OSError, KeyError):
            pass
        return zero

    def progressed(folder, before, amount=12):
        now = success_progress(folder)
        return all(now[kind] >= before[kind] + amount for kind in ("batch_get", "batch_put"))

    try:
        fixture.start()
        capture()
        for transport in ("tonic_stream", "tonic_unary"):
            name = "batch-" + transport.replace("_", "-")
            folder = out / name
            leader = fixture.wait("leader before keyspace creation", fixture.leader)
            receipt = fixture.command([build / "kv9", "client", "create-keyspace", "--addr",
                                       fixture.addresses[leader], "--name", name, "--api-type", "raw"])
            keyspace = int(dict(line.split("=", 1) for line in receipt.splitlines())["keyspace_id"])
            configuration = dict(version=1,
                client=dict(version=1, peers=[dict(node_id=n, address=fixture.addresses[n]) for n in fixture.nodes],
                    keyspace_id=keyspace, epoch_conf_ver=1, epoch_version=1, max_in_flight=4,
                    max_attempts=6, deadline_ms=1500, retry_backoff_ms=100),
                run_id=name, keyspace_name=name, workers=4, keys=4, batch_size=8,
                value_bytes=128, seed=40, mix=[10, 10, 10, 35, 35], max_calls=2000,
                measure_ms=30000, interval_ms=20, history_bytes=128 * 1024 * 1024)
            # Omission exercises the normal client default; the output must record its selection.
            if transport == "tonic_unary":
                configuration["rpc_transport"] = transport
            config_path = out / (name + "-config.json")
            stop_path = out / (name + ".stop")
            save(config_path, configuration)
            workload = fixture.launch([build / "workload/kv9-batch-workload", "--config", config_path,
                "--build-manifest", build / "workload/build.json", "--output", folder,
                "--stop-file", stop_path], out / (name + ".log"), client=True)
            fixture.workloads.append(workload)
            fixture.wait("measured success before leader loss", lambda: progressed(folder, {"batch_get":0,"batch_put":0}), 20)
            client_identity = process_sample(workload.pid)
            client_identity["executable_sha256"] = digest(Path("/proc", str(workload.pid), "exe"))
            if client_identity["executable_sha256"] != workload_manifest["binary_sha256"]:
                raise ValueError("executing workload binary differs from manifest")
            lost = fixture.wait("leader selected for termination", fixture.leader)
            dead = fixture.nodes.pop(lost)
            dead.kill()
            dead.wait(timeout=10)
            fault_start = time.time_ns()
            before = success_progress(folder)
            new_leader = fixture.wait("surviving quorum elects leader", fixture.leader, 20)
            if new_leader == lost:
                raise ValueError("leader did not change")
            fixture.wait("continued successful operations under voter loss",
                         lambda: progressed(folder, before), 20)
            fault_end = time.time_ns()
            fixture.restart(lost)
            capture()
            restart_start = time.time_ns()
            recovered_success = success_progress(folder)
            fixture.wait("successful operations after original-directory restart",
                         lambda: progressed(folder, recovered_success), 20)
            stop_path.touch()
            restart_end = time.time_ns()
            code = workload.wait(timeout=90)
            if code:
                raise ValueError(f"{name} workload failed; see retained log")
            checked = validate(folder, build / "workload", expected_revision=manifest["revision"], seconds=60)
            if not checked["full_history_independently_checked"]:
                raise ValueError("complete history was not independently checked")
            report = read(folder / "report.json")
            if report["version"] != 1 or report["configuration"] != dict(configuration, rpc_transport=transport):
                raise ValueError("normal workload did not record its exact native version 1 transport/configuration")
            records = [json.loads(line) for line in (folder / "history.jsonl").read_text().splitlines()]
            invocations = {row["id"]: row for row in records if row["type"] == "invoke"}
            offset = report["wall_anchor_unix_ns"] - report["wall_anchor_monotonic_ns"]
            windows = []
            for label, start, end in (("voter-lost", fault_start, fault_end),
                                      ("voter-restarted", restart_start, restart_end)):
                successes = {"get": [], "put": [], "delete": [], "batch_get": [], "batch_put": []}
                for returned in records:
                    if returned["type"] != "return" or returned["outcome"] != "ok":
                        continue
                    invoked = invocations[returned["id"]]
                    if (invoked["phase"] == "measure" and start + 1_000_000 <= offset + invoked["monotonic_ns"]
                        and offset + returned["monotonic_ns"] <= end - 1_000_000):
                        successes[invoked["op"]].append(invoked["id"])
                if not successes["batch_get"] or not successes["batch_put"]:
                    raise ValueError("missing successful atomic BatchGet/BatchPut fully within " + label)
                windows.append(dict(label=label, start_unix_ns=start, end_unix_ns=end, successes=successes))
            save(out / (name + "-checked.json"), checked)
            summary["cases"].append(dict(transport=transport, lost_leader=lost, new_leader=new_leader,
                fault_start_unix_ns=fault_start, successful_after_kill_boundary=before,
                successful_after_restart_boundary=recovered_success, windows=windows,
                input_transport_explicit="rpc_transport" in configuration,
                client_identity=client_identity, checked=checked))
            drain_started = time.time_ns()
            baseline = {str(n): fixture.state(n) for n in fixture.nodes}
            if not all(baseline.values()):
                raise ValueError("drain baseline lacks bound status")
            first_exports = {}
            def drained():
                states = {str(n): fixture.state(n) for n in fixture.nodes}
                for node, state in states.items():
                    if not state or any(state.get(key) != baseline[node][key] for key in
                                        ("pid", "process_start_ticks", "process_boot_id")):
                        raise ValueError("drain status writer changed or became unavailable")
                    exports = int(state["metrics_export_successes"])
                    if exports > int(baseline[node]["metrics_export_successes"]) and node not in first_exports:
                        first_exports[node] = dict(observed_unix_ns=time.time_ns(), status=state)
                if any(node not in first_exports or int(state["metrics_export_successes"]) <=
                       int(first_exports[node]["status"]["metrics_export_successes"]) for node, state in states.items()):
                    return None
                zero = ("public_rpc_in_flight", "public_rpc_queued", "public_rpc_running", "public_rpc_encoded_bytes",
                        "raft_async_apply_queued", "raft_async_apply_in_flight", "raft_async_read_queued",
                        "raft_async_read_active", "raft_async_read_in_flight", "raft_async_read_active_groups")
                if all(state.get("bootstrap_state") == "Serving" and state.get("fatal") == "" and
                       all(state.get(key) == "0" for key in zero) and
                       state.get("raft_async_apply_stopped") == state.get("raft_async_read_stopped") == "false"
                       for state in states.values()):
                    return states
                return None
            summary["cases"][-1]["drained_status"] = fixture.wait("bound public/read/apply ledgers drain", drained, 15)
            summary["cases"][-1]["drain_freshness"] = dict(started_unix_ns=drain_started,
                baseline=baseline, first_advances=first_exports, completed_unix_ns=time.time_ns())
            save(out / "summary.json", summary)
            print(f"PASS: {transport} full atomic batch history with leader loss and original-directory restart", flush=True)
        fixture.finish()
        summary["workloads_complete"] = True
    except Exception as error:
        summary["failure"] = str(error)
        raise
    finally:
        # Persist the original failure even if cleanup or identity collection
        # also fails. A child created before launch attestation remains owned.
        save(out / "summary.json", summary)
        cleanup_errors = []
        try:
            fixture.close()
        except Exception as error:
            cleanup_errors.append("fixture cleanup: " + str(error))
            for process in fixture.children:
                try:
                    if process.poll() is None:
                        process.kill()
                    process.wait(timeout=10)
                except Exception as child_error:
                    cleanup_errors.append(f"child {process.pid}: {child_error}")
        for identity in summary["lifetimes"]:
            try:
                ticks = Path("/proc", str(identity["pid"]), "stat").read_text().rsplit(")", 1)[1].split()[19]
                identity["exited"] = ticks != str(identity["start_ticks"])
            except FileNotFoundError:
                identity["exited"] = True
            except (OSError, IndexError) as error:
                identity["exited"] = False
                cleanup_errors.append("lifetime readback: " + str(error))
        summary["owned_lifetimes_exited"] = all(row["exited"] for row in summary["lifetimes"])
        summary["owned_children"] = [dict(**fixture.identities.get(process.pid, {"pid": process.pid}),
                                         identity_captured=process.pid in fixture.identities,
                                         exit_code=process.poll())
                                     for process in fixture.children]
        summary["owned_children_exited"] = all(process.poll() is not None for process in fixture.children)
        summary["owned_children_identified"] = all(row["identity_captured"] for row in summary["owned_children"])
        try:
            summary["bound_artifacts_unchanged"] = all(digest(Path(path)) == expected
                for path, expected in bound_hashes.items())
            summary["helpers_unchanged"] = all(digest(Path(__file__).parent / name) == value
                for name, value in helper_sources.items())
        except OSError as error:
            summary["bound_artifacts_unchanged"] = summary["helpers_unchanged"] = False
            cleanup_errors.append("artifact readback: " + str(error))
        summary["cleanup_errors"] = cleanup_errors
        summary["complete"] = (summary.get("workloads_complete", False) and summary["owned_lifetimes_exited"]
                               and summary["owned_children_exited"] and summary["owned_children_identified"]
                               and summary["bound_artifacts_unchanged"] and summary["helpers_unchanged"]
                               and not cleanup_errors)
        if not summary["complete"]:
            summary.setdefault("failure", "runtime cleanup or binary identity failed")
        save(out / "summary.json", summary)
    if not summary["complete"]:
        raise ValueError("runtime cleanup or binary identity failed")
    print("PASS: native atomic batches and overlapping point calls retain checked histories across leader restart", flush=True)


if __name__ == "__main__":
    main()
