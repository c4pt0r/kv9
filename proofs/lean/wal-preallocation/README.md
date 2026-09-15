# WAL payload capacity proof

`Capacity.lean` proves eleven universally quantified statements: the emitted
mutation/batch lengths equal the checked size formula; initial capacity and
growth policy cannot change emitted bytes; a complete reservation covers every
append prefix; and subtracting the 32-byte header plus four-byte CRC recovers
the payload length within the 64 MiB record bound. The allocation model allows
an arbitrary growth policy. It proves that growth is unnecessary after a
successful complete reservation, not a physical allocator call count.

The strict checker compiles a fresh proof with pinned Lean 4.33.1, treats warnings
as errors, audits every theorem's transitive axioms, and permits only Lean's
standard `propext`, `Classical.choice` and `Quot.sound` foundations. No `sorry`,
custom axiom or external native computation can discharge a theorem. Seven
negative controls test undercounted Put records, incorrect frame subtraction,
a proof hole, a custom axiom, a changed mutation tag, a wrong reservation and
a removed synchronization call. All must fail at the intended check.

`source-contract.json` binds the complete Rust files and reviewed declarations.
The checker also reads baseline `0fc2753` from Git and verifies that:

- the byte emitter changes only its signature and initial allocation;
- the default wrapper supplies zero capacity;
- CF coding, little-endian encoding and checked size validation are unchanged;
- append guards, frame/checksum generation, write/sync ordering, failed-writer
  fencing and publication after successful synchronization are unchanged.

The source mapping is reviewed, not a mechanized translation of Rust. Its
assumptions include safe Rust/stdlib `Vec` and slice semantics, successful
allocation/copying, the pinned compiler and ordinary 32/64-bit `usize` targets.
This proof does not establish compiler or allocator correctness, allocation
failure liveness, physical storage durability, or the complete Raft protocol.
The original CRC proof is separately rechecked with unchanged CRC declarations
and the updated whole-file source hash. The feature stays off by default until
the separate runtime/Chaos/performance selection gates pass.

Run locally, with fresh output directories on the data volume:

```sh
python3 scripts/check-wal-preallocation-proof.py \
  --lean /path/to/pinned/lean \
  --output /mnt/data/kv9-work/wal-capacity-proof-NEW
python3 scripts/check-crc32-slicing8-proof.py \
  --source-root "$PWD" --lean /path/to/pinned/lean \
  --output /mnt/data/kv9-work/wal-capacity-crc-proof-NEW
```

See the [experiment report](../../../docs/WAL-PAYLOAD-PREALLOCATION.md) for
actual results and the exact retained execution packet.
