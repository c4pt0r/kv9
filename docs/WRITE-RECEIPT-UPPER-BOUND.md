# Receipt lookup: conservative upper-bound experiment

Candidate [`e2e23cc`](https://github.com/c4pt0r/kv9/commit/e2e23cca5e70a9ea0cc241877b3b35b5b6433d27)
adds a conservative negative-lookup filter to the applied-receipt vector.
Its source-bound proof, local development checks, retained default/diagnostic
releases and ordinary three-voter recovery pass. **It is experimental: actual
Chaos Mesh and the full matched performance screen remain open. CRC remains
selected.**

## Why this candidate

The completed [write observer capture](WRITE-OBSERVER-CAPTURE.md) measures
86.00–86.11% receipt lookup misses at c64 Put, with approximately 1,022 logical
comparisons per lookup. These measurements motivate testing a filter for
impossible future-index queries. They do not establish how many observed
misses this filter will avoid or predict an end-to-end speedup.

This experiment is independent of the [held receipt-tail candidate](WRITE-RECEIPT-TAIL-PERFORMANCE.md).
That candidate's direct-hint/fallback counts remain separate unfinished work.
The new filter does not cache a previous pending outcome or bypass receipt
inspection when other driver state changes.

## Algorithm and safety boundary

The original vector gains one `u64` upper bound. It starts at zero and becomes
`max(previous_bound, inserted_index)` for every inserted receipt. Eviction never
reduces the bound. Both values remain protected by the existing applied-ring
mutex; vector append, prefix eviction, capacity and allocation order remain
unchanged.

Every retained receipt therefore has an index at most the bound. A query above
that bound cannot match, so it returns `None` without a scan. All other queries
use the original first-match search and return the complete stored
`(index, term, outcome)`. Arbitrary gaps, duplicate or reordered indexes and
evicting `u64::MAX` preserve this property. An obsolete high bound only causes
additional scans; no index arithmetic can wrap.

Six unique inverse driver edits reconstruct the parent driver byte for byte.
The surrounding fatal check, command and unified watermarks, eviction-unknown
decision, replacement discriminator and full outcome propagation remain
unchanged. The same applies to original deadlines, cancellation/stop handling,
quorum commitment, persistence, local apply and client-success rules. An absent
receipt does not authorize success or by itself select a terminal error.

## Verified source checkpoint

The [parameterized TLA+/TLAPS model and code contract](https://github.com/c4pt0r/kv9/blob/e2e23cca5e70a9ea0cc241877b3b35b5b6433d27/proofs/tlaps/receipt-upper-bound/README.md)
pass **14 theorem statements and 82 fresh obligations**. The semantic audit
checks exact imported modules, the sole legal-input assumption, all theorem
names and absence of proof holes. The induction permits arbitrary receipt
order and any positive capacity; first-match refinement preserves complete
receipt values. A context-congruence theorem preserves any subsequent observer
of the same lookup result.

TLC exhausts 37 distinct states at capacity 1 and 523 at capacity 2. Both
deliberately broken variants produce the required actual wrong-lookup witness:

- Setting the bound to the latest index loses an existing index-1 receipt
  after inserting index 0.
- Rejecting queries equal to the bound loses an index-0 receipt immediately
  after its insertion.

The negative configurations check lookup refinement directly, so an internal
bound-invariant failure cannot substitute for the intended witness. Omitted
proof and false-axiom controls are rejected; three proof-output controls also
pass. Both earlier failed proof drafts remain preserved.

Default workspace checks pass **792 tests/doctests, with 23 ignored**. The
initial diagnostic Raft run has 258 passed and one TCP fixture bind failure;
the exact isolated failed test then passes with unchanged source. The original
run remains failed, and the fixture's released-port race is not yet repaired.
The server's scoped diagnostic schema/JSON-size check, default and diagnostic
warnings-denied Clippy, formatting and explicit production experimental-lease
compilation pass. The successful Raft tests were not replayed after only the
server's schema assertions changed.

The committed blobs match all 13 proof-contract file hashes. This combines
mechanized abstract refinement, reviewed source mapping and local Rust checks;
it is not a verified Rust compiler, whole-Raft proof or fault-injected end-to-end
acceptance. It establishes lookup equality under the existing mutex, without
assuming identical wall-clock scheduling or proving global liveness.

## Actual-path observations

Diagnostic schema 2 records the lengths of rings whose scans were skipped,
including empty-ring skip counts. Its separate offline checker preserves
process/exporter continuity, bounded histograms, monotonicity, invalid/saturated
rejection and the original service/Ready accounting. It checks:

`ring_length.sum = linear_probes.sum + hit_slots_behind_tail.sum + skipped_ring_length.sum`

Skipped samples must be a subpopulation of ring lengths and misses. Zero-probe
counts must equal skipped lookups plus non-skipped empty-ring lookups. Eleven
new synthetic controls pass. No live schema-2 capture has been interpreted,
and schema-1 baseline captures retain their original meaning. Aggregate data
cannot disambiguate every empty-miss reclassification; no per-event claim is
made beyond the available counters.

## Next gates

The [release and ordinary recovery checkpoint](WRITE-RECEIPT-UPPER-BOUND-RUNTIME.md)
binds the same 1,116-file candidate to retained default server/native client and
diagnostic server builds. Actual Cargo records and compiler invocations confirm
the intended features and release settings. Both stream and unary leader-loss/
original-directory-restart histories pass independent checks: 360 completed
operations, 330 successes and 30 explicit unknowns. Six fresh drains and all
seven owned client/voter lifetimes are checked. This is ordinary process
recovery on one host, not actual Chaos Mesh or physical power-loss evidence.

1. Complete actual full21 Chaos Mesh against the retained default candidate.
   Preserve unknown outcomes and all original failures.
2. Run the original eight-smoke/sixteen-ten-second point/batch matched screen
   with the fixed native v3 client and selected CRC reference. Judge throughput
   and latency by both opposite orders; no diagnostic sweep replaces this gate.
3. Capture actual upper-bound skip/fallback populations separately and keep
   the held tail-hint comparison distinct. Promote only after the complete
   correctness/performance decision, then continue the industrial storage and
   dynamic multi-Raft/split route recorded in issue #9.

All checks remain local; no hosted workflow is dispatched for this checkpoint.
[Original proof, failed drafts, source checks and synthetic observer evidence](receipt-upper-bound-source-v1/README.md).
