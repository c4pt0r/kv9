# Two RPC workers: exact-source eleven-window Chaos acceptance

Accept isolated source `5ee897a2f58c57bdf17ea1757c96224adf0f0dbb` as the next read-performance increment.
Runtime session 36214 and unchanged independent audit session 48057 exited 0;
mandatory leaf-only and retained-input checks exited 0. There was one runtime
attempt, with no rerun or changed acceptance predicate.

The complete atomic history contains **2,114 calls: 1,816 OK, 232 refused and
66 unknown**. Its 4,229 JSONL records comprise one header and 4,228 invocation/
return events. BatchGet contains 741 calls / 5,922 positional items; BatchPut
contains 740 / 5,917. Repeated key positions remain counted.

| Window | Seconds | Contained BatchGet OK / refused / unknown | Contained BatchPut OK / refused / unknown |
| --- | ---: | --- | --- |
| baseline | 16.311 | 45 / 0 / 0 | 45 / 0 / 0 |
| vip-delay | 24.694 | 47 / 0 / 0 | 45 / 0 / 0 |
| delay-healed | 16.872 | 46 / 0 / 0 | 46 / 0 / 0 |
| vip-partial-loss | 35.156 | 79 / 0 / 1 | 78 / 0 / 2 |
| loss-healed | 17.419 | 49 / 0 / 0 | 49 / 0 / 0 |
| vip-partition | 17.870 | 0 / 0 / 13 | 0 / 0 / 14 |
| partition-healed | 18.204 | 50 / 0 / 0 | 50 / 0 / 0 |
| exact-tcp-reset | 18.338 | 50 / 0 / 0 | 51 / 0 / 0 |
| reset-healed | 18.864 | 52 / 0 / 0 | 51 / 0 / 0 |
| quorum-loss | 18.997 | 0 / 48 / 0 | 0 / 48 / 0 |
| quorum-healed | 19.395 | 54 / 0 / 0 | 55 / 0 / 0 |

These counts include only operations fully contained within each window;
operations crossing its boundaries remain in the complete history. Both negative
windows have no newly successful contained operation, and both retain completed
batch unknown/refused outcomes. The original history checker rejects its
restricted zero-unknown-effect hypothesis, then finds a valid witness with the
complete uncertain-effect semantics. That search is not a runtime retry.
Unknown writes include 23 BatchPut, eight PUT and six DELETE calls; their
uncertainty remains explicit in the full history.

Actual Chaos Mesh exercises Service-VIP delay, uncorrelated 30% client-link
loss, complete client partition and quorum loss, with recovery windows.
Exact-tuple same-process TCP reset uses explicitly non-Chaos SOCK_DESTROY.
The same native Pod/container/netns and exact netem leaf `5:` parent `1:4`
advance from 4 to 187, proving **183 new leaf drops**. Parent and child queue
counters are not added as separate packets.

Both fresh post-client empty drain publications pass, using six serial sample
sets. All five owned Pod/container lifetimes and host launcher 2369671 have
exited. Namespace UID `ac3298d5-d86c-41a2-aa24-606e9346de53` and both exact
inode-bound node directories are absent; all eight historical namespace UIDs
remain unchanged. All 50 retained files / 12,720,362 bytes passed readback.
There are no remaining owned helpers. All 584 source files and 93 frozen
preparation inputs remain unchanged.

Batch counters over the full fixture envelope record 636 inline completions
and three blocking submissions. They include setup and verification and are
not attributed to individual calls or fault windows.

## Selection and limits

The [matched read comparison](RPC-WORKER-PAIR-PERFORMANCE.md) gains about 5%
GET and BatchGet(1) throughput over event8, improves mean latency and preserves
or improves p99 buckets. Focused tests, actual production-runtime stream/unary
leader-kill/restart histories and correctness smoke pass. Full default and
experimental-RPC workspace checks pass 707 and 717 tests, respectively, with
23 existing ignored tests in each overlapping configuration. Workspace all-target
experimental Clippy passes with warnings denied.

The [exact source contract](https://github.com/c4pt0r/kv9/blob/5ee897a2f58c57bdf17ea1757c96224adf0f0dbb/docs/RPC-WORKER-PAIR.md)
changes only async worker count. Its conditional arbitrary-interleaving safety
argument retains the base's invariant and synchronization premises. This does
not close machine-checked composition or executable refinement of the whole
implementation. Fresh quorum, successful-pump, applied-index and write
acknowledgement conditions are unchanged; no new database-wide singleton is
introduced. Progress still requires the stated scheduling, callback/storage
and live-quorum assumptions.

This campaign establishes one shared Kind host's volatile-tmpfs client-link/
quorum correctness. It does not establish performance, disk/power-loss,
cross-host, inter-voter partial-loss, storage-stall or original 21-window
acceptance for this source. Master/default promotion, sustained traffic,
writes/mixed workloads, larger batches and other CPU budgets remain open.
The original #9 checklist remains unchanged. All checks were local; hosted CI
was not dispatched.

Raw evidence: `/tmp/kv9-native-link-acceptance-5ee897a-attempt1`. Frozen preparation:
`/tmp/kv9-rpc-pair-chaos-preparation`. Compact closeout:
`/tmp/kv9-rpc-pair-chaos-acceptance-first/RESULT.json`. The closeout's `events` field counts all
JSONL records, including the single header, as distinguished above.

| Artifact | SHA-256 |
| --- | --- |
| summary.json | `001a4db03cbe7207308fb4c792ed07b8832538c82d09ed665ef0dd0a2a743dc5` |
| acceptance-audit.json | `bc5594d82773ada744ce27fa76b90f2c4e4ed40a34fce723c6f31808c753597f` |
| acceptance-leaf-readback.json | `7622ffebedcf3d17bca0d7dc369ebf71a004958642485d71d1b87319ada64585` |
| acceptance-retained-inputs.json | `a9f32c8767bc2b84b60c6e60f56d6b715341f36dbc33cd33bce0bade7525f620` |
| Closeout RESULT.json | `84e10219db950a64435f9a7dd86e6a51718d4eb30738b4559ca5d128e67cb6c5` |
