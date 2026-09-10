# Native atomic batch Chaos Mesh checkpoint

The original 21 voter/storage/endpoint fault windows now have accepted native
atomic batch histories on clean source
`5cc98617b7e299e9f0fc3f46c65ef80ed4182c8a`. This is an independently checked
retained-evidence acceptance. The original fixture exited **1** because its
final added observer parser rejected GNU `date`'s fractional-comma timestamp.
The original failure remains preserved. A corrected, separately bound read-only
adapter accepted the same artifacts; no workload, fault or runtime was rerun.

The separate [dedicated client-link, reset and quorum-loss fixture](NATIVE-BATCH-LINK-ACCEPTANCE.md)
has since passed its eleven windows and complete histories on the same runtime.
This checkpoint does not promote the candidate to main or establish batch
throughput, latency, Redis parity or production readiness.

## Runtime and workload

The standalone server and both persistent clients were separate default-feature
debug builds. Their production Cargo feature arrays were empty. Executing
images, binaries and process lifetimes were checked against the frozen build
and readiness plan. Production Rust inputs remain those of the normal-port
streaming adapter at `e52e72b4d1020eb078a818f71854d8c179f3dd27`;
`e246eae6ff48ef1f27dd8f0527b20ae48dc26280` added the native history workload and
`5cc9861` recorded its process acceptance. Later proof and documentation commits
do not relabel the executing source revision.

The native and point clients used `tonic_stream` through ordinary Services on
port 20160. Native traffic used a separate Raw keyspace, four workers, four
shared keys, eight ordered items per batch, 128-byte values and a
GET/PUT/DELETE/BatchGet/BatchPut mix of 10/10/10/35/35. Duplicate-key batches
overlapped point operations. A batch remains one atomic history event, with
ordered item positions and whole-operation uncertain outcomes.

| Complete history | Calls and completions | Success | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 5,815 | 5,225 | 574 | 16 |
| Persistent point | 1,542 | 1,521 | 15 | 6 |
| Native batches and shared-key point calls | 2,716 | 2,669 | 32 | 15 |

The native history contains 953 BatchGet calls with 7,618 ordered input items,
950 BatchPut calls with 7,597 input items, and 271 each of point GET, PUT and
DELETE. These totals include initialization and final verification. Traffic
contains 1,900 eight-item batch calls. Item counts are not independent RPCs or
independent atomic effects. All unsuccessful operations remain in the checker
input and outcome accounting.

## Actual fault windows and checks

The additive native lane retained the original point/CLI predicates and effects:

- Registration-seed blackhole and three voter Pod failures.
- Inter-voter partition, public admission pressure and follower delay.
- Six actual Raft-log I/O write failures: errno 5 and 28 on each voter.
- Three missing-log restart refusals and three separately prepared
  replacement-PVC refusals.
- Pending and recovered endpoint migration.

Each of the 21 windows contains a complete successful native BatchGet and
BatchPut. Their invocations follow a serial live-prefix barrier, and both
invocation and completion fall inside the checked fault or physical envelope.
Complete histories, wall-clock anchors, actual effects and original
refusal/endpoint brackets were independently checked. The native baseline is
retained separately from the fault windows.

The continuous observer retained 1,404 exact-runtime samples across 31 server
lifetimes. The native collector kept the same Pod/container/boot/PID/start
identity throughout. The point collector's executing binary, process identity
and ordinary-port connections were also captured independently. After both
clients exited, each of the four final registered replicas published two serial
fresh empty Serving statuses within the fixed 20-second drain gate.

Controls comprise 43 preserved original observer cases, 18 native
window/identity cases and 24 configuration/drain cases. Earlier preflight and
missing-helper-import failures remain indexed with the original parser failure.
The source, image and readiness plan stayed frozen throughout the run.

## Readback correction and cleanup

The original final observer rejected
`2026-09-10T13:34:44,807368542-07:00`. Its adapter accepted only dot-separated
fractions. The separate corrected parser accepts comma or dot, one through
nine fractional digits and timezone offsets without discarding nanoseconds.
Seven valid and seven invalid timestamp controls passed. All 19 independent
identity/history/effect/observer commands then accepted the unchanged run.

The acceptance records explicitly retain `original_harness_complete=false`,
`original_harness_exit_code=1` and `evidence_accepted=true`. The original log and
failed `native-window-audit.json` were not replaced with corrected outputs.
The executed overlay is retained outside the repository; canonical fixture
integration is separate work and does not constitute another executed matrix.

