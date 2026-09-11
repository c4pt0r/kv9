#!/usr/bin/env python3
"""Run only explicitly released local Redis batch correctness cases.

This prepared script does not build clients or invoke Kubernetes. Every run
needs a fresh output directory. Dirty debug artifacts are explicitly allowed
for development correctness; none of these short runs is a timing result.
"""
import argparse
import base64
import hashlib
import importlib
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
FATAL = {"wrong_count", "oversized_length"}
EXPECTED_BINARY = "4dc511149d70c89b27a2ba62ce6107b9059686c143fd0ec46ef1c383a4f1faa7"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(Path(path).read_text())


def save(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + "\n")


def process_identity(pid):
    root = Path("/proc") / str(pid)
    stat = (root / "stat").read_text()
    executable = (root / "exe").stat()
    return dict(pid=pid, start_ticks=int(stat.rsplit(")", 1)[1].split()[19]),
                boot_id=Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
                executable=str((root / "exe").readlink()), executable_device=executable.st_dev,
                executable_inode=executable.st_ino, command_line=(root / "cmdline").read_bytes().split(b"\0")[:-1],
                affinity=sorted(os.sched_getaffinity(pid)), observed_unix_ns=time.time_ns())


def serial_identity(row):
    return dict(row, command_line=[os.fsdecode(x) for x in row["command_line"]])


def listener_identity(pid, port):
    root = Path("/proc") / str(pid)
    owned = set()
    for fd in (root / "fd").iterdir():
        try:
            target = str(fd.readlink())
        except FileNotFoundError:
            continue
        if target.startswith("socket:["):
            owned.add(target[8:-1])
    rows = [line.split() for line in (root / "net/tcp").read_text().splitlines()[1:]]
    rows = [r for r in rows if r[3] == "0A" and r[9] in owned and r[1] == f"0100007F:{port:04X}"]
    require(len(rows) == 1, "owned backend does not own the requested loopback listener")
    return dict(pid=pid, port=port, inode=rows[0][9], observed_unix_ns=time.time_ns())


class Children:
    def __init__(self, output):
        self.output, self.rows, self.logs = output, [], []

    def launch(self, label, command, executable):
        log = (self.output / (label + ".log")).open("x")
        self.logs.append(log)
        executable_stat = Path(executable).stat()
        started = time.time_ns()
        child = subprocess.Popen(list(map(str, command)), stdout=log, stderr=log,
                                 env=dict(os.environ, PYTHONOPTIMIZE="0", PYTHONDONTWRITEBYTECODE="1"))
        row = dict(label=label, process=child, launched_unix_ns=started, identity=None)
        self.rows.append(row)
        # Capture the post-exec identity, not an inherited pre-exec Python image.
        for _ in range(100):
            require(child.poll() is None, "child exited before its lifetime could be captured: " + label)
            identity = process_identity(child.pid)
            if (identity["executable_device"], identity["executable_inode"]) == (executable_stat.st_dev, executable_stat.st_ino):
                row["identity"] = identity
                save(self.output / (label + "-identity.json"), serial_identity(identity))
                return child
            time.sleep(.001)
        raise ValueError("post-exec process identity did not converge: " + label)

    def cleanup(self):
        errors = []
        for row in reversed(self.rows):
            child = row["process"]
            try:
                if child.poll() is None:
                    current = process_identity(child.pid)
                    captured = row["identity"]
                    if captured is not None:
                        require((current["start_ticks"], current["boot_id"]) ==
                                (captured["start_ticks"], captured["boot_id"]), "owned lifetime changed before cleanup")
                    child.terminate()
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait(timeout=5)
                else:
                    child.wait()
                row["exit_code"] = child.returncode
                row["exited_unix_ns"] = time.time_ns()
                row["pid_absent_after_reap"] = not Path("/proc", str(child.pid)).exists()
                require(row["pid_absent_after_reap"], "owned PID remains after wait/reap")
            except BaseException as error:
                errors.append(dict(label=row["label"], error=repr(error)))
        for log in self.logs:
            log.close()
        save(self.output / "cleanup.json", dict(errors=errors, children=[
            {k: (serial_identity(v) if k == "identity" and v is not None else v)
             for k, v in row.items() if k != "process"} for row in self.rows]))
        return errors


