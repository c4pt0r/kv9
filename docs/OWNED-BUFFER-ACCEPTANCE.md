# Owned mutation buffers: process and Chaos acceptance

Candidate `9be0c1963515ff974426faadef80482b847b13a5` passes its own process
recovery and eleven-window Chaos fixture, including the independent history,
packet-effect and final source checks. The candidate is pushed to
`codex/owned-apply-batch`; the selected production baseline remains `ca0002c7`.
Performance is unmeasured. Correctness acceptance does not establish a speedup.

The [source change and equivalence argument](https://github.com/c4pt0r/kv9/blob/9be0c1963515ff974426faadef80482b847b13a5/docs/OWNED-MEM-BATCH.md)
consume already-owned mutation buffers at final resident-index insertion.
Mutation order, duplicate-key behavior, retained snapshots, data revisions,
applied positions, locking and WAL-before-memory ordering remain unchanged.
There is no RPC, index structure, log-format or Raft protocol change. Local
validation passes three public-API regressions, 712 workspace tests (23
ignored), all-target Clippy with warnings denied and formatting.

The equivalence argument is source-level reasoning about byte sequences and
the unchanged persistent-map contract, not a newly machine-checked whole-Rust
or Raft refinement. The existing core proof and full industrial fault-coverage
requirements remain open beyond this bounded implementation change.

## Exact build and process recovery

The original `scripts/build-native-batch.py --release` invocation performs
both real Cargo builds: the standalone correctness workload followed by the
default server. There is no retrospective manifest assembly or reuse of an
earlier server under a new source label. All 592 source inputs match both
build records, with clean revision identity and default opt3 feature graphs.

| Artifact | SHA-256 |
| --- | --- |
| Server | `07c02871276f9c52cd19bd8213ab3288d6c51a1d7391370b5a6282f0018b03f7` |
| Correctness workload | `600733e2d61cf862b4b4c4496778126c0dd3224b3ecd69ba40213c3d6b783607` |
| Original build manifest | `5a2d5e031f0a24a0704dd3a2d4cf4f0fc95f868d694b54d94c3fc56fb1243a21` |
| Source inventory | `ff23597ea569b54377bbaec94452a8c87ad4c48d983ea71db6237217d3abf851` |

The leader-SIGKILL/original-directory restart fixture and independent auditor
accept both streaming and unary RPC histories:

| Transport | Complete-history calls | OK | Unknown |
| --- | ---: | ---: | ---: |
| Streaming gRPC | 190 | 172 | 18 |
| Unary gRPC | 177 | 159 | 18 |
| Total | 367 | 331 | 36 |

Atomic batch and overlapping point histories remain complete, including all
unknown outcomes. The auditor confirms two client and five server lifetimes
exited, and fresh drained publications from all three voters in each case.
This is ordinary WAL process recovery, not a disk/power-loss experiment.

## Actual fault injection and independent acceptance

The unchanged eleven-window fixture covers baseline, Service-VIP delay,
partial loss, full client partition, exact TCP reset, quorum loss and healing
after each fault. Delay/loss/partition use actual Chaos Mesh; the same-process
exact-tuple socket reset is explicitly non-Chaos `SOCK_DESTROY`.

The complete atomic history has **2,047 calls: 1,740 OK, 220 refused and
87 unknown**. The original checker first tries a zero-unknown-effect search,
records its inconclusive result, and then produces a valid guided witness
allowing unknown effects. Both attempts remain retained. No call is removed
and no unknown outcome is relabeled as an acknowledged write.

The contained full-client-partition window has **zero OK / 38 unknown**;
the contained quorum-loss window has **zero OK / 136 refused**. These are
operations fully inside the qualified windows; overlapping operations remain
in the full history and must not be counted as contradictory contained calls.

The same native network-namespace lifetime and netem leaf `5:` (parent `1:4`)
advance from **0 to 154 drops**. The independent leaf reader checks the
original before/after command bytes and exact queue identity. Parent and
child counters are not added together.

The runtime exits zero. Independent history/lifecycle, same-leaf packet-loss
and final source verifiers also exit zero and accept the original artifacts.
Fresh post-client-exit drains cover all three voters. All five owned container
lifetimes exit; namespace UID `463e9e91-10f0-44a3-a218-da4625f2a014` and both
owned node directories are absent. Eight historical namespace UIDs are
preserved. The image binding verifies the same original executable payloads
and source defaults, without Pod limit overrides.

One ancillary root summary reader initially applied `len()` to the runner's
integer window count. That readback failure precedes any independent audit
launch and remains retained. A separately recorded corrected preflight checks
the original integer against 11. No runtime or validator is rerun or weakened.

This is one-host volatile-tmpfs client-link/quorum correctness evidence. It
does not replace the full 21-window matrix, establish inter-voter partial-loss
or storage-stall coverage, prove power-loss durability, or measure throughput.

The [publication index](../scripts/redis-reference/owned-buffer-acceptance-v1/index.json)
binds **140 exact copies / 13,345,120 bytes**, including both complete process
histories, the full Chaos history and witness attempts, window/effect records,
original leaf-counter outputs, drains, cleanup and build/source/image bindings.
The [independent readout](../scripts/redis-reference/owned-buffer-acceptance-v1/readout/REPORT.md)
retains every contained window's outcome counts. Binaries, raw WALs and
host-wide process listings remain outside this compact publication. Referenced
manifests identify additional original local inputs; the bundle is not a
self-contained runtime archive.

An initial publication inventory construction detected a duplicate destination
for two different configuration records before any copying. The retained
failure and distinct corrected destination preserve both originals. This
metadata correction changes no runtime, report or acceptance predicate.

## Performance gate

The next comparison uses CRC `ca0002c7` as control and this exact `9be0c19`
release as candidate, with unchanged native/Redis clients. It retains 36
two-second smoke cells and 72 ten-second timed cohorts: c1/c64, point/batch64,
read/mixed/write, forward/reverse repetitions. Whole-call mean/p95/p99, calls/s,
keys/s, all outcomes, original deadlines and resource budgets remain explicit.
No speedup is inferred from the removed clones.

The [CRC baseline report](CRC-WORKLOAD-PERFORMANCE.md) remains the current
performance result. All testing for this phase is local; no hosted workflow
is dispatched.
