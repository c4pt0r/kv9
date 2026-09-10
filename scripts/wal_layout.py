"""Read local WAL fixture evidence without mistaking a topology for data.

This observer checks format/checksum/chain metadata, not committed Raft or SST
authority. The server and real restart assertions retain those obligations.
An embedded segmented checkpoint takes precedence even when a stale legacy
sidecar exists. Never fall back after encountering an invalid new topology.
"""
from dataclasses import dataclass
import hashlib
from pathlib import Path
import stat
import zlib

from workload_report import bounded, strict_json

TOPOLOGY_MAGIC = b"KV9W\x03SEG1"
CHECKPOINT_MAGIC = b"KV9CHECKPOINT\x01"
MAX_TOPOLOGY_BYTES = 4 * 1024 * 1024
MAX_CHECKPOINT_BYTES = 1024 * 1024


def require(condition, message):
    if not condition:
        raise ValueError("WAL fixture evidence: " + message)


def checkpoint_manifest(data):
    require(data.startswith(CHECKPOINT_MAGIC), "invalid checkpoint magic")
    manifest = strict_json(data[len(CHECKPOINT_MAGIC):])
    require(isinstance(manifest, dict), "checkpoint must be an object")
    for name in ("term", "index"):
        require(type(manifest.get(name)) is int and manifest[name] > 0,
                "invalid checkpoint position")
    require(isinstance(manifest.get("scope"), dict) and isinstance(manifest.get("files"), list),
            "missing checkpoint scope or files")
    return manifest


class Input:
    def __init__(self, data):
        self.data = data

    def take(self, size):
        require(size <= len(self.data), "truncated topology")
        value, self.data = self.data[:size], self.data[size:]
        return value

    def integer(self, size=8):
        return int.from_bytes(self.take(size), "little")

    def position(self):
        kind, term, index = self.integer(1), self.integer(), self.integer()
        require(kind in (0, 1) and (kind or (term == 0 and index == 0)), "invalid position")
        return (term, index) if kind else None


def header(data):
    require(len(data) == 60 and data[:8] == b"KV9SEG01", "invalid segment header")
    require(zlib.crc32(data[:56]) == int.from_bytes(data[56:], "little"), "segment header checksum")
    identity = data[8:24].hex()
    sequence = int.from_bytes(data[24:32], "little")
    kind = data[32]
    term, index = int.from_bytes(data[40:48], "little"), int.from_bytes(data[48:56], "little")
    require(identity != "00" * 16 and sequence > 0, "invalid stream identity or sequence")
    require(kind in (0, 1) and data[33:40] == bytes(7) and (kind or (term == 0 and index == 0)),
            "invalid segment predecessor")
    return dict(stream=identity, sequence=sequence, previous=(term, index) if kind else None)


def covers(cut, previous):
    return previous is None or previous == cut or (previous[1] < cut[1] and previous[0] <= cut[0])


@dataclass
class Layout:
    directory: Path
    kind: str
    authority: bytes | None
    checkpoint_bytes: bytes | None
    checkpoint: dict | None
    active: dict | None = None
    closed: tuple = ()
    generation: int | None = None

    def segment_path(self, item):
        return self.directory / "catalog.segments" / item["stream"] / f'{item["sequence"]:020}.wal'

    def evidence(self):
        return dict(kind=self.kind, generation=self.generation, active=self.active,
                    closed=list(self.closed), checkpoint=self.checkpoint,
                    authority_sha256=hashlib.sha256(self.authority).hexdigest() if self.authority else None)


