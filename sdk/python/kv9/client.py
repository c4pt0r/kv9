"""Synchronous kv9 client built directly on ``grpcio``.

One :class:`Client` holds one authenticated connection per node address it has
spoken to and follows ``NotLeader`` hints through a ``peers`` map. A
:class:`Keyspace` carries the request context (keyspace id + region epoch) every raw
operation needs and refreshes the epoch once when the server reports a stale view.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable, Iterator, Mapping, Optional, Sequence

import grpc

from . import errors
from ._gen import kv9_pb2 as pb
from ._gen import kv9_pb2_grpc as rpc

MAX_KEY_BYTES = 4096
MAX_VALUE_BYTES = 65_536
MAX_BATCH_ITEMS = 256
MAX_MESSAGE_BYTES = 1_048_576
MAX_DEADLINE_SECONDS = 30.0


@dataclass(frozen=True)
class Epoch:
    """The region epoch a request believes in. Legacy keyspaces live at ``(1, 1)``."""

    conf_ver: int = 1
    version: int = 1


@dataclass(frozen=True)
class Applied:
    """The exact applied position of a write. Compare both fields, never the index alone."""

    term: int
    index: int


@dataclass(frozen=True)
class RegionLocation:
    region_id: int
    epoch: Epoch
    leader_node_id: Optional[int]


@dataclass(frozen=True)
class KeyspaceInfo:
    id: int
    name: str
    tenant_id: int


@dataclass(frozen=True)
class ClusterInfo:
    node_count: int
    keyspace_count: int
    region_count: int


@dataclass(frozen=True)
class ScanPage:
    pairs: list[tuple[bytes, bytes]]
    #: Nonempty when the walk paused at a foreign-leader chunk: continue exactly here,
    #: even if ``pairs`` is empty.
    resume_from: bytes


@dataclass(frozen=True)
class DeleteRangeResult:
    committed_chunks: int
    last_applied: Applied
    resume_from: bytes


@dataclass(frozen=True)
class Mutation:
    """One entry of an atomic range batch. ``value is None`` means delete."""

    key: bytes
    value: Optional[bytes] = None

    @classmethod
    def put(cls, key: bytes, value: bytes) -> "Mutation":
        return cls(key, value)

    @classmethod
    def delete(cls, key: bytes) -> "Mutation":
        return cls(key, None)


def _check_key(key: bytes, what: str = "key") -> None:
    if not isinstance(key, (bytes, bytearray)):
        raise errors.BoundsError(f"{what} must be bytes")
    if len(key) > MAX_KEY_BYTES:
        raise errors.BoundsError(f"{what} exceeds {MAX_KEY_BYTES} bytes")


def _check_value(value: bytes) -> None:
    if not isinstance(value, (bytes, bytearray)):
        raise errors.BoundsError("value must be bytes")
    if len(value) > MAX_VALUE_BYTES:
        raise errors.BoundsError(f"value exceeds {MAX_VALUE_BYTES} bytes")


def _check_count(count: int) -> None:
    if not 1 <= count <= MAX_BATCH_ITEMS:
        raise errors.BoundsError(f"a batch carries 1..={MAX_BATCH_ITEMS} items, got {count}")


def _check_message(message) -> None:
    if message.ByteSize() > MAX_MESSAGE_BYTES:
        raise errors.BoundsError(f"encoded request exceeds {MAX_MESSAGE_BYTES} bytes")


class Client:
    """An authenticated client for one kv9 cluster.

    :param address: ``host:port`` of any node's public endpoint.
    :param token: a client token from the server's ``KV9_CLIENT_TOKENS``.
    :param timeout: per-call deadline in seconds (at most 30).
    :param peers: ``node_id -> address`` used to follow ``NotLeader`` hints.
    :param max_redirects: how many hints to follow for one logical call.
    """

    def __init__(
        self,
        address: str,
        token: str,
        *,
        timeout: float = 5.0,
        peers: Optional[Mapping[int, str]] = None,
        max_redirects: int = 3,
        channel_options: Optional[Sequence[tuple[str, object]]] = None,
    ) -> None:
        if not token:
            raise errors.BoundsError("client token must be non-empty")
        if not 0 < timeout <= MAX_DEADLINE_SECONDS:
            raise errors.BoundsError(f"timeout must be in (0, {MAX_DEADLINE_SECONDS}] seconds")
        self._address = address
        self._metadata = (("authorization", f"Bearer {token}"),)
        self._timeout = timeout
        self._peers = dict(peers or {})
        self._max_redirects = max_redirects
        self._options = list(channel_options or ()) + [
            ("grpc.max_receive_message_length", MAX_MESSAGE_BYTES),
            ("grpc.max_send_message_length", MAX_MESSAGE_BYTES),
        ]
        self._stubs: dict[str, tuple[grpc.Channel, rpc.Kv9Stub]] = {}

    # ------------------------------------------------------------------ lifecycle
    def close(self) -> None:
        for channel, _ in self._stubs.values():
            channel.close()
        self._stubs.clear()

    def __enter__(self) -> "Client":
        return self

    def __exit__(self, *exc) -> None:
        self.close()

    @property
    def address(self) -> str:
        """The node the client currently addresses (moves with followed hints)."""
        return self._address

    def _stub(self, address: str) -> rpc.Kv9Stub:
        entry = self._stubs.get(address)
        if entry is None:
            channel = grpc.insecure_channel(address, options=self._options)
            entry = (channel, rpc.Kv9Stub(channel))
            self._stubs[address] = entry
        return entry[1]

    def _invoke(self, method: str, request, *, read: bool):
        hops = 0
        while True:
            stub = self._stub(self._address)
            try:
                return getattr(stub, method)(request, metadata=self._metadata, timeout=self._timeout)
            except grpc.RpcError as rpc_error:
                error = errors.classify(rpc_error, read=read)
                if isinstance(error, errors.NotLeaderError) and hops < self._max_redirects:
                    target = self._peers.get(error.leader_node_id or -1)
                    if target and target != self._address:
                        self._address = target
                        hops += 1
                        continue
                raise error from None

    # ---------------------------------------------------------------------- admin
    def create_keyspace(self, name: str, tenant_id: int = 0) -> int:
        """Create a raw keyspace served by the metadata group; returns its id."""
        response = self._invoke(
            "CreateKeyspace",
            pb.CreateKeyspaceRequest(name=name, tenant_id=tenant_id, api_type=pb.API_TYPE_RAW),
            read=False,
        )
        return response.keyspace_id

    def list_keyspaces(self) -> list[KeyspaceInfo]:
        response = self._invoke("ListKeyspaces", pb.ListKeyspacesRequest(), read=True)
        return [KeyspaceInfo(k.id, k.name, k.tenant_id) for k in response.keyspaces]

    def cluster_info(self) -> ClusterInfo:
        r = self._invoke("ClusterInfo", pb.ClusterInfoRequest(), read=True)
        return ClusterInfo(r.node_count, r.keyspace_count, r.region_count)

    def get_region(self, keyspace_id: int, key: bytes = b"") -> RegionLocation:
        _check_key(key)
        r = self._invoke("GetRegion", pb.GetRegionRequest(keyspace_id=keyspace_id, key=key), read=True)
        region = r.region
        leader = region.leader_node_id if region.HasField("leader_node_id") else None
        return RegionLocation(region.region_id, Epoch(region.epoch.conf_ver, region.epoch.version), leader)

    def keyspace(self, keyspace_id: int, epoch: Epoch = Epoch(), *, auto_refresh: bool = True) -> "Keyspace":
        return Keyspace(self, keyspace_id, epoch, auto_refresh=auto_refresh)


class Keyspace:
    """Raw operations on one keyspace under one region epoch."""

    def __init__(self, client: Client, keyspace_id: int, epoch: Epoch, *, auto_refresh: bool) -> None:
        if not 0 < keyspace_id <= 0xFF_FFFF:
            raise errors.BoundsError("keyspace id must be a nonzero 3-byte value")
        self._client = client
        self.id = keyspace_id
        self.epoch = epoch
        self._auto_refresh = auto_refresh

    def _context(self) -> pb.RequestContext:
        return pb.RequestContext(
            keyspace_id=self.id,
            region_epoch=pb.RegionEpoch(conf_ver=self.epoch.conf_ver, version=self.epoch.version),
        )

    def refresh_epoch(self, key: bytes = b"") -> Epoch:
        """Re-read the region epoch from the cluster and adopt it."""
        self.epoch = self._client.get_region(self.id, key).epoch
        return self.epoch

    def _call(self, method: str, build, *, read: bool, anchor: bytes):
        request = build()
        _check_message(request)
        refreshed = False
        while True:
            try:
                return self._client._invoke(method, request, read=read)
            except errors.RpcError as error:
                if not (error.precondition and self._auto_refresh and not refreshed):
                    raise
                refreshed = True
                before = self.epoch
                self.refresh_epoch(anchor)
                if self.epoch == before:
                    raise
                request = build()

    # ---------------------------------------------------------------------- reads
    def get(self, key: bytes) -> Optional[bytes]:
        _check_key(key)
        r = self._call("RawGet", lambda: pb.RawGetRequest(context=self._context(), key=key), read=True, anchor=key)
        return r.value.value if r.value.found else None

    def batch_get(self, keys: Sequence[bytes]) -> list[Optional[bytes]]:
        """One quorum-confirmed view; results align with ``keys`` (duplicates included)."""
        _check_count(len(keys))
        for key in keys:
            _check_key(key)
        r = self._call(
            "RawBatchGet",
            lambda: pb.RawBatchGetRequest(context=self._context(), keys=list(keys)),
            read=True,
            anchor=keys[0],
        )
        if len(r.values) != len(keys):
            raise errors.ProtocolError("batch_get returned a different cardinality")
        return [v.value if v.found else None for v in r.values]

    def scan(self, start: bytes = b"", end: bytes = b"", limit: int = 1024) -> ScanPage:
        """One page of ``[start, end)``; an empty ``end`` means "to the keyspace end"."""
        _check_key(start, "start")
        _check_key(end, "end")
        if limit <= 0:
            raise errors.BoundsError("limit must be positive")
        r = self._call(
            "RawScan",
            lambda: pb.RawScanRequest(context=self._context(), start=start, end=end, limit=limit),
            read=True,
            anchor=start,
        )
        return ScanPage([(p.key, p.value) for p in r.pairs], bytes(r.resume_from))

    def scan_all(self, start: bytes = b"", end: bytes = b"", page: int = 1024) -> Iterator[tuple[bytes, bytes]]:
        """Walk ``[start, end)`` page by page, following ``resume_from``."""
        cursor = start
        while True:
            result = self.scan(cursor, end, page)
            yield from result.pairs
            if result.resume_from:
                cursor = result.resume_from
            elif len(result.pairs) < page:
                return
            else:
                cursor = result.pairs[-1][0] + b"\x00"

    # --------------------------------------------------------------------- writes
    @staticmethod
    def _applied(r) -> Applied:
        if r.applied_term == 0 or r.applied_index == 0:
            raise errors.ProtocolError("write acknowledged without an applied position")
        return Applied(r.applied_term, r.applied_index)

    def put(self, key: bytes, value: bytes) -> Applied:
        _check_key(key)
        _check_value(value)
        return self._applied(
            self._call(
                "RawPut",
                lambda: pb.RawPutRequest(context=self._context(), key=key, value=value),
                read=False,
                anchor=key,
            )
        )

    def delete(self, key: bytes) -> Applied:
        _check_key(key)
        return self._applied(
            self._call("RawDelete", lambda: pb.RawDeleteRequest(context=self._context(), key=key), read=False, anchor=key)
        )

    def batch_put(self, pairs: Sequence[tuple[bytes, bytes]]) -> Applied:
        """Put-only atomic batch: every pair lands at one applied position."""
        _check_count(len(pairs))
        for key, value in pairs:
            _check_key(key)
            _check_value(value)
        return self._applied(
            self._call(
                "RawBatchPut",
                lambda: pb.RawBatchPutRequest(
                    context=self._context(), pairs=[pb.KeyValue(key=k, value=v) for k, v in pairs]
                ),
                read=False,
                anchor=pairs[0][0],
            )
        )

    def batch(self, mutations: Iterable[Mutation]) -> Applied:
        """The atomic range batch: ordered puts and deletes applied as ONE Raft entry.

        Every key must lie in the same region; a batch crossing a region is refused
        whole (``FAILED_PRECONDITION``), nothing is proposed, and the SDK never splits
        it for you. A later mutation for the same key wins; deleting an absent key is a
        no-op."""
        mutations = list(mutations)
        _check_count(len(mutations))
        wire = []
        for m in mutations:
            _check_key(m.key)
            if m.value is None:
                wire.append(pb.RawMutation(delete=m.key))
            else:
                _check_value(m.value)
                wire.append(pb.RawMutation(put=pb.KeyValue(key=m.key, value=m.value)))
        return self._applied(
            self._call(
                "RawBatch",
                lambda: pb.RawBatchRequest(context=self._context(), mutations=wire),
                read=False,
                anchor=mutations[0].key,
            )
        )

    def delete_range(self, start: bytes, end: bytes) -> DeleteRangeResult:
        """Delete ``[start, end)`` in bounded chunks. Not atomic as a whole: see
        :class:`~kv9.errors.PartialWriteError` and ``resume_from``."""
        _check_key(start, "start")
        _check_key(end, "end")
        r = self._call(
            "RawDeleteRange",
            lambda: pb.RawDeleteRangeRequest(context=self._context(), start=start, end=end),
            read=False,
            anchor=start,
        )
        return DeleteRangeResult(
            r.committed_chunks,
            Applied(r.last_applied_term, r.last_applied_index),
            bytes(r.resume_from),
        )
