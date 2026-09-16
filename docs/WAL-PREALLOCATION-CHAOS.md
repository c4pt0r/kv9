# WAL preallocation: actual Chaos Mesh acceptance

Completed locally on 2026-09-16 UTC. The default-off WAL payload preallocation
candidate passes all **21 actual Chaos Mesh windows**, independent complete
history checks, full archive/readback and exact cleanup. Four final replica
drains, all 31 observed server lifetimes and all 25 supporting containers pass.
CRC remains selected. End-to-end write throughput and latency are next; this
checkpoint establishes no database QPS improvement or Redis parity.

| Complete history | Operations | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 4,987 | 4,528 | 448 | 11 |
| Persistent point stream | 1,530 | 1,513 | 12 | 5 |
| Native point/atomic batch | 2,724 | 2,693 | 22 | 9 |
| Total | **9,241** | **8,734** | **482** | **25** |

Every invocation has a recorded return. Unknown outcomes remain in the complete
consistency checks. Independent readers verify successful work during applicable
fault windows, actual fault effects, acknowledged-value recovery and final
reads. Native batches retain their atomic semantics.

The windows cover registration-seed blackholing, failure of each voter, leader
partition, admission overload, network delay, EIO and ENOSPC on every voter,
missing-log and replacement-PVC refusal on every voter, and pending/recovered
endpoint migration. Formation crashes and container restart are also checked.
Storage refusal cases require fail-closed behavior; they do not establish the
still-pending automatic destination installation capability.

## Exact runtime and acceptance

The [matching release/recovery checkpoint](WAL-PREALLOCATION-RUNTIME.md) supplies
the candidate built from clean `86aa6fc`, with all 1,529 source files verified.
Its Rust/Cargo/proof inputs are unchanged from the
[source/proof checkpoint](WAL-PAYLOAD-PREALLOCATION.md) at `fb9d390`.
The server ELF is
`1a38b0a0223650f42aa306131787937f55d7cfc9db2963b2d4bdf8f3d2c8566e`;
the tested image ID is
`sha256:0bf2fbb9da3dc1ffebef5998baf7e71ea3001537692d20a021eaf2dac2373640`.

Only the server's root and engine packages enable `wal-payload-preallocation`.
Production Raft/server packages, the persistent point client and native client
retain empty feature lists. A separate remote admission-pressure example has
an explicitly bound test feature graph and does not instantiate the database.
The same-source point/pressure auxiliary builds, actual compiler invocations,
image payload hashes, no-network loader probe and Kind image identity all pass.
The image contains the retained executables; it does not rebuild the server.

Runtime **47781/38e7ef/0** completes once. Post execution **55667/4cbed9/0**
passes all six phases: independent history/effect audit, cleanup capture,
process-tree readback, full archive/member readback, exact-UID cleanup and
all-lifetime verification. The original audit precedes cleanup and therefore
retains `cleanup_complete=false`; subsequent cleanup records complete that
scope. The owned namespace is absent and all eight historical namespace UIDs
remain unchanged. No failed runtime was repeated.

The observer retains 443 batches and 1,450 identity-matched runtime samples.
These are correctness/identity observations, not performance samples. The
fixture uses the existing single-node local Kind cluster. It establishes
neither cross-host failure-domain independence nor physical power-loss safety.
Quorum, WAL synchronization and response-fence requirements remain enabled.

## Preparation and retained evidence

Ten helper-control groups were freshly qualified because old `/tmp` receipts
were missing and the current feature/path bindings changed. Synthetic timestamp
and stale-format samples are explicitly unit-control inputs. They do not stand
in for actual faults. The current corrected delay-selection predicates and
their rejection controls remain intact. A fresh process/resource check replaces
the obsolete reference to a historical cold capacity transaction.

The original missing-Cargo auxiliary attempt, readback path mismatch and one
incorrect proof-control fixture are retained. The first stopped before Cargo;
the second ran no build. The corrected proof fixture reran only its failed
method. No passing source proof, workspace suite, encoder microbenchmark or
ordinary recovery scenario was repeated. These preparation corrections did not
weaken runtime fault, history, drainage or cleanup requirements.

All new artifacts use `/mnt/data/kv9-work`. Root and output devices retain
separate launch/floor checks and a combined maximum decrease of 12 GiB;
runtime and post phases inherit the same finalizer baseline. The original
20 GiB + 8 MiB launch requirement, 8 GiB floor and 1,200-second phase deadlines
remain in force. These guards measure global available space, not a private
reservation or exact per-run consumption.

The full local archive has **4,836 members**, including **4,486 files /
908,891,813 file bytes**, in **89,254,172 compressed bytes**. Every member was
read back and every original file rehashed before cleanup. SHA-256:
`7ba029e91b670cd2ecd797ea69b696f8b256356cea06abb6f2bbc6f2e7f3c761`.

The [portable packet](wal-preallocation-chaos-v1/README.md) retains all three
complete histories and selected original acceptance records. Large intermediate
snapshots, observer streams, duplicate evidence and executable payloads remain
in the full local archive. The portable selection is not a standalone replay
of every original payload check.

## Next write comparison

The prepared comparison uses the same clean source for default and preallocated
servers and the existing qualified performance client `1b8060e`. Fresh role
readback and feature controls pass. The measured request loops, workloads,
storage guards and fixed client are unchanged: eight smokes, then sixteen
ten-second cohorts covering Put and BatchPut(64), c1/c64 and both orders.
Active storage is volatile tmpfs; keep that panel separate from real-disk
durability and state the Redis reference's acknowledgment settings explicitly.
Final selection requires useful throughput and latency results.

The broad industrial roadmap, separate C04 pre-upload acceptance, clock/lease
qualification and dynamic multi-Raft/split dependencies remain open. CI stays
local; no hosted workflow is dispatched for this experimental checkpoint.
