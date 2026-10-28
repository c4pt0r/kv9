"""Marker classification, exercised without a server."""

import grpc
import pytest

from kv9 import errors


class FakeRpcError(grpc.RpcError):
    def __init__(self, code, details="", trailing=()):
        self._code = code
        self._details = details
        self._trailing = tuple(trailing)

    def code(self):
        return self._code

    def details(self):
        return self._details

    def trailing_metadata(self):
        return self._trailing


def test_not_leader_with_and_without_hint():
    error = FakeRpcError(
        grpc.StatusCode.FAILED_PRECONDITION,
        "not leader",
        [("kv9-not-leader", "true"), ("kv9-leader-node-id", "3")],
    )
    assert errors.classify(error, read=False) == errors.NotLeaderError(3)
    bare = FakeRpcError(grpc.StatusCode.FAILED_PRECONDITION, "", [("kv9-not-leader", "true")])
    assert errors.classify(bare, read=True) == errors.NotLeaderError(None)


def test_not_leader_marker_must_be_exact():
    for trailing in (
        [("kv9-not-leader", "yes")],
        [("kv9-not-leader", "true"), ("kv9-leader-node-id", "0")],
        [("kv9-not-leader", "true"), ("kv9-leader-node-id", "03")],
    ):
        with pytest.raises(errors.ProtocolError):
            errors.classify(FakeRpcError(grpc.StatusCode.FAILED_PRECONDITION, "", trailing), read=False)
    with pytest.raises(errors.ProtocolError):
        errors.classify(FakeRpcError(grpc.StatusCode.UNAVAILABLE, "", [("kv9-not-leader", "true")]), read=False)


def test_read_unconfirmed_is_a_read_only_unavailable_outcome():
    error = FakeRpcError(grpc.StatusCode.UNAVAILABLE, "", [("kv9-read-unconfirmed", "quorum")])
    assert errors.classify(error, read=True) == errors.ReadUnconfirmedError("quorum")
    with pytest.raises(errors.ProtocolError):
        errors.classify(error, read=False)
    with pytest.raises(errors.ProtocolError):
        errors.classify(FakeRpcError(grpc.StatusCode.UNAVAILABLE, "", [("kv9-read-unconfirmed", "later")]), read=True)


def test_admission_and_partial_write():
    refused = FakeRpcError(grpc.StatusCode.RESOURCE_EXHAUSTED, "", [("kv9-admission-refused", "encoded_bytes")])
    assert errors.classify(refused, read=False) == errors.AdmissionRefusedError("encoded_bytes")
    partial = FakeRpcError(
        grpc.StatusCode.UNAVAILABLE,
        "leader changed",
        [
            ("kv9-partial-write", "true"),
            ("kv9-committed-chunks", "2"),
            ("kv9-last-applied-term", "4"),
            ("kv9-last-applied-index", "77"),
        ],
    )
    assert errors.classify(partial, read=False) == errors.PartialWriteError(2, 4, 77, "leader changed")


def test_unmarked_status_stays_a_plain_rpc_error():
    error = errors.classify(FakeRpcError(grpc.StatusCode.FAILED_PRECONDITION, "stale epoch"), read=False)
    assert isinstance(error, errors.RpcError)
    assert error.precondition
    other = errors.classify(FakeRpcError(grpc.StatusCode.DEADLINE_EXCEEDED, "x"), read=False)
    assert isinstance(other, errors.RpcError) and not other.precondition
    with pytest.raises(errors.ProtocolError):
        errors.classify(FakeRpcError(grpc.StatusCode.INTERNAL, "", [("kv9-something-new", "1")]), read=False)
