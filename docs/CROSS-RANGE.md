# Cross-range scans and delete-range (D02)

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D02 [#23](https://github.com/c4pt0r/kv9/issues/23). After a split, spans
that cross the child boundary finally serve.

## What this increment adds

1. **Range-sized chunking over the partition.** `raw_scan` and
   `raw_delete_range` walk the unsealed partition in key order: resolve
   the cursor's covering group, clamp the chunk to that range's end,
   serve it under that group's OWN authorization, and continue from the
   boundary. Each chunk is one group's linearizable read or committed
   delete; the span as a whole is never one snapshot — exactly the
   per-chunk contract `delete_range` has always documented.
2. **Foreign leaders pause, never break — with explicit resume.** Child
   groups can be led by different nodes, so no single server owns a
   whole span. Both `ScanResponse` and `RawDeleteRangeResponse` gain a
   `resume_from` cursor: a walk that reaches a foreign-leader chunk
   returns everything served so far plus the exact continuation point —
   even over an EMPTY page, which plain last-key pagination cannot
   express (the e2e found precisely that gap). Committed delete chunks
   stay committed; clients re-issue from the cursor, idempotently, at
   whichever node leads the next chunk. With no local progress at the
   very first chunk, the ordinary typed NotLeader redirect applies.
3. Bounded scans clamp at the boundary, limits are honored across
   chunks, keys outside a deleted span survive, and the retired parent
   stays untouched throughout.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting.
- Accepted five-process e2e (`scripts/cross-range-e2e.py`, `e2e-sixth`;
  five earlier launches retained: the first exposed the real
  foreign-leader design gap, the fourth exposed the empty-page resume
  gap — both solved by the explicit cursor — and three were harness
  defects including a probe-file string-sort bug): the full
  split-and-retire chain, cross-range scans returning BOTH halves' keys
  in order via resume cursors, bounded scans clamping at the boundary,
  a delete-range clearing keys on both sides with per-chunk receipts,
  and surviving keys intact.
- Eleven-theorem Lean model
  ([proofs/lean/cross-range](../proofs/lean/cross-range/README.md)) with
  five semantic mutation controls and two proof-policy controls;
  twenty-two sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/cross-range-v1](cross-range-v1/README.md).
