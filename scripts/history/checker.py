#!/usr/bin/env python3
"""Independent interval-history checker for Raw KV and catalog uniqueness.

No kv9 implementation is imported. Search returns a witness, exhaustive failure,
or inconclusive. Snapshot-selected range deletion is checked as a multi-step
operation, separately from the atomic point/scan and catalog operations.
"""
from __future__ import annotations

import argparse
from collections import Counter
from dataclasses import dataclass
import json
from pathlib import Path
import time


class Malformed(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise Malformed(message)


def integer(value, minimum=0):
    return type(value) is int and value >= minimum


def hex_bytes(value):
    require(isinstance(value, str) and len(value) % 2 == 0, "invalid hex bytes")
    require(all(c in "0123456789abcdef" for c in value), "hex must be canonical lowercase")
    return value


@dataclass(frozen=True)
class Operation:
    id: int
    kind: str
    args: dict
    invocation: int
    response: int | None
    outcome: str
    result: dict


@dataclass
class History:
    header: dict
    events: list[dict]
    operations: list[Operation]
    initial: tuple
    ids: tuple[int, ...]

    @classmethod
    def parse(cls, records):
        require(bool(records), "empty history")
        header, *events = records
        require(isinstance(header, dict), "header must be an object")
        require(header.get("type") == "header" and type(header.get("version")) is int and header["version"] == 1,
                "expected history version 1 header")
        chunk = header.get("range_chunk_size")
        require(integer(chunk, 1), "range_chunk_size must be positive")
        initial = header.get("initial", {})
        require(isinstance(initial, dict), "initial state must be an object")
        require(isinstance(initial.get("keyspaces", []), list) and isinstance(initial.get("kv", []), list),
                "initial catalog and KV must be arrays")
        catalog, kv = {}, {}
        for item in initial.get("keyspaces", []):
            require(isinstance(item, dict) and set(item) == {"name", "id"}, "invalid initial catalog row")
            name, kid = item["name"], item["id"]
            require(isinstance(name, str) and name and integer(kid, 1) and kid < 1 << 24, "invalid initial catalog")
            require(name not in catalog and kid not in catalog.values(), "duplicate initial catalog")
            catalog[name] = kid
        for item in initial.get("kv", []):
            require(isinstance(item, dict) and set(item) == {"keyspace", "key", "value"}, "invalid initial KV row")
            kid, key, value = item["keyspace"], hex_bytes(item["key"]), hex_bytes(item["value"])
            require(integer(kid, 1) and kid in catalog.values(), "initial KV has no catalog entry")
            require((kid, key) not in kv, "duplicate initial KV key")
            kv[kid, key] = value
        calls, returns, previous, observed_ids = {}, {}, -1, set(catalog.values())
        for event in events:
            require(isinstance(event, dict), "event must be an object")
            seq, oid, kind = event.get("seq"), event.get("id"), event.get("type")
            require(integer(seq) and seq > previous, "event sequence is not strictly increasing")
            require(integer(oid), "operation id must be a nonnegative integer")
            previous = seq
            if kind == "invoke":
                require(oid not in calls, "duplicate invocation")
                require(isinstance(event.get("client"), str) and event["client"], "missing client")
                require(isinstance(event.get("phase", "unspecified"), str) and event.get("phase", "unspecified"), "invalid phase")
                cls.validate_args(event.get("op"), event.get("args"))
                calls[oid] = event
                if "keyspace" in event["args"]:
                    observed_ids.add(event["args"]["keyspace"])
            elif kind == "return":
                require(oid in calls and oid not in returns, "unmatched or duplicate response")
                require(isinstance(event.get("observation", {}), dict), "observation must be an object")
                require(not event.get("observation", {}).get("malformed"), "recorder observed a malformed response")
                cls.validate_result(calls[oid]["op"], event.get("outcome"), event.get("result"))
                returns[oid] = event
                if calls[oid]["op"] == "create_keyspace" and event["outcome"] == "ok":
                    observed_ids.add(event["result"]["id"])
            else:
                raise Malformed("unknown history event type")
        ops = []
        for oid, call in calls.items():
            response = returns.get(oid)
            ops.append(Operation(oid, call["op"], call["args"], call["seq"],
                                 response["seq"] if response else None,
                                 response["outcome"] if response else "unknown",
                                 response["result"] if response else {}))
        require(bool(ops), "zero operations cannot constitute a history")
        return cls(header, events, ops, freeze(kv, catalog), tuple(sorted(observed_ids)))

    @staticmethod
    def validate_args(kind, args):
        require(isinstance(kind, str), "operation must be a string")
        require(isinstance(args, dict), "arguments must be an object")
        if kind == "create_keyspace":
            require(set(args) == {"name"} and isinstance(args["name"], str) and args["name"],
                    "invalid create_keyspace arguments")
            return
        expected = {
            "put": {"keyspace", "key", "value"}, "get": {"keyspace", "key"},
            "delete": {"keyspace", "key"}, "scan": {"keyspace", "start", "end", "limit"},
            "delete_range": {"keyspace", "start", "end"},
        }
        require(kind in expected and set(args) == expected[kind], "unsupported operation/arguments")
        require(integer(args["keyspace"], 1) and args["keyspace"] < 1 << 24, "invalid keyspace id")
        for name in ("key", "value", "start", "end"):
            if name in args:
                hex_bytes(args[name])
        if kind == "scan":
            require(integer(args["limit"]), "invalid scan limit")

    @staticmethod
    def validate_result(kind, outcome, result):
        require(isinstance(outcome, str) and outcome in {"ok", "unknown", "refused"} and isinstance(result, dict),
                "invalid outcome/result")
        if outcome == "refused":
            require(result == {"proof": "precommit"}, "refusal needs explicit precommit evidence")
            return
        if outcome == "unknown":
            require(not result or (kind == "delete_range" and set(result) == {"committed_chunks"}
                                   and integer(result["committed_chunks"])), "invalid unknown result")
            return
        if kind in {"put", "delete"}:
            require(not result, "write result must be empty (receipts belong in observation fields)")
        elif kind == "get":
            require(set(result) == {"value"}, "invalid get result")
            if result["value"] is not None:
                hex_bytes(result["value"])
        elif kind == "scan":
            require(set(result) == {"rows"} and isinstance(result["rows"], list), "invalid scan result")
            for row in result["rows"]:
                require(isinstance(row, list) and len(row) == 2, "invalid scan row")
                hex_bytes(row[0]); hex_bytes(row[1])
        elif kind == "delete_range":
            require(set(result) == {"committed_chunks"} and integer(result["committed_chunks"]),
                    "invalid range receipt")
        else:
            require(set(result) == {"id"} and integer(result["id"], 1) and result["id"] < 1 << 24,
                    "invalid created id")


def freeze(kv, catalog):
    return tuple(sorted(kv.items())), tuple(sorted(catalog.items()))


def in_range(key, args):
    return key >= args["start"] and (not args["end"] or key < args["end"])


# Progress is None (not selected), "done" (atomic/fully applied), or
# (immutable selected keys, applied chunk count) for a range delete.
def transitions(history, index, state, progress):
    op = history.operations[index]
    if progress == "done":
        return
    kv, catalog = map(dict, state)
    args = op.args
    if op.outcome == "refused" or (op.outcome == "unknown" and op.kind in {"get", "scan"}):
        yield state, "done", {"phase": "omit"}
        return
    if op.kind == "create_keyspace":
        name = args["name"]
        if name in catalog:
            return
        candidates = [op.result["id"]] if op.outcome == "ok" else [*history.ids, f"unobserved:{op.id}"]
        for kid in candidates:
            if isinstance(kid, str) and sum(isinstance(v, str) for v in catalog.values()) >= (1 << 24) - 1 - len(history.ids):
                continue
            if kid not in catalog.values():
                yield freeze(kv, {**catalog, name: kid}), "done", {"phase": "create", "allocated": kid}
        return
    kid = args["keyspace"]
    if kid not in catalog.values():
        return
    if op.kind == "put":
        kv[kid, args["key"]] = args["value"]
    elif op.kind == "delete":
        kv.pop((kid, args["key"]), None)
    elif op.kind == "get":
        if kv.get((kid, args["key"])) != op.result["value"]:
            return
    elif op.kind == "scan":
        rows = [[key, value] for (space, key), value in sorted(kv.items())
                if space == kid and in_range(key, args)][:args["limit"]]
        if rows != op.result["rows"]:
            return
    else:
        chunk = history.header["range_chunk_size"]
        if progress is None:
            keys = tuple(key for space, key in sorted(kv) if space == kid and in_range(key, args))
            total = (len(keys) + chunk - 1) // chunk
            if op.outcome == "ok" and total != op.result["committed_chunks"]:
                return
            if op.outcome == "unknown" and total < op.result.get("committed_chunks", 0):
                return
            yield state, (keys, 0), {"phase": "select", "keys": list(keys)}
            return
        keys, count = progress
        if count * chunk >= len(keys):
            return
        # A typed partial return terminates its server loop: at most one
        # additional proposal may remain unresolved after its known prefix.
        if op.outcome == "unknown" and "committed_chunks" in op.result and count >= op.result["committed_chunks"] + 1:
            return
        deleted = keys[count * chunk:(count + 1) * chunk]
        for key in deleted:
            kv.pop((kid, key), None)
        yield freeze(kv, catalog), (keys, count + 1), {"phase": "chunk", "keys": list(deleted)}
        return
    yield freeze(kv, catalog), "done", {"phase": "atomic"}


def response_satisfied(history, index, progress):
    op = history.operations[index]
    if op.kind == "delete_range" and op.outcome != "refused":
        if op.outcome == "unknown" and not op.result:
            return True
        return progress is not None and progress[1] >= op.result["committed_chunks"]
    return op.outcome == "unknown" or progress == "done"


def advance(history, cursor, progress, indexes):
    while cursor < len(history.events):
        event = history.events[cursor]
        if event["type"] == "return" and not response_satisfied(history, indexes[event["id"]], progress.get(indexes[event["id"]])):
            break
        if event["type"] == "return" and history.operations[indexes[event["id"]]].outcome != "unknown":
            progress.pop(indexes[event["id"]], None)
        cursor += 1
    return cursor


def verify_witness(history, witness):
    """Replay search output from scratch, including every response barrier."""
    indexes = {op.id: i for i, op in enumerate(history.operations)}
    state, progress, cursor = history.initial, {}, 0
    require(isinstance(witness, list), "witness must be an array")
    for action in witness:
        require(isinstance(action, dict) and set(action) == {"id", "before_event", "detail"}, "invalid witness action")
        require(integer(action["id"]) and action["id"] in indexes, "unknown witness operation")
        boundary = action["before_event"]
        require(integer(boundary) and cursor <= boundary < len(history.events), "invalid witness boundary")
        for event in history.events[cursor:boundary]:
            if event["type"] == "return":
                i = indexes[event["id"]]
                require(response_satisfied(history, i, progress.get(i)), "witness skipped a response")
        i = indexes[action["id"]]
        op = history.operations[i]
        require(op.invocation < history.events[boundary]["seq"], "witness effect before invocation")
        matches = [(s, p) for s, p, detail in transitions(history, i, state, progress.get(i))
                   if detail == action["detail"]]
        require(len(matches) == 1, "witness violates the sequential model")
        state, progress[i] = matches[0]
        cursor = boundary
    for event in history.events[cursor:]:
        if event["type"] == "return":
            i = indexes[event["id"]]
            require(response_satisfied(history, i, progress.get(i)), "witness missed a confirmed result")
    return True


def observation_distance(history, index, state):
    """Witness-ordering heuristic only; never used by unrestricted exhaustion."""
    op = history.operations[index]
    kv, catalog = map(dict, state)
    args = op.args
    if op.kind == 'create_keyspace':
        return int(args['name'] in catalog or op.result.get('id') in catalog.values())
    kid = args['keyspace']
    if kid not in catalog.values():
        return 1 << 30
    if op.kind == 'get':
        return int(kv.get((kid, args['key'])) != op.result['value'])
    if op.kind == 'scan':
        rows = [(key, value) for (space, key), value in sorted(kv.items())
                if space == kid and in_range(key, args)][:args['limit']]
        return len(set(rows).symmetric_difference(map(tuple, op.result['rows'])))
    if op.kind == 'delete_range':
        count = sum(space == kid and in_range(key, args) for space, key in kv)
        chunk = history.header['range_chunk_size']
        return abs((count + chunk - 1) // chunk - op.result.get('committed_chunks', 0))
    return 0


def search(history, max_states=200000, seconds=10.0, unknown_limit=None, guided_unknown=False):
    start = time.monotonic()
    indexes = {op.id: i for i, op in enumerate(history.operations)}
    stack = [(0, history.initial, (), None, 0)]
    seen = set()
    while stack:
        if len(seen) >= max_states or time.monotonic() - start >= seconds:
            return {"verdict": "inconclusive", "reason": "search_budget", "states": len(seen)}
        cursor, state, frozen_progress, parent, unknown_effects = stack.pop()
        progress = dict(frozen_progress)
        cursor = advance(history, cursor, progress, indexes)
        signature = cursor, state, tuple(sorted(progress.items()))
        if signature in seen:
            continue
        seen.add(signature)
        if cursor == len(history.events):
            witness = []
            while parent is not None:
                action, parent = parent
                witness.append(action)
            witness.reverse()
            verify_witness(history, witness)
            return {"verdict": "valid", "states": len(seen), "witness": witness}
        event = history.events[cursor]
        wanted = indexes[event["id"]]
        # Search likely witnesses first without pruning other legal orders.
        order = sorted(range(len(indexes)), key=lambda i: (i != wanted, history.operations[i].outcome == "unknown", i))
        children = []
        for i in order:
            op = history.operations[i]
            if op.invocation >= event["seq"] or (op.outcome != "unknown" and op.response is not None and op.response < event["seq"]):
                continue
            if op.outcome == "unknown" and op.kind in {"get", "scan"}:
                continue  # No observation or side effect: omission commutes with every transition.
            for next_state, next_progress, detail in transitions(history, i, state, progress.get(i)):
                additional = int(op.outcome == "unknown" and detail["phase"] != "omit")
                if guided_unknown and additional:
                    lookahead = next_state
                    if detail['phase'] == 'select':
                        # A range selection has no immediate effect. Rank it by
                        # the deletion it could subsequently perform.
                        values, catalog = map(dict, next_state)
                        for key in next_progress[0]:
                            values.pop((op.args['keyspace'], key), None)
                        lookahead = freeze(values, catalog)
                    if observation_distance(history, wanted, lookahead) >= observation_distance(history, wanted, state):
                        continue
                if unknown_limit is not None and unknown_effects + additional > unknown_limit:
                    continue
                updated = dict(progress); updated[i] = next_progress
                action = {"id": op.id, "before_event": cursor, "detail": detail}
                children.append((cursor, next_state, tuple(sorted(updated.items())), (action, parent), unknown_effects + additional))
                if len(stack) + len(children) + len(seen) >= max_states:
                    return {"verdict": "inconclusive", "reason": "search_frontier_budget", "states": len(seen)}
        stack.extend(reversed(children))
    return {"verdict": "invalid", "reason": "no_legal_execution", "states": len(seen)}


def check(history, max_states=200000, seconds=10.0):
    # Fast witness attempts do not establish invalidity. Only an unrestricted
    # exhausted search may return invalid. Every positive result is replayed
    # against the complete model, including unknown-write effects.
    started, used, attempts = time.monotonic(), 0, []
    for limit, guided in [(0, False), (None, True), (1, False), (2, False), (4, False), (None, False)]:
        remaining = seconds - (time.monotonic() - started)
        budget = max_states - used
        if remaining <= 0 or budget <= 0:
            return {"verdict": "inconclusive", "reason": "search_budget", "states": used, "attempts": attempts}
        cap = budget if limit is None else min(3000, budget)
        if guided:
            cap = min(20000, budget)
        result = search(history, cap, remaining, limit, guided)
        used += result["states"]
        attempts.append({"unknown_effect_limit": limit, "guided_unknown": guided, "verdict": result["verdict"], "states": result["states"]})
        if result["verdict"] == "valid" or (limit is None and not guided):
            return {**result, "states": used, "attempts": attempts}
    raise AssertionError("unrestricted search must return a verdict")


def coverage(history):
    phases = {e['id']: e.get('phase', 'unspecified') for e in history.events if e['type'] == 'invoke'}
    by_phase = {}
    for op in history.operations:
        if op.outcome == 'ok':
            counts = by_phase.setdefault(phases[op.id], Counter())
            counts[op.kind] += 1
    return {
        "operations": len(history.operations),
        "outcomes": dict(Counter(op.outcome for op in history.operations)),
        "successful": dict(Counter(op.kind for op in history.operations if op.outcome == "ok")),
        "kinds": dict(Counter(op.kind for op in history.operations)),
        "successful_by_phase": {name: dict(counts) for name, counts in by_phase.items()},
    }


def minimize_prefix(history, seconds=5.0):
    """Find a smaller proven-invalid prefix; never remove causal writes.

    Removing an arbitrary write can manufacture a stale-read counterexample.
    Prefixes preserve all invocations; incomplete calls become unknown. Budget
    exhaustion returns the last proven-invalid prefix, never a guessed result.
    """
    end, lower, upper = time.monotonic() + seconds, 1, len(history.events)
    best = history.events
    while lower < upper and time.monotonic() < end:
        middle = (lower + upper) // 2
        candidate = History.parse([history.header, *history.events[:middle]])
        result = check(candidate, seconds=max(0.001, end - time.monotonic()))
        if result["verdict"] == "invalid":
            best, upper = candidate.events, middle
        elif result["verdict"] == "valid":
            lower = middle + 1
        else:
            break
    return [history.header, *best]


def load(path):
    try:
        records = [json.loads(line) for line in Path(path).read_text().splitlines()]
        require(all(isinstance(record, dict) for record in records), "records must be objects")
        return History.parse(records)
    except (KeyError, TypeError, json.JSONDecodeError) as error:
        raise Malformed(f"malformed history: {error}") from error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("history", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--max-states", type=int, default=200000)
    parser.add_argument("--seconds", type=float, default=10.0)
    parser.add_argument("--acceptance", action="store_true")
    parser.add_argument("--require", nargs="*", default=[])
    parser.add_argument("--require-phase", nargs="*", default=[])
    args = parser.parse_args()
    started = time.monotonic()
    try:
        history = load(args.history)
        result = check(history, args.max_states, args.seconds)
        result["coverage"] = coverage(history)
        if args.acceptance and result["verdict"] == "valid":
            successes = result["coverage"]["successful"]
            if not any(successes.get(k) for k in ("put", "delete", "delete_range")) or not any(successes.get(k) for k in ("get", "scan")) or any(not successes.get(k) for k in args.require):
                result.update(verdict="inconclusive", reason="insufficient_successful_coverage")
            for phase in args.require_phase:
                counts = result['coverage']['successful_by_phase'].get(phase, {})
                if not counts.get('put') or not (counts.get('get') or counts.get('scan')):
                    result.update(verdict='inconclusive', reason='insufficient_fault_phase_coverage')
        if result["verdict"] == "invalid":
            prefix = minimize_prefix(history)
            counterexample = args.output.with_suffix(".counterexample.jsonl")
            counterexample.write_text("".join(json.dumps(e, sort_keys=True) + "\n" for e in prefix))
            result["counterexample"] = str(counterexample)
    except (Malformed, OSError) as error:
        result = {"verdict": "inconclusive", "reason": "malformed_or_unreadable_history", "error": str(error)}
    result["elapsed_seconds"] = time.monotonic() - started
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(json.dumps({k: v for k, v in result.items() if k != "witness"}, sort_keys=True))
    return {"valid": 0, "invalid": 1, "inconclusive": 2}[result["verdict"]]


if __name__ == "__main__":
    raise SystemExit(main())
