"""Independent, bounded validation of workload artifacts; imports no kv9 code."""
import hashlib
import ipaddress
import json
import math
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent / "history"))
from checker import History, check, coverage, verify_witness

PHASES = ["initialization", "warmup", "measure", "verify", "baseline", "healing",
          "registration-seed-blackhole", "pod-failure-1", "pod-failure-2", "pod-failure-3",
          "partition", "public-admission-overload", "delay"] + [
              f"io-voter-{node}-errno-{errno}" for node in (1, 2, 3) for errno in (5, 28)] + [
                  f"store-loss-voter-{node}-log-missing" for node in (1, 2, 3)] + [
                      f"store-loss-voter-{node}-pvc-replacement" for node in (1, 2, 3)]
OPERATIONS = ["get", "put", "delete"]
POPULATIONS = ["success", "not_leader", "admission_count", "admission_bytes", "admission_oversize",
               "read_quorum_unconfirmed", "read_apply_unconfirmed", "rpc_status", "protocol", "deadline",
               "client_capacity", "client_input"]
OUTCOMES = ["success", "error", "aborted", "released", "replaced", "rejected", "unconfirmed"]
U64 = (1 << 64) - 1
REPORT_KEYS = "version complete failure configuration config_sha256 build build_sha256 history_sha256 process_id wall_anchor_unix_ns wall_anchor_monotonic_ns elapsed_ns runtime_threads workload_model independently_checked stop stages measured_issued measured_completed measured_successful cohort_elapsed_ns cohort_terminal_ops_per_second cohort_success_ops_per_second resources_before resources_after history metrics".split()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def uint(value, lower=0, upper=U64):
    require(type(value) is int and lower <= value <= upper, "invalid bounded integer")
    return value


def keys(value, expected):
    require(isinstance(value, dict) and set(value) == set(expected), "unexpected schema fields")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def bounded(path, limit):
    with Path(path).open("rb") as stream:
        data = stream.read(limit + 1)
    require(len(data) <= limit, "artifact exceeds its byte bound")
    return data


def strict_json(data):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result, "duplicate JSON field")
            result[key] = value
        return result
    def constant(_):
        raise ValueError("non-finite JSON number")
    return json.loads(data, object_pairs_hook=pairs, parse_constant=constant)


def config_check(c):
    keys(c, "version client mode run_id keyspace_name seed workers keys value_bytes mix warmup_operations max_operations measure_ms interval_ms history_bytes".split())
    require(c["version"] == 1 and c["mode"] in ("correctness", "performance"), "unsupported workload configuration")
    for name in ("run_id", "keyspace_name"):
        require(isinstance(c[name], str) and re.fullmatch(r"[A-Za-z0-9_-]{1,64}", c[name]), "invalid workload name")
    client = c["client"]
    keys(client, "version peers keyspace_id epoch_conf_ver epoch_version max_in_flight max_attempts deadline_ms retry_backoff_ms".split())
    require(client["version"] == client["epoch_conf_ver"] == client["epoch_version"] == 1, "unsupported routing epoch")
    uint(client["keyspace_id"], 1, (1 << 24) - 1)
    require(isinstance(client["peers"], list) and 1 <= len(client["peers"]) <= 32, "invalid peer count")
    ids, addresses = set(), set()
    for peer in client["peers"]:
        keys(peer, ["node_id", "address"])
        uint(peer["node_id"], 1)
        host, port = peer["address"].rsplit(":", 1)
        address = (ipaddress.ip_address(host.strip("[]")), int(port))
        uint(address[1], 1, 65535)
        require(peer["node_id"] not in ids and address not in addresses, "duplicate peer identity/address")
        ids.add(peer["node_id"])
        addresses.add(address)
    uint(client["max_in_flight"], 1, 256)
    uint(client["max_attempts"], 1, 16)
    uint(client["deadline_ms"], 1, 30000)
    uint(client["retry_backoff_ms"], 0, client["deadline_ms"])
    uint(c["workers"], 1, client["max_in_flight"])
    uint(c["keys"], 1, 4096)
    uint(c["value_bytes"], 16, 8192)
    uint(c["seed"])
    keys(c["mix"], OPERATIONS)
    require(sum(uint(v, 0, 100) for v in c["mix"].values()) == 100, "invalid operation mix")
    uint(c["warmup_operations"], 0, 100000)
    uint(c["measure_ms"], 1, 3600000)
    uint(c["interval_ms"], 0, 1000)
    setup = 2 * (c["keys"] + 1) + len(ids) - 1
    verification = (c["keys"] + 1) * len(ids)
    uint(c["max_operations"], setup + c["warmup_operations"] + verification + 1,
         100000 if c["mode"] == "correctness" else 1000000)
    key_bytes = len(c["run_id"]) + 17
    require((c["keys"] + 1) * (key_bytes + c["value_bytes"] + 64) <= 8 * 1024 * 1024, "dataset exceeds bound")
    reservation = 65536 + c["max_operations"] * (8192 + 2 * key_bytes + 2 * c["value_bytes"])
    uint(c["history_bytes"], reservation if c["mode"] == "correctness" else 0,
         64 * 1024 * 1024 if c["mode"] == "correctness" else 0)


