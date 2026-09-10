"""Controls against stale-sidecar and topology-only acceptance false positives."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import zlib

from wal_layout import (CHECKPOINT_MAGIC, TOPOLOGY_MAGIC, decode_topology,
                        read_checkpoint, read_layout, wal_contains, wal_snapshot)


def checkpoint(index=7, term=2):
    return CHECKPOINT_MAGIC + json.dumps(dict(term=term, index=index,
        scope=dict(cluster="fixture", region=1, conf_ver=1, version=1), files=[])).encode()


def position(at):
    term, index = at or (0, 0)
    return bytes([at is not None]) + term.to_bytes(8, "little") + index.to_bytes(8, "little")


def header(sequence=1, previous=None, identity=bytes([31]) * 16):
    p = position(previous)
    data = b"KV9SEG01" + identity + sequence.to_bytes(8, "little") + p[:1] + bytes(7) + p[1:]
    return data + zlib.crc32(data).to_bytes(4, "little")


def topology(active=None, anchor=None, closed=()):
    active = active or header()
    anchor = anchor or b""
    data = (TOPOLOGY_MAGIC + (1).to_bytes(8, "little") + active + len(anchor).to_bytes(4, "little")
            + anchor + len(closed).to_bytes(4, "little") + b"".join(closed))
    return data + hashlib.sha256(data).digest()


def sealed(sequence=1, previous=None, last=(2, 7)):
    return (header(sequence, previous) + (100).to_bytes(8, "little") + (1).to_bytes(8, "little")
            + position(last) + position(last) + b"\x00")


class LayoutTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="kv9-wal-fixture-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def write(self, data):
        (self.root / "catalog.wal").write_bytes(data)

    def test_legacy_checkpoint_remains_supported_without_reading_the_whole_wal(self):
        self.write(b"KV9W\x02" + bytes(5 * 1024 * 1024))
        self.assertIsNone(read_checkpoint(self.root))
        (self.root / "catalog.checkpoint").write_bytes(checkpoint())
        self.assertEqual(read_checkpoint(self.root)["index"], 7)
        self.assertEqual(read_layout(self.root).kind, "legacy")

    def test_embedded_checkpoint_wins_over_stale_or_malformed_legacy_sidecar(self):
        self.write(topology(anchor=checkpoint(20)))
        for old in (checkpoint(999), b"corrupt old sidecar"):
            (self.root / "catalog.checkpoint").write_bytes(old)
            self.assertEqual(read_checkpoint(self.root)["index"], 20)
        self.assertEqual(read_layout(self.root).checkpoint_bytes, checkpoint(20))

    def test_empty_legacy_compacted_tail_keeps_checkpoint_authority(self):
        self.write(b"")
        self.assertIsNone(read_checkpoint(self.root))
        (self.root / "catalog.checkpoint").write_bytes(checkpoint(7))
        self.assertEqual(read_checkpoint(self.root)["index"], 7)
        self.assertEqual(read_layout(self.root).kind, "legacy")

    def test_missing_or_short_root_beside_segments_is_not_absent_legacy_data(self):
        self.assertIsNone(read_layout(self.root))
        (self.root / "catalog.segments").mkdir()
        with self.assertRaises(ValueError):
            read_layout(self.root)
        for short in (b"", b"KV9W", TOPOLOGY_MAGIC):
            self.write(short)
            with self.assertRaises(ValueError):
                read_layout(self.root)

    def test_new_stream_without_anchor_does_not_use_legacy_sidecar(self):
        self.write(topology())
        (self.root / "catalog.checkpoint").write_bytes(checkpoint(999))
        self.assertIsNone(read_checkpoint(self.root))

    def test_corrupt_or_future_topology_never_falls_back_to_sidecar(self):
        (self.root / "catalog.checkpoint").write_bytes(checkpoint(999))
        valid = topology(anchor=checkpoint())
        for data in (valid[:-1] + bytes([valid[-1] ^ 1]), valid[:20], b"KV9W\x04anything"):
            self.write(data)
            with self.assertRaises(ValueError):
                read_checkpoint(self.root)
            self.assertEqual((self.root / "catalog.wal").read_bytes(), data)

    def test_complete_topology_requires_covered_prefix_and_continuous_chain(self):
        valid = topology(header(2, (2, 7)), closed=(sealed(),))
        layout = decode_topology(valid, self.root)
        self.assertEqual(layout.active["sequence"], 2)
        self.assertEqual(layout.closed[0]["last"], (2, 7))
        for data in (topology(header(2, (2, 7))),
                     topology(header(2, (2, 7)), anchor=checkpoint(6)),
                     topology(header(3, (2, 7)), closed=(sealed(),)),
                     topology(header(2, (2, 8)), closed=(sealed(),))):
            with self.assertRaises(ValueError):
                decode_topology(data, self.root)

    def test_migration_predecessor_can_be_covered_by_a_later_checkpoint(self):
        layout = decode_topology(topology(header(1, (2, 7)), anchor=checkpoint(20)), self.root)
        self.assertEqual(layout.active["previous"], (2, 7))
        with self.assertRaises(ValueError):
            decode_topology(topology(header(1, (2, 7)), anchor=checkpoint(6)), self.root)

    def test_checksums_do_not_hide_bad_counts_flags_or_trailing_metadata(self):
        good = topology(header(2, (2, 7)), closed=(sealed(),))
        variants = []
        body = bytearray(good[:-32]); body[-1] = 2; variants.append(body)
        variants.append(good[:-32] + b"extra")
        body = bytearray(good[:-32]); body[81:85] = (4097).to_bytes(4, "little"); variants.append(body)
        for body in variants:
            with self.assertRaises(ValueError):
                decode_topology(bytes(body) + hashlib.sha256(body).digest(), self.root)

    def test_payload_search_includes_selected_segments_and_unselected_leftovers(self):
        self.write(topology(anchor=checkpoint()))
        layout = read_layout(self.root)
        active = layout.segment_path(layout.active)
        active.parent.mkdir(parents=True)
        active.write_bytes(b"payload exists outside topology")
        self.assertTrue(wal_contains(self.root, [b"payload"]))
        active.write_bytes(b"no target data")
        orphan = active.with_name("99999999999999999999.wal")
        orphan.write_bytes(b"old payload not yet unlinked")
        self.assertTrue(wal_contains(self.root, [b"payload"]))
        orphan.unlink()
        self.assertFalse(wal_contains(self.root, [b"payload"]))

    def test_payload_search_matches_across_bounded_read_chunks(self):
        self.write(b"KV9W\x02" + b"x" * (65536 - 5 - 3) + b"needle-crosses-boundary")
        self.assertTrue(wal_contains(self.root, [b"needle-crosses-boundary"]))
        self.assertFalse(wal_contains(self.root, [b"absent"]))

    def test_preflight_snapshot_detects_segment_edits_additions_and_deletion(self):
        self.write(topology())
        layout = read_layout(self.root)
        active = layout.segment_path(layout.active)
        active.parent.mkdir(parents=True)
        active.write_bytes(b"original")
        before = wal_snapshot(self.root)
        self.assertEqual(before[str(active.relative_to(self.root))]["bytes"], 8)
        active.write_bytes(b"modified")
        self.assertNotEqual(wal_snapshot(self.root), before)
        active.write_bytes(b"original")
        self.assertEqual(wal_snapshot(self.root), before)
        extra = active.with_name("other.wal"); extra.write_bytes(b"created")
        self.assertNotEqual(wal_snapshot(self.root), before)
        extra.unlink(); active.unlink()
        self.assertNotEqual(wal_snapshot(self.root), before)

    def test_segment_symlink_cannot_escape_evidence_inventory(self):
        self.write(topology())
        segments = self.root / "catalog.segments"
        segments.mkdir()
        (segments / "outside").symlink_to("/etc/passwd")
        with self.assertRaises(ValueError):
            wal_snapshot(self.root)


if __name__ == "__main__":
    unittest.main()
