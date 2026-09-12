# Write reference and CRC qualification checkpoint

Updated: 2026-09-12. This checkpoint completes independent Redis version-4
report accounting and a clean release build. It also records fresh source,
release and ordinary recovery checks for an isolated CRC write-path candidate.
There is no new accepted throughput, latency or actual Chaos Mesh result.

## Independent reference accounting

The main report readers now retain the exact version-3 point/native behavior
from `0be806d9671e2c50701a64aa7889c8859b7648ba` and extend Redis accounting
for version 4. Successful WAIT writes require sufficient replica replies;
short acknowledgments remain unknown writes. Readers check separate data and
confirmation attempts, combined RESP counts, deadlines, encoded sizes,
whole-call latency, scheduled latency and paired point/batch API selection.
The pairing records Redis's confirmation mode without asserting Raft or fsync
equivalence. Version 1–3 report shapes remain unchanged.

All **44 Python tests** pass. The reader accepts all four original debug-client
SET/MSET × WAIT 1/2 reports and rejects **32 altered copies** with missing or
fabricated acknowledgments, attempt-count errors, omitted latency, inflated QPS
or an invalid timing-build claim. Those are validation controls over retained
reports, not new workloads. The preliminary 36 tests overlap the final 44.

The clean default-feature Redis reference release comes from
`fc07bd64f02e4079e248388062d9ffc7148c295c`. Its executable SHA-256 is
`504c55e2ef0ddf350753e380171f1e418ea5070dbd25752f42dc75fec913b9de`.
Independent readback verifies the binary, release manifest, Cargo feature graph
and all 802 retained source hashes. This build has not yet completed the matched
release-client smoke and timing campaign. The previous real Redis correctness
checks used the same Rust source in a debug build; their
[original evidence](../redis-replication-reference-v1/README.md) remains separate.

## CRC candidate on selected ThinLTO

Experimental commit
[`e748620a7b0ba326ac5f4fa8e3a6b1ff48553c6a`](https://github.com/c4pt0r/kv9/commit/e748620a7b0ba326ac5f4fa8e3a6b1ff48553c6a)
reapplies the existing slicing-by-eight Engine CRC implementation to selected
ThinLTO `11113f6`. It changes no Raft checksum, WAL format, polynomial,
synchronization, commit/apply fence or acknowledgment rule. Main's runtime
selection is unchanged. The branch's
[source and proof report](https://github.com/c4pt0r/kv9/blob/e748620a7b0ba326ac5f4fa8e3a6b1ff48553c6a/docs/WRITE-CRC-SLICING8.md)
retains the exact restoration and proof evidence.

Fresh Lean checks cover **47 distinct theorem statements**, checked initially
and after source restoration, with eight rejected negative controls. The
compiled 256 fallback and 2,048 slicing-table entries are bound to the source.
This proves the checksum arithmetic under the documented Rust primitive,
chunking and compiler premises; it is not a proof of the entire WAL or Raft
implementation. Historical checksum microbenchmarks are not a database speedup.

The exact clean candidate passes **710 workspace tests and doctests**, with
**23 existing ignored tests**, and workspace/all-target Clippy. Its earlier
135 Engine tests overlap that population. Source snapshots are unchanged
across the checks. Release logs record actual `opt-level=3`, ThinLTO and one
codegen unit for the default server and native batch client. The server hash is
`616eed1b823dfaa192a22b2b03a3e7d778bf71efdad7d2b875a49c4bfc94e476`;
the native batch client hash is
`bd18336b5da137adbe008bc69db15dbf7769b73a74a876b9461c493665086d83`.

Ordinary three-voter process tests combine atomic BatchGet/BatchPut with point
GET/PUT/DELETE, terminate the leader and restart from its original WAL directory.
Default streaming and explicit unary each pass independent complete-history
checks, including progress during voter loss and after restart:

| Transport | Complete operations | Successful outcomes | Unknown outcomes |
| --- | ---: | ---: | ---: |
| Streaming | 179 | 167 | 12 |
| Unary | 184 | 170 | 14 |
| Total | 363 | 337 | 26 |

Six fresh voter drains and all seven owned process lifetimes are verified.
Unknown outcomes remain in both histories. These local process tests establish
neither power-loss durability nor actual Chaos Mesh acceptance.

## Remaining work and retained evidence

The [write plan](../WRITE-PERFORMANCE-NEXT.md) remains the execution contract:
12 fresh smokes, then 24 matched selected-KV9/Redis-WAIT-1/Redis-WAIT-2 timing
cohorts, followed by the isolated CRC comparison. The candidate still needs its
actual Chaos Mesh matrix and useful throughput/latency results before promotion.
The environment preparation must retain the original CPU isolation, dataset,
process-lifetime, storage and disk guards. No timing overlaps build or compression.
No industrial work package is closed and no GitHub CI was dispatched.

[Validation summary](validation-summary.json) contains source/build hashes,
counts and terminal receipts. [Inventory](inventory.json) binds every member of
[the original evidence archive](original-evidence.tar.gz): 162 members,
4,500,002 uncompressed bytes, 464,024 compressed bytes, SHA-256
`58d9e706ce5e5240c1da4c950a0db78f117b139bedebf6917587b52325f4b1c2`.
It includes original tests, build diagnostics/manifests, recovery histories,
independent audits and source snapshots. Three compiled executables stay local
with their hashes and original paths in the inventory. The archive was read
back member by member; it does not replace the separately published proof
evidence on the candidate branch.
