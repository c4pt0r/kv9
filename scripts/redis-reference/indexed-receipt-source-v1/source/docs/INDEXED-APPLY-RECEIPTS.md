# Bounded apply receipts with checked ordered lookup

This isolated candidate starts from selected CRC `ca0002c7`. The accepted
point-PUT portion of the post-CRC CPU diagnostic has 165 of 3,324 selected
leaf samples (4.964%) in `NodeDriver::inspect_applied`. Its batch profile fails
the unchanged prefix-coverage check and supplies no accepted batch attribution.
The point observation identifies a concrete lookup target; it does not predict
a throughput gain or an end-to-end latency fraction.

The existing receipt vector retains 1,024 complete entries. Every full-ring
append shifts the retained suffix, and every lookup scans from the oldest
receipt. This change uses a `VecDeque` for FIFO retention and binary search
only while an incrementally checked certificate proves strictly increasing
indexes. A duplicate or reordered append permanently selects the original
first-match iterator lookup for that ring lifetime. Nothing sorts receipts or
replaces an earlier receipt with a later duplicate.

`AppliedRing` owns the sequence and its certificate privately. Its only
mutation is `push`; consumers can inspect length, endpoints and one exact
index. `RingEntry` retains the original index, term and exclusive apply outcome.
`NodeDriver::inspect_applied` changes only its receipt lookup expression.
The capacity, fatal check, watermark observation, locks, term comparison,
fence/manifest verdicts, eviction uncertainty and replacement classification
remain unchanged. The rare configuration-change receipt vector is untouched.

## Representation refinement

Let N = 1,024, and let a receipt be the full `(index, term, outcome)` tuple.
Let L be the previous vector sequence and D the deque's logical front-to-back
sequence. The observation relation is D = L. Let C be the new certificate.
The invariant is:

1. D = L and length(D) <= N.
2. C implies that D's indexes are strictly increasing.

No uniqueness or ordering premise is imposed on the input trace. These claims
hold for arbitrary finite receipt traces, including duplicates, descending
indexes, gaps, zero, maximum u64 indexes and different outcomes at one index.

### Initialization and retention

Both sequences are empty initially, so the relation and size bound hold.
The certificate starts true, and strict ordering of the empty list is vacuous.

Assume the invariant before appending receipt e. If length(L) < N, both
implementations produce L followed by e. If length(L) = N, the previous
implementation appends e and removes the first entry; the new implementation
removes the first entry and appends e. Since N > 0, both yield
`tail(L) ++ [e]`. Thus D = L and the size bound hold after every successful
append by induction. Every retained tuple and its order are identical.

### Checked ordering certificate

Before changing D, the new code sets
`C' = C && (D is empty || last(D).index < e.index)`.
If C' is true, D was strictly increasing by the induction hypothesis and e
exceeds its last index. Removing an optional first element preserves ordering;
appending e preserves strict ordering. Therefore C' implies the new D is
strictly increasing. If C' is false, the implication is vacuous. No other
operation can set C to true or mutate D. The invariant is inductive even when
the receipt producer supplies a non-increasing trace.

### Lookup equivalence

If C is false, lookup is the original first-match iterator expression over
the same sequence, so the returned complete tuple or absence is identical.
If C is true, strict ordering makes indexes unique. Rust's documented
[VecDeque binary-search contract](https://doc.rust-lang.org/std/collections/struct.VecDeque.html#method.binary_search_by_key)
returns a matching index when present and an error when absent for sorted
input. Uniqueness means the returned receipt is also the first matching
receipt. The arbitrary-match behavior for duplicates is never used: duplicates
clear C before any lookup can take this branch. Physical deque wraparound
changes neither its logical indexing nor the library's sorted-input contract.

Consequently every lookup returns exactly the same complete receipt for every
input trace and query. Length, first and last observations also agree because
the whole logical sequences agree. This proves observational equivalence of
the ring APIs, not merely equality of successful lookup indexes.

### Driver consequence

For the same fatal/watermark/state-machine observations and receipt trace,
`inspect_applied` therefore returns the same result. An exact receipt still
requires the same term; its fence or manifest verdict cannot turn into plain
success. A missing receipt below the full ring's floor remains Unconfirmed.
A missing receipt whose position was passed uses the unchanged replacement
rules. Storage, proposal eligibility, Raft confirmation, apply publication,
notification, deadlines and cancellation paths are not modified.

This is a source-level representation proof using Rust ownership, integer
comparison and standard-container contracts. It does not verify the Rust
compiler/library implementation, identical allocation-failure behavior, the
pre-existing driver's lock/state-machine composition, or the whole Raft
protocol. The project's broader formal-composition requirements remain open.

## Cost and validation status

The normal strictly increasing trace uses logarithmic lookup and avoids
shifting the entire retained suffix on eviction. The checked fallback keeps
linear first-match lookup. Removing this work may improve the measured path;
the point CPU percentage is not a guaranteed speedup.

Two passing regressions compare complete FIFO tuples and lookup results
against the previous vector behavior through multiple wraps, gaps, extreme
indexes, duplicate terms/outcomes and out-of-order inserts. They also exercise
both certificate branches. The integrated workspace suite also passes existing
driver checks for exact receipts, eviction uncertainty, overwrite/fence outcomes
and asynchronous completion.

The first compilation failed in two test-fixture constructors because the
fence/manifest region fields require `RegionId`, not a raw u64. The first
failure is retained; the correction wraps those same fixture values without
changing production code or test expectations. After that correction, local
checks pass **two focused regressions, 711 workspace tests/doctests (23
ignored), warnings-denied Clippy and formatting**. The source hashes remain
unchanged through the complete corrected validation sequence. These checks
ran only after the independent proposal candidate's timing had terminated.

An original exact-source release, process/Chaos histories and a matched
throughput/latency comparison are still required before selection. No
machine-checked proof execution is claimed. The candidate is not on master,
and no performance improvement is claimed.
