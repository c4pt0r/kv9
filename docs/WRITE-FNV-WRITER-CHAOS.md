# Bounded FNV writer: actual Chaos Mesh acceptance

The experimental writer at `12f44d35590ede5f89337fe731dd950162865154`
passes the actual 21-window Chaos Mesh campaign, independent complete-history
audit, archive readback and owned cleanup. This qualifies the tested failure
and recovery behavior; **there is no new database throughput or latency result**.
CRC main remains selected.

| Complete history | Operations | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI/catalog | 5,674 | 5,152 | 507 | 15 |
| Persistent point stream | 1,502 | 1,487 | 13 | 2 |
| Native point/atomic batch | 2,696 | 2,661 | 21 | 14 |
| Total | 9,872 | 9,300 | 541 | 31 |

Every invocation has a recorded return. Unknown and refused outcomes retain
their original classifications. Four fresh final replica drains pass. All
31 observed server lifetimes and 25 containers exit; the exact owned namespace
is absent and all eight historical namespace UIDs remain unchanged.

## Faults and exact runtime

The windows cover registration-seed blackholing; failure of each voter;
partition; public admission overload; delay; EIO and ENOSPC on every voter;
missing-log and replacement-PVC refusal on every voter; and pending/recovered
endpoint migration. A normal native baseline precedes these 21 windows.
Separate checks verify observed fault effects, complete point/batch/catalog
histories, inline/apply fences and final drained replica observations.

The default ThinLTO server SHA-256 is
`d84b0ec8e466a9dc3f4953751605d91c90a90a66df1b03b57a3412e43e42603e`.
The image ID is
`sha256:3358c794b30dd04ae5f514d1acb981e2c46bc5ab43f04dbd4a46900e94fd04a5`.
Source/binary readback binds all 887 source files, default production features,
compiler settings, auxiliary clients, image payloads and observed processes.
The separate pressure example's test dependencies do not change the production
server's features. Build, image, runtime and audit tasks execute serially.

The [source proof and local checks](WRITE-FNV-WRITER.md) and
[default release and ordinary recovery](WRITE-FNV-WRITER-RECOVERY.md) remain
separate prerequisite populations: 797 workspace tests/doctests, 23 existing
ignored, 15 writer plus six kernel Lean statements, and 365 ordinary recovery
operations. They must not be added to the Chaos history count. The proof has
explicit Rust primitive, protobuf, file and compiler premises.

This campaign uses one Kind host. Independent-host and power-loss recovery,
the dedicated client-link/quorum-loss matrix and whole-system industrial
durability acceptance remain open. No original industrial checklist item closes.

## Execution and evidence

The actual fixture and observer complete session **18770**, terminal
**6a6ff0/0**. Post session **60785**, terminal **93d95b/0**, completes all six
phases: independent audit, cleanup capture, process-tree evidence, archive,
exact-UID cleanup and all-lifetime readback. Neither sequence was retried.

The independent audit correctly binds revision `12f44d3` and the exact binary
above. Its inherited human-readable scope sentence still names `bd42e60`;
that stale prose is retained in the original evidence, not treated as source
identity. The original audit also predates cleanup and correctly retains
`cleanup_complete=false`; separate successful cleanup evidence completes it.

The full local archive contains 4,174 files, 942,123,824 decoded file bytes
and 93,282,766 compressed bytes, SHA-256
`b00cd0b02bc8d9a87dbf0d009bd434bda78d4caee5602f66b7b4c82349e1830b`.
All members are independently read back and original inputs rehashed before
cleanup. Original histories, executable/WAL payloads and preparation records
remain retained. The earlier release preflight failure is published with the
ordinary recovery checkpoint. Hosted CI was not dispatched.

The [portable reporting package](fnv-writer-chaos-v1/README.md) contains 2,609 original members, 444,857,018
decoded bytes and 23,235,128 compressed bytes in 12 bounded parts. Independent
member verification passes. It includes complete histories and observer/fault
records; local executable/WAL payloads and duplicate independent copies are
omitted with original hash/size inventories. The retained full archive binds
those payloads. The reporting package alone cannot replay every full-file audit.

## Next: matched database measurements

The latest accepted c64 panel remains **139,188.639 point Put/s**, p99
**737.280–745.471 us**, and **1,065,680.142 BatchPut(64) items/s**, whole-call
p99 **6.947–7.012 ms**. These are shared-host volatile-tmpfs measurements;
Redis was not rerun and equal durability is not claimed.

Next compare the exact candidate against CRC main with the fixed native v3
client: point Put and BatchPut(64), c1/c64, two opposite orders, throughput and
tail latency. All 28 driver/auditor/schema controls pass; eight smokes and
sixteen timed cohorts have not run. The standalone checksum speedup does not
establish a database gain. Preserve the existing Raft quorum, synchronization, apply and response
fences before any performance-based promotion.

Capacity remains a prerequisite. The previous screen retained 52,317,179,904
allocated bytes. With the existing 96 GiB floor, 16 GiB restoration reserve
and 1 GiB margin, the same planning calculation requires 173,650,006,016 free
bytes; the candidate's actual allocation is not yet known. The post-run root
observation is 112,002,768,896 bytes. A bounded cache review finds 6,585,503,744
bytes of unreferenced, non-executable compiler intermediates, insufficient to
close the gap. No benchmark duration, capacity floor or evidence coverage was
reduced, and no original test payload was deleted.