def resp_encode(args):
    return b"*" + str(len(args)).encode() + b"\r\n" + b"".join(
        b"$" + str(len(x)).encode() + b"\r\n" + x + b"\r\n" for x in args)


def resp_read(stream, depth=0):
    require(depth <= 3, "independent Redis reply nesting bound exceeded")
    line = stream.readline(1024)
    require(line.endswith(b"\r\n"), "independent Redis reply is truncated")
    tag, value = line[:1], line[1:-2]
    if tag == b"-":
        raise ValueError("independent Redis request failed: " + repr(value))
    if tag == b"+":
        return value
    if tag == b":":
        return int(value)
    if tag == b"$":
        size = int(value)
        if size == -1:
            return None
        require(0 <= size <= 1_048_576, "independent Redis bulk bound exceeded")
        body = stream.read(size + 2)
        require(len(body) == size + 2 and body.endswith(b"\r\n"), "independent Redis bulk truncated")
        return body[:-2]
    if tag == b"*":
        size = int(value)
        require(0 <= size <= 1024, "independent Redis array bound exceeded")
        return [resp_read(stream, depth + 1) for _ in range(size)]
    raise ValueError("independent Redis reply has unknown type")


def request(address, args):
    host, port = address.rsplit(":", 1)
    with socket.create_connection((host, int(port)), timeout=2) as connection:
        connection.sendall(resp_encode(args))
        with connection.makefile("rb") as stream:
            return resp_read(stream)


def mix(value):
    mask = (1 << 64) - 1
    value = (value + 0x9e3779b97f4a7c15) & mask
    value = ((value ^ (value >> 30)) * 0xbf58476d1ce4e5b9) & mask
    value = ((value ^ (value >> 27)) * 0x94d049bb133111eb) & mask
    return value ^ (value >> 31)


