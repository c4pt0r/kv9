# Applied receipts: validated tail-offset experiment

Candidate `a6ac335ef4567f2e1a2b33da1a723d694cd5b7d9` adds a checked tail-offset
hint to receipt lookup, based on selected CRC runtime `bd42e60`. The candidate
passes source-mapped proof and local source checks. It remains experimental:
there is no database throughput or latency result, and CRC main remains selected.
The unmeasured FNV writer is a separate candidate and is not combined here.

## Why this experiment

The [current exact-main profile](WRITE-CRC-MAIN-CPU-PROFILE.md) attributes
166/3,259 point samples (5.094%) to the applied-receipt linear scan. Earlier
deque and binary-search candidates produced mixed results, so this is a
different, isolated hypothesis: exploit consecutive log indexes near the tail
without assuming all receipts are consecutive or changing retention.

Subtract the requested index from the tail index, check conversion and bounds,
then inspect the calculated slot. Return its complete index/term/verdict only
after the stored index matches. A matching hint takes constant work. Gaps can
make the hint miss; checked strictly increasing sequences then use binary
search. Duplicate or decreasing appends permanently select the original
oldest-first lookup. Queries above the tail, arithmetic limits, missing keys
and eviction do not manufacture a receipt.

The container still performs the original Vec append and oldest-prefix drain,
with capacity 1,024. The source contract reconstructs the complete selected
parent driver byte-for-byte by reversing only the adapter/import/constructor/
lookup edits. Exact term and exclusive outcome checks, fatal behavior, lock
order, watermarks, eviction uncertainty, notifications, admission, quorum,
synchronization and client-success rules remain in the unchanged driver text.

## Proof and local checks

The [parameterized TLA+/TLAPS model and source mapping](https://github.com/c4pt0r/kv9/blob/a6ac335ef4567f2e1a2b33da1a723d694cd5b7d9/proofs/tlaps/receipt-tail-hint/README.md)
prove retention/certificate invariants and lookup equivalence for arbitrary
finite receipt traces and positive capacity. A separate density theorem
establishes the direct slot calculation for contiguous suffixes; density is
not needed for safety. All **18 statements / 158 fresh obligations** pass:
13 inherited statements and five new ones. Both proof modules are freshly
checked; imports, assumptions and proof holes are semantically audited.

Finite capacity-one/two exploration passes. Removing the index validation or
bypassing the checked certificate produces the required counterexample. A
restored model passes. Omitted proof and unapproved false-axiom controls are
rejected, along with six empty/zero/missing-output controls. This is conditional
representation refinement, with explicit Rust/library/compiler/proof-backend
premises; it is not whole-Raft, crash or liveness verification.

The clean committed source passes all seven local gate commands: **793
workspace tests/doctests, with 23 existing ignored**, formatting, warnings-denied
Clippy and explicit experimental-lease compilation. The **49 focused driver
tests** are included in that workspace total. Four receipt regressions compare
the complete returned tuple and original vector behavior across dense runs,
gaps, multiple evictions, duplicate/decreasing indexes and u64 extremes.
Existing async/fence/replacement/fatal/eviction cases also pass.

Proof session `99436` ends at `5ef6ec/0`; source session `56653` ends at
`93bf26/0`. The proof ran before the source commit; subsequent independent Git
blob comparison verifies that all 12 proof-bound files match `a6ac335` exactly.
The source gate starts clean and binds all 869 files, unchanged after every
phase. Existing shared-cache locking and explicit first-party recompilation
remain. Available space stays within the 8 GiB consumption allowance above
the 96 GiB floor; no benchmark overlaps qualification.

[Portable original proof and source-check evidence](https://github.com/c4pt0r/kv9/blob/09c8f4d4f324e2cca84cd6fe7b778c0adfd9d4fc/docs/receipt-tail-hint-source-v1/README.md)
preserves all 137 reporting files / 2,042,035 bytes, with independent size/SHA
readback. Original logs and diff whitespace remain unedited. Runtime source
and all local records remain separately available.

## Next gates

Retain this source checkpoint and finish FNV's matched screen when its storage
prerequisite is resolved. Before selecting this receipt candidate, build and
bind its exact default release, run ordinary recovery and actual Chaos Mesh,
then measure point/batch throughput and whole-call tails against CRC main.
Small kernel costs or the profile percentage cannot establish an end-to-end
gain. Main's accepted performance numbers remain unchanged. All CI is local.
