# CRC candidate passes native client-link and quorum fault acceptance

The exact CRC candidate `ca0002c7f8e9ee6f595efcc9f4151085ccce87cb` passes
the eleven-window native workload fixture, followed by its unchanged independent
history/lifecycle auditor, same-netem-leaf loss check and final source verifier.
Runtime and all three checks exit zero. These results complement the completed
[write screen and process recovery](CRC-WRITE-SCREENING.md). The subsequent
[full c1/c64 workload matrix](CRC-WORKLOAD-PERFORMANCE.md) supports baseline
selection with an explicit small batch-read throughput/p99 tradeoff.

## History and actual fault effects

The complete atomic history contains **2,068 calls: 1,761 OK, 232 refused and
75 unknown**. The checker retains both the unsuccessful search that assumes no
unknown write took effect and the subsequent valid witness allowing unknown
effects. This is one fixture execution, with the original checker behavior;
unknown writes are neither discarded nor silently treated as successful.

| Window | OK | Refused | Unknown |
| --- | ---: | ---: | ---: |
| Baseline | 132 | 0 | 0 |
| Client-link delay | 132 | 0 | 0 |
| Delay healed | 136 | 0 | 0 |
| Client-link partial loss | 194 | 0 | 4 |
| Loss healed | 140 | 0 | 0 |
| Client-link partition | 0 | 0 | 36 |
| Partition healed | 144 | 0 | 0 |
| Exact TCP reset | 148 | 0 | 0 |
| Reset healed | 152 | 0 | 0 |
| Quorum loss | 0 | 136 | 0 |
| Quorum healed | 156 | 0 | 0 |

Window counts include only calls fully contained in a window and do not sum to
the complete history. The negative-window statement concerns those contained
calls, not every overlapping operation. Delay, loss and partition use actual
Chaos Mesh injection; exact-tuple TCP socket destruction is a separate,
explicitly non-Chaos mechanism.

The native client's same Pod/container/network namespace and netem leaf `5:`
(parent `1:4`) advance from **0 to 158 drops**. Original command outputs 1152
and 1336 are retained. Parent and child counters are not added together.
The native writer lifetime remains constant across the fixture.

## Identity, drains and cleanup

The server SHA-256 remains
`b33b5d302f901aac5cf6a95449b9bb5dd68473747e011220dbed7aed2e591d13`:
the original timed binary is reused unchanged. The native correctness client
was built separately from the same clean source, with SHA-256
`8358b36fe8d394460d624fdde106070b1aac87e92053912b5511a32b0b3eed6c`.
The recovery assembly explicitly retains both original default-release Cargo
builds. Source, image, process lifetime and default-feature bindings pass.

Six observations after client exit cover all three voters, with two fresh
empty Serving exports per voter. All five owned container lifetimes exit;
namespace UID `3bc58cea-caa4-4930-9823-070abf5761cc` and both owned node
directories are absent. Eight historical namespace UIDs remain unchanged.
The unchanged auditor rehashes 50 retained files totaling 12,382,528 bytes.

The [publication index](../scripts/redis-reference/crc-chaos-v1/index.json)
binds 52 exact copies totaling 8,069,119 bytes, including the full native
history, checker witness, window effects, original leaf counters, drains,
cleanup, independent results and helper source. This compact publication and
the auditor's retained-file inventory are different populations; additional
original command logs and build inputs remain at their recorded local paths.
The [independent readout](../scripts/redis-reference/crc-chaos-v1/acceptance/REPORT.md)
and [structured result](../scripts/redis-reference/crc-chaos-v1/acceptance/RESULT.json)
retain detailed provenance.

This is one-host, volatile-tmpfs client-link/quorum correctness acceptance. It
does not establish disk/power-loss durability, inter-voter partial-loss or
storage-stall coverage, replace the full 21-window suite, or measure performance.
Redis parity and full industrial fault coverage remain open. Tests ran locally;
no GitHub workflow was dispatched.
