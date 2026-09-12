# Lease read-view integration: retained local evidence

This checkpoint adds experimental GET/BatchGet lease reads bound to one owned
applied view. [Design and refinement](../LEASE-READ-VIEW.md) define its scope.
Default startup remains Safe ReadIndex; no production clock, lease Chaos Mesh,
performance or industrial checklist acceptance follows from these local tests.

## Accepted commands

- Source supervisor: `/tmp/kv9-lease-read-service-auth-root-first`, session
  `66379`, terminal `07b0e2`, exit 0. Source before/after identities match.
- Source results: **136 Engine + 243 Raft + 207 server = 586 passing tests**,
  zero failed. One existing server workload test remains ignored because it
  needs an isolated output path and independent history verification. Nineteen
  tests are new (2 Engine, 12 Raft/admission, 5 server).
- Formatting, the feature-disabled durable-lease restart probe, default and
  experimental server compilation, and warnings-denied all-target Clippy for
  Engine/Raft/server pass.
- Source fault supervisor: `/tmp/kv9-lease-read-controls-root-first`, session
  `73964`, terminal `87b8b6`, exit 0. All **40 commands** have their declared
  exit code. Nine separately compiled mutations fail their exact named
  assertions; eight distinct targets pass on baseline and restored source.

The five new server tests use real three-voter runtimes and disk logs. Unary
and streaming RPCs actually enter the lease path; tests also cover both epoch
halves, lifecycle deferral, large-batch copy deferral and typed RPC refusal with
expired authority and unavailable follower owners. Their logical test clock
does not qualify a host clock, and stopping test owners is not Chaos Mesh.

| Injected source defect | Required failure |
| --- | --- |
| Skip final lease validation | Read completes before required authority/application. |
| Capture an old commit frontier | An unapplied newer committed write is ignored. |
| Bypass application coverage | The same fresh-frontier/application requirement fails. |
| Accept another installation's ticket | A foreign installation authorizes the view. |
| Remove local admission bound | A valid lease bypasses the shared request limit. |
| Reset fallback deadline | The original start/deadline pair changes. |
| Replace deferred GET view | Retained metadata authorization is lost. |
| Replace deferred BatchGet view | Retained metadata authorization is lost. |
| Replace large batch view after authorization | Deferred values differ from the authorized view. |

These populations overlap the full tests; controls enforce concrete protocol
obligations, not nine separate linearizability histories. The full conditional
algorithm proof and remaining platform/fault gates are linked from the design.

## Original failures and retained payloads

Original source, commands and outcomes are preserved in the archive:

- `lease-read-view-root-first`: absent optional `.cargo` directory during
  source retention, before Cargo started (`f76d29`, exit 1).
- `lease-read-view-retention-root-first`: missing test imports (`4e9222`, exit 1).
  The first repaired Engine/Raft-only validation remains separate
  (`lease-read-view-imports-root-first`, `7e0a42`, exit 0).
- `lease-read-service-root-first`: standard/Tokio deadline type mismatch in
  the new stream client test (`df9ca9`, exit 1).
- `lease-read-service-clock-root-first`: 205 server tests pass, two new RPC
  tests fail authentication, one existing test is ignored (`fefb4b`, exit 1).
  The fixture used the principal as the token. Those failures never exercised
  the lease path and are not counted as safety evidence.
- The first publisher rejected a 6,554,318-byte mutant assertion log under its
  unchanged 2 MiB member cap (`c24ea1`, exit 1). The repaired publication stores
  it losslessly as a 10,025-byte gzip member, verifies decoded equality and
  records both hashes in `publication/large-logs.json`. The complete original
  log remains at its original local path; no result or test was changed.

The accepted Raft test executable remains locally as
`/tmp/kv9-lease-read-service-auth-source-first/raft-tests.gz`, 40,330,396 bytes.
Its decoded SHA-256 is
`5cff75a66035bb90c22e87ba8063465413a333adf71ddd9ac1fbfca2ab3ebda6`.
Decode to a new, unused path and verify that hash before execution; do not
overwrite an existing executable. Full source capsules and compressed binary
payloads stay local; the archive contains source identities and the relevant
source files, logs and compiler records. Control executable records identify
each compiled variant; their payloads are not archived.

## Archive and resource accounting

[Inventory](inventory.json) binds **358 files**, including source, command logs,
original failures, cache ownership records and the publication script.
[The archive](original-evidence.tar.gz) is **1,371,121 bytes**, SHA-256
`66e57dacb45208667024a7731c4715aef08d545e5e19b4e27f92c32372621638`.
Every member was read back byte-for-byte; publisher terminal `f1f1b7`, exit 0.
[Source summary](source-summary.json) and [control summary](controls-summary.json)
retain the unmodified accepted results.

Source and fault runs retain the 96 GiB start preflight, 80 GiB free floor,
16 GiB additional-consumption reservation, five-second sampling, original
1,200-second command caps, helper CPUs, offline dependencies and the shared
build-cache lock. Only reproducible first-party debug artifacts were invalidated.
Cold conversion of nine earlier inactive probes recovered 543,252,480 bytes net;
two further approved probes recovered 120,713,216 bytes. Their exact source,
compressed/decoded hashes, open-reference checks, terminal records and
no-overwrite restoration instructions are archived. Original failed probes
remain failed; no runtime, target or live artifact was cold-converted.

CI was local. Clock qualification, actual lease-enabled Chaos Mesh histories,
independent-host acceptance and matched throughput/latency measurements remain
open. The existing `11113f6` performance baseline is unchanged as evidence.
