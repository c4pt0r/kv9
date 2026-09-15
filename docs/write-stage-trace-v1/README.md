# Write-stage tracing: development validation

This packet publishes the local qualification of default-off bounded write-stage
instrumentation and its independent retained-window reader. It is not a release,
live trace capture, performance improvement, observer-overhead result or new
Chaos Mesh acceptance. CRC remains selected.

## Results

| Check | Actual result |
| --- | --- |
| Default workspace library tests | 731 passed, four existing fixture tests ignored |
| Final tracing + write-path-diagnostics workspace libraries | 744 passed, four ignored |
| All-target warnings-denied Clippy | Default and both-feature builds pass |
| Formatting and whitespace | Pass |
| Independent reader controls | 16 original controls pass; one added oversized-integer API/CLI control passes |
| Existing strict proof compositions | 37 theorems / 187 obligations; 15 positive models, 29 expected counterexamples, 16 proof rejection controls, one ordered witness |

All Rust commands use Rust/Cargo 1.94.0 and the existing NVMe target at
`/tmp/kv9-c04-configuration-development-20260915-first/target`. Test fixtures
keep process-local `TMPDIR=/tmp`. Logs, proof outputs and packet assembly use
`/mnt/data/kv9-work`; no hosted workflow was dispatched.

The new Rust tests exercise real committed-but-unapplied requests, successful
exact-position receipt joins and failed apply without receipt insertion, as
well as ring overwrite, actual gapped/mixed-term sampling, contention, poison,
clock/counter overflow, trace recreation and the 384 KiB JSON bound. Reader
controls are synthetic and separately scoped; they have not accepted a live
server capture. Compatible joins preserve outcomes such as `fence_rejected`
and do not certify a successful client response.

The proof refresh covers anchor binding (7/26), checkpoint base (7/27),
checkpoint publication (9/36), checkpoint owners (7/43) and retention ledger
(7/55). Their existing models/theorems are unchanged. The reviewed mapping only
refreshes the runtime source pin for the feature-gated status export. These
compositions are not a new mechanized proof of the observer; see the structural
safety and boundedness argument in [the design](../WRITE-STAGE-TRACE.md).

## Original attempts and receipts

The first trace-only run passes 483 Raft/server tests, with two ignored; the
first default workspace passes 731, with four ignored. Its following Clippy
command fails with `clippy::let_unit_value` (exit 101). The corrected cfg-specific
service binding and subsequent timestamp/schema refinements remain visible in
separately pinned attempts. Second-run default and combined diagnostics tests
and both Clippy configurations pass. The third run checks the final trace-instance
schema across the workspace and all five proof compositions.

Final sequential execution: session `55635`, launch `ff3bed`, terminal
`e29785`, exit 0. Independent reader controls: `3d4da4/0`; added integer refusal
control: `4f2ef8/0`. Original/final Python helper bytes and diffs are retained.
The original 16 controls were not replayed after that parser-only refinement.
Packaging and complete readback: `4744e3/0`. Source review and command details
are in [validation.json](validation.json).

## Portable contents

[runs.tar.gz](runs.tar.gz) contains 504 regular members / 2,218,828 logical bytes:
all three development log/command/pin sets, complete outputs of the five proof
compositions, reader qualification and controls, relevant final source bytes,
and the packet assembler. Every member was decoded, hashed and compared with
its original input: [readback.json](readback.json). The archive is 417,628 bytes,
SHA-256 `2bc921154ca1a57a72add41076a05a8c9311b6c8b87b7967cb06de0e4d41dddf`.
The [inventory](inventory.json) binds every member and original path.

Cargo caches, executables, downloaded proof tools and raw GitHub issue snapshots
are excluded. Tool paths and digests remain in the original records. This
packet does not reconstruct the complete old workspace or supply offline build
dependencies. The source-pin snapshots also identify an existing uncommitted
C04 Chaos helper draft; that unrelated draft was not exercised by these checks
and is not part of this publication.

Capacity observations are point-in-time readings, not reservations. The `/tmp`
size survey returned permission errors, preserved in the packet; its partial
sum is not a complete inventory or a deletion budget. Existing evidence was
not moved or deleted. New bulk automation output stays on the data volume.

The separate [retained batch-tail review](../write-batch-tail-review-v1/README.md)
explains why these boundaries were selected. The next step is matching release
qualification and an actual loaded-batch capture with loss, coverage, lifetime
and overhead checks. No current throughput claim follows from these tests.
