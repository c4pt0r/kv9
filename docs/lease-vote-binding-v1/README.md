# Lease vote binding validation

See the [design and source mapping](../LEASE-VOTE-BINDING.md),
[source results](source-summary.json), [fault-control results](controls-summary.json)
and [member inventory](inventory.json).

The [original evidence archive](original-evidence.tar.gz) contains 216 files,
647,415 compressed bytes, SHA-256
`3e39d69ae057670c57809cfac4cc4d8954219be4e52a52a6bc5a76aa79b7ab91`.
Every member was independently read back against its original bytes. Aggregate
published evidence archives occupy 66,574,010 bytes, below the unchanged
67,108,864-byte allowance.

## Accepted local checks

- Source supervisor: `/tmp/kv9-lease-vote-binding-root-first`, runtime session
  `66556`, terminal receipt `eaa91b`, exit 0. The source directory is
  `/tmp/kv9-lease-vote-binding-source-first`. All 219 Raft library tests pass,
  zero failed or ignored: 14 new tests include 10 actual peer-binding tests.
  Default test compilation, formatting, experimental-feature compilation and
  warnings-denied experimental all-target Clippy pass. The before/after source
  inventories match; tested Rust/Cargo bytes match the published implementation.
- The three-voter test isolates the old leader, demonstrates that its granting
  quorum prevents a forced replacement election during the promise, then
  permits replacement, majority write commitment and fresh Safe ReadIndex at
  expiry. This is an in-process Raft history, not a new service lease-read E2E.
- The durable-epoch matrix covers 180 I/O/crash combinations, including errors
  before/after writes and sync, short writes, EIO/ENOSPC, loss/retention of
  unsynced bytes and 16 seeded crash schedules. Failed installation never
  publishes an epoch; replay preserves policy and advances the recovered epoch.
- Repaired control supervisor:
  `/tmp/kv9-lease-vote-binding-controls-repaired-root-first`, runtime session
  `5547`, terminal receipt `210d2e`, exit 0. Results and source capsule are in
  `/tmp/kv9-lease-vote-binding-controls-repaired-first`. A fresh normal library
  build passes the feature-disabled restart test. Eight freshly compiled
  faulty-source variants each fail the intended test; the six distinct target
  tests pass on baseline and restored source. Original/restored/caller source
  hashes agree. These tests overlap the 219-test population.

Both source supervisors retain the original 80 GiB free floor and 16 GiB
additional reservation, helper CPUs 6–15/22–31, offline Cargo and four build
jobs. Minimum observed free bytes are 102,368,882,688 and 102,350,643,200,
respectively. There is no hosted CI, benchmark or lease-enabled server run.

## Preserved first failure and storage maintenance

The initial control capsule omitted protobuf inputs and did not explicitly
select the shared target. Its feature-disabled compile failed before any
semantic control ran: session `14658`, receipt `3b2259`, exit 1. The exact
original runner is hash-bound to its invocation and retained, with compiler
output and source inventory. The repair adds `proto/` and explicit shared-target,
offline and build-job settings; no test assertion, time or resource bound changes.

Locked first-party debug cache invalidations are retained at
`/tmp/kv9-lease-vote-binding-cache-first` and `cache-second` (receipts `2ad6ad`
and `8c83c3`, both exit 0). The failed capsule's exclusively owned compiler
cache was inventoried and removed after owner exit using `cargo clean`; receipt
`8d8611`, exit 0. Its source, logs and failed result remain intact. These are
reproducible cache cleanups, not deletion of a failed test history.

Inactive TLC state directories contained no reclaimable payload. Separately,
12 inactive retained test executables were compressed and independently decoded,
rehash-checked and checked for live references before removing their original
copies. Exact no-overwrite restore instructions are retained at:

- `/tmp/kv9-lease-vote-binding-cold-states-first/BINARY-COLD-RESTORE.md`
  (one earlier controller library test binary, session `51911`, receipt
  `2ff10a`, exit 0).
- `/tmp/kv9-lease-vote-binding-more-cold-first/README.md`
  (this stage's library test binary plus ten earlier optimized controller
  executables, session `36872`, receipt `3c472a`, exit 0).

The current 219-test executable has SHA-256
`1c1bdf360062256b2300a0f4d92027989280ba5c6d7559cb592fc92b2b40092c`.
Its original-path rerun requires restoration first. Cold executable payloads,
reproducible full workspace copies and compiler caches are excluded from the
published archive; source inventories, executable hashes, original failures,
fault outcomes, maintenance inventories and restore instructions are included.

This stage binds voter installation and election gates. Leader-controller
lifetime, grant envelopes, whole-pump certificate publication, exact read views,
qualified clocks and actual Chaos Mesh lease histories remain open. It closes
no original industrial-readiness checkbox in issue #9.