def decode_topology(data, directory):
    require(len(data) <= MAX_TOPOLOGY_BYTES and data.startswith(TOPOLOGY_MAGIC), "invalid topology format or size")
    require(len(data) >= len(TOPOLOGY_MAGIC) + 8 + 60 + 4 + 4 + 32, "truncated topology")
    require(hashlib.sha256(data[:-32]).digest() == data[-32:], "topology checksum")
    stream = Input(data[len(TOPOLOGY_MAGIC):-32])
    generation, active = stream.integer(), header(stream.take(60))
    require(generation > 0, "invalid topology generation")
    size = stream.integer(4)
    require(size <= MAX_CHECKPOINT_BYTES, "checkpoint exceeds byte bound")
    checkpoint_bytes = stream.take(size) if size else None
    checkpoint = checkpoint_manifest(checkpoint_bytes) if checkpoint_bytes else None
    count = stream.integer(4)
    require(count <= 4096, "closed segment count exceeds bound")
    closed = []
    for _ in range(count):
        item = header(stream.take(60))
        item.update(bytes=stream.integer(), records=stream.integer(), first=stream.position(), last=stream.position())
        flag = stream.integer(1)
        require(flag in (0, 1), "invalid unpositioned flag")
        item["has_unpositioned"] = bool(flag)
        require(item["bytes"] >= 60 + item["records"] * 40, "invalid closed summary length")
        require((item["first"] is None) == (item["last"] is None), "invalid summary position pair")
        require(not checkpoint or not flag, "checkpoint cannot reclaim unpositioned history")
        closed.append(item)
    require(not stream.data, "trailing topology bytes")
    first = closed[0] if closed else active
    cut = (checkpoint["term"], checkpoint["index"]) if checkpoint else None
    if first["sequence"] == 1:
        require(first["previous"] is None or (cut is not None and covers(cut, first["previous"])),
                "initial predecessor lacks migration checkpoint")
    else:
        require(cut is not None and covers(cut, first["previous"]), "missing prefix lacks checkpoint coverage")
    expected = {key: first[key] for key in ("stream", "sequence", "previous")}
    for item in closed:
        require(all(item[key] == value for key, value in expected.items()), "discontinuous closed segment chain")
        for before, after in ((item["previous"], item["first"]),):
            require(before is None or after is None or (after[1] > before[1] and after[0] >= before[0]),
                    "closed predecessor position regresses")
        require(item["first"] is None or (item["last"][1] >= item["first"][1] and item["last"][0] >= item["first"][0]),
                "closed summary positions regress")
        expected.update(sequence=item["sequence"] + 1, previous=item["last"] or item["previous"])
    require(active == expected, "active segment does not extend closed chain")
    return Layout(Path(directory), "segmented", data, checkpoint_bytes, checkpoint, active, tuple(closed), generation)


def read_layout(directory):
    directory = Path(directory)
    path = directory / "catalog.wal"
    try:
        with path.open("rb") as stream:
            prefix = stream.read(5)
    except FileNotFoundError:
        require(not (directory / "catalog.segments").exists(), "missing WAL root beside segment directory")
        return None
    if prefix == TOPOLOGY_MAGIC[:5]:
        return decode_topology(bounded(path, MAX_TOPOLOGY_BYTES), directory)
    # The legacy compactor may publish an entirely empty tail after a full
    # checkpoint. It is not an empty/truncated segmented topology.
    empty_legacy = prefix == b"" and not (directory / "catalog.segments").exists()
    require(empty_legacy or prefix in (b"KV9W\x01", b"KV9W\x02"), "unsupported WAL format")
    try:
        data = bounded(directory / "catalog.checkpoint", MAX_CHECKPOINT_BYTES)
    except FileNotFoundError:
        data = None
    return Layout(directory, "legacy", data, data, checkpoint_manifest(data) if data else None)


def read_checkpoint(directory):
    layout = read_layout(directory)
    return layout.checkpoint if layout else None


def wal_files(directory):
    """Enumerate all physical segment files, including unselected leftovers."""
    directory = Path(directory)
    files = [directory / "catalog.wal"]
    segments = directory / "catalog.segments"
    if segments.exists():
        require(not segments.is_symlink(), "segment directory is a symlink")
        for path in sorted(segments.rglob("*")):
            mode = path.lstat().st_mode
            require(stat.S_ISDIR(mode) or stat.S_ISREG(mode), "non-regular segment artifact")
            if stat.S_ISREG(mode):
                files.append(path)
    return files


def wal_snapshot(directory):
    """Hash the complete physical WAL set; call only while the owner is stopped."""
    directory = Path(directory)
    result = {}
    files = wal_files(directory)
    for suffix in ("migration", "topology.tmp", "upgrade.tmp", "tail.tmp", "checkpoint", "checkpoint.tmp"):
        path = directory / ("catalog." + suffix)
        if path.exists():
            require(stat.S_ISREG(path.lstat().st_mode), "non-regular WAL publication artifact")
            files.append(path)
    for path in files:
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
            size = stream.tell()
        result[str(path.relative_to(directory))] = dict(bytes=size, sha256=digest)
    return result


def wal_contains(directory, needles):
    """Search physical WAL payloads with bounded buffers, never topology alone.

    A concurrent unlink asks the live fixture to retry. This is a reclamation
    observation, not an atomic backup or an engine recovery implementation.
    """
    require(needles and all(needles), "empty payload search")
    overlap = max(map(len, needles)) - 1
    try:
        for path in wal_files(directory):
            with path.open("rb") as stream:
                previous = b""
                while block := stream.read(64 * 1024):
                    data = previous + block
                    if any(needle in data for needle in needles):
                        return True
                    previous = data[-overlap:] if overlap else b""
    except FileNotFoundError:
        return True
    return False
