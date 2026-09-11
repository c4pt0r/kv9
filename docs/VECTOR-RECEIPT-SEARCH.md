# Indexed receipt lookup with unchanged vector retention

This experiment isolates the lookup portion of `74d24116`. It starts from
current main `d084e58`, whose database sources match selected CRC `ca0002c7`.
The previous combined deque/indexed candidate improves c64 mixed throughput
2.775% and GET mean 2.924%, but pure c64 GET falls 0.411% with higher tails.
The selected-source profile places 4.189% of mixed CPU samples in linear
receipt search. Neither observation identifies the cause of the pure-read
shift or predicts this experiment's gain.

## Exact change

`AppliedRing` privately owns the same `Vec<RingEntry>` and one Boolean ordering
certificate. `push` performs the original operations in the original order:
append, inspect length, and drain the oldest excess prefix above 1,024. It does
not preallocate, pop before append, convert the container, sort receipts or
discard duplicates. Vector growth and eviction behavior remain unchanged.
The only added work on insertion is checking whether the last retained index
is strictly below the new index and combining that fact with the certificate.

While the certificate is true, lookup uses slice binary search. Duplicate or
decreasing indexes permanently clear it and use the original oldest-first
iterator. The whole index/term/exclusive outcome tuple is returned unchanged.
The driver changes only the receipt container adapter and lookup expression.
Its locks, fatal/watermark checks, exact term/verdict interpretation, eviction
unknowns and replacement rules remain unchanged. Proposal admission, persistence,
ReadIndex, apply publication, notification and cancellation are unchanged.

## Conditional representation proof

Let N be any positive capacity. Let V be the reference vector and S the
candidate's vector of complete receipts. Let C be the checked certificate.
For any finite input trace, including duplicate/decreasing/gapped indexes and
distinct outcomes at one index, maintain:

1. S = V and their lengths are at most N.
2. C implies strict increasing order of the retained indexes.

Both vectors start empty and C starts true. Before appending e, compute
`C' = C && (S is empty || last(S).index < e.index)`.
Both vectors then execute the identical append-and-trim transition. Equality,
bounded length and complete payload/order preservation therefore follow
directly. If C' is true, the induction hypothesis and the last-index comparison
give strict order after append; taking a suffix preserves strict order. If
C' is false, the ordering implication is vacuous, and future conjunctions
cannot make it true again. This proves the invariant by induction without
assuming producer ordering or uniqueness.

When C is false, both lookups use the identical first-match expression over
identical vectors. When C is true, strict order gives unique indexes. The
standard slice sorted-search contract then returns exactly the unique matching
receipt, or absence exactly when no match exists. Thus every query returns the
reference's first matching complete receipt or absence. Length and endpoint
observations also agree. For identical surrounding driver observations,
`inspect_applied` therefore produces the same exact outcome, including a
fence rejection, manifest verdict, term replacement or eviction uncertainty.

The existing parameterized AppliedReceipts model proves the corresponding
sequence, checked-order and lookup facts. Its deque-named logical transition
is equivalent to append-and-trim for every positive capacity under the bound;
the new source mapping must explicitly use that equality rather than claim
that Rust still uses a deque. Standard vector/slice operations, integer
comparison, successful allocation and unchanged caller synchronization remain
trust boundaries. This does not prove rustc, the standard library, the whole
Ready/Raft implementation, crash durability or liveness.

## Validation and selection

The two focused regressions compare every retained tuple, vector capacity,
endpoints and queried results with the original vector implementation through
multiple evictions, gaps, u64 extremes, duplicate terms/outcomes and decreasing
indexes. Inherited driver tests cover exact receipts, async completion,
watermarks, fence outcomes and eviction uncertainty.

This file describes an unselected experiment. Fresh source checks, mapped
formal validation and a matched c1/c64 pure/mixed screen are required before
claiming an improvement. A useful candidate still needs broad workspace,
full workload and exact-source actual Chaos Mesh acceptance before promotion.
All CI remains local unless explicitly selected for a release or milestone.
