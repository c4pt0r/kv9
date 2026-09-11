# Borrowed batch context: local acceptance

Candidate `850f0deeed8d03426d28d65bbcdd94a503f16cf7` removes the temporary
batch-key reference vector and reuses the region already resolved for the first
key. Every remaining key, including duplicate positions, still passes the same
region check. The keyspace, epoch, fence, immutable view and Raft read barrier
remain in force. This candidate is committed and pushed on
`codex/borrowed-batch-context`; it has not been promoted to master.

The [source-mapped proof contract](https://github.com/c4pt0r/kv9/blob/850f0deeed8d03426d28d65bbcdd94a503f16cf7/docs/BORROWED-BATCH-CONTEXT.md)
states the assumptions explicitly. Six TLAPS theorems discharged 16 fresh
obligations with strict checking and no fingerprint reuse. They cover arbitrary
batch lengths, positional coverage, authorization and first-error ordering on
the same logical metadata view. They do not prove the whole Rust/Raft adapter
or identical transient I/O-error traces: removing a redundant lookup can avoid
an I/O failure that the old implementation would have encountered.

## Local gates at the candidate revision

| Gate | Result |
|---|---|
| Default workspace | 705 passed, 0 failed, 23 ignored |
| RPC experiment workspace | 715 passed, 0 failed, 23 ignored |
| Both all-target Clippy configurations | Passed with warnings denied |
| Compiled implementation controls | All 9 controls / 27 baseline, mutant and restoration phases passed |
| Scoped TLAPS proof | 6 theorems / 16 fresh obligations passed; independent SANY parse passed |
| Process kill/restart E2E | 350 calls: 324 OK, 26 unknown; both complete atomic histories valid |
| Actual Chaos Mesh fixture | All 11 windows passed; existing independent audit passed |

The process E2E retains all unknown outcomes and does not replay uncertain
writes. It explicitly configures a 16 MiB public byte limit. The Chaos fixture
uses the actual source defaults, 64 requests and 64 MiB, without Pod overrides.
Ignored workspace tests are not counted as passing. The real process and Chaos
runs provide their separate bounded acceptance evidence.

## Actual Chaos Mesh result

The first run at the exact release image completed with **2,094 logical calls:
1,804 OK, 216 refused and 74 unknown**. Traffic accounts for 2,091 calls;
initialization and final verification account for three. The full atomic history
has a checked witness allowing whole unknown-write effects. The preliminary
zero-unknown-effect search was invalid and remains retained with the valid
subsequent search. Unknown writes remain unknown.

The unchanged windows cover baseline, Service-VIP delay/heal, partial loss/heal,
client partition/heal, exact TCP reset/heal, and quorum loss/heal. Delay, packet
loss and partitions use actual Chaos Mesh. The exact-tuple reset is separately
identified as non-Chaos Linux `SOCK_DESTROY`.

- Actual 250 ms delay produced selected probes of 251,312–251,923 microseconds.
- Actual 30% packet loss produced 376 netem drops and 122 TCP retransmissions.
- The contained client-partition interval had no successful operation of any
  kind, with 13 unknown BatchGet and 12 unknown BatchPut calls.
- The contained quorum-loss interval had no successful operation of any kind,
  with 47 refused BatchGet and 48 refused BatchPut calls. Per-voter DROP counters
  advanced by 44, 48 and 42.
- Every positive/healed window completed successful batch reads and writes.
  The same native client process survived the exact socket reset.
- Two serial fresh empty Serving publications per replica passed after the
  native client exited. The complete fixture counter envelope recorded 635
  inline batch reads and no blocking submissions; this includes setup and
  verification and is not measurement-window attribution.

Run session 90373 and audit session 99515 exited 0. All five owned Pod/container
lifetimes exited; namespace UID `d9cf5019-ec59-4d5e-be2d-aa47e40841c0` is absent.
All eight historical namespace UIDs are unchanged. The retained 50 data members
total 12,631,210 bytes, with matching hashes. All 124 frozen inputs and 580 source
files remained unchanged. No workload/fault rerun occurred.

The fixture uses volatile tmpfs on one shared Kind host. It does not establish
power-loss durability, cross-host availability, sustained performance, or the
full remaining fault/proof matrix. No hosted CI was dispatched.

## Retained evidence

Release build: `/tmp/kv9-borrowed-batch-context-release-first`.
Server SHA-256: `916738e118bf3d9f2c373f4fe81d7523db0db0f9a401acda9bfbc0450c9dd684`.
Build manifest: `a15cae984b2dc18f24cce6733dc99be039d63379e09dcdaf1764403befa4fdec`.
Default and experimental workspace/Clippy logs use the
`/tmp/kv9-borrowed-batch-context-` prefix. Compiled controls, scoped proof and
process E2E are retained in the corresponding `controls-first`, `proof-first`
and `process-e2e-first` directories.

Raw Chaos run: `/tmp/kv9-native-link-acceptance-850f0de-attempt1`.
Preparation, image binding, full result and existing read-only audit:
`/tmp/kv9-borrowed-batch-context-chaos-preparation`.

| Artifact | SHA-256 |
|---|---|
| Compiled-control manifest | `bbd6b3f6cdc076e0297cdbb5a51a7155659b3ffe3c0b5119a5516e2c386bb71f` |
| Scoped proof summary | `ca36c7af568f6b7fabf911986ae1b4462905b138e86665cea1b0aed66721f9bb` |
| Process E2E summary | `21c63b1e6b538869e2aef2dc5123a7b52c5ba34037588d00f07ea1defe595005` |
| Chaos plan | `011716bdb21f5ea5425e3ea890b96bd1c9dda3f16c788f69ef406c6026d3a65b` |
| Chaos raw summary | `dd587fee23dc0bed1469f97844e751140ec62555f5e1eb7209d197d4fa483961` |
| Chaos independent audit | `012afe3fdcbdfd9c2463a623c007b1b5a447f16adbe49f7a65d615a7b415f665` |
| Final retained-input check | `564afe2159e5cf4b73e98e3749819051d1cd0773664d4e4022278c5aca2f9dd2` |
| Full Chaos result | `d8c2ff2060c915b2f406f58bea71d37f9dd529a8d23dcb635577b563dc63926e` |

## Profiling context

The prior async-batch revision was sampled separately before this change.
Allocation/copy/compare occupied 20.0% of selected point-GET leaf samples and
23.2% for BatchGet(1); RPC occupied 17.4% and 15.6%. Inclusive metadata stacks
appeared in 7.0% and 9.2%. Inclusive categories overlap, sampled CPU is not wall
latency, and these observations do not establish the benefit of this change.
The instrumented run is not accepted throughput evidence. Its retained report
is `/tmp/kv9-point-batch1-profile-first/REPORT.md`, SHA-256
`6ffca3b9bb3a923fd765fdacba51c7630481b530d1cfe36c391f12d4bb1b5eb9`.

[Matched release timing, including actual Redis GET](BORROWED-BATCH-GET-PERFORMANCE.md),
ran after all correctness and background audit work was terminal. BatchGet(1)
improved by 12.9–14.6%; point GET showed no improvement. Redis parity and the
broader roadmap gates remain open.
