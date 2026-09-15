# Replicated retention ledger evidence v1

Actual local validation on 2026-09-15, based on `f7929c2` plus the exact sources
retained in this packet. [Implementation, protocol and remaining work](../RETENTION-LEDGER.md).
The ledger records explicitly registered resources and ownership transitions in
the existing metadata Raft group. It supplies no complete-reference or physical
deletion capability and does not close C04/#14 or S07/#21.

## Packets

| Packet | Members | Decoded bytes | Gzip bytes | SHA-256 |
| --- | ---: | ---: | ---: | --- |
| [Development](development.tar.gz), [inventory](development-inventory.json) | 1,101 | 7,568,174 | 1,431,448 | `0d369ff3ce97abec1e14f8154fcfdf2ddcb15559e52c2f9faf24747e8cc029ac` |
| [Runtime/environment](runtime/evidence.tar.gz), [selection](runtime/selection.json) | 2,902 | 130,129,773 | 1,580,549 | `cc10ce2d639f977c34ec2bc3ddbedbba5e7c0b858b1d417c3844a859d2c01a62` |

[Development readback](development-verification.json) matches every member to
its original bytes and checks all 165 frozen source pins. Creation and readback
terminal: **8fbc3f / 0**. The packet includes original failed attempts, complete
proof/model outputs and exact compiled-fault source copies. Executables remain
local with hashes. Generated Java classes and proof fingerprint/state caches are
explicitly excluded. The selector and per-file omissions are inside the packet.

[Runtime, environment and complete retained protocol evidence](runtime/README.md)
are packaged separately. They include the actual Chaos workload and independent
audits, the six-test environment discriminator, tool restoration and storage
guard evidence. No credentials, kubeconfig, CA payload or container image is
published. Packet readback is evidence verification, not workload replay.
The runtime packet includes a 761,259-byte selection manifest in addition to
129,368,514 original payload bytes. Packaging/readback terminals are
**4fd8bc / 0**, **9e0278 / 0** and final copy **545d8a / 0**. A scan of both
packets against seven known actual fixture credentials finds zero matches
(**5e7f5d / 0**); it does not claim detection of arbitrary unknown secrets.
The final [source and packet audit](source-and-packet-verification.json) independently
compares both archives with their originals, checks companion file hashes and
decodes all three nested complete Raft logs (**75e68a / 0**). All 165 build inputs
and five current proof source inventories still match their accepted inputs.

## Local development gates

| Gate | Actual terminal / outcome | Packet path |
| --- | --- | --- |
| Workspace/all-target check, focused metadata/runtime tests | **8510 / a6f93c / 0**; nine metadata tests and four runtime/gRPC tests pass | `runs/kv9-c04-ledger-runtime-development-20260915-second` |
| Ambiguous planner drainage, leadership term fence, Clippy | **14494 / 0f1c5f / 0**; two new planner tests and workspace/all-target warnings-denied Clippy pass | `runs/ledger-runtime-development-20260915-fifth` |
| Workspace library attempt | **79524 / f8b1ad / 101**; 703 pass, six old runtime tests fail, four ignored; transaction library not reached | `runs/ledger-development-acceptance-20260915-first` |
| Same six tests, frozen test binary, original NVMe fixture placement | **96013 / 6d85ae / 0**; all six pass with the original four threads and unchanged assertions/timeouts | Runtime environment packet |
| Remaining transaction library | **a82b7a / 0**; 17 pass | `runs/ledger-remaining-libraries-20260915-first` |
| Default runtime build | **48228 / f8dbec / 0**; 165 current source pins and frozen executable | `runs/ledger-runtime-build-20260915-first`, `runs/ledger-runtime-inputs-20260915-first` |
| Ledger strict protocol | **69477 / 7cbda1 / 0**; 7 statements / 55 obligations, three models, six actual counterexamples, three proof rejections | `runs/ledger-protocol-20260915-second` |
| Current-source composition | **16529 / fd747a / 0**; retention 10/59, anchor 7/26, base 7/27, publication 9/36, with every existing model and negative control | `runs/ledger-composition-*-20260915-first` |
| Compiled source controls | **92251 / 1ae1c2 / 0**; nine baseline tests, four isolated faults, each exact-filter baseline green and fault red at the named assertion | `runs/ledger-source-controls-20260915-first` |
| Formatting and diff check | **a6a0f3 / 0** | Publication metadata |

