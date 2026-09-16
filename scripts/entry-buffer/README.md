# Single-buffer engine experiment

This isolated candidate stores key and value bytes in one private `Box<[u8]>`
with a checked key boundary. The persistent map uses `EntryBuffer` as its key
and unit as its value. Production engine files are unchanged.

From source base `e9ebdc42f2171fd5ab62f3fc2d2d010d88e55d22`, with these tools
available, use a fresh evidence directory under `/mnt/data/kv9-work`:

```sh
python3 scripts/entry-buffer/prepare.py ROOT
python3 scripts/inline-key/test.py ROOT
python3 scripts/entry-buffer/layout.py ROOT
python3 scripts/entry-buffer/prove.py ROOT /path/to/pinned/lean
python3 scripts/entry-buffer/audit.py ROOT
```

The source contract pins the actual reviewed candidate bytes and Lean binary.
Preparation and output directories intentionally refuse reuse; retain failures
and adapt a separately identified attempt when source or tooling changes.

The reused test runner and generated BTreeMap model are unchanged. Both arms
also run `model.rs`, which checks repeated same-key payload replacement,
different key boundaries for identical concatenated bytes, old snapshots,
owned output mutation and range deletion. `entry_buffer.rs` adds scalar overflow,
byte round-trip, ownership and key-only comparison tests. `layout.rs` uses the
existing jemalloc request counter, emits 198 rows and no latency measurement.

The [report](../../docs/ENTRY-BUFFER-QUALIFICATION.md) links the evidence packet
and the prospective staged timing plan. This directory does not yet implement
or execute that timing screen. Do not rerun the failed inline40 screen or infer
database throughput from allocator requests.
