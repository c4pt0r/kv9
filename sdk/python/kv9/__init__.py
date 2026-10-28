"""kv9 — client SDK for the kv9 distributed raw key-value store.

    import kv9

    with kv9.Client("127.0.0.1:20161", token="...", peers={1: "127.0.0.1:20161"}) as client:
        ks = client.keyspace(client.create_keyspace("app"))
        ks.put(b"k", b"v")
        ks.batch([kv9.Mutation.put(b"a", b"1"), kv9.Mutation.delete(b"k")])
        assert ks.get(b"k") is None

See ``sdk/README.md`` for the contract shared by every kv9 SDK.
"""

from .client import (
    MAX_BATCH_ITEMS,
    MAX_KEY_BYTES,
    MAX_MESSAGE_BYTES,
    MAX_VALUE_BYTES,
    Applied,
    Client,
    ClusterInfo,
    DeleteRangeResult,
    Epoch,
    Keyspace,
    KeyspaceInfo,
    Mutation,
    RegionLocation,
    ScanPage,
)
from .errors import (
    AdmissionRefusedError,
    BoundsError,
    Kv9Error,
    NotLeaderError,
    PartialWriteError,
    ProtocolError,
    ReadUnconfirmedError,
    RpcError,
)

__all__ = [
    "Applied",
    "Client",
    "ClusterInfo",
    "DeleteRangeResult",
    "Epoch",
    "Keyspace",
    "KeyspaceInfo",
    "Mutation",
    "RegionLocation",
    "ScanPage",
    "MAX_BATCH_ITEMS",
    "MAX_KEY_BYTES",
    "MAX_MESSAGE_BYTES",
    "MAX_VALUE_BYTES",
    "AdmissionRefusedError",
    "BoundsError",
    "Kv9Error",
    "NotLeaderError",
    "PartialWriteError",
    "ProtocolError",
    "ReadUnconfirmedError",
    "RpcError",
]

__version__ = "0.1.0"
