# Segmented WAL fixture readiness

Tracking: S01/#15. This fixture adaptation does not accept the production
integration, deductive protocol proof, performance claims or Chaos history.
Those gates run against the integrated executable separately, locally by
default. Hosted workflows remain manual release or milestone operations.

## Reading checkpoint and reclamation evidence

`scripts/wal_layout.py` observes both the legacy WAL and the segmented topology.
For `KV9W\x03SEG1`, it validates bounded topology bytes, SHA-256, segment-header
CRC, basic descriptor positions and the selected predecessor chain. It extracts
the embedded checkpoint. An old `catalog.checkpoint` sidecar is ignored, even
when it contains a newer index or malformed bytes. Invalid new topology never
falls back to that sidecar. A zero-length legacy compacted tail is supported
only without a segment directory; missing or short roots beside segments refuse.

The observer is intentionally narrower than the production recovery decoder.
It does not validate committed Raft membership/history, retrieve SST objects,
check all payload records, or establish durability. The real server and restart
assertions retain those responsibilities. Reading live metadata is an
observation, not a consistent backup.

The MinIO process fixture now follows the physical layout:

1. Confirm the original user values and deletion are remotely checkpointed.
2. Capture each voter's selected segment set and raw topology. Overwrite one
   filler key with 60 KiB values until every voter advances its active sequence.
   At most 300 acknowledged puts force the default 16 MiB production rotation.
   The 120 KiB hex argument remains below Linux's per-argument limit, and reusing
   one key keeps the remote snapshot small.
3. Delete the filler key and wait for a checkpoint covering that deletion on
   every voter. Confirm the initially selected prefix files are actually absent.
   Active and straddling files remain governed by production retention rules.
4. Search every physical segment file, including unselected leftovers, for the
   original absorbed values. Searching `catalog.wal` alone would inspect only
   metadata and could falsely report reclamation.
5. Keep the existing leader failover, all-process remote recovery, unflushed
   tail recovery and post-restart write/read assertions. The pending-flush
   variant hashes the complete stopped WAL file set, including segment files
   and publication artifacts, before each invalid pending-journal restart.

`wal-reclamation.json` retains the selected prefixes, filler count and final
checkpoint observations. The workload E2E and benchmark also read checkpoints
from the selected layout. Benchmark checkpoint artifacts retain the exact
embedded manifest bytes, preserving the existing report verifier's hash check.

Run the observer controls locally:

```sh
python3 -m unittest discover -s scripts -p test_wal_layout.py -v
python3 -m py_compile scripts/wal_layout.py scripts/minio-kv-e2e.py scripts/workload-e2e.py scripts/benchmark.py
```

Once the integrated binary is ready, the process fixture accepts an explicit
binary without rebuilding or changing a shared target directory:

```sh
KV9_BIN=/absolute/path/to/kv9 python3 scripts/minio-kv-e2e.py
KV9_BIN=/absolute/path/to/kv9 KV9_TEST_PENDING_CRASHES=1 python3 scripts/minio-kv-e2e.py
```

The pending variant requires the repository's existing testing-feature build.
These are real object-store and three-process checks; they are separate from
the full concurrent-history Chaos matrix.

## Existing Chaos environment

The Kind fixture mounts the entire `/data` PVC. Pod replacement, endpoint
migration and all-process restart retain `catalog.segments` automatically.
The active Raft log-loss and IOChaos windows intentionally target
`/data/raft/raft.log`; segmenting the engine WAL does not change that file.
Do not silently retarget those accepted Raft tests. Additional engine-segment
write/sync/publication fault windows belong to S01 integration acceptance.

An explicit local readiness check on 2026-09-09 verified the node identity and
Ready condition for `kv9-chaos-ci-p0-20260908` using only
`KUBECONFIG=/tmp/kv9-p0-ci-chaos.kubeconfig`. The controller, daemon and DNS pods
were Ready and the PodChaos, NetworkChaos and IOChaos APIs were available.
The check also started a separate pinned MinIO container on an ephemeral local
port, created an owned bucket, wrote and read exact object bytes, then removed
that container and its credential file.

This proves environment readiness at that observation, not fault injection or
database acceptance. Recheck the explicit context and node identity before a
new matrix. Preserved resources from another failed matrix must be handled by
that run's owner rather than removed by an unrelated readiness probe.
