# Published-directory candidate: actual Chaos Mesh acceptance

Completed locally on 2026-09-15 UTC. Experimental runtime
`483b8c3629b033734f5d7a2b8653a1352304a4b5` passes the unchanged 21-window
Chaos Mesh baseline, independent complete-history audit, archive readback
and exact-owned cleanup. CRC main remains selected. This adds correctness
evidence; there is no new throughput or latency result.

| Complete history | Operations | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 5,368 | 4,889 | 471 | 8 |
| Persistent point stream | 1,444 | 1,424 | 15 | 5 |
| Native point/atomic batch | 2,560 | 2,531 | 19 | 10 |
| Total | 9,372 | 8,844 | 505 | 23 |

Every invocation has a recorded return. Unknown outcomes remain unknown.
Four fresh final replica drains pass. All 33 observed server lifetimes and
25 containers exit; the owned namespace is absent and all eight historical
namespace UIDs remain unchanged.

The windows cover registration-seed blackholing, failure of each voter,
partition, public admission overload, delay, EIO and ENOSPC on every voter,
missing-log and replacement-PVC refusal on every voter, and pending/recovered
endpoint migration. Separate readers verify actual fault effects, complete
catalog/point/atomic-batch histories, source/process identity and final drains.
The original Raft quorum, WAL synchronization, apply and response fences remain.

## Exact runtime and execution

All 871 clean source files and the default ThinLTO production build are bound
to the independently checked server SHA256
`43d8bcb2d852d6fcbebb7f528808e53b3eb93588e2637195e9053cd91e8d5450`.
The actual image ID is
`sha256:48f3c8f8e2a39d10cca1d7a68de2a4a4d59f1c98724f25c2dab1ec1ce35de1c6`.
Production server and clients use default features. The separate pressure
example's test dependencies do not change the production server feature set.

Runtime `81428/548855/0` and post checks `71123/2a4eb4/0` are terminal.
All six post phases pass: independent audit, cleanup capture, process-tree
readback, full archive/readback, exact-UID cleanup and all-lifetime readback.
No workload or acceptance sequence was repeated. The original audit predates
cleanup and retains `cleanup_complete=false`; the subsequent cleanup records
complete that scope. Its inherited scope sentence still names `bd42e60`, while
both machine revision fields, source inventory and executable identity bind
`483b8c3`. The original audit is preserved unchanged.

The explicitly separate Chaos v2 policy requires 64 GiB available at launch,
a continuous 48 GiB floor and the original 12 GiB maximum sampled decrease.
Runtime and all post phases share one finalizer baseline. They record 116
resource samples, with a minimum 71,846,604,800 bytes available. Original
payload, fault, history, process and deadline requirements remain. This
operational policy is separate from the benchmark and release/recovery policies.

## Additional rotation coverage

The baseline does not establish positive engine WAL segment-rotation coverage.
The production threshold remains 16 MiB. A read-only review of the predecessor's
19 named metrics captures, covering 17 exporter identities, found only the
initial recovery namespace sync in each capture. Those samples do not cover
every lifetime, and no selected topology generations were retained. Running
the new binary alone does not prove that `create_successor` was exercised.

A separate supplement must cross the default threshold using the existing
batch API, verify checksum-valid selected successor topology, inject an actual
leader container-kill on the same store, verify acknowledged-value recovery,
then verify another rotation after recovery. Full histories, drains and owned
cleanup remain required. The [completed supplement](WRITE-PUBLISHED-DIRECTORY-ROTATION.md)
now independently establishes selected generation 2 topology on all three
voters, actual leader container-kill and same-store value recovery, followed
by selected generation 3 topology on all three voters. Both 56-call histories,
five fresh drains, archive/readback and exact cleanup pass. Earlier namespace
opt-in and no-op watermark fixture failures remain preserved. The
[source-level syscall fault gate](WRITE-PUBLISHED-DIRECTORY.md)
already covers explicit successor-file and parent-sync cuts, but does not
replace this real cluster recovery check.

After positive rotation coverage, run the complete matched write screen with
CRC main and the fixed native v3 client: c1/c64, Put/BatchPut(64), two opposite
orders, eight smokes and sixteen timed cohorts. Report throughput and tail
latency together. The [latest completed results](WRITE-FNV-WRITER-PERFORMANCE.md)
remain unchanged. The [development route](WRITE-PERFORMANCE-NEXT.md) retains
dynamic multi-Raft, routing, membership and automatic splits after the write
phase. Cross-host, physical power-loss and whole-system industrial acceptance
remain open. No original roadmap work-package checkbox closes.

## Retained evidence

The full local archive contains 4,279 files / 900,304,014 decoded bytes in
90,568,968 compressed bytes, SHA256
`06375fdb07f690c41f58b585b6f6ca670c182b4b390988c379a26766621ef49b`.
Every member was read back and every original input rehashed before cleanup.

The [portable reports, complete histories and original audit records](https://github.com/c4pt0r/kv9/blob/6e44598043be90c5f509c16b5ec7700944660b32/docs/published-directory-chaos-v1/README.md)
contain 2,370 members / 421,046,560 decoded bytes in 11 parts totaling
21,956,844 compressed bytes. Packaging `63977/2b5843/0` and independent
verification `6fe983/0` pass. Original executable/WAL payloads and duplicate
audit copies remain local with complete hash references; the portable package
alone cannot replay every full-file audit. The first portable selector omitted
two required recovery terminal files and refused. The narrow root-list repair
and original failure are retained; no runtime or acceptance audit was rerun.
All work ran locally; no hosted CI was dispatched.
