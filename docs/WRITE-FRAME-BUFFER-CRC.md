# Experimental Raft frame buffer on the CRC baseline

This candidate reapplies the single-buffer Raft WAL encoder from `01d128f` to
qualified CRC main `1ffd98a83f7095048970e51e629acda444885ef8`. The historical
standalone experiment and its measurements remain separate. No combined
throughput or latency result is established yet.

`DiskRaftStorage::write_record_unsynced` allocates the final record once,
reserves its eight-byte header, appends kind and payload, hashes the final body
in place, and fills checksum bytes 4 through 7. This removes the separate body
allocation and the copy into the complete record. The record remains
`length_BE32 || FNV1a_BE32(kind || payload) || kind || payload`.

The existing payload bound, FNV function, one `write_all`, write metrics/error
mapping, caller synchronization, failed-writer fence and publication order stay
unchanged. The current lease-epoch record and its durable incarnation handling
also remain intact. Engine slicing-by-eight CRC is unchanged; the two changes
affect different encoders. Existing Raft and engine group commit remain in use.

## Proof and qualification route

The source-bound checker requires all production bytes in `storage.rs` outside
the buffer replacement to match the exact current baseline. It checks universal
frame equality, body preservation after writing the checksum, and representable
lengths/safe slice bounds. Three deliberately incorrect layouts must yield
countermodels. FNV is an arbitrary deterministic 32-bit function: identical body
bytes imply identical checksums. Rust vector, slice, serialization and compiler
semantics remain explicit premises; this is not a whole-Rust or Raft proof.

The models retain the original assertions. A comment formatting repair places
`produce-models` on its own line. The compatibility test exercises the actual
writer and historical encoder across all 256 kind bytes and 15 lengths, then
replays all 3,840 frames. Existing crash, torn-tail and failure-fencing tests
remain relevant.

```sh
python3 -B scripts/check-raft-frame-buffer-proof.py --z3 /path/to/z3 --output /fresh/proof-output
```

Qualify the exact candidate with the frame proof, unchanged CRC proof, local
tests, formatting, Clippy, a clean default release, ordinary WAL recovery and
actual Chaos Mesh histories. Then compare against the qualified CRC baseline
using a fixed client, both run orders, point/batch writes and whole-call latency.
Retain unknown writes as unknown, all original resource guards and every cohort.
Full read/write/mixed regressions are required before promotion. The main CRC
performance result cannot be attributed to this new binary. All CI stays local.
