"""Typed outcomes of the kv9 public API.

Every refusal the server can express is decoded from the *trailing metadata markers*
it sends, never from the human-readable message (see ``sdk/README.md``). Anything the
markers do not name stays a plain :class:`RpcError`: for a write that means the
outcome is unknown and the caller decides whether replaying is safe.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional

import grpc

NOT_LEADER_KEY = "kv9-not-leader"
LEADER_HINT_KEY = "kv9-leader-node-id"
READ_UNCONFIRMED_KEY = "kv9-read-unconfirmed"
ADMISSION_REFUSED_KEY = "kv9-admission-refused"
PARTIAL_WRITE_KEY = "kv9-partial-write"
COMMITTED_CHUNKS_KEY = "kv9-committed-chunks"
LAST_APPLIED_TERM_KEY = "kv9-last-applied-term"
LAST_APPLIED_INDEX_KEY = "kv9-last-applied-index"


class Kv9Error(Exception):
    """Base class of every error raised by this SDK."""


class BoundsError(Kv9Error, ValueError):
    """The request violates a documented bound; nothing was sent."""


class ProtocolError(Kv9Error):
    """The server answered with a marker the SDK cannot interpret."""


@dataclass(eq=True)
class RpcError(Kv9Error):
    """An RPC failed without a typed marker. A write's outcome is unknown."""

    code: grpc.StatusCode
    message: str

    def __str__(self) -> str:  # pragma: no cover - formatting only
        return f"{self.code.name}: {self.message}"

    @property
    def precondition(self) -> bool:
        """``FAILED_PRECONDITION`` without a marker: stale epoch, a batch crossing a
        region, or an API-type mismatch."""
        return self.code == grpc.StatusCode.FAILED_PRECONDITION


@dataclass(eq=True)
class NotLeaderError(Kv9Error):
    """The addressed node is not the leader. Nothing executed; retry elsewhere."""

    leader_node_id: Optional[int]

    def __str__(self) -> str:  # pragma: no cover
        return f"not leader (hint: node {self.leader_node_id})"


@dataclass(eq=True)
class ReadUnconfirmedError(Kv9Error):
    """The read barrier did not establish within the deadline; nothing was answered.

    ``phase`` is ``"quorum"`` (the ReadIndex round-trip) or ``"apply"`` (catch-up)."""

    phase: str

    def __str__(self) -> str:  # pragma: no cover
        return f"read unconfirmed ({self.phase})"


@dataclass(eq=True)
class AdmissionRefusedError(Kv9Error):
    """Refused before execution: ``request_count``, ``encoded_bytes`` or
    ``request_too_large``. Back off and retry."""

    reason: str

    def __str__(self) -> str:  # pragma: no cover
        return f"admission refused ({self.reason})"


@dataclass(eq=True)
class PartialWriteError(Kv9Error):
    """A ``delete_range`` committed some chunks and then failed. The committed chunks
    stay committed; ``last_applied_*`` is the last position that applied."""

    committed_chunks: int
    last_applied_term: int
    last_applied_index: int
    cause: str

    def __str__(self) -> str:  # pragma: no cover
        return (
            f"partial write: {self.committed_chunks} chunk(s) committed through "
            f"({self.last_applied_term}, {self.last_applied_index}): {self.cause}"
        )


def _markers(error: grpc.RpcError) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    trailing = error.trailing_metadata() if hasattr(error, "trailing_metadata") else None
    for key, value in trailing or ():
        if isinstance(value, bytes):
            value = value.decode("utf-8", "replace")
        out.setdefault(key, []).append(value)
    return out


def _single(markers: dict[str, list[str]], key: str) -> Optional[str]:
    values = markers.get(key)
    if values is None:
        return None
    if len(values) != 1:
        raise ProtocolError(f"marker {key} repeated")
    return values[0]


def _integer(markers: dict[str, list[str]], key: str) -> int:
    text = _single(markers, key)
    if text is None or not text.isdigit() or str(int(text)) != text:
        raise ProtocolError(f"marker {key} is not a canonical integer")
    return int(text)


def classify(error: grpc.RpcError, *, read: bool) -> Kv9Error:
    """Turn a ``grpc.RpcError`` into the typed error the markers describe."""
    code = error.code()
    message = error.details() or ""
    markers = _markers(error)

    if NOT_LEADER_KEY in markers:
        if _single(markers, NOT_LEADER_KEY) != "true" or code != grpc.StatusCode.FAILED_PRECONDITION:
            raise ProtocolError("malformed not-leader marker")
        leader = None
        if LEADER_HINT_KEY in markers:
            leader = _integer(markers, LEADER_HINT_KEY)
            if leader == 0:
                raise ProtocolError("leader hint names node 0")
        return NotLeaderError(leader)

    if ADMISSION_REFUSED_KEY in markers:
        reason = _single(markers, ADMISSION_REFUSED_KEY)
        if reason not in ("request_count", "encoded_bytes", "request_too_large"):
            raise ProtocolError(f"unknown admission refusal {reason!r}")
        return AdmissionRefusedError(reason)

    if PARTIAL_WRITE_KEY in markers:
        if _single(markers, PARTIAL_WRITE_KEY) != "true":
            raise ProtocolError("malformed partial-write marker")
        return PartialWriteError(
            committed_chunks=_integer(markers, COMMITTED_CHUNKS_KEY),
            last_applied_term=_integer(markers, LAST_APPLIED_TERM_KEY),
            last_applied_index=_integer(markers, LAST_APPLIED_INDEX_KEY),
            cause=message,
        )

    if READ_UNCONFIRMED_KEY in markers:
        phase = _single(markers, READ_UNCONFIRMED_KEY)
        if not read or code != grpc.StatusCode.UNAVAILABLE or phase not in ("quorum", "apply"):
            raise ProtocolError("malformed read-unconfirmed marker")
        return ReadUnconfirmedError(phase)

    if any(key.startswith("kv9-") for key in markers):
        raise ProtocolError(f"unexpected control marker(s): {sorted(markers)}")
    return RpcError(code, message)