Across the original library attempt, its exact six-test discriminator and the
remaining transaction library, **726 tests pass; four remain ignored**. This is
not a single clean full-suite execution. The failed attempt used data-volume
TMPDIR during overlapping bulk tool installation; recorded successful Raft WAL
sync reaches 2.542 seconds. The same frozen executable passes all six selected
tests on the original NVMe filesystem after installation completes. That
comparison does not isolate disk placement from overlapping I/O. Neither
timeouts nor assertions changed, and the original failures remain visible.
The runtime packet's 207-pass / six-failure count describes the server library
within that original attempt; the table above gives the whole-attempt count.

The default-feature server is **46,897,048 bytes**, SHA-256
`2b4d70aac139672a5627e5a0f789245ea039b647f2510aec6f6b938551b1d36e`.
The six-test discriminator uses the original test ELF, SHA-256
`3f0fef3afcb0fde0ee15a45e849825d13e78d1b2bcb0487328677e941ab7153e`.
Neither is a release/performance candidate qualification.

## Actual leader failure

The new binary completes **96465 / d5c80a / 0** in 120.04 seconds on the existing
local Kind/Chaos Mesh cluster. Independent Raw audit **268089 / 0** verifies
296 successful first-attempt serial operations: one keyspace creation, 285 puts,
two deletes and eight reads. Actual container kill produces exit 137, a new
container ID and recovery on the same Pod, PVC and store. Fresh writes apply on
all three voters, and scoped cleanup preserves protected namespaces and faults.

Independent ledger audit **6d7cff / 0** verifies 16 successful first-attempt
calls: eight changed transactions, two new confirmations and six exact canonical
owner reads. Leader 3 is killed with a Published source and Held destination.
Leader 1 returns identical overlapping owner observations after recovery;
repeating the share confirms revision 4 at a fresh position. The destination
then becomes Published, and the source is quiesced and released at term 3/index
558, ledger revision 7. All three replicas reach the final applied receipt.
The audit also checks 526 completed command lifetimes and 1,530 samples spanning
two distinct filesystems. Conservative maximum decrease is 587,988,992 bytes,
below the unchanged 1 GiB budget; both original floors remain satisfied.

The registered subject is the earlier checkpoint at index 526; its 143,108-byte
SST descriptor is retained. The separately captured recovery checkpoint is later,
at index 543, with a different 147,730-byte SST whose actual body is retained and
hashed. The earlier registered object's body was not separately retained before
owned MinIO cleanup. These objects must not be conflated. The test verifies
ownership records and recovery of the later checkpoint, not an offline byte
audit of that earlier object or a garbage-collection capability.

## Preserved failures and scope

The original compile attempt fails because three fake gRPC implementations lack
the two new methods; the corrected all-target check passes. Two planner-test
attempts use an unsuitable helper which reacquires the already-held catalog
lock. The first requires a recorded exact-process stop. The corrected harness
observes the driver directly and exercises the original production term/drain
fences without weakening assertions or deadlines.

The first two proof drafts leave actual obligations unproved; the final proof
decomposes acquisition preservation into its type, crosslink, shape, coverage
and stale-token obligations. The first complete protocol attempt reaches an
incomplete successor state in a malformed stale-token negative control. Explicit
parentheses fix that mutation expression; the final run obtains a real invariant
counterexample. An earlier inventory invocation cannot compile its Java auditor
after the old `/tmp` tool disappears and emits no audit log. Restored pinned tools
complete the strict run. None of these failures is relabeled as an accepted
negative control.

The committed-state proof assumes immutable validated identities, ordered Raft
application, atomic data/position persistence and the existing fresh metadata
planner. It is not a mechanized Rust refinement or proof of upstream Raft.
The actual Chaos cell covers a single leader process failure with a tmpfs object
store; it does not establish physical power-loss safety, concurrent-history
linearizability, the full21 matrix or multi-host failure tolerance.

Next integrate checkpoint and pending-attempt owners, avoid checkpoint feedback
from ledger bookkeeping, and backfill/fence existing references. Reader drainage,
transition/history authority, destination admission, atomic installation and safe
physical retirement remain open. New bulk artifacts now use `/mnt/data/kv9-work`;
latency-sensitive fixtures retain their selected NVMe placement. See
[capacity and missing performance inputs](../LOCAL-ARTIFACTS.md). No new QPS,
candidate promotion, original checkbox closure or hosted CI is claimed.
