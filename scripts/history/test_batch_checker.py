#!/usr/bin/env python3
"""Independent atomic batch history controls and a small permutation oracle.

The oracle consumes raw records and uses ordinary Python dictionary operations.
It shares no transition, search, or state representation with the checker. Its
scope is a fixed existing keyspace, point get/put/delete and atomic batch get/put.
Unknown writes may have zero or one whole effect after invocation, including
after their unknown return. These tests do not authenticate receipts, establish
recorder completeness, or infer that a client did not replay an omitted call.
"""

import copy
import itertools
import random
import unittest

from checker import History, Malformed, check, verify_witness


def header(kv=(), version=2):
    return {
        "type": "header", "version": version, "range_chunk_size": 2,
        "initial": {
            "keyspaces": [{"name": "batch-test", "id": 1}],
            "kv": [{"keyspace": 1, "key": key, "value": value} for key, value in kv],
        },
    }


def invoke(oid, kind, **args):
    return {
        "type": "invoke", "id": oid, "client": str(oid), "op": kind,
        "args": {"keyspace": 1, **args},
    }


def returned(oid, outcome="ok", **result):
    return {"type": "return", "id": oid, "outcome": outcome, "result": result}


def records(*events, initial=(), version=2):
    return [header(initial, version), *[{**event, "seq": seq} for seq, event in enumerate(events)]]


def permutation_oracle(raw):
    """Enumerate every zero/full choice and legal atomic total order.

    Unknown returns are observations without a completion upper bound. Only
    confirmed returns impose real-time precedence; an unknown effect still
    cannot precede its invocation. Missing returns are handled as unknown.
    Refusals and unobserved reads have no effect and need no linearization slot.
    All inputs supplied to this small oracle are already schema-valid fixtures.
    """
    calls, replies = {}, {}
    for event in raw[1:]:
        if event["type"] == "invoke":
            calls[event["id"]] = event
        else:
            replies[event["id"]] = event
    required, optional = [], []
    for oid, call in calls.items():
        reply = replies.get(oid)
        outcome = reply["outcome"] if reply else "unknown"
        op = {
            "id": oid, "kind": call["op"], "args": call["args"],
            "start": call["seq"],
            "end": reply["seq"] if reply and outcome == "ok" else None,
            "result": reply["result"] if reply else {},
        }
        if outcome == "ok":
            required.append(op)
        elif outcome == "unknown" and call["op"] in {"put", "delete", "batch_put"}:
            optional.append(op)
    initial = {
        (row["keyspace"], row["key"]): row["value"]
        for row in raw[0]["initial"]["kv"]
    }
    for full_effect in itertools.product((False, True), repeat=len(optional)):
        selected = required + [op for op, full in zip(optional, full_effect) if full]
        for order in itertools.permutations(selected):
            rank = {op["id"]: i for i, op in enumerate(order)}
            if any(
                left["end"] is not None and left["end"] < right["start"]
                and rank[left["id"]] >= rank[right["id"]]
                for left in selected for right in selected
            ):
                continue
            values = initial.copy()
            for op in order:
                args, kind = op["args"], op["kind"]
                kid = args["keyspace"]
                if kind == "put":
                    values[kid, args["key"]] = args["value"]
                elif kind == "delete":
                    values.pop((kid, args["key"]), None)
                elif kind == "batch_put":
                    # The whole ordered input is one transition: duplicates
                    # overwrite earlier pairs without exposing intermediate KV.
                    for key, value in args["pairs"]:
                        values[kid, key] = value
                elif kind == "get":
                    if values.get((kid, args["key"])) != op["result"]["value"]:
                        break
                elif kind == "batch_get":
                    observed = [values.get((kid, key)) for key in args["keys"]]
                    if observed != op["result"]["values"]:
                        break
                else:
                    raise AssertionError(f"operation outside independent oracle: {kind}")
            else:
                return True
    return False


