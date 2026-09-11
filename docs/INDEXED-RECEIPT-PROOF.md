# Indexed receipt representation proof

The TLA+/TLAPS gate for source `74d24116` passes **13 parameterized theorems
and 122 fresh obligations**. It proves local FIFO representation equivalence,
maintenance of the checked ordering certificate, and identical found/absent
and full receipt observations under the documented standard-library contract.
This is a conditional container refinement, not a whole-Rust, Raft, liveness
or crash-durability proof. The candidate remains experimental for performance.

The [source mapping](../scripts/redis-reference/indexed-receipt-proof-v1/formal/inputs/MAPPING.md)
binds the original vector and candidate deque to exact source hashes. The
model covers every positive capacity and arbitrary incoming index sequences,
including duplicates, decreases and gaps. It adds no producer-ordering or
success-only assumption. Instantiating the index domain with every u64 value
covers absent queries as well as stored indexes. Complete index, term and
exclusive outcome payloads are preserved.

The proof first establishes equal logical retained sequences and bounded
length. A true certificate implies strict index ordering and therefore a
unique matching position. Sorted binary search then agrees with the original
first-match iterator. Once the certificate becomes false it remains false,
and lookup follows that original iterator. Rust's sorted-search contract,
ordinary container operations, integer comparison and unchanged caller locking
remain explicit trust boundaries. Allocation, rustc, cancellation and whole
Ready/apply composition are outside this proof.

## Controlled verification

The first controlled gate passes all **14 cases**. Capacity 1 explores
**37 distinct states / 667 generated**; capacity 2 explores **451 / 8,119**,
both with empty final queues. These finite checks supplement the parameterized
proof rather than establish its unbounded claims.

- A permanently true ordering certificate produces an `AROrdering`
  counterexample after duplicate indexes. This trace uses identical payloads:
  it proves the ordering certificate is invalid, not an observed wrong lookup.
- Evicting from the wrong end produces an `ARRefinement` counterexample: a
  retained `fenceRejected` outcome is replaced with `plain`.
- An omitted proof is parsed and then rejected by the semantic auditor.
- An unapproved false axiom is parsed and rejected before TLAPS.

Each mutation has a fresh positive baseline and restored run. The four
positive TLAPS phases each discharge the same 122 obligations; **488 is a
repeated-check total, not the number of distinct obligations**. Three output
controls additionally reject empty, zero-obligation and missing-summary output.
Imports, model assumptions and declared theorem names are audited against
the frozen inventory. No unchecked axiom or proof hole is accepted.

The original draft failed three of 55 obligations. The corrected proof adds
typed intermediate facts and separates tail/append lemmas, without adding
model assumptions. Its 122-obligation run and the original failed sources and
logs remain retained. The final controlled run does not modify the measured
Rust source or typed proof and passes on its first execution.

## Evidence and remaining work

The [publication index](../scripts/redis-reference/indexed-receipt-proof-v1/index.json)
selects **194 exact files / 1,088,855 bytes**, including model/proof sources,
finite configurations, original checker and semantic auditors, per-case
commands/results/logs, counterexamples, source mappings and actual terminal
records. The [detailed result](../scripts/redis-reference/indexed-receipt-proof-v1/REPORT.md)
records the precise scope. Tool binaries, caches and TLC state stores are
excluded; original absolute source/tool paths remain in the reproduction
records. This compact evidence publication is not a complete environment image.

The [process and write measurements](INDEXED-RECEIPT-PERFORMANCE.md) remain
independent evidence. Exact-source Chaos Mesh, broader reads/mixed traffic and
whole-system proof composition are still open. No hosted CI was dispatched.
