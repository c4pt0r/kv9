# Retained-log write admission

Updated 2026-10-05. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
storage [#20](https://github.com/c4pt0r/kv9/issues/20).

`KV9_MAX_RAFT_LOG_ENTRIES` sets a local admission threshold for each data group.
The runtime default is **65,536 entries**; `0` explicitly disables it. Invalid,
negative, non-decimal and overflowing values fail startup. The configuration is
applied before a group's driver starts, including adopted groups and recovery.

The count is `last_index - first_index + 1`, saturated at zero, and includes
unstable, uncommitted and committed entries. Typed data proposals check it under
the **same peer lock as append**. At the threshold, the proposal returns
`Error::WriteBackpressure { region }` before append. An early Raw API check is
only an optimization: a successful preparation reserves no capacity. Replacing
a `RawGroup` wrapper does not reset its peer's limit.

This applies to ordinary Raw writes, aggregated fenced batches, TTL Write,
Observe and Cleanup, Raw population and the protected TTL split Append path.
The all-member format gate still runs for split; capacity is an additional check.
Batch participants retain the typed refusal. Raw gRPC maps it to
`RESOURCE_EXHAUSTED`; TTL reports its explicit Backpressure refusal. This attempt
has not been submitted and can be retried with backoff.

Ordinary ReadIndex reads continue at capacity. TTL reads that append Observe can
be refused; they must not silently use an older clock watermark or a local wall
clock. Capability queries do not require an append.

Already accepted entries continue through Ready persistence and apply. Followers
still accept replication. Checkpoint publication, compaction confirmation,
membership and other control operations remain available. Consequently **this is
not an absolute cap on all log entries or disk bytes**: control entries and
elections may increase the count past the threshold. Physical files also contain
framing, obsolete records and independent engine WAL.

Compaction advances the first retained index and restores admission capacity.
Physical Raft-log rewrite and engine checkpoint adoption are separate operations.
Automatic compaction/reclamation still use their existing explicit configuration;
without a working drain, the finite default eventually stops new data proposals.
Operators must configure and verify that drain. In addition, node-shared
[`KV9_WAL_RAFT_BYTES`](RAW-WAL-RAFT-BUDGET.md) now defaults to 4 GiB (minimum 16 MiB).
It checks actual file debt plus pending-position reservations under the append lock.
Cancellation/owner drop does not refund accepted positions; recovery reconstructs them.
Logical compaction does not refund file bytes; actual rewrite/reclamation does. This
threshold also permits accepted apply and follower/control writes above capacity.
Full disk-space guarantees, remote pressure and sustained multi-group qualification
remain open.

[`KV9_MIN_FREE_DISK_BYTES`](RAW-DISK-HEADROOM.md) now adds a local filesystem gate:
256 MiB by default, minimum 16 MiB. Each data proposal uses a fresh space sample,
deducting pending claims and overlapping writes; failed or stale probes refuse
before append. The node's log sources must use the monitored filesystem. Control
and follower persistence continue. This is a headroom policy, not preallocation or
a guarantee that replication/control cannot exhaust the disk.

[Remote replica pressure feedback](RAW-REPLICA-PRESSURE.md) additionally gates runtime
data groups using authenticated reports from their actual full Raft membership. Errors,
stale replies and same-disk address changes cannot clear known pressure. Metadata keeps
its local admission so catalog repair remains possible. Unknown reports preserve existing
quorum availability; reports reserve no remote capacity. Cross-group remote reservations,
full control/recovery space and sustained capacity qualification remain open.

Status includes `log_first_index`, `log_last_index`, `retained_log_entries` and
`max_write_log_entries` for each active group. These observations do not reserve
capacity or authorize compaction.
Node status additionally exposes `wal_raft_budget` with file/reserved/peak bytes,
refusals, sources, pending positions and uncertain sources.
`replica_pressure` reports peer identity, route, capacity/uncertainty, age and errors;
`replica_pressure_worker` reports bounded polling activity.

[Raft rewrite workspace](RAW-RAFT-REWRITE-WORKSPACE.md) now reserves exact streamed
temporary output against `KV9_RAFT_REWRITE_BYTES` (512 MiB by default, minimum 16 MiB)
and fresh disk headroom. Data proposals and remote capacity reports deduct this active
reservation. Reclamation may run above the data-debt threshold; insufficient workspace
defers before file mutation. Status exposes `wal_raft_budget.rewrite_workspace`.

## Correction to the 2026-09-24 implementation

The old gate checked `committed - first_index` in request preparation. It omitted
the first retained entry and all uncommitted proposals; concurrent preparations
could all pass before append. Its former claim of an absolute bound was incorrect.
Invalid configuration also silently disabled it, and aggregation erased the error type.

The old [Lean model](../proofs/lean/write-backpressure/) and historical process
artifacts describe their original assumptions. They do not prove the corrected
implementation, concurrency or physical disk capacity. Current component evidence
is documented in [atomic data admission](RAW-WRITE-ADMISSION.md). The real-process
runner [write-backpressure-e2e.py](../scripts/write-backpressure-e2e.py) now observes
the inclusive retained count; this increment does not claim a fresh TCP/MinIO run.
