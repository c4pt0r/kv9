#!/usr/bin/env python3
"""Run the real workload executable against three local replicas and owned MinIO."""
from root_provision import prepare_stores
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import urllib.request

from workload_report import validate
from workload_e2e_support import wait as wait_for
from wal_layout import read_checkpoint, read_layout

ROOT = Path(__file__).resolve().parents[1]
MINIO = "quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--server", type=Path, required=True)
    parser.add_argument("--expected-revision")
    args = parser.parse_args()
    output, build, server = args.output.resolve(), args.build.resolve(), args.server.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = {k: v for k, v in os.environ.items() if not k.startswith("KV9_")}
    env.update(KV9_STORAGE="minio", KV9_BOOTSTRAP_TOKEN=secrets.token_hex(24),
               KV9_CLUSTER_TOKEN=secrets.token_hex(24), KV9_CLIENT_TOKEN=secrets.token_hex(24),
               KV9_FLUSH_INTERVAL_MS="100")
    env["KV9_CLIENT_TOKENS"] = "workload-e2e=" + env["KV9_CLIENT_TOKEN"]
    nodes, workloads, logs, sockets = {}, [], [], []
    container = "kv9-workload-e2e-" + secrets.token_hex(6)
    secret_file = output / "minio.env"
    created = False
    summary = dict(version=1, topology="single-host three-process MinIO functional E2E", minio_image=MINIO,
                   cpu_clock_ticks_per_second=os.sysconf("SC_CLK_TCK"), cases=[])

    def run(command, timeout=30):
        result = subprocess.run(list(map(str, command)), env=env, text=True, capture_output=True, timeout=timeout)
        if result.returncode:
            raise RuntimeError(f"{command[0:2]} failed: {result.stdout}{result.stderr}")
        return result.stdout.strip()

    def wait(label, condition, seconds=45, workload=None):
        return wait_for(label, condition, nodes, seconds, workload)

    def state(node):
        try:
            fields = dict(line.split("=", 1) for line in (output / f"n{node}/status").read_text().splitlines())
            return fields if int(fields["pid"]) == nodes[node].pid else {}
        except (OSError, ValueError, KeyError):
            return {}

    def leader():
        states = {node: state(node) for node in nodes}
        if any(s.get("bootstrap_state") != "Serving" or s.get("fatal") for s in states.values()):
            return None
        chosen = {s.get("leader_id") for s in states.values()}
        if len(chosen) != 1 or chosen & {None, "0"}:
            return None
        node = int(chosen.pop())
        return node if node in states and states[node].get("role") == "leader" else None

    def start(node):
        log = (output / f"n{node}.log").open("a")
        logs.append(log)
        nodes[node] = subprocess.Popen([str(server), "start", "--node-id", str(node), "--addr", addresses[node],
            "--data-dir", str(output / f"n{node}")], env=env, stdout=log, stderr=log)

    def client(command, *extra):
        node = wait("agreed serving leader", leader)
        # Setup writes are never retried by this harness. Any uncertain result
        # fails with artifacts; it cannot become a hidden extra invocation.
        return run([server, "client", command, "--addr", addresses[node], *extra])

    def config(name, mode="correctness", peers=None):
        created_keyspace = client("create-keyspace", "--name", name, "--api-type", "raw")
        keyspace = int(dict(line.split("=", 1) for line in created_keyspace.splitlines())["keyspace_id"])
        return dict(version=1, client=dict(version=1, peers=peers or [dict(node_id=n, address=addresses[n]) for n in nodes],
                    keyspace_id=keyspace, epoch_conf_ver=1, epoch_version=1, max_in_flight=4, max_attempts=6,
                    # Six immediate refusals previously exhausted the attempt
                    # budget in 36 ms. Space this functional fixture's retries
                    # across 500 ms; the original 1500 ms deadline still applies.
                    deadline_ms=1500, retry_backoff_ms=100), mode=mode, run_id=name, keyspace_name=name,
                    seed=40, workers=4, keys=4, value_bytes=128, mix=dict(get=50, put=40, delete=10),
                    warmup_operations=12, max_operations=250, measure_ms=2000, interval_ms=5,
                    history_bytes=64 * 1024 * 1024 if mode == "correctness" else 0)

    def launch(name, configuration, extra=()):
        path = output / (name + "-config.json")
        path.write_text(json.dumps(configuration) + "\n")
        log = (output / (name + ".log")).open("w")
        logs.append(log)
        command = [str(build / "kv9-workload"), "--config", str(path), "--output", str(output / name),
                   "--build-manifest", str(build / "build.json"), *map(str, extra)]
        process = subprocess.Popen(command, env=env, stdout=log, stderr=log)
        workloads.append(process)
        return process

    def collect(name, process, expected=0):
        code = process.wait(timeout=45)
        if code != expected:
            raise RuntimeError(f"{name} exited {code}, expected {expected}; see retained workload log")
        if expected == 0:
            verdict = validate(output / name, build, args.expected_revision)
            (output / (name + "-checked.json")).write_text(json.dumps(verdict, indent=2) + "\n")
            summary["cases"].append(dict(name=name, exit_code=code, independently_checked=verdict["full_history_independently_checked"],
                                         measured_operations=verdict["measured_operations"]))
        print(f"PASS: workload E2E {name}", flush=True)

    try:
        for _ in range(3):
            connection = socket.socket()
            connection.bind(("127.0.0.1", 0))
            sockets.append(connection)
        addresses = {n: f"127.0.0.1:{connection.getsockname()[1]}" for n, connection in enumerate(sockets, 1)}
        env.update(KV9_OBJECT_STORE_BUCKET="kv9-workload", KV9_OBJECT_STORE_ACCESS_KEY="kv9" + secrets.token_hex(8),
                   KV9_OBJECT_STORE_SECRET_KEY=secrets.token_hex(24))
        with open(secret_file, "x", opener=lambda path, flags: os.open(path, flags, 0o600)) as stream:
            stream.write("MINIO_ROOT_USER=" + env["KV9_OBJECT_STORE_ACCESS_KEY"] + "\nMINIO_ROOT_PASSWORD=" + env["KV9_OBJECT_STORE_SECRET_KEY"] + "\n")
        run(["docker", "run", "-d", "--name", container, "-p", "127.0.0.1::9000", "--env-file", secret_file, MINIO, "server", "/data"])
        created = True
        endpoint = "http://" + run(["docker", "port", container, "9000/tcp"])
        env["KV9_OBJECT_STORE_ENDPOINT"] = endpoint
        def healthy():
            try:
                with urllib.request.urlopen(endpoint + "/minio/health/live", timeout=1) as response:
                    return response.status == 200
            except OSError:
                return False
        wait("owned MinIO health", healthy)
        host = "http://" + env["KV9_OBJECT_STORE_ACCESS_KEY"] + ":" + env["KV9_OBJECT_STORE_SECRET_KEY"] + "@127.0.0.1:9000"
        # The credential is supplied through Docker's environment pass-through,
        # never printed in a command or retained in public artifacts.
        env["MC_HOST_workload"] = host
        run(["docker", "exec", "-e", "MC_HOST_workload", container, "mc", "mb", "workload/" + env["KV9_OBJECT_STORE_BUCKET"]])
        del env["MC_HOST_workload"]
        secret_file.unlink()
        root = output / "root.bin"
        prepared = prepare_stores(run, server, {n: output / f"n{n}" for n in addresses})
        run([server, "root-create", "--output", root, "--voters", ",".join(f"{n}@{address}" for n, address in addresses.items()), "--store-incarnations", prepared])
        for connection in sockets:
            connection.close()
        for node in addresses:
            run([server, "init", "--root", root, "--node-id", node, "--data-dir", output / f"n{node}"])
            start(node)
        wait("three serving replicas", leader)
        with server.open("rb") as stream:
            summary["server_sha256"] = hashlib.file_digest(stream, "sha256").hexdigest()
        summary["addresses"] = addresses
        collect("correctness", launch("correctness", config("correctness")))

        performance = config("performance-stop", "performance")
        performance.update(max_operations=10000, measure_ms=30000)
        stop = output / "performance.stop"
        process = launch("performance-stop", performance, ("--stop-file", stop))
        wait("performance measured progress", lambda: (output / "performance-stop/progress.json").exists(),
             workload=("performance-stop", process))
        stop.touch()
        collect("performance-stop", process)
        report = json.loads((output / "performance-stop/report.json").read_text())
        if report["stop"]["reason"] != "stop_file":
            raise RuntimeError("performance workload did not observe graceful stop")

        lost = next(n for n in nodes if n != leader())
        dead_config = config("dead-seed", peers=[dict(node_id=n, address=addresses[n]) for n in [lost, *(n for n in nodes if n != lost)]])
        dead = nodes.pop(lost)
        dead.kill()
        dead.wait(timeout=10)
        wait("surviving quorum", leader)
        collect("dead-seed", launch("dead-seed", dead_config))
        dead_report = json.loads((output / "dead-seed/report.json").read_text())
        if sum(dead_report["metrics"]["logical_counts"][0][0][5:]) < 1:
            raise RuntimeError("unavailable initial seed was not actually observed")
        start(lost)
        wait("restarted seed", leader)

        invalid_phase = output / "invalid-phase.txt"
        invalid_phase.write_text("initialization\n")
        process = launch("invalid-phase", config("invalid-phase"), ("--phase-file", invalid_phase))
        collect("invalid-phase", process, 1)
        try:
            validate(output / "invalid-phase", build)
        except ValueError:
            summary["cases"].append(dict(name="invalid-phase", exit_code=1, rejected=True))
        else:
            raise RuntimeError("invalid phase report was accepted")

        killed_config = config("killed-generator")
        killed_config.update(max_operations=1000, measure_ms=30000, interval_ms=100)
        process = launch("killed-generator", killed_config)
        wait("generator measured progress before kill", lambda: (output / "killed-generator/progress.json").exists(),
             workload=("killed-generator", process))
        process.kill()
        code = process.wait(timeout=10)
        if code >= 0 or (output / "killed-generator/report.json").exists():
            raise RuntimeError("generator kill did not leave an incomplete artifact")
        # An independent new client still writes and reads after collector loss.
        keyspace = str(killed_config["client"]["keyspace_id"])
        receipt = client("raw-put", "--keyspace", keyspace, "--key-hex", "696e646570656e64656e74", "--value-hex", "7375727669766564")
        observed = client("raw-get", "--keyspace", keyspace, "--key-hex", "696e646570656e64656e74")
        if observed != "value_hex=7375727669766564":
            raise RuntimeError("database progress failed after generator/collector loss")
        (output / "after-generator-kill-put.txt").write_text(receipt + "\n")
        (output / "after-generator-kill-get.txt").write_text(observed + "\n")
        summary["cases"].append(dict(name="killed-generator", exit_code=code, database_progress=True, complete_report=False))
        wait("remote checkpoints on every replica", lambda: all(read_checkpoint(output / f"n{n}") is not None for n in nodes))
        summary["wal_layouts"] = {str(n): read_layout(output / f"n{n}").evidence() for n in nodes}
        for n in nodes:
            (output / f"n{n}-final-status.txt").write_text((output / f"n{n}/status").read_text())
        (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        print("PASS: persistent workload executable, MinIO, dead seed, drain, fail-closed reports and collector-loss independence", flush=True)
    finally:
        for process in workloads + list(nodes.values()):
            if process.poll() is None:
                process.kill()
            process.wait(timeout=10)
        for log in logs:
            log.close()
        for connection in sockets:
            connection.close()
        secret_file.unlink(missing_ok=True)
        if created:
            subprocess.run(["docker", "rm", "-f", container], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30, check=True)


if __name__ == "__main__":
    main()