def histogram(samples):
    buckets = [0] * 65
    for value in samples:
        buckets[value.bit_length()] += 1
    return dict(buckets=buckets, count=len(samples), sum_ns=sum(samples),
                min_ns=min(samples) if samples else None, max_ns=max(samples) if samples else None)


def histogram_check(h):
    keys(h, "outcome buckets count sum_ns min_ns max_ns count_saturated sum_saturated duration_clamped p50 p95 p99".split())
    require(h["count_saturated"] is h["sum_saturated"] is h["duration_clamped"] is False,
            "saturated/clamped histogram cannot support this bounded run")
    require(isinstance(h["buckets"], list) and len(h["buckets"]) == 65, "invalid histogram buckets")
    count = uint(h["count"], 0, 16_000_000)
    total = uint(h["sum_ns"])
    require(sum(uint(v, 0, count) for v in h["buckets"]) == count, "histogram count differs from buckets")
    lower = lambda i: 1 << (i - 1) if i else 0
    upper = lambda i: (1 << i) - 1
    if count:
        lo, hi = uint(h["min_ns"]), uint(h["max_ns"])
        require(lo <= hi and lo * count <= total <= hi * count, "invalid histogram duration bounds")
        require(sum(lower(i) * n for i, n in enumerate(h["buckets"])) <= total <=
                sum(upper(i) * n for i, n in enumerate(h["buckets"])), "sum outside histogram buckets")
        require(next(i for i, n in enumerate(h["buckets"]) if n) == lo.bit_length() and
                max(i for i, n in enumerate(h["buckets"]) if n) == hi.bit_length(), "extrema outside histogram support")
    else:
        require(total == 0 and h["min_ns"] is h["max_ns"] is None, "nonempty zero-count histogram")
    for p in (50, 95, 99):
        expected = None
        if count:
            rank, cumulative = (count * p + 99) // 100, 0
            for i, n in enumerate(h["buckets"]):
                cumulative += n
                if cumulative >= rank:
                    expected = dict(lower_ns=lower(i), upper_ns=upper(i))
                    break
        require(h[f"p{p}"] == expected, "percentile interval differs from histogram")