def valid_dataset(config, dataset):
    for index in range(config["keys"] + 1):
        key = f'{config["run_id"]}:{index:016x}'.encode()
        value = dataset.get(key)
        require(value is not None and len(value) == config["value_bytes"], "final key missing or wrong size")
        nonce = int.from_bytes(value[8:16], "big")
        require(nonce <= config["warmup_calls"] + config["max_calls"], "final value nonce exceeds workload bound")
        if index == config["keys"]:
            require(nonce == 0, "reserved sentinel changed")
        expected = index.to_bytes(8, "big") + nonce.to_bytes(8, "big")
        for word in range((config["value_bytes"] - 16 + 7) // 8):
            expected += mix(config["seed"] ^ mix(index) ^ mix(nonce) ^ word).to_bytes(8, "big")
        require(value == expected[:config["value_bytes"]], "final deterministic value mismatch")
    return dict(keys=len(dataset), sentinel_unchanged=True, deterministic_values_valid=True)


def build_binding(build, source):
    manifest, sources = read(build / "build.json"), read(build / "sources.json")
    require(manifest["binary_sha256"] == EXPECTED_BINARY == digest(build / "kv9-redis-batch-reference"), "wrong frozen development client")
    for key in ("binary_sha256", "source_tree_sha256", "revision", "dirty"):
        require(manifest[key] == sources[key], "build/source manifest mismatch: " + key)
    require(manifest["profile"] == "debug" and manifest["dirty"] is True, "development fixture expects explicitly dirty debug build")
    require(hashlib.sha256(json.dumps(sources["sources"], sort_keys=True, separators=(",", ":")).encode()).hexdigest() == manifest["source_tree_sha256"], "source inventory digest differs")
    selected = {k: v for k, v in sources["sources"].items() if
                k.startswith("scripts/redis-reference/") and (k.endswith(".rs") or k.endswith("Cargo.toml") or k.endswith("Cargo.lock"))
                or k == "crates/server/src/bin/kv9-batch-benchmark/common.rs"}
    require("scripts/redis-reference/src/bin/kv9-redis-batch-reference.rs" in selected, "client source absent from build inventory")
    require(all(digest(source / k) == v for k, v in selected.items()), "compiled Redis/shared source changed since development build")
    cargo = [json.loads(line) for line in (build / "cargo.jsonl").read_text().splitlines()]
    rows = [r for r in cargo if r.get("reason") == "compiler-artifact" and r.get("target", {}).get("name") == "kv9-redis-batch-reference"]
    require(len(rows) == 1 and rows[0]["features"] == [] and rows[0]["profile"]["test"] is False,
            "missing/ambiguous/nondefault standalone Cargo artifact")
    require(cargo[-1].get("reason") == "build-finished" and cargo[-1]["success"] is True, "Cargo build did not finish successfully")
    return dict(manifest=manifest, source_hashes=selected, cargo_artifact=rows[0], command=sources["command"],
                build_sha256=digest(build / "build.json"), sources_sha256=digest(build / "sources.json"),
                cargo_sha256=digest(build / "cargo.jsonl"))


def check_case(case, path, build, source, client_pid):
    report = read(path / "client/report.json")
    config = read(path / "requested-config.json")
    fatal = case.get("action") in FATAL
    require(report["complete"] is not fatal, "unexpected report completion verdict")
    require(report["configuration"] == config and report["build"] == read(build / "build.json"), "report source/configuration differs")
    require(report["config_sha256"] == digest(path / "requested-config.json") == digest(path / "client/config.json"), "configuration byte identity differs")
    require(report["build_sha256"] == digest(build / "build.json") == digest(path / "client/build.json"), "build byte identity differs")
    require(report["process_id"] == client_pid == read(path / "client/ready.json")["process_id"], "report/ready process identity differs")
    require(report["runtime_threads"] == 2 and report["protocol"] == "resp2" and report["preconnected_workers"] == config["workers"], "client connection/runtime contract differs")
    require(report["timing_eligible"] is False, "development case was mislabeled timing eligible")
    metrics = report["metrics"]["measurement"]
    require(metrics["operations"] == ["mget", "mset"] and metrics["outcomes"] == ["success", "unknown_write", "read_failure"], "metric axes differ")
    populations = [p for op in metrics["statistics"] for p in op["populations"]]
    calls = sum(p["calls"] for p in populations)
    require(calls == report["measured_issued"] == report["measured_completed"] == sum(w["issued"] for w in report["workers"]), "issued/terminal/worker counts differ")
    require(calls > 0 and report["task_failures"] == 0, "empty cohort or failed worker task")
    require(sum(p["input_items"] for p in populations) == calls * config["batch_size"], "input-item conservation differs")
    require(all(sum(op["reasons"]) == sum(p["calls"] for p in op["populations"]) for op in metrics["statistics"]), "reason populations do not conserve calls")
    require(all(op["command_attempts"] <= sum(p["calls"] for p in op["populations"]) for op in metrics["statistics"]), "a logical call sent multiple commands")
    require(all(p["whole_call"]["raw"]["count"] == p["client_call"]["raw"]["count"] == p["calls"] for p in populations), "terminal latency samples do not cover calls")
    require(sum(p["completed_before_cutoff"] for p in populations) == calls - sum(w["issued"] > 0 and w["last_terminal_ns"] >= config["measure_ms"] * 1_000_000 for w in report["workers"]), "exact nominal cutoff accounting differs")
    require(report["dropped_slots"] == sum(w["dropped_slots"] for w in report["workers"]), "per-worker shed counts differ")
    if report["offered_slots"] is not None:
        require(calls + report["dropped_slots"] <= report["offered_slots"], "issued/dropped exceeds offered slots")
        if not fatal:
            require(calls + report["dropped_slots"] == report["offered_slots"], "offered slots not fully accounted")
    if fatal:
        require(report["stop_reason"] == "worker_failure" and metrics["valid"] is False and
                "verification" not in report["stages"] and report["failure"], "protocol failure did not cancel and fail closed")
    else:
        require(report["failure"] is None and metrics["valid"] is True and "verification" in report["stages"], "complete report skipped final verification")
        # The maintained complete-report validator is additionally required.
        sys.path.insert(0, str(source / "scripts"))
        validator = importlib.import_module("redis_batch_report")
        validated = validator.validate(path / "client", build, path / "requested-config.json")
        save(path / "maintained-validator.json", validated)
    outcome_counts = {name: sum(op["populations"][i]["calls"] for op in metrics["statistics"])
                      for i, name in enumerate(metrics["outcomes"])}
    reason_counts = {name: sum(op["reasons"][i] for op in metrics["statistics"])
                     for i, name in enumerate(metrics["reasons"])}
    if case["backend"] == "redis" or case.get("action") == "split":
        require(outcome_counts["success"] == calls, "healthy/split-frame case returned an unsuccessful call")
    else:
        reason = {"wrong_count": "protocol", "oversized_length": "protocol", "server_error": "server_error",
                  "consume_eof": "io", "consume_timeout": "deadline"}[case["action"]]
        require(reason_counts[reason] == 1 and sum(reason_counts[k] for k in reason_counts if k != "success") == 1,
                "one-shot fault did not produce exactly one matching terminal failure")
    return dict(calls=calls, input_items=calls * config["batch_size"], outcomes=outcome_counts,
                reasons=reason_counts, zero_issued_workers=sum(w["issued"] == 0 for w in report["workers"]),
                dropped_slots=report["dropped_slots"], expected_incomplete=fatal, report_sha256=digest(path / "client/report.json"))


def check_peer(case, path):
    events = [json.loads(line) for line in (path / "peer/events.jsonl").read_text().splitlines()]
    faults = [e for e in events if e["kind"] == "fault_selected"]
    require(len(faults) == 1, "controlled peer fault did not occur exactly once")
    fault = faults[0]
    report = read(path / "client/report.json")
    start = report["measurement_start_unix_ns"]
    require(start <= fault["unix_ns"] <= start + report["cohort_elapsed_ns"], "fault outside actual measured call/drain cohort")
    commands = [e for e in events if e["kind"] == "command_consumed"]
    require(all(digest(e["file"]) == e["sha256"] for e in commands), "retained consumed command bytes differ")
    measured = [e for e in commands if start <= e["unix_ns"] <= start + report["cohort_elapsed_ns"]]
    require(len(measured) == report["measured_completed"], "peer observed command count does not match logical terminals")
    selected = next(e for e in commands if e["sequence"] == fault["sequence"])
    if case.get("action") in FATAL:
        require(not any(e["sequence"] > selected["sequence"] for e in measured), "new measurement dispatched after fatal protocol response")
    if case.get("action") in ("consume_eof", "consume_timeout"):
        nonce = selected["write_nonces"]
        require(nonce and len(set(nonce)) == 1 and nonce[0] > 0, "injected mutation nonce is not one measured logical call")
        writes = [e for e in commands if e["command"] == "MSET"]
        require(sum(e["sha256"] == selected["sha256"] for e in writes) == 1 and
                sum(nonce[0] in e["write_nonces"] for e in writes) == 1, "uncertain mutation was replayed")
        later = [e for e in measured if e["command"] == "MSET" and e["sequence"] > selected["sequence"]]
        require(later and later[0]["connection"] != selected["connection"] and later[0]["write_nonces"] != nonce,
                "subsequent new logical mutation did not reconnect")
        applied = [e for e in events if e["kind"] == "mutation_applied" and e["sequence"] == selected["sequence"]]
        require(len(applied) == 1 and applied[0]["monotonic_ns"] <= fault["monotonic_ns"], "unknown mutation was not consumed/applied before lost response")
    opened = {e["connection"] for e in events if e["kind"] == "connection_open"}
    closed = {e["connection"] for e in events if e["kind"] == "connection_closed"}
    require(opened == closed, "controlled peer connection lifetime did not close")
    require(events[-1]["kind"] == "peer_terminal", "peer did not retain terminal drain record")
    return dict(fault=fault, commands=len(commands), measured_commands=len(measured),
                connections=len(opened), all_connections_closed=True, uncertain_mutation_replayed=False)


def run_case(case, output, args):
    output.mkdir()
    children = Children(output)
    state = dict(complete=False, case=case["name"], backend=case["backend"], started_unix_ns=time.time_ns())
    config = dict(case["configuration"])
    try:
        if case["backend"] == "redis":
            reserve = socket.socket()
            reserve.bind(("127.0.0.1", 0))
            port = reserve.getsockname()[1]
            config["address"] = f"127.0.0.1:{port}"
            # Correctness cases tolerate ordinary host scheduling pauses.
            config["deadline_ms"] = 1000
            (output / "redis.conf").write_text(f'bind 127.0.0.1\nport {port}\nprotected-mode yes\nsave ""\nappendonly no\ndir {output}\ndaemonize no\nlogfile ""\nio-threads 1\nmaxclients 512\n')
            reserve.close()
            backend = children.launch("redis", [args.redis_server, output / "redis.conf"], args.redis_server)
            for _ in range(100):
                require(backend.poll() is None, "fresh Redis exited before readiness")
                try:
                    if request(config["address"], [b"PING"]) == b"PONG":
                        break
                except OSError:
                    pass
                time.sleep(.02)
            else:
                raise TimeoutError("fresh Redis readiness deadline")
            settings = request(config["address"], [b"CONFIG", b"GET", b"save", b"appendonly", b"io-threads"])
            settings = {settings[i].decode(): settings[i + 1].decode() for i in range(0, len(settings), 2)}
            require(settings == {"save": "", "appendonly": "no", "io-threads": "1"}, "Redis memory settings differ")
            replication = request(config["address"], [b"INFO", b"replication"]).decode()
            require("role:master\r\n" in replication and "connected_slaves:0\r\n" in replication, "Redis is not a standalone reference")
            save(output / "redis-live-config.json", dict(configuration=settings, replication=replication))
        else:
            backend = children.launch("peer", [sys.executable, HERE / "resp_peer.py", "--output", output / "peer", "--ready-file", output / "client/ready.json", "--operation", case["operation"], "--action", case["action"], "--hold-ms", "250"], sys.executable)
            for _ in range(100):
                require(backend.poll() is None, "controlled peer exited before readiness")
                if (output / "peer/ready.json").is_file():
                    config["address"] = read(output / "peer/ready.json")["address"]
                    break
                time.sleep(.02)
            else:
                raise TimeoutError("controlled peer readiness deadline")
            port = int(config["address"].rsplit(":", 1)[1])
        save(output / "backend-listener.json", listener_identity(backend.pid, port))
        save(output / "requested-config.json", config)
        client = children.launch("client", [args.build / "kv9-redis-batch-reference", "--config", output / "requested-config.json", "--build-manifest", args.build / "build.json", "--output", output / "client"], args.build / "kv9-redis-batch-reference")
        state["client_exit_code"] = client.wait(timeout=30)
        state["client_terminal_unix_ns"] = time.time_ns()
        save(output / "client-exit.json", dict(exit_code=client.returncode, observed_unix_ns=time.time_ns()))
        require(client.returncode == (1 if case.get("action") in FATAL else 0), "unexpected client exit status")
        state["accounting"] = check_case(case, output, args.build, args.source, client.pid)
        if case["backend"] == "redis":
            dataset = {}
            for first in range(0, config["keys"] + 1, 128):
                keys = [f'{config["run_id"]}:{i:016x}'.encode() for i in range(first, min(config["keys"] + 1, first + 128))]
                values = request(config["address"], [b"MGET", *keys])
                require(len(values) == len(keys), "independent final Redis count differs")
                dataset.update(zip(keys, values))
            save(output / "final-dataset.json", {base64.b64encode(k).decode(): base64.b64encode(v).decode() if v is not None else None for k, v in dataset.items()})
            state["dataset"] = valid_dataset(config, dataset)
        state["execution_checks_passed"] = True
    except BaseException as error:
        state["failure"] = repr(error)
    finally:
        # Retain original verdict before cleanup, even when identity capture or
        # a validator fails. Cleanup never overwrites the initial failure.
        save(output / "pre-cleanup.json", state)
        state["cleanup_errors"] = children.cleanup()
    if state.get("execution_checks_passed") and not state["cleanup_errors"]:
        try:
            if case["backend"] == "controlled_resp":
                state["peer"] = check_peer(case, output)
                raw = read(output / "peer/dataset.json")
                state["dataset"] = valid_dataset(config, {base64.b64decode(k): base64.b64decode(v) for k, v in raw.items()})
            state["complete"] = True
        except BaseException as error:
            state["post_cleanup_failure"] = repr(error)
    state["finished_unix_ns"] = time.time_ns()
    save(output / "summary.json", state)
    return state


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--build", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--redis-server", type=Path, default=Path("/usr/bin/redis-server"))
    parser.add_argument("--cases", default="all")
    parser.add_argument("--execute-released-fixture", action="store_true")
    args = parser.parse_args()
    require(args.execute_released_fixture, "preparation only: explicit released-fixture invocation required")
    require(set(os.sched_getaffinity(0)) <= set(range(6, 32)), "correctness fixture must stay on CPUs 6-31")
    args.output, args.build, args.source = args.output.resolve(), args.build.resolve(), args.source.resolve()
    require(not args.output.exists(), "output directory must be new; failed attempts are retained")
    binding = build_binding(args.build, args.source)
    cases = read(HERE / "cases.prepared.json")["cases"]
    if args.cases != "all":
        wanted = args.cases.split(",")
        require(len(wanted) == len(set(wanted)) and set(wanted) <= {c["name"] for c in cases}, "unknown or duplicate case selection")
        cases = [c for c in cases if c["name"] in wanted]
    args.output.mkdir(parents=True)
    validators = ["redis_batch_report.py", "batch_benchmark_report.py", "workload_report.py"]
    summary = dict(complete=False, scope="Development correctness/accounting only; no performance or Chaos claim", binding=binding,
                   source=str(args.source), build=str(args.build), redis_server_sha256=digest(args.redis_server),
                   validator_hashes={name: digest(args.source / "scripts" / name) for name in validators},
                   helper_hashes={p.name: digest(p) for p in [HERE / "run_fixture.py", HERE / "resp_peer.py", HERE / "cases.prepared.json"]}, cases=[])
    (args.output / "source-snapshot").mkdir()
    for name in validators:
        (args.output / "source-snapshot" / name).write_bytes((args.source / "scripts" / name).read_bytes())
    for name in summary["helper_hashes"]:
        (args.output / "source-snapshot" / name).write_bytes((HERE / name).read_bytes())
    save(args.output / "plan.json", summary)
    for case in cases:
        result = run_case(case, args.output / case["name"], args)
        summary["cases"].append(result)
        save(args.output / "summary.json", summary)
        if not result["complete"]:
            raise SystemExit("FAIL: retained case " + case["name"])
    require(all(digest(args.source / k) == h for k, h in binding["source_hashes"].items()), "compiled source changed during fixture")
    require(digest(args.build / "kv9-redis-batch-reference") == EXPECTED_BINARY, "client binary changed during fixture")
    require(all(digest(HERE / k) == h for k, h in summary["helper_hashes"].items()), "fixture helper changed during execution")
    require(all(digest(args.source / "scripts" / k) == h for k, h in summary["validator_hashes"].items()), "maintained validator changed during fixture")
    summary["complete"] = True
    save(args.output / "summary.json", summary)
    print("PASS: retained development Redis batch correctness cases", len(cases))


if __name__ == "__main__":
    main()
