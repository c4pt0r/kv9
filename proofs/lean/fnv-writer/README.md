# Bounded Raft WAL checksum staging proof

The experimental writer groups already available Entry bodies within one
`write_entries_unsynced` call. At most four bodies, including each kind byte,
occupy at most 65,536 logical bytes. Four bodies use the independently proven
FNV kernel. Incomplete groups use scalar checksums. A body exceeding the budget
flushes previous work and uses the existing single-record path. Every call
flushes before returning; no request is delayed waiting for a future Ready.

Run the complete gate from a fresh output directory:

```sh
python3 scripts/check-fnv-writer-proof.py \
  --source-root "$PWD" --output /tmp/kv9-fnv-writer-proof-fresh \
  --lean /path/to/lean-4.33.1/bin/lean --rustc /path/to/rustc
```

The checker first runs the complete [kernel gate](../fnv-interleave/README.md)
against this checkout, including its original committed scalar comparison,
debug/optimized Rust tests and rejection controls. It prepends that exact Lean
kernel model to `Writer.lean`, checks all 15 writer theorems, and inspects each
theorem's transitive axioms. Only `propext`, `Quot.sound` and `Classical.choice`
are permitted. Source, model, contract, runner and toolchain inputs are retained;
every subprocess has an invocation and terminal record. Inputs must be unchanged.

## Source mapping and statements

| Rust operation | Lean mapping / checked property |
| --- | --- |
| Entry serialization, then kind byte | A nonempty `Body`; protobuf encoding is an explicit external premise. |
| `EntryBodies::can_push` | `fits`; `payload_guard` proves the subtraction guard equals the body-inclusive bound without adding to an arbitrary payload length. |
| `EntryBodies::push` | `push`; appends to pending bodies in order. `push_bound` permits at most four bodies within the budget before immediate flush. |
| Budget flush, oversized fallback, full group flush | `stage`; `stage_view` preserves the consumed body sequence and `stage_bound` leaves fewer than four pending bodies. |
| Loop and final flush | `fold_view`, `fold_bound`, `complete_stream`; arbitrary finite input lists preserve order, bounds and the complete output body sequence. |
| Four checksums, same length/checksum/body encoding | `four_frames` specializes the kernel equality to any deterministic existing frame encoding, preserving every byte and lane. |
| Encoding or append failure | `written_prefix` and `partial_frame_prefix`; unwritten pending bodies and a short current write can leave only an ordered prefix of the original frame stream before a crash. |
| Sync, acknowledgment and failure fencing | `with_writer`, `append`, `persist_ready`, `sync_records` and `next_record` declarations must match committed CRC baseline `bd42e60` exactly. Actual storage/Ready fault tests cover their composition with the new writer. |

The source contract pins the entire writer, kernel, test module and both models,
and the exact mapped declarations. It is a reviewed source-to-model mapping,
not a machine-verified Rust frontend or proof of the compiler, allocator,
protobuf library, filesystem or whole Raft protocol. Rust iteration, slice/index,
integer and `Write::write_all` semantics are explicit premises. The original
64 MiB replay cap bounds the `u32` frame-length cast. The successful-sync
durability assumption is unchanged.

The 64 KiB bound counts staged body lengths/requested body allocation sizes,
not allocator metadata or physical allocation rounding. Original serialization
and one output-frame allocation still exist; large records take the original
single-record path after dropping staged bodies. `mem::take` releases each
staged allocation instead of carrying its old capacity into the next group.
An error exits the writer closure and poisons it. Encoding ahead may change the
amount of unacknowledged data written before an error; identical failure-prefix
lengths are neither promised nor required. Physical crash persistence and
corruption are separate recovery-model and Chaos obligations.

## Controls and current result

Seven writer controls reject reordered pending bodies, a budget overrun, missing
flush at four bodies, wrong checksum lane, an enlarged Rust budget, missing sync
and a custom false axiom. Six separate kernel controls and eight Rust test
passes remain part of the gate.

The qualified gate is retained at
`/tmp/kv9-fnv-writer-proof-20260914-qualified`, actual session `42579`, terminal
`133027/0`. The first development model had tactic/type errors; its corrected
second model compiled. Two complete checker attempts then exposed control
construction errors: removing an `if` caused a tactic-shape failure, and a sync
mutation matched multiple source sites. Both attempts are retained. The final
controls use a wrong flush threshold and the exact sync helper statement,
respectively; no production change was needed for these corrections.

This checkpoint supplies no database performance, release, ordinary-recovery
or actual Chaos Mesh acceptance for the new candidate. Those remain required
before runtime selection.