def metrics_check(m):
    keys(m, "populations phases operations logical_counts attempt_counts logical_rpc_codes attempt_rpc_codes logical_latency attempt_latency".split())
    require(m["populations"] == POPULATIONS and m["phases"] == PHASES and m["operations"] == OPERATIONS, "metrics vocabulary changed")
    for name in ("logical_counts", "attempt_counts", "logical_rpc_codes", "attempt_rpc_codes"):
        rows = m[name]
        require(isinstance(rows, list) and len(rows) == len(PHASES), "missing metric phases")
        for row in rows:
            require(isinstance(row, list) and len(row) == 3, "missing metric operations")
            for counts in row:
                require(isinstance(counts, list) and len(counts) == (17 if "codes" in name else 12), "missing outcome counts")
                for count in counts:
                    uint(count, 0, 16_000_000)
    for level in ("logical", "attempt"):
        require(len(m[f"{level}_latency"]) == len(PHASES), "missing latency phases")
        for p, row in enumerate(m[f"{level}_latency"]):
            require(len(row) == 3, "missing latency operations")
            for op, latency in enumerate(row):
                keys(latency, ["valid", "outcomes"])
                require(latency["valid"] is True and len(latency["outcomes"]) == 7, "invalid latency population")
                for outcome, h in zip(OUTCOMES, latency["outcomes"]):
                    require(h["outcome"] == outcome, "latency outcome order changed")
                    histogram_check(h)
                counts = m[f"{level}_counts"][p][op]
                expected = [counts[0], 0, 0, 0, 0, sum(counts[1:5]), 0]
                if level == "logical":
                    # Deadline expiry before the first attempt is a local
                    # rejection; after an attempt it is an unknown/read failure.
                    released = latency["outcomes"][3]["count"]
                    local_deadlines = released - sum(counts[10:12])
                    require(0 <= local_deadlines <= counts[9], "local deadline population is invalid")
                    expected[3] = released
                    expected[1 if op == 0 else 6] = sum(counts[5:10]) - local_deadlines
                else:
                    expected[1 if op == 0 else 6] = sum(counts[5:])
                require([h["count"] for h in latency["outcomes"]] == expected, "latency and outcome counts disagree")
                require(sum(m[f"{level}_rpc_codes"][p][op]) == counts[7], "RPC status counts disagree")


def reason_population(reason):
    if reason is None:
        return 0
    require(isinstance(reason, dict) and reason.get("kind") in POPULATIONS[1:], "invalid outcome reason")
    kind = reason["kind"]
    keys(reason, ["kind", "leader"] if kind == "not_leader" else ["kind", "code"] if kind == "rpc_status" else ["kind"])
    if kind == "rpc_status":
        uint(reason["code"], 0, 16)
    if kind == "not_leader" and reason["leader"] is not None:
        uint(reason["leader"], 1)
    require(kind != "protocol", "protocol failure invalidates a workload")
    return POPULATIONS.index(kind)


