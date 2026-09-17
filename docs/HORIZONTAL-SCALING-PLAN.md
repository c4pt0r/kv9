# Horizontal scaling: implementation and benchmark acceptance

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9).

## Priority and performance closeout

Horizontal scaling is now the mainline, by explicit user direction. Redis
parity and completion of the experimental radix implementation are no longer
prerequisites. Keep CRC/rpds, ThinLTO, Safe ReadIndex, Raft quorum confirmation
and existing WAL synchronization. WAL payload preallocation remains default-off.
This closes the current optimization iteration, not the remaining industrial,
storage, performance or proof acceptance issues.

The latest qualified [write comparison](WAL-PREALLOCATION-PERFORMANCE.md) used
source `86aa6fc`, three voters on one host, concurrency 64, 128-byte values and
volatile tmpfs WALs. It measured **137,873.776 Put/s**, mean **0.464 ms**, p99
**0.745–0.754 ms**, and **1,022,750.059 BatchPut(64) items/s**, mean **4.004 ms**
and p99 **9.306–9.437 ms per batch**. These are retained measurements, not a
benchmark of the new transport or physical-disk durability. Redis parity is
not achieved. Do not add together gains from held experiments.

Radix stays outside production. The last published checkpoint contains
[789 checked theorems](RADIX-NATIVE-PRIMITIVES-PROOF.md); complete native
refinement and differential execution remain open. The two subsequent observer
drafts passed development checks only and are preserved locally with hashes
under `/mnt/data/kv9-work/radix-native-composition-development-20260916-first/deferred-source`.
They do not add qualified theorems or new performance evidence.

## Delivery sequence

Current increment: [replicated intents and durable local preparation](GROUP-PREPARATION.md)
now connect metadata Raft to isolated group storage and restart discovery.
The [data-group RPC fence](GROUP-WIRE-FENCING.md) now prevents legacy receivers
from consuming group traffic, including after endpoint downgrade/replacement.
The [fixed-voter activation increment](GROUP-ACTIVATION.md) now runs/restarts
independent durable groups on two shared data workers per runtime. The
[online control increment](GROUP-CONTROL.md) now submits authenticated durable
creation/activation requests and reconciles them on each eligible node, including
after process loss. The [initial public routing increment](DATA-RANGE-ROUTING.md)
now binds new Raw keyspaces to their own groups with ordered namespace/range
fences and terminal sealing. It requires an offline V3 writer upgrade. The
current mapping is one full-keyspace range per group. The [scoped client](ROUTED-CLIENT.md)
adds surviving metadata discovery, bounded scope refresh and terminal unknown
writes. The [multi-group throughput baseline](MULTI-GROUP-THROUGHPUT.md) now
shows the runtime converting added groups into added throughput on one host
(equal-load S(8)=2.32 on tmpfs), makes the shared data-worker pool
configurable with its default kept by evidence, and records the measured
admission, pipeline-depth, worker-count, fleet-startup and disk-fsync bounds;
it is a development diagnostic, not multi-host scaling. Split/move publication, routed scans/delete-range, retirement and complete
resource/fault acceptance remain open. No full stage acceptance or measured
scaling gain follows from these checks.
The [protocol snapshot storage prerequisite](PROTOCOL-SNAPSHOT.md) now persists
and recovers an exact snapshot/HardState pair, with uncoordinated reception and
startup fenced. It does not enable remote install or reclaim logs. The next D03
work now includes an [offline joint-generation installer](JOINT-SNAPSHOT-INSTALL.md):
it verifies the destination-bound engine/protocol pair and atomically selects a
durable generation. Existing peer startup and network snapshot guards remain.
The [committed migration authority increment](MIGRATION-AUTHORITY.md) now
replicates at most one migration intent per group, bound to the destination's
exact store incarnation, and pins one described image's complete SST closure
under exact source/destination retention owners; quiescing or releasing those
pins is fenced until committed destination-install evidence exists. The
[unified-cut source capture increment](SOURCE-CAPTURE.md) now captures a live
group's image at its exact durable cut with the configuration committed
at-or-before that cut, pins before uploading across split group/metadata
leadership, and closes the component loop: a captured image installs through
the unchanged joint installer at a learner destination with full value
readback. The [learner-attach increment](LEARNER-ATTACH.md) now commits the
destination as a learner through the group's own log with an advanced cut,
and completes offline installation at the destination's real store via a new
`install-migration-image` command, with the installed generation isolated on
restart. The [runtime-adoption increment](RUNTIME-ADOPTION.md) now turns a
selected installed generation into the destination's live replica through a
one-way adoption receipt: the driver restores the exact installed cut, and
the learner catches the source leader's retained tail via MsgAppend with
network snapshots still fenced. The [destination-evidence increment](DESTINATION-EVIDENCE.md)
now commits the destination's install evidence as an immutable catalog row
cross-checked against the published image pin, and makes that committed row
the one key unlocking the source pin's quiesce and release through the
retention ledger; the destination pin never drops. The [source-truncation
increment](SOURCE-TRUNCATION.md) now compacts the source leader's log
prefix under a committed decision bounded by the evidence cut, with every
tracked peer matched past the floor, a tail-preserving durable compaction
record, and restart accepted only under the same committed authority.
The [voter-promotion increment](VOTER-PROMOTION.md) now promotes the
evidenced destination to a voter through the group's own log, with
restarts accepted under committed configuration history and quorum
demonstrated with an original voter down. The [replica-removal
increment](REPLICA-REMOVAL.md) now retires one exact committed source
replica through the group's own log — never the destination, never below
three voters — leaving the removed replica isolated, not deleted: the
migration finally moves the group instead of only growing it. The
[storage-retirement increment](STORAGE-RETIREMENT.md) now fences the
removed replica locally and durably — the committed decision retires it
automatically, restarts open nothing, and the storage stays intact.
Next are physical reclamation, stranded-learner recovery after
truncation and split publication (D04).

