# Applied receipts: validated tail-offset experiment

Candidate `a6ac335ef4567f2e1a2b33da1a723d694cd5b7d9` adds a checked tail-offset
hint to receipt lookup, based on selected CRC runtime `bd42e60`. The candidate
passes source-mapped proof, local source checks, its exact default release,
ordinary three-voter recovery and the complete actual 21-window Chaos Mesh
campaign. It remains experimental:
there is no database throughput or latency result, and CRC main remains selected.
The [FNV writer](WRITE-FNV-WRITER-PERFORMANCE.md) and
[published-directory candidate](WRITE-PUBLISHED-DIRECTORY-PERFORMANCE.md) have
completed their separate matched screens. Neither is combined here.

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

## Default release and ordinary recovery

The detached `a6ac335` source reproduces all 869 qualified source-file hashes
and all 12 proof-bound committed blobs. Its fresh default release passes
independent Cargo/artifact/codegen readback: first-party production features
are empty, optimization is level 3 with ThinLTO and one codegen unit, and
source and protected earlier binaries remain unchanged. The shared build-cache
lock and explicit first-party invalidation remain in force.

The exact server SHA-256 is
`d83b4e2ede7bcd81e4a6c4adbc407d2fbd6790d9fcab5851314ed907a6fb0a8c`;
the native batch client is
`64aa434166ce05deac8fa6e3d6359c8b13d3dbbc432280ca275701feb499429c`.
Release session `18954` terminates at `924704/0`, and independent readback
terminates at `9ade34/0`.

The three-voter ordinary WAL fixture passes both default stream and explicit
unary transport histories with concurrent point/batch operations, actual
leader SIGKILL and original-directory restart. Independent full-history and
process checks accept **359 complete operations: 329 OK / 30 unknown**.
Unknown outcomes remain in the checked histories. Both lost-voter and
restarted-voter intervals retain successful operations of all five API kinds.
Six fresh voter drains pass; all five server and two client lifetimes exit.
Recovery session `57860` terminates at `f5f864/0`; independent audit is
`23d005/0`. This is ordinary local process recovery; actual Chaos Mesh remains
a separate required gate.

[Portable original release/recovery evidence](receipt-tail-recovery-v1/README.md)
retains 160 members / 4,381,993 decoded bytes, including both complete histories,
small fixture WALs, original build/checker records and frozen preparation. The
451,551-byte archive has SHA-256
`41f16ed42dbfdd6283a7eb1f568ba809191b65c2a3c9f5661deaadde8255859c`.
Independent portable readback (`163612/0`) verifies all bytes and recomputes the
history populations. Original release executables remain locally retained.

The prospective release/recovery storage policy uses an 8 GiB host floor plus
the unchanged 16 GiB maximum sampled decrease and 8 MiB launch allowance.
Six focused floor, boundary and original-baseline controls pass (`34b3e0/0`).
The release's lowest observed available space is 26,451,251,200 bytes. These
operational limits do not change quorum, synchronization, workload or history
requirements; each subsequent stage requires a fresh capacity check.

The subsequent [actual Chaos Mesh campaign](WRITE-RECEIPT-TAIL-CHAOS.md) also
passes all 21 windows and six post phases: 9,360 complete operations
(8,852 OK / 484 unknown / 24 refused), four fresh final replica drains, complete
archive readback and all 32 observed server lifetimes exited. The owned
namespace is removed and eight historical namespace identities remain intact.

## Matched performance preparation

The [frozen comparison and original local checks](receipt-tail-performance-preparation-v1/README.md)
now bind the existing CRC server, receipt server and fixed native v3 client:
859, 869 and 581 source files, their original executable hashes, compiler and
default feature records. Eight source-binding controls and eight smoke-reader
controls pass (`b4e952/0`). Independent portable readback (`1097f4/0`) verifies
all 98 reporting/preparation members and 1,174,930 original bytes.

The full eight-smoke/sixteen-timed protocol, latency accounting, final dataset
checks, drains, lifetimes and storage-v3 requirements are unchanged. No new
workload has run; fresh capacity remains necessary before launch. The last
complete comparison retained 52,208,918,528 physical bytes, so the roughly
25 GB currently available cannot support another full campaign and its
operating envelope. Estimated launch capacity is 80–100 GB available, not a
claim that this amount is a guaranteed output bound.

## Next gates

Retain this source, release and Chaos checkpoint. Measure point/batch throughput
and whole-call tails against CRC main with the fixed native v3 client, both
opposite orders and freshly qualified storage capacity.
Small kernel costs or the profile percentage cannot establish an end-to-end
gain. Main's accepted performance numbers remain unchanged. All CI is local.