def history_check(records, r, c, seconds):
    require(records and records[0] == dict(type="header", version=1, range_chunk_size=1024,
        generator="kv9-persistent-workload", configuration=c,
        initial=dict(keyspaces=[dict(name=c["keyspace_name"], id=c["client"]["keyspace_id"])], kv=[])), "unexpected history header")
    issued, active, completed, workers = {}, {}, {}, set()
    peak, last_time = 0, 0
    keyset = {f'{c["run_id"]}:{i:016x}'.encode().hex() for i in range(c["keys"] + 1)}
    worker_ids = {str(i) for i in range(c["workers"])}
    peer_ids = {peer["node_id"] for peer in c["client"]["peers"]}
    counts = {level: [[[0] * 12 for _ in OPERATIONS] for _ in PHASES] for level in ("logical", "attempt")}
    codes = {level: [[[0] * 17 for _ in OPERATIONS] for _ in PHASES] for level in ("logical", "attempt")}
    samples = {level: [[[[] for _ in OUTCOMES] for _ in OPERATIONS] for _ in PHASES] for level in ("logical", "attempt")}
    for seq, event in enumerate(records[1:]):
        require(event["seq"] == seq, "missing/reordered history event")
        now = uint(event["monotonic_ns"], last_time, r["elapsed_ns"])
        last_time = now
        identity = uint(event["id"], 0, c["max_operations"] - 1)
        if event["type"] == "invoke":
            keys(event, "type seq monotonic_ns id client phase op args".split())
            require(identity == len(issued), "missing/duplicate invocation")
            require(event["client"] in worker_ids and event["client"] not in workers, "worker has concurrent invocations")
            require(event["phase"] in PHASES and event["op"] in OPERATIONS, "unknown invocation population")
            args = event["args"]
            keys(args, ["keyspace", "key", "value"] if event["op"] == "put" else ["keyspace", "key"])
            require(args["keyspace"] == c["client"]["keyspace_id"] and args["key"] in keyset, "operation outside configured dataset")
            if event["op"] == "put":
                require(isinstance(args["value"], str) and re.fullmatch(r"[0-9a-f]+", args["value"]) and len(args["value"]) == c["value_bytes"] * 2, "invalid generated value size")
            if event["phase"] not in ("initialization", "verify"):
                require(args["key"] != f'{c["run_id"]}:{c["keys"]:016x}'.encode().hex(), "traffic touched immutable sentinel")
            span_name = {"initialization": "initialization", "warmup": "warmup", "verify": "verification"}.get(event["phase"], "measurement")
            span = r["stages"][span_name]
            end = r["stages"]["drain"]["end_ns"] if span_name == "measurement" else span["end_ns"]
            require(span["start_ns"] <= now <= end, "invocation outside its lifecycle stage")
            issued[identity] = event
            active[identity] = (event, end)
            workers.add(event["client"])
            peak = max(peak, len(active))
            require(peak <= c["workers"], "in-flight work exceeds bound")
        else:
            keys(event, "type seq monotonic_ns id outcome result observation".split())
            require(event["type"] == "return" and identity in active, "missing/duplicate terminal record")
            invocation, end = active.pop(identity)
            workers.remove(invocation["client"])
            require(now <= end, "terminal outside drained lifecycle stage")
            completed[identity] = event
            observation = event["observation"]
            keys(observation, "attempts elapsed_ns stop reason receipt malformed".split())
            require(observation["malformed"] is None, "malformed response in history")
            elapsed = uint(observation["elapsed_ns"], 0, now - invocation["monotonic_ns"])
            p, op = PHASES.index(invocation["phase"]), OPERATIONS.index(invocation["op"])
            population = reason_population(observation["reason"])
            attempts = observation["attempts"]
            require(isinstance(attempts, list) and len(attempts) <= c["client"]["max_attempts"], "attempt bound exceeded")
            local = not attempts
            require(not local or population in (9, 10, 11), "zero-attempt outcome lacks a local refusal")
            require(local or population not in (10, 11), "local refusal has network attempts")
            outcome = event["outcome"]
            expected_outcome = "ok" if population == 0 else "refused" if local or population in (1, 2, 3, 4) else "unknown"
            require(outcome == expected_outcome, "reason does not justify history outcome")
            if outcome == "ok" and op:
                receipt = observation["receipt"]
                keys(receipt, ["applied_term", "applied_index"])
                uint(receipt["applied_term"], 1)
                uint(receipt["applied_index"], 1)
            else:
                require(observation["receipt"] is None, "unexpected write receipt")
            require(observation["stop"] in ("terminal", "attempt_limit", "deadline", "client_rejected"), "invalid stop reason")
            if population in (10, 11):
                require(observation["stop"] == "client_rejected", "local rejection lacks terminal classification")
            if local and population == 9:
                require(observation["stop"] == "deadline", "local deadline lacks stop classification")
            group = 0 if outcome == "ok" else 3 if local else 5 if outcome == "refused" else 1 if op == 0 else 6
            counts["logical"][p][op][population] += 1
            samples["logical"][p][op][group].append(elapsed)
            if population == 7:
                codes["logical"][p][op][observation["reason"]["code"]] += 1
            attempt_sum = 0
            for ordinal, attempt in enumerate(attempts, 1):
                keys(attempt, "ordinal node_id elapsed_ns failure".split())
                require(attempt["ordinal"] == ordinal and attempt["node_id"] in peer_ids, "attempt identity/routing mismatch")
                duration = uint(attempt["elapsed_ns"], 0, elapsed)
                attempt_sum += duration
                ap = reason_population(attempt["failure"])
                require(ordinal == len(attempts) or ap == 1, "retry lacks exclusive NotLeader refusal")
                if ordinal == len(attempts):
                    require(attempt["failure"] == observation["reason"], "last attempt differs from terminal reason")
                ag = 0 if ap == 0 else 5 if ap in (1, 2, 3, 4) else 1 if op == 0 else 6
                counts["attempt"][p][op][ap] += 1
                samples["attempt"][p][op][ag].append(duration)
                if ap == 7:
                    codes["attempt"][p][op][attempt["failure"]["code"]] += 1
            require(attempt_sum <= elapsed, "attempt durations exceed logical duration")
    require(not active and len(issued) == len(completed) == r["history"]["issued"] and len(records) - 1 == 2 * len(issued), "history lacks exact terminal accounting")
    require(peak == r["history"]["peak_in_flight"], "reported concurrency differs from full history")
    for level in ("logical", "attempt"):
        require(r["metrics"][f"{level}_counts"] == counts[level] and r["metrics"][f"{level}_rpc_codes"] == codes[level], "report counts differ from full history")
        for p in range(len(PHASES)):
            for op in range(3):
                for group in range(7):
                    actual = r["metrics"][f"{level}_latency"][p][op]["outcomes"][group]
                    require(all(actual[k] == v for k, v in histogram(samples[level][p][op][group]).items()), "report histogram differs from full history")
    dataset_check(issued, completed, c)
    history = History.parse(records)
    verdict = check(history, max_states=200000, seconds=seconds)
    require(verdict["verdict"] == "valid", "full independent history is invalid or inconclusive")
    require(verify_witness(history, verdict["witness"]), "independent witness replay failed")
    return dict(**verdict, coverage=coverage(history))


