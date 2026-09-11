# Read-credit candidate passes the local eleven-window Chaos protocol

Exact source `57ff6851e40ed63c837189d6eb0a11190704725a` passes its first
complete local eleven-window fault recording, the unchanged independent audit,
exact netem-leaf readback and final source verification. All commands exit 0;
no runtime rerun or acceptance-predicate change is needed. This completes this
source's existing local fault protocol, not the broader industrial fault matrix.

The full atomic operation history contains **2,079 calls: 1,780 OK, 224 refused
and 75 unknown**. Both existing checker search attempts are preserved: the
zero-unknown-effect search does not produce a valid history, while the guided
whole-effect search finds a valid witness. This accounts for uncertain effects
without silently treating unknown writes as failures or deleting the first
search result.

## Fault effects and recovery

The original windows cover baseline, actual Service-VIP delay and recovery,
30% loss with zero correlation and recovery, full client partition and recovery,
exact TCP reset and recovery, and quorum loss and recovery. Active Chaos Mesh
identities, Service/EndpointSlice bindings, packet/socket observations and
simultaneously healthy control paths remain part of acceptance.

Both negative windows have zero contained successful operations of any kind.
Client partition completes 37 unknown calls; quorum loss completes 136 refused
calls. Those counts use operations contained within the checked wall windows;
coarse phase labels include transition activity and cannot replace that check.

The exact native netem leaf, handle `5:` under parent `1:4`, advances from zero
to **153 drops** in the same attested namespace/process lifetime. Parent and
child counters are not summed as a packet count. The TCP reset is explicitly
implemented by a non-Chaos exact-tuple `SOCK_DESTROY` helper, with old/new socket
evidence and an unchanged native workload process. It is not mislabeled as a
Chaos Mesh reset experiment.

After the native workload exits 0, all three replicas produce two serial fresh
empty observations, Serving and nonfatal with running read/apply registries.
The fault fixture's public limit remains 64 requests / 64 MiB, and asynchronous
read/apply bounds remain 128 each. These are the fault protocol's original
settings; the separate performance fixture uses its own fixed configuration.

Five owned Pod/container lifetimes exit, and the native workload process is
absent. The newly created namespace and two inode-bound owned node directories
are removed. All eight historical namespace UIDs remain unchanged; the image
compatibility probe is also removed. The fixture archives and checks 50 files /
12,437,174 bytes before cleanup.

## Source and evidence

All 594 candidate source files and 80 frozen preparation inputs retain their
original hashes. The 93 predecessor inputs are read back before preparation.
Runner, auditor, contracts, observers, vendor tools and leaf reader are unchanged;
only source/build/image/owned paths and probe identifiers are adapted.

| Artifact | SHA-256 |
| --- | --- |
| Server | `6d76779a3e421388692b01ce08bd289477283fc05fd28b734eaed9138e0f7e30` |
| Release manifest | `7e5179f856b8828c26a6bf68ce0056755e16e12bbdc35c9a11845178712caabd` |
| Native fault workload | `071c51ca191875b442089c851d6c5f44d4a546b4fcc4aebc8c1be77dca6bf311` |
| Independent audit | `6f1aae6e28c9fb9e3a0b119fa31c660a8dd499b1e9839dda39fb21b378a2552e` |

The [retained full report](../scripts/redis-reference/read-credit-followup-v1/chaos-acceptance/REPORT.md)
contains the exact source/image identities, commands, all window populations
and artifact hashes. Original report directory:
`/tmp/kv9-read-group-credit-chaos-acceptance-first`. Raw recording:
`/tmp/kv9-native-link-acceptance-57ff685-attempt1`. Image, fixture and readback
sessions 22297, 56168 and 60804 all terminate before the longer c1 timing study.

This is correctness evidence on one shared Kind host with volatile node tmpfs.
It establishes neither cross-host availability nor disk/power-loss recovery,
inter-voter partial-loss/storage-stall coverage or the full 21-window matrix.
The outstanding grouped-read/Ready/Rust formal composition also remains open.
No hosted CI runs. The separate [longer c1 follow-up](READ-CREDIT-C1-FOLLOWUP.md)
also completes; broader load/semantic gates and a consistently better latency
result remain open before general promotion.