class BatchCheckerControls(unittest.TestCase):
    def verdict(self, raw, wanted):
        parsed = History.parse(raw)
        result = check(parsed, max_states=100_000, seconds=5)
        self.assertEqual(result["verdict"], wanted, result)
        if wanted == "valid":
            self.assertTrue(verify_witness(parsed, result["witness"]))
        return result

    def test_batch_operations_require_version_two(self):
        for kind, args, result in [
            ("batch_get", {"keys": ["61"]}, {"values": [None]}),
            ("batch_put", {"pairs": [["61", "31"]]}, {}),
        ]:
            with self.subTest(kind=kind):
                raw = records(invoke(0, kind, **args), returned(0, **result), version=1)
                with self.assertRaises(Malformed):
                    History.parse(raw)
                raw[0]["version"] = 2
                self.verdict(raw, "valid")
        self.verdict(records(invoke(0, "put", key="61", value="31"), returned(0),
                             invoke(1, "get", key="61"), returned(1, value="31"), version=1), "valid")

    def test_batch_argument_shapes_and_per_item_bounds_are_strict(self):
        bad = [
            ("batch_get", {"keys": []}),
            ("batch_get", {"keys": ["61"] * 257}),
            ("batch_get", {"keys": "61"}),
            ("batch_get", {"keys": [None]}),
            ("batch_get", {"keys": ["AA"]}),
            ("batch_get", {"keys": ["a"]}),
            ("batch_get", {"keys": ["00" * 4097]}),
            ("batch_get", {"keys": ["61"], "key": "61"}),
            ("batch_put", {"pairs": []}),
            ("batch_put", {"pairs": [["61", "31"]] * 257}),
            ("batch_put", {"pairs": {"61": "31"}}),
            ("batch_put", {"pairs": ["6131"]}),
            ("batch_put", {"pairs": [["61"]]}),
            ("batch_put", {"pairs": [["61", "31", "32"]]}),
            ("batch_put", {"pairs": [["61", None]]}),
            ("batch_put", {"pairs": [["AA", "31"]]}),
            ("batch_put", {"pairs": [["61", "A0"]]}),
            ("batch_put", {"pairs": [["00" * 4097, "31"]]}),
            ("batch_put", {"pairs": [["61", "00" * 65537]]}),
            ("batch_put", {"pairs": [["61", "31"]], "split": True}),
        ]
        for number, (kind, args) in enumerate(bad):
            with self.subTest(case=number, kind=kind):
                with self.assertRaises(Malformed):
                    History.parse(records(invoke(0, kind, **args), returned(0, "unknown")))

    def test_inclusive_item_bounds_and_empty_keys_values_are_legal(self):
        key, value = "61" * 4096, "31" * 65536
        self.verdict(records(
            invoke(0, "batch_put", pairs=[["", ""], [key, value]]), returned(0),
            invoke(1, "batch_get", keys=[key, "", key]), returned(1, values=[value, "", value]),
        ), "valid")
        self.verdict(records(
            invoke(0, "batch_put", pairs=[[f"{i:04x}", ""] for i in range(256)]), returned(0),
            invoke(1, "batch_get", keys=[f"{i:04x}" for i in reversed(range(256))]),
            returned(1, values=[""] * 256),
        ), "valid")

    def test_batch_result_shapes_cardinality_and_values_are_strict(self):
        malformed = [
            {}, {"value": None}, {"values": None}, {"values": "31"},
            {"values": []}, {"values": [None]}, {"values": [None] * 3},
            {"values": [False, None]}, {"values": ["AA", None]},
            {"values": ["a", None]}, {"values": ["00" * 65537, None]},
            {"values": [None, None], "partial": True},
        ]
        for number, result in enumerate(malformed):
            with self.subTest(case=number):
                with self.assertRaises(Malformed):
                    History.parse(records(invoke(0, "batch_get", keys=["61", "62"]),
                                          returned(0, **result)))
        for result in [{"applied_index": 1}, {"committed_items": 1}, {"values": []}]:
            with self.subTest(write_result=result):
                with self.assertRaises(Malformed):
                    History.parse(records(invoke(0, "batch_put", pairs=[["61", "31"]]),
                                          returned(0, **result)))

    def test_refusal_proof_and_unknown_result_shapes_remain_strict(self):
        for kind, args in [("batch_put", {"pairs": [["61", "31"]]}),
                           ("batch_get", {"keys": ["61"]})]:
            for outcome, result in [
                ("refused", {}), ("refused", {"proof": "timeout"}),
                ("refused", {"proof": "precommit", "committed_items": 1}),
                ("unknown", {"values": [None]}),
                ("unknown", {"committed_items": 1}),
                ("unknown", {"committed_chunks": 1}),
            ]:
                with self.subTest(kind=kind, outcome=outcome, result=result):
                    with self.assertRaises(Malformed):
                        History.parse(records(invoke(0, kind, **args), returned(0, outcome, **result)))

    def test_atomic_batch_read_cannot_observe_a_fractured_batch_write(self):
        for values, wanted in [(["30", "30"], "valid"), (["31", "31"], "valid"),
                               (["30", "31"], "invalid"), (["31", "30"], "invalid")]:
            with self.subTest(values=values):
                self.verdict(records(
                    invoke(0, "batch_put", pairs=[["61", "31"], ["62", "31"]]),
                    invoke(1, "batch_get", keys=["61", "62"]),
                    returned(1, values=values), returned(0),
                    initial=[("61", "30"), ("62", "30")],
                ), wanted)

    def test_confirmed_batch_write_has_no_partial_effect(self):
        for values in [["31", None], [None, "32"], [None, None]]:
            with self.subTest(values=values):
                self.verdict(records(
                    invoke(0, "batch_put", pairs=[["61", "31"], ["62", "32"]]), returned(0),
                    invoke(1, "batch_get", keys=["61", "62"]), returned(1, values=values),
                ), "invalid")

    def test_batch_read_retains_input_order_and_duplicate_positions(self):
        for values, wanted in [(["32", "31", "32"], "valid"),
                               (["31", "32", "31"], "invalid")]:
            with self.subTest(values=values):
                self.verdict(records(
                    invoke(0, "batch_get", keys=["62", "61", "62"]), returned(0, values=values),
                    initial=[("61", "31"), ("62", "32")],
                ), wanted)

    def test_duplicate_batch_read_keys_use_one_view_despite_overlapping_point_write(self):
        for values, wanted in [(["30", "30"], "valid"), (["31", "31"], "valid"),
                               (["30", "31"], "invalid"), (["31", "30"], "invalid")]:
            with self.subTest(values=values):
                self.verdict(records(
                    invoke(0, "batch_get", keys=["61", "61"]),
                    invoke(1, "put", key="61", value="31"), returned(1),
                    returned(0, values=values), initial=[("61", "30")],
                ), wanted)

    def test_missing_keys_and_present_empty_values_are_distinct(self):
        for values, wanted in [([None, "", None, ""], "valid"),
                               (["", None, "", None], "invalid")]:
            with self.subTest(values=values):
                self.verdict(records(
                    invoke(0, "batch_get", keys=["62", "61", "62", "61"]),
                    returned(0, values=values), initial=[("61", "")],
                ), wanted)

    def test_duplicate_batch_put_pairs_apply_in_order_with_last_pair_winning(self):
        for value, wanted in [("33", "valid"), ("31", "invalid")]:
            with self.subTest(value=value):
                self.verdict(records(
                    invoke(0, "batch_put", pairs=[["61", "31"], ["62", "32"], ["61", "33"]]), returned(0),
                    invoke(1, "batch_get", keys=["61", "62", "61"]),
                    returned(1, values=[value, "32", value]),
                ), wanted)

    def test_unknown_batch_write_may_have_zero_or_one_whole_effect(self):
        for values, wanted in [([None, None], "valid"), (["31", "32"], "valid"),
                               (["31", None], "invalid"), ([None, "32"], "invalid")]:
            with self.subTest(values=values):
                self.verdict(records(
                    invoke(0, "batch_put", pairs=[["61", "31"], ["62", "32"]]), returned(0, "unknown"),
                    invoke(1, "batch_get", keys=["61", "62"]), returned(1, values=values),
                ), wanted)

    def test_unknown_batch_write_may_settle_after_its_unknown_return(self):
        self.verdict(records(
            invoke(0, "batch_put", pairs=[["61", "31"], ["62", "32"]]), returned(0, "unknown"),
            invoke(1, "batch_get", keys=["61", "62"]), returned(1, values=[None, None]),
            invoke(2, "batch_get", keys=["61", "62"]), returned(2, values=["31", "32"]),
        ), "valid")

    def test_unknown_batch_cannot_explain_values_observed_before_its_invocation(self):
        self.verdict(records(
            invoke(0, "batch_get", keys=["61", "62"]), returned(0, values=["31", "32"]),
            invoke(1, "batch_put", pairs=[["61", "31"], ["62", "32"]]), returned(1, "unknown"),
        ), "invalid")

    def test_one_unknown_invocation_cannot_apply_twice_around_a_confirmed_overwrite(self):
        self.verdict(records(
            invoke(0, "batch_put", pairs=[["61", "31"], ["62", "31"]]), returned(0, "unknown"),
            invoke(1, "batch_get", keys=["61", "62"]), returned(1, values=["31", "31"]),
            invoke(2, "batch_put", pairs=[["61", "32"], ["62", "32"]]), returned(2),
            invoke(3, "batch_get", keys=["61", "62"]), returned(3, values=["31", "31"]),
        ), "invalid")

    def test_later_point_mutation_can_explain_a_mixed_final_batch_read(self):
        # Mixed values are not intrinsically a fracture: this point deletion
        # legally follows the unknown batch's one atomic application.
        self.verdict(records(
            invoke(0, "batch_put", pairs=[["61", "31"], ["62", "32"]]), returned(0, "unknown"),
            invoke(1, "delete", key="62"), returned(1),
            invoke(2, "batch_get", keys=["61", "62"]), returned(2, values=["31", None]),
        ), "valid")

    def test_incomplete_batch_write_is_unknown_but_still_atomic(self):
        for values, wanted in [(["31", "32"], "valid"), (["31", None], "invalid")]:
            with self.subTest(values=values):
                self.verdict(records(
                    invoke(0, "batch_put", pairs=[["61", "31"], ["62", "32"]]),
                    invoke(1, "batch_get", keys=["61", "62"]), returned(1, values=values),
                ), wanted)

    def test_refused_batch_writes_have_no_effect_and_unknown_reads_add_no_observation(self):
        prefix = [invoke(0, "batch_put", pairs=[["61", "31"]]), returned(0, "refused", proof="precommit"),
                  invoke(1, "batch_get", keys=["61"]), returned(1, "unknown"),
                  invoke(2, "batch_get", keys=["61"])]
        self.verdict(records(*prefix, returned(2, values=[None])), "valid")
        self.verdict(records(*prefix, returned(2, values=["31"])), "invalid")

    def test_batch_effects_do_not_escape_their_keyspace(self):
        raw = records(invoke(0, "batch_put", pairs=[["61", "31"]]), returned(0),
                      invoke(1, "batch_get", keys=["61"], keyspace=2), returned(1, values=[None]))
        raw[0]["initial"]["keyspaces"].append({"name": "other", "id": 2})
        self.verdict(raw, "valid")
        raw[-1]["result"]["values"] = ["31"]
        self.verdict(raw, "invalid")

    def test_independent_permutation_oracle_exhausts_overlapping_point_and_batch_choices(self):
        # These calls overlap, but the final observation follows their confirmed
        # returns. Enumerate both whole/zero unknown alternatives, every point
        # mutation, and every observed pair from a small finite alphabet.
        verdict_counts = {True: 0, False: 0}
        for outcome, mutation, observed in itertools.product(
            ("ok", "unknown", "refused"), ("put", "delete"),
            itertools.product((None, "", "31", "32"), repeat=2),
        ):
            point = invoke(1, mutation, key="61", **({"value": "32"} if mutation == "put" else {}))
            raw = records(
                invoke(0, "batch_put", pairs=[["61", "31"], ["62", "31"]]),
                point, returned(0, outcome, **({"proof": "precommit"} if outcome == "refused" else {})),
                returned(1), invoke(2, "batch_get", keys=["61", "62"]),
                returned(2, values=list(observed)),
            )
            expected = permutation_oracle(raw)
            verdict_counts[expected] += 1
            with self.subTest(outcome=outcome, mutation=mutation, observed=observed):
                self.verdict(raw, "valid" if expected else "invalid")
        self.assertGreater(verdict_counts[True], 0)
        self.assertGreater(verdict_counts[False], 0)

    def test_seeded_permutation_oracle_covers_mixed_intervals_duplicates_and_late_unknowns(self):
        rng = random.Random(20260910)
        verdict_counts = {True: 0, False: 0}
        for case in range(160):
            initial = [] if case % 2 else [("61", ""), ("62", "32")]
            scheduled = []
            kinds = ["batch_put", "batch_get", rng.choice(["put", "delete", "get"]),
                     rng.choice(["put", "delete", "get"])]
            for oid, kind in enumerate(kinds):
                if kind == "batch_put":
                    args = {"pairs": [[rng.choice(["61", "62"]), rng.choice(["", "31", "32"])]
                                      for _ in range(rng.randint(1, 3))]}
                elif kind == "batch_get":
                    args = {"keys": [rng.choice(["61", "62"]) for _ in range(rng.randint(1, 3))]}
                else:
                    args = {"key": rng.choice(["61", "62"])}
                    if kind == "put":
                        args["value"] = rng.choice(["", "31", "32"])
                outcome = rng.choice(["ok", "ok", "unknown", "refused"])
                if outcome == "refused":
                    result = {"proof": "precommit"}
                elif outcome == "unknown":
                    result = {}
                elif kind == "get":
                    result = {"value": rng.choice([None, "", "31", "32"])}
                elif kind == "batch_get":
                    result = {"values": [rng.choice([None, "", "31", "32"]) for _ in args["keys"]]}
                else:
                    result = {}
                start = rng.randrange(9)
                end = start + rng.randrange(1, 7)
                scheduled.append((start, 2 * oid, invoke(oid, kind, **args)))
                # A missing response is a live unknown operation, even if the
                # random intended outcome would otherwise have been confirmed.
                if case % 11 or oid != 0:
                    scheduled.append((end, 2 * oid + 1, returned(oid, outcome, **result)))
            raw = records(*(event for _, _, event in sorted(scheduled)), initial=initial)
            expected = permutation_oracle(copy.deepcopy(raw))
            verdict_counts[expected] += 1
            with self.subTest(case=case):
                self.verdict(raw, "valid" if expected else "invalid")
        self.assertGreater(verdict_counts[True], 0)
        self.assertGreater(verdict_counts[False], 0)


if __name__ == "__main__":
    unittest.main()