| Step | Implementation | Required evidence before acceptance |
| --- | --- | --- |
| D01a / #22 | Shared transport, explicit group registration, isolated bounded inboxes and driver ownership | Actual multi-group Raft replication over shared gRPC streams; duplicate, unknown and saturated-group controls |
| D01b / #22 | Durable RegionManager: creation intents, per-group WAL/engine/manifests, recovery, activation and retirement; bounded Ready/tick workers with reserved metadata capacity | Interrupted/duplicate creation and restart; distinct leaders; no cross-group watermarks or pending IDs; scheduler and memory budgets |
| D02 / #23 | Range/epoch-aware public routing, stale-route refresh and explicit cross-range batch behavior | Route changes during reads/writes; refusal before side effects for unsupported atomic operations; unknown writes never blindly retried |
| D03 / #24 | Snapshot/retained-tail transfer, learner catchup, joint membership and safe replica removal | Destination install authority and recovery cuts; acknowledged writes survive every voter/role failure |
| D04 / #25 | Durable manual split followed by automatic size/load triggers with hysteresis and cooldown | Parent cut, child readiness and atomic ownership publication; no overlap or gap; resume after coordinator loss |
| D06 / #27 | Placement, balancing and online expansion; #26 retains the merge requirement for full D06 completion | Measured 3/6/9-node scaling, migration impact and actual Chaos Mesh acceptance |

Existing snapshot, retention, admission, compaction and GC prerequisites still
gate dependent operations. Starting D01 infrastructure does not close #19/#20
or authorize unproved snapshot installation, reclamation or split publication.
Creation/split/placement decisions must be durable and resumable by another
coordinator. No external single-instance coordinator is introduced.

## Benchmark contract: demonstrate effective horizontal scaling