Every archive member was read back before scoped cleanup. Namespace
`kv9-chaos-1789072205-3596016`, UID
`9a140381-3035-48da-8b9d-7fbe14a28a91`, was removed. All eight final node process
identities are absent, and all 25 containers supporting the 31 observed server
lifetimes exited or were removed. Both collectors and host helpers exited.
All eight historical namespace UIDs were preserved. A separate supplement
contains post-archive cleanup observations and acceptance cross-references.

## Retained evidence

Raw fixture: `/tmp/kv9-chaos-e2e.uIE2VV`. Frozen overlay:
`/tmp/kv9-native-batch-chaos-overlay`. These are local retained artifacts, not
downloadable release assets.

| Artifact | SHA-256 |
| --- | --- |
| `/tmp/kv9-native-batch-chaos-preparation/ready-plan.json` | `839956ca1e45489503d6d734d3b256065e3de11ae15afd9a007e9ef655b10aec` |
| `/tmp/kv9-native-batch-chaos-independent-audit/audit.json` | `deab07949b6f1e814245d5378c5c6dfdca130b87995412fd57ce28f20ae62077` |
| `/tmp/kv9-native-batch-chaos-independent-copy/native-window-audit.json` | `ddaacac48c1b86002c36cf025792a3d353bf24758394f3aa7ca88927b17ad9f9` |
| `/tmp/kv9-native-batch-chaos-cleanup/summary.json` | `65c1e589f73f8928da03b2480b8c330f959ba5549ef1431c3cde6c15a45e587b` |
| `/tmp/kv9-native-batch-chaos-closeout/closeout.json` | `2426a722205896281b4059f0b49c79b6c9c5f01f723a6120fe7d0fdf9f59cf2c` |

Primary archive: `/tmp/kv9-native-batch-chaos-5cc9861-20260910.tar.gz`,
621,651,600 bytes and 5,602 members, SHA-256
`31a5d5bf0930f5a6f7c52d02496618f59fe667aa1dc58e0394d3be89895743f3`.
Cleanup supplement: `/tmp/kv9-native-batch-chaos-cleanup-supplement.tar.gz`,
68,306 bytes and 85 members, SHA-256
`7f0fee87ac5f0082817b009bbd79e5d09747e0d626dde4f7c3f1678b37f8d81e`.

| Executing artifact | SHA-256 |
| --- | --- |
| Default server | `64ec25c6454a3a8bb080a9b65cbea22945cd8e5a25e45ea65661f8b330123745` |
| Native client | `9bf536888a70e915c17c3b19016640b7c176b80bb429624d6625feb0cbb36e7d` |
| Point client | `6086f1a586fc71586403f2a3333001a7f8a2fd87bd06d0f1f4aaee057a0733e1` |
| Image config | `eb9781ba71e8fb642a565a7f2e52d40e548b8e8d53a757981b056bb8aaa12002` |
| Loaded CRI manifest | `d08204d69f58342a123ef7cfc7291987454c47d34a53d280b1266d63aea23d9d` |

## Remaining scope

This was a local shared-host kind correctness run. Observed Pod CPU masks were
`0-31`; host helpers used `6-31`. It supplies no timing, independent-host,
power-loss or new object-store evidence. No hosted workflow was dispatched.

The [native TLA+/TLAPS proof](NATIVE-BATCH-PROOF.md) separately establishes its
parameterized safety properties under explicit Raft, epoch-fencing, atomic
engine, read-authority and receipt premises. Neither this history acceptance
nor that proof establishes whole-Rust adapter refinement.

The [client-link preflight](NATIVE-BATCH-LINK-PREFLIGHT.md) identifies effective
Service VIP selectors and a same-process socket reset. The subsequent
[dedicated acceptance](NATIVE-BATCH-LINK-ACCEPTANCE.md) covers client-link
delay/partial-loss/partition/reset and actual quorum-loss/recovery. Inter-voter
partial loss, storage stalls and full adapter refinement remain open.
Batch performance also needs paired Redis MGET/MSET runs,
explicit RPC and item counts, offered load, all outcomes, and whole-batch
mean/p50/p95/p99 latency. Issue #50 remains open, and the original issue #9
roadmap checklist is unchanged.
