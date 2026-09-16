# Next resident-index experiment: packed persistent nodes

Status: planned, not implemented or measured. Production still uses the pinned
`rpds::RedBlackTreeMapSync`. The [composed lowering screen](RAW-LOWERING-COMPOSED.md)
does not justify runtime integration of that smaller optimization.

## Hypothesis and changed variable

The current rpds 1.2.1 source stores one entry per binary-tree node, with separate
shared pointers for the node and its entry. Traversal uses copy-on-write on the
path, and replacement constructs a new shared entry. Existing retained CPU
profiles identify insertion work, but do not prove an exclusive allocation or
cache-miss fraction. Do not infer a speedup from this source structure alone.

Test a standalone persistent ordered index with multiple entries per leaf and
multiple children per internal node. Start with a fixed 32-entry/32-child bound,
safe Rust and path copying through owned shared roots. Keep individual entry
sharing so copying a leaf need not copy every value payload. This changes node
layout and traversal, rather than repeating borrowed-upsert, clone removal,
sorting or duplicate-write coalescing. No new production dependency or runtime
feature is selected by this plan.

## Required behavior before timing

- Bytewise ordered lookup, replacement, deletion, bounded forward scans and
  predecessor lookup must agree with an independent `BTreeMap` model.
- Cloning a root must retain an immutable old view. Insert, overwrite, delete,
  split, redistribution, merge and root contraction must preserve that view.
- Validate sorted unique leaf keys, separator/child correspondence, occupancy
  bounds, uniform leaf depth and exact size after every operation in mixed
  histories. Include empty and binary keys/values, shared prefixes, boundary
  keys, absent deletion, repeated overwrites and both ascending/descending input.
- Use structural sharing rather than an O(N) whole-map clone for each snapshot.
  Test snapshot histories with more than one retained generation.

## Bounded comparison and advancement

Compare unchanged rpds with the new index on the retained original group corpus
and explicitly synthetic unique-insert case, with/without snapshots. Keep
jemalloc, toolchain, source identities and opposite-order execution fixed. Cover
point lookup, predecessor and scans as well as writes; write gains must not
silently degrade the existing read contract. Record mean, p99, allocation
requests and live-memory behavior separately, using an uninstrumented executable
for latency. Do not pool with historical runs or sweep fanout after observing
the first result without a new causal hypothesis.

Only a material, repeatable benefit across insertion and overwrite without a
snapshot/read regression warrants engine integration. Preserve failed or losing
results; an attractive isolated batch mean is insufficient.

Before any production selection, provide a strict proof of ordered-map
correspondence and structural invariants for every mutator, with explicit
assumptions about safe shared-pointer ownership. Bind the proved operations to
the exact implementation. Then retain the engine's atomic cross-CF publication,
applied-position rules and durability boundary, run local source/integration
checks, ordinary recovery and actual Chaos Mesh histories, and compare matched
database throughput and latency. The index experiment changes no Raft consensus
or fence rule and cannot authorize relaxing those gates.
