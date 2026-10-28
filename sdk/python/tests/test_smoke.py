"""End-to-end smoke against a running cluster. Skipped unless the harness sets
KV9_ADDR, KV9_TOKEN and KV9_KEYSPACE (see scripts/sdk-e2e.sh)."""

import os

import pytest

import kv9

ADDR = os.environ.get("KV9_ADDR")
TOKEN = os.environ.get("KV9_TOKEN")
KEYSPACE = os.environ.get("KV9_KEYSPACE")

pytestmark = pytest.mark.skipif(not (ADDR and TOKEN and KEYSPACE), reason="no cluster configured")


def peers():
    text = os.environ.get("KV9_PEERS", "")
    out = {}
    for item in filter(None, text.split(",")):
        node, address = item.split("=", 1)
        out[int(node)] = address
    return out


def test_raw_round_trip_batch_scan_and_delete_range():
    with kv9.Client(ADDR, TOKEN, peers=peers()) as client:
        info = client.cluster_info()
        assert info.node_count >= 1
        ks = client.keyspace(int(KEYSPACE or 0))
        prefix = b"py-smoke/"

        first = ks.put(prefix + b"k1", b"v1")
        assert first.term > 0 and first.index > 0
        assert ks.get(prefix + b"k1") == b"v1"
        assert ks.get(prefix + b"absent") is None

        second = ks.batch(
            [
                kv9.Mutation.put(prefix + b"k2", b"v2"),
                kv9.Mutation.put(prefix + b"k3", b"v3"),
                kv9.Mutation.delete(prefix + b"k1"),
            ]
        )
        assert (second.term, second.index) > (first.term, first.index) or second.term > first.term
        assert ks.batch_get([prefix + b"k1", prefix + b"k2", prefix + b"k3", prefix + b"k2"]) == [
            None,
            b"v2",
            b"v3",
            b"v2",
        ]
        assert [k for k, _ in ks.scan_all(prefix, prefix + b"\xff")] == [prefix + b"k2", prefix + b"k3"]

        ks.batch_put([(prefix + b"k4", b"v4"), (prefix + b"k5", b"v5")])
        page = ks.scan(prefix, prefix + b"\xff", limit=2)
        assert [k for k, _ in page.pairs] == [prefix + b"k2", prefix + b"k3"]

        deleted = ks.delete_range(prefix, prefix + b"\xff")
        assert deleted.committed_chunks >= 1
        assert list(ks.scan_all(prefix, prefix + b"\xff")) == []

        with pytest.raises(kv9.BoundsError):
            ks.put(b"x" * (kv9.MAX_KEY_BYTES + 1), b"v")
        with pytest.raises(kv9.BoundsError):
            ks.batch([])