def dataset_check(issued, completed, c):
    rows = list(issued.values())
    key = lambda i: f'{c["run_id"]}:{i:016x}'.encode().hex()
    def value(nonce):
        def mix(v):
            v = (v + 0x9e3779b97f4a7c15) & U64
            v = ((v ^ (v >> 30)) * 0xbf58476d1ce4e5b9) & U64
            v = ((v ^ (v >> 27)) * 0x94d049bb133111eb) & U64
            return v ^ (v >> 31)
        return (nonce.to_bytes(8, "big") + b"".join(mix(c["seed"] ^ nonce ^ i).to_bytes(8, "big")
                for i in range((c["value_bytes"] - 1) // 8)))[:c["value_bytes"]].hex()
    init = [event for event in rows if event["phase"] == "initialization"]
    sentinel = key(c["keys"])
    probes = 0
    while init and init[0]["op"] == "get" and init[0]["args"]["key"] == sentinel:
        event = init.pop(0)
        probes += 1
        terminal = completed[event["id"]]
        if terminal["outcome"] == "ok":
            require(terminal["result"] == {"value": None}, "initial sentinel was not absent")
            break
    else:
        raise ValueError("no acknowledged initial sentinel read")
    require(1 <= probes <= len(c["client"]["peers"]), "initial read probe budget exceeded")
    for i in range(c["keys"] + 1):
        if i != c["keys"]:
            require(bool(init), "missing initial absence read")
            event = init.pop(0)
            require(event["op"] == "get" and event["args"]["key"] == key(i) and
                    completed[event["id"]]["outcome"] == "ok" and completed[event["id"]]["result"] == {"value": None}, "dataset was not established absent")
        require(bool(init), "missing initialization write")
        event = init.pop(0)
        require(event["op"] == "put" and event["args"]["key"] == key(i) and event["args"]["value"] == value(i)
                and completed[event["id"]]["outcome"] == "ok", "initial dataset lacks an exact acknowledged write")
    require(not init, "extra initialization work")
    warmup = [event for event in rows if event["phase"] == "warmup"]
    require(len(warmup) == c["warmup_operations"] and all(completed[event["id"]]["outcome"] == "ok" for event in warmup), "warmup incomplete")
    verification = [event for event in rows if event["phase"] == "verify"]
    for i in [c["keys"], *range(c["keys"])]:
        for _ in c["client"]["peers"]:
            require(bool(verification), "missing final read")
            event = verification.pop(0)
            require(event["op"] == "get" and event["args"]["key"] == key(i), "final read order/dataset mismatch")
            terminal = completed[event["id"]]
            if terminal["outcome"] == "ok":
                if i == c["keys"]:
                    require(terminal["result"] == {"value": value(i)}, "immutable sentinel was lost")
                break
        else:
            raise ValueError("final read never acknowledged")
    require(not verification, "extra final reads")


def validate(directory, build_directory, expected_revision=None, seconds=30):
    directory, build_directory = Path(directory), Path(build_directory)
    config_bytes = bounded(directory / "config.json", 65536)
    build_bytes = bounded(directory / "build.json", 65536)
    report_bytes = bounded(directory / "report.json", 2 * 1024 * 1024)
    c, b, r = map(strict_json, (config_bytes, build_bytes, report_bytes))
    config_check(c)
    keys(r, REPORT_KEYS)
    require(r["version"] == 1 and r["complete"] is True and r["failure"] is None and r["independently_checked"] is False, "workload did not produce a complete unverified report")
    require(r["configuration"] == c and r["config_sha256"] == sha(config_bytes) and r["build"] == b and r["build_sha256"] == sha(build_bytes), "report configuration/build provenance differs")
    keys(b, "version revision dirty source_tree_sha256 binary_sha256 profile rustc".split())
    require(b["version"] == 1 and type(b["dirty"]) is bool and b["profile"] in ("debug", "release") and isinstance(b["rustc"], str) and 0 < len(b["rustc"]) <= 4096, "invalid build manifest")
    for name, length in (("revision", 40), ("source_tree_sha256", 64), ("binary_sha256", 64)):
        require(isinstance(b[name], str) and re.fullmatch(f"[0-9a-f]{{{length}}}", b[name]), "invalid build hash")
    require(build_bytes == bounded(build_directory / "build.json", 65536), "run used a different retained build manifest")
    binary = build_directory / "kv9-workload"
    require(0 < binary.stat().st_size <= 512 * 1024 * 1024, "invalid retained executable size")
    with binary.open("rb") as stream:
        require(hashlib.file_digest(stream, "sha256").hexdigest() == b["binary_sha256"], "retained executable hash mismatch")
    inventory = strict_json(bounded(build_directory / "sources.json", 2 * 1024 * 1024))
    require(inventory["revision"] == b["revision"] and inventory["dirty"] == b["dirty"] and
            inventory["binary_sha256"] == b["binary_sha256"], "source inventory build mismatch")
    require(sha(json.dumps(inventory["sources"], sort_keys=True, separators=(",", ":")).encode()) ==
            inventory["source_tree_sha256"] == b["source_tree_sha256"], "source inventory hash mismatch")
    if expected_revision:
        require(b["revision"] == expected_revision and b["dirty"] is False, "run is not from the required clean revision")
    require(r["workload_model"] == "closed_loop", "unsupported measurement model")
    uint(r["runtime_threads"], 1, 256)
    uint(r["process_id"], 1)
    elapsed = uint(r["elapsed_ns"], 1)
    uint(r["wall_anchor_unix_ns"], 1)
    anchor = uint(r["wall_anchor_monotonic_ns"], 0, elapsed)
    stages = r["stages"]
    keys(stages, "initialization warmup measurement drain verification".split())
    previous = anchor
    for stage in ("initialization", "warmup", "measurement", "drain", "verification"):
        span = stages[stage]
        keys(span, ["start_ns", "end_ns"])
        previous = uint(span["end_ns"], uint(span["start_ns"], previous, elapsed), elapsed)
    require(stages["measurement"]["end_ns"] == stages["drain"]["start_ns"], "measurement/drain boundary differs")
    stop = r["stop"]
    keys(stop, ["reason", "monotonic_ns"])
    require(stop["reason"] in ("duration", "operation_limit", "stop_file") and stop["monotonic_ns"] == stages["drain"]["start_ns"], "invalid measurement stop")
    cohort = stages["drain"]["end_ns"] - stages["measurement"]["start_ns"]
    require(uint(r["cohort_elapsed_ns"], 1) == cohort, "throughput denominator excludes drain or includes setup")
    for resources in (r["resources_before"], r["resources_after"]):
        keys(resources, "user_ticks system_ticks rss_bytes peak_rss_bytes".split())
        for value in resources.values():
            if value is not None:
                uint(value)
    # VmHWM is approximate asynchronous RSS accounting, not a monotonic counter.
    # Keep both raw samples; only CPU tick counters establish conservation.
    for counter in ("user_ticks", "system_ticks"):
        before, after = r["resources_before"][counter], r["resources_after"][counter]
        require(before is None or after is None or after >= before, "process resource counter regressed")
    h = r["history"]
    keys(h, "version mode issued terminal events bytes accounting_complete full_history_complete failure independently_checked recorder_ns peak_in_flight phase_successes".split())
    require(h["version"] == 1 and h["mode"] == c["mode"] and h["accounting_complete"] is True and h["failure"] is None and h["independently_checked"] is False, "incomplete operation accounting")
    issued = uint(h["issued"], 1, c["max_operations"])
    require(issued == h["terminal"] and h["events"] == issued * 2, "summary lacks terminal records")
    uint(h["peak_in_flight"], 1, c["workers"])
    uint(h["recorder_ns"], 0, elapsed * c["workers"])
    m = r["metrics"]
    metrics_check(m)
    require(sum(sum(sum(row) for row in phase) for phase in m["logical_counts"]) == issued, "total logical counts differ from issued work")
    for p in range(len(PHASES)):
        for op in range(3):
            logical = sum(m["logical_counts"][p][op])
            attempts = sum(m["attempt_counts"][p][op])
            local = m["logical_latency"][p][op]["outcomes"][3]["count"]
            require(logical - local <= attempts <= (logical - local) * c["client"]["max_attempts"], "attempt accounting exceeds configured limits")
    require(h["phase_successes"] == [dict(phase=phase, **{op: m["logical_counts"][p][i][0] for i, op in enumerate(OPERATIONS)}) for p, phase in enumerate(PHASES)], "phase successes differ from logical counters")
    rows = c["keys"] + 1
    setup, warmup, verification = (m["logical_counts"][p] for p in (0, 1, 3))
    require(setup[0][0] == setup[1][0] == verification[0][0] == rows and
            sum(setup[1]) == rows and not any(setup[2]) and not any(verification[1] + verification[2]),
            "required setup/final-read populations are missing")
    require(rows <= sum(setup[0]) <= rows + len(c["client"]["peers"]) - 1 and
            rows <= sum(verification[0]) <= rows * len(c["client"]["peers"]), "read probe reservation exceeded")
    require(sum(sum(row) for row in warmup) == sum(row[0] for row in warmup) == c["warmup_operations"], "warmup population is incomplete")
    measured = [phase for p, phase in enumerate(m["logical_counts"]) if p == 2 or p >= 4]
    count, success = sum(sum(sum(row) for row in phase) for phase in measured), sum(sum(row[0] for row in phase) for phase in measured)
    require(count > 0 and r["measured_issued"] == r["measured_completed"] == count and r["measured_successful"] == success, "measured cohort accounting differs")
    for name, n in (("cohort_terminal_ops_per_second", count), ("cohort_success_ops_per_second", success)):
        require(type(r[name]) in (int, float) and math.isfinite(r[name]) and math.isclose(r[name], n * 1e9 / cohort, rel_tol=1e-12, abs_tol=1e-12), "reported throughput differs from its cohort")
    result = dict(version=1, accepted=True, mode=c["mode"], report_sha256=sha(report_bytes),
                  build_sha256=sha(build_bytes), revision=b["revision"], dirty=b["dirty"],
                  full_history_independently_checked=False, measured_operations=count)
    if c["mode"] == "correctness":
        data = bounded(directory / "history.jsonl", c["history_bytes"])
        require(data.endswith(b"\n") and len(data) == h["bytes"] and sha(data) == r["history_sha256"] and h["full_history_complete"] is True, "full history bytes/hash/completion differ")
        lines = data.splitlines()
        require(len(lines) == 1 + h["events"] and len(lines[0]) + 1 <= 65536, "missing serialized history events")
        allowance = 8192 + 2 * (len(c["run_id"]) + 17) + 2 * c["value_bytes"]
        require(all(len(line) + 1 <= allowance for line in lines[1:]), "event exceeds reservation")
        verdict = history_check([strict_json(line) for line in lines], r, c, seconds)
        result.update(full_history_independently_checked=True, history=verdict)
    else:
        require(h["full_history_complete"] is False and h["bytes"] == 0 and r["history_sha256"] is None and not (directory / "history.jsonl").exists(), "performance-only run claims a complete history")
    return result