Successful node joins and additional groups are not scaling evidence. C03
[#13](https://github.com/c4pt0r/kv9/issues/13) owns measurement; #22 and #27
consume the same raw results and acceptance rules. The following is the
predeclared target, not an observed result.

**Topology and controls.** Run 3, 6 and 9 equally provisioned storage nodes
on independent machines/failure domains, always three voters per data group.
Record physical topology, CPUs/affinity, RAM, disk type, filesystem, fsync
policy, NIC/link capacity, object-store placement, group/leader distribution,
server/client hashes and all configuration. Separate client machines must have
measured CPU/NIC headroom. Keep object-store capacity and client semantics
constant. Same-host runs are development diagnostics only. Real persistent
disks are the primary acceptance panel; tmpfs is a separately labeled panel.

**Separate parallelism from added hardware.** First vary active group count on
the same three-node cluster. Then hold an adequately partitioned dataset/group
count fixed (initial primary panel: 18 groups) while distributing replicas and
leaders across 3/6/9 nodes. Also report a weak-scaling panel with constant data
and groups per node. Record actual leader/replica balance and bottlenecks;
additional processes on the same CPU/disk budget do not count as added nodes.

**Workload matrix.** Measure individual Put and linearizable GET, mixed 50/50,
and atomic within-range BatchPut(64), with 128-byte and 1-KiB values. Report
batch calls/s and items/s separately. Use a fixed seeded dataset, key count,
working-set size, routing algorithm and per-cell workload across topologies.
Include uniform keys, multiple independent hot ranges, and one unsplittable hot
key. The single-key panel is a limitation/control, not an expectation of linear
write scaling. Cross-range batches must preserve their documented atomicity or
refuse; do not silently replace one atomic batch with independent RPCs.

**Two load experiments.** (1) Sweep offered load to find maximum sustainable
successful throughput under the *same per-workload p99 budget*, with bounded
queues and no hidden drops/retries. (2) Hold aggregate offered load constant
across topologies to compare latency and resource cost directly. Use scheduled
arrival timestamps and latency from intended arrival, reporting both scheduling
delay and service latency; report overload/refusal instead of omitting it.
Fix numerical latency budgets and load steps in the run manifest before timing,
after an excluded pilot; never revise them to fit an observed result.

**Duration and statistics.** Use at least 60 seconds of warmup, 300 seconds of
measurement and five independently seeded repetitions per primary cell. Rotate
topology order and retain all attempts, including failures. Compute rates from
success counts and elapsed time; merge histogram counts for p50/p95/p99/p99.9,
never average percentiles. Report each repetition, a paired-bootstrap 95%
confidence interval for speedup and raw histograms. Repetitions, not individual
requests, are the resampling unit. Do not pool different storage or durability
panels. Sustained bounded-storage acceptance additionally requires enough data
and duration to exercise steady-state flush/compaction, not only resident RAM.

**Predeclared primary scaling targets.** For the uniform 128-byte Put, GET and
mixed panels at unchanged latency budgets, define `S(N) = Q(N) / Q(3)` and
`E(N) = S(N) / (N / 3)`. The target is at least **1.6x at 6 nodes** and **2.4x at
9 nodes** (80% scaling efficiency), with the lower 95% speedup bound above 1.
All three panels must meet the target to claim this primary gate. Publish batch,
larger-value and hotspot results even if they fail. A failed target requires a
bottleneck report and follow-up work, not relabeling the target after timing.
Also show CPU-seconds/op, RSS and queued bytes per node, disk throughput/fsync
latency, network traffic, Raft apply lag, metadata latency and balance.

**Online expansion.** Under constant offered load at 70% of qualified three-node
capacity, add nodes 3→6→9 without restarting existing nodes. Record split and
movement events, bytes transferred, time to balance, time-series QPS and p99,
errors/unknowns, backlog and client routing convergence. Re-run the capacity
sweep after convergence. A cluster that only scales after manual pre-sharding
has not passed automatic split/placement acceptance.

**Correctness and fault acceptance.** Pair every measured cell with complete
outcome accounting, post-run convergence/readback and an independent history
check; reject missing or undecidable evidence. Preserve exact request attempts
and unknown outcomes. Run separate actual Chaos Mesh campaigns during split,
migration and coordinator/leader replacement, including observed partitions,
pod loss/restart and applicable I/O faults. Validate the three-voter failure
budget and metadata/placement takeover on separate hosts. Retain failed cuts
and raw artifacts. Faulted runs report their own throughput, outage and recovery
curves; do not mix them into healthy scaling numbers. Formal protocol proofs
and implementation refinement remain required independently of benchmarking.

There are **no measured horizontal-scaling results yet**. This contract does
not assert that today's single-group server already supports these topologies.
Daily checks remain local; GitHub CI is manual for releases/key milestones.
