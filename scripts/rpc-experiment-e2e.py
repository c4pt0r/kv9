#!/usr/bin/env python3
"""Check both public transports against three real Raft voters and leader restart."""
import argparse
import hashlib
import json
from pathlib import Path
import socket
import time

from benchmark import Fixture, process_sample, read, save
from workload_report import validate


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


class RpcFixture(Fixture):
    def __init__(self, out, build):
        super().__init__(out, "wal", build, dict(client_cpus=[6, 7], server_cpus=list(range(8, 32)),
                                               separated=True, exclusive_host=False))
        self.rpc_sockets = []
        for _ in range(3):
            stream = socket.socket()
            stream.bind(("127.0.0.1", 0))
            self.rpc_sockets.append(stream)
        self.rpc_addresses = {n: f"127.0.0.1:{stream.getsockname()[1]}"
                              for n, stream in enumerate(self.rpc_sockets, 1)}
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

    def launch(self, command, logfile, client=False):
        values = list(map(str, command))
        if len(values) > 1 and values[1] == "start":
            node = int(values[values.index("--node-id") + 1])
            self.rpc_sockets[node - 1].close()
            self.env["KV9_RPC_EXPERIMENT_ADDR"] = self.rpc_addresses[node]
        try:
            return super().launch(command, logfile, client)
        finally:
            self.env.pop("KV9_RPC_EXPERIMENT_ADDR", None)

    def restart(self, node):
        self.nodes[node] = self.launch(
            [self.build / "kv9", "start", "--node-id", node, "--addr", self.addresses[node],
             "--data-dir", self.out / "data" / f"n{node}"], self.out / f"n{node}-restart.log")
        self.wait("restarted original directory", self.leader)

    def close(self):
        super().close()
        for stream in self.rpc_sockets:
            stream.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--build", type=Path, required=True)
    args = parser.parse_args()
    out, build = args.output.resolve(), args.build.resolve()
    out.mkdir(parents=True, exist_ok=False)
    manifest = read(build / "build.json")
    server_hash = digest(build / "kv9")
    if server_hash != manifest["binary_sha256"] or manifest["features"] != ["rpc-experiment"]:
        raise ValueError("incorrect RPC experiment build")
    workload_manifest = read(build / "workload/build.json")
    if (digest(build / "workload/build.json") != manifest["workload_build_sha256"] or
        any(workload_manifest[key] != manifest[key] for key in ("revision", "dirty", "source_tree_sha256", "profile", "rustc"))):
        raise ValueError("workload and server source/build identities differ")
    fixture = RpcFixture(out / "fixture", build)
    summary = dict(complete=False, cases=[], lifetimes=[], server_sha256=server_hash,
                   build_sha256=digest(build / "build.json"),
                   scope="Local three-process Raft transport correctness with ordinary WAL storage; no throughput, Chaos Mesh, cross-host or power-loss claim")

    def capture():
        for node, process in fixture.nodes.items():
            state = fixture.state(node)
            if not state:
                raise ValueError("status writer identity unavailable")
            identity = process_sample(process.pid)
            actual = digest(Path("/proc", str(process.pid), "exe"))
            if actual != server_hash or str(identity["start_ticks"]) != state["process_start_ticks"]:
                raise ValueError("executing voter differs from bound artifact/lifetime")
            if not any(row["pid"] == process.pid for row in summary["lifetimes"]):
                summary["lifetimes"].append(dict(node_id=node, **identity, executable_sha256=actual,
                                                  boot_id=fixture.boot_id, status=state))

    def success_progress(folder):
        try:
            phases = read(folder / "progress.json")["progress"]["phase_successes"]
            return sum(row["get"] + row["put"] + row["delete"] for row in phases if row["phase"] == "measure")
        except (OSError, KeyError):
            return 0

    try:
        fixture.start()
        capture()
        for transport in ("tonic_unary", "tarpc_tcp"):
            name = transport.replace("_", "-")
            folder = out / name
            leader = fixture.wait("leader before keyspace creation", fixture.leader)
            receipt = fixture.command([build / "kv9", "client", "create-keyspace", "--addr",
                                       fixture.addresses[leader], "--name", name, "--api-type", "raw"])
            keyspace = int(dict(line.split("=", 1) for line in receipt.splitlines())["keyspace_id"])
            endpoints = fixture.rpc_addresses if transport == "tarpc_tcp" else fixture.addresses
            configuration = dict(version=1, rpc_transport=transport,
                client=dict(version=1, peers=[dict(node_id=n, address=endpoints[n]) for n in fixture.nodes],
                            keyspace_id=keyspace, epoch_conf_ver=1, epoch_version=1, max_in_flight=4,
                            max_attempts=6, deadline_ms=1500, retry_backoff_ms=100),
                mode="correctness", run_id=name, keyspace_name=name, seed=40, workers=4, keys=4,
                value_bytes=128, mix=dict(get=50, put=40, delete=10), warmup_operations=12,
                max_operations=6000, measure_ms=30000, interval_ms=20, history_bytes=64 * 1024 * 1024)
            config_path = out / (name + "-config.json")
            stop_path = out / (name + ".stop")
            save(config_path, configuration)
            workload = fixture.launch([build / "workload/kv9-workload", "--config", config_path,
                "--build-manifest", build / "workload/build.json", "--output", folder,
                "--stop-file", stop_path], out / (name + ".log"), client=True)
            fixture.workloads.append(workload)
            fixture.wait("measured success before leader loss", lambda: success_progress(folder) >= 20, 20)
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
                         lambda: success_progress(folder) >= before + 20, 20)
            fault_end = time.time_ns()
            fixture.restart(lost)
            capture()
            restart_start = time.time_ns()
            recovered_success = success_progress(folder)
            fixture.wait("successful operations after original-directory restart",
                         lambda: success_progress(folder) >= recovered_success + 20, 20)
            stop_path.touch()
            restart_end = time.time_ns()
            code = workload.wait(timeout=90)
            if code:
                raise ValueError(f"{name} workload failed; see retained log")
            checked = validate(folder, build / "workload", seconds=60)
            if not checked["full_history_independently_checked"]:
                raise ValueError("complete history was not independently checked")
            report = read(folder / "report.json")
            records = [json.loads(line) for line in (folder / "history.jsonl").read_text().splitlines()]
            invocations = {row["id"]: row for row in records if row["type"] == "invoke"}
            offset = report["wall_anchor_unix_ns"] - report["wall_anchor_monotonic_ns"]
            windows = []
            for label, start, end in (("voter-lost", fault_start, fault_end),
                                      ("voter-restarted", restart_start, restart_end)):
                successes = {"get": [], "put": [], "delete": []}
                for returned in records:
                    if returned["type"] != "return" or returned["outcome"] != "ok":
                        continue
                    invoked = invocations[returned["id"]]
                    if (invoked["phase"] == "measure" and start + 1_000_000 <= offset + invoked["monotonic_ns"]
                        and offset + returned["monotonic_ns"] <= end - 1_000_000):
                        successes[invoked["op"]].append(invoked["id"])
                if not successes["get"] or not successes["put"]:
                    raise ValueError("missing successful GET/PUT fully within " + label)
                windows.append(dict(label=label, start_unix_ns=start, end_unix_ns=end, successes=successes))
            save(out / (name + "-checked.json"), checked)
            summary["cases"].append(dict(transport=transport, lost_leader=lost, new_leader=new_leader,
                fault_start_unix_ns=fault_start, successful_after_kill_boundary=before,
                successful_after_restart_boundary=recovered_success, windows=windows,
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
                if all(all(state.get(key) == "0" for key in zero) and
                       state.get("raft_async_apply_stopped") == state.get("raft_async_read_stopped") == "false"
                       for state in states.values()):
                    return states
                return None
            summary["cases"][-1]["drained_status"] = fixture.wait("bound public/read/apply ledgers drain", drained, 15)
            summary["cases"][-1]["drain_freshness"] = dict(started_unix_ns=drain_started,
                baseline=baseline, first_advances=first_exports, completed_unix_ns=time.time_ns())
            save(out / "summary.json", summary)
            print(f"PASS: {transport} full history with leader loss and original-directory restart", flush=True)
        fixture.finish()
        summary["workloads_complete"] = True
    except Exception as error:
        summary["failure"] = str(error)
        raise
    finally:
        fixture.close()
        for identity in summary["lifetimes"]:
            try:
                ticks = Path("/proc", str(identity["pid"]), "stat").read_text().rsplit(")", 1)[1].split()[19]
                identity["exited"] = ticks != str(identity["start_ticks"])
            except FileNotFoundError:
                identity["exited"] = True
        summary["owned_lifetimes_exited"] = all(row["exited"] for row in summary["lifetimes"])
        summary["owned_children_exited"] = all(process.poll() is not None for process in fixture.children)
        summary["binary_unchanged"] = digest(build / "kv9") == server_hash
        summary["complete"] = (summary.get("workloads_complete", False) and summary["owned_lifetimes_exited"]
                               and summary["owned_children_exited"] and summary["binary_unchanged"])
        if not summary["complete"]:
            summary.setdefault("failure", "runtime cleanup or binary identity failed")
        save(out / "summary.json", summary)
    if not summary["complete"]:
        raise ValueError("runtime cleanup or binary identity failed")
    print("PASS: both RPC transports retain checked histories and progress across leader restart", flush=True)


if __name__ == "__main__":
    main()
