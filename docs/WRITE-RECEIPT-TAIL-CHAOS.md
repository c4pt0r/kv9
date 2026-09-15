# Receipt tail hint: actual Chaos Mesh acceptance

Completed locally on 2026-09-15 UTC. Experimental runtime
`a6ac335ef4567f2e1a2b33da1a723d694cd5b7d9` passes the complete 21-window
Chaos Mesh baseline, independent full-history audit, complete local archive
readback and exact-owned cleanup. CRC main remains selected. This adds
correctness evidence; throughput and latency were unmeasured at this checkpoint.
The later [complete matched performance screen](WRITE-RECEIPT-TAIL-PERFORMANCE.md)
records point-write gains and batch tradeoffs; CRC remains selected.

| Complete history | Operations | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 5,331 | 4,881 | 441 | 9 |
| Persistent point stream | 1,443 | 1,426 | 16 | 1 |
| Native point/atomic batch | 2,586 | 2,545 | 27 | 14 |
| Total | 9,360 | 8,852 | 484 | 24 |

Every invocation has a recorded return. Unknown outcomes remain unknown and
are included in the complete consistency checks. Four final replica lifetimes
each publish two fresh drained statuses. All 32 observed server lifetimes and
25 containers exit; the owned namespace is absent and all eight historical
namespace UIDs remain unchanged.

The windows cover registration-seed blackholing, failure of each voter,
partition, public admission overload, delay, EIO and ENOSPC on every voter,
missing-log and replacement-PVC refusal on every voter, and pending/recovered
endpoint migration. Earlier formation and container-restart stages also pass.
Separate readers verify actual fault effects, complete catalog/point/atomic
batch histories, source/process identity and final drains. Raft quorum, WAL
synchronization, apply, publication and client-response fences remain intact.

## Exact source, build and execution

All 869 clean source files bind to the already independently checked default
ThinLTO release. The server SHA-256 is
`d83b4e2ede7bcd81e4a6c4adbc407d2fbd6790d9fcab5851314ed907a6fb0a8c`.
The actual image ID is
`sha256:9d1a2cbf12dfbdd111dc4ec94a74789545ac0d074f2a93e6a4f5a492f0b8a5a8`.
Default server, point client and native batch client features are independently
checked. The separate remote pressure example includes testing dependencies;
it does not instantiate the database or alter production-server features.

Actual auxiliary builds (`79902/cda607/0`), independent codegen readback
(`4754c2/0`), image build/probe/Kind loading (`5621/7bc221/0`), fresh
finalization (`77c219/0`) and independent prebuilt verification (`b95452/0`)
precede the one-use runtime release. Five finalizer metadata/codegen controls
pass (`a66b91/0`). The image probe verifies actual payload hashes and loader
compatibility without network access; its temporary container is absent.

Runtime `9509/1578ac/0` and post checks `73715/d1e0ca/0` are terminal.
All six post phases pass: independent audit, cleanup capture, process-tree
readback, full archive/readback, exact-UID cleanup and all-lifetime readback.
No workload or acceptance sequence was repeated. The original audit precedes
cleanup and retains `cleanup_complete=false`; subsequent cleanup records
complete that scope. Original records remain unchanged.

The explicit Chaos v3 policy requires 20 GiB plus 8 MiB available at launch,
an 8 GiB continuous floor and the unchanged 12 GiB maximum sampled decrease.
Runtime and every post phase use the same finalizer baseline. All 117 resource
samples meet it: minimum available space is 25,306,357,760 bytes and maximum
observed decrease is 955,617,280 bytes. Original payload, fault, history,
process and deadline requirements remain. Resource guards reserve no disk and
do not authorize another performance campaign.

## Evidence and next development step

The complete local archive contains 4,479 members, including 4,178 original
files / 891,033,646 file bytes, in 90,257,234 compressed bytes. Its SHA-256 is
`0223ce679994e34718bd7ba6f58ec9b94657f8e0cabd92b05eb180e620c4458b`.
Every member was read back and every original input rehashed before cleanup.

The [portable original histories, fault records and independent acceptance](https://github.com/c4pt0r/kv9/blob/10c946b198fdf11c99369034578b9cab70389c28/docs/receipt-tail-chaos-v1/README.md)
contains 2,304 members / 416,355,930 decoded bytes in 11 parts totaling
21,836,739 compressed bytes. Packaging `93527/eee8ee/0` and independent
verification `74789c/0` pass. The latter reads every selected byte and
independently recomputes all 9,360 invocation/return pairs and outcomes.
Original ELF/WAL payloads and duplicate audit copies remain local, with
explicit inventory references; the portable package cannot replay every full
runtime/source/payload check. Five publication boundary/selection controls pass
(`9e62f3/0`). Publication uses its own fresh 8 GiB floor, unchanged 1 GiB maximum
decrease and 8 MiB launch reserve; it does not reset the runtime/post baseline.

The [receipt design, source-mapped proof and ordinary recovery](WRITE-RECEIPT-TAIL-HINT.md)
remain separately documented. This campaign does not claim cross-host,
physical power-loss, dedicated client-link/quorum-loss or whole-Rust/Raft
verification. No original industrial roadmap work-package checkbox closes.

The subsequent independent matched write screen against CRC main now passes
with the fixed native v3 client: point Put and BatchPut(64), c1/c64, both
opposite orders and complete throughput/whole-call latency populations. Its
[results and next diagnostic](WRITE-RECEIPT-TAIL-PERFORMANCE.md) keep the receipt,
FNV and directory candidates separate. The [development route](WRITE-PERFORMANCE-NEXT.md) keeps dynamic
multi-Raft, routing, recoverable membership and automatic splits after the
write phase. Each future campaign requires fresh capacity. The Chaos campaign
itself establishes no QPS or Redis parity. All work ran locally, with no hosted
CI dispatched.
