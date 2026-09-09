# Raft I/O error acceptance with Chaos Mesh

Date: 2026-09-08. C01 increment under
[#11](https://github.com/c4pt0r/kv9/issues/11), with the proof/refinement record in
[Raft persistence](RAFT-PERSISTENCE.md). The broader issue remains open.

## What the matrix tests

`scripts/chaos-mesh-e2e.sh` runs its existing root, Pod and Network fault matrix,
then sources `scripts/chaos-mesh-io.sh`. The additional six cells cover every
voter (1, 2, 3) crossed with EIO (5) and ENOSPC (28). Each cell:

1. Confirms a healthy cluster and records a successful baseline mutation.
2. Stops one database process while preserving its Pod, container namespace,
   stable identity and volume. Confirms the other two voters can elect/serve.
3. Creates an actual `IOChaos` resource selecting that exact Pod, container and
   `/data/raft/raft.log`, with `methods: [WRITE]` and `percent: 100`.
4. Confirms injection and the FUSE mount, then starts the default production
   binary. Replay can read existing data; a real Raft catch-up write must fail.
5. Requires the database's Raft persistence error with the selected errno,
   `status.fatal`, a normal runtime failure log and exit code 1. A panic, startup
   identity error, timeout, replacement Pod or transport-only failure fails the
   experiment. The supervisor keeps the stopped process stopped for observation.
6. Records successful majority writes and reads while the replica is failed.
7. Removes the resource, restarts the replica, and requires its actual driver
   apply watermark to reach the majority's acknowledged position. Confirms the
   replicated value remains readable after healing.

CI requires one completion marker for each distinct voter/errno pair, as well as
the complete matrix marker. Success and failure artifacts retain fault resource
selectors/conditions, process logs, status snapshots, mount information,
compatibility checks and operation receipts. Exit observation files include the
Pod UID, exit code and timestamp.

The shell supervisor and compatibility launcher exist only in the Chaos test
image. The shipped `kv9` binary has no new fault-control environment variables,
marker files or runtime features. Its production behavior changed only in the
explicit propagation and terminal handling of persistence failures.

## Routing setup writes across elections

At `273426a`, the hosted run stopped before the voter-3/EIO injection: its
pre-fault setup write received the CLI's explicit `NotLeader` refusal after the
status-based leader observation. The empty `before-put.out` and absence of that
cell's fault artifacts distinguish this from an I/O failure.

Setup/majority writes and verification reads now use `scripts/chaos_client.py`
from the independent client Pod. It rotates only through the allowed voter
endpoints (excluding the failed voter for majority probes), with one 25-second
monotonic deadline. It retries only an exclusive typed `NotLeader` response with
empty stdout and exit 1. Transport errors, timeout, partial writes, mixed output
and every ambiguous outcome are terminal. It does not retry them merely because
the same key/value might appear harmless. Each attempt retains its node, address,
remaining budget, outcome, stdout/stderr and times in a JSONL artifact.

Four deterministic controls check refusal routing, budget consumption, terminal
unknown timeouts and rejection of ambiguous/mixed results. Fault effect, exit,
receipt and recovered-prefix requirements remain unchanged.

The same helper now routes positive Pod/Network/recovery point probes. The hosted
ce508ef run failed after both histories made progress during voter 2's Pod
failure: its extra Put used the earlier observed leader 3, which returned an
exclusive `NotLeader` with leader 1. The retained scene confirms leader 1 and
follower 3 in term 16, with the selected voter 2 still failed. A leader
observation does not reserve leadership for a later RPC. Shared routing now
excludes the failed/isolated voter for majority probes and retains every attempt.
The direct requests to the isolated old leader remain direct fencing probes;
their required refusal is never routed away. Unknown writes are still terminal.
The failed hosted run is retained at
[Correctness 34339349174](https://github.com/c4pt0r/kv9/actions/runs/34339349174);
it is not being relabeled as passing acceptance.

## Backend limits and observed controls

The pinned [Chaos Mesh IOChaos API](https://chaos-mesh.org/docs/simulate-io-chaos-on-kubernetes/)
uses a FUSE mount. This matrix opens the database files after mounting, making
fault participation explicit. It covers restarted replicas during replay/catch-up;
it does not establish correct replacement of already-open descriptors in a live
process, nor does it test a live leader failing its own disk mid-request.

On the local Linux 6.17 host, the first attempt failed before the intended test
point: the pinned FUSE server logged `Unknown FUSE opcode (52)` and disconnected
the mount. Linux identifies opcode 52 as
[`FUSE_STATX`](https://github.com/torvalds/linux/blob/v6.17/include/uapi/linux/fuse.h).
That run failed acceptance and is retained at `/tmp/kv9-chaos-e2e.ADlapG`; it is
not EIO/ENOSPC success evidence.

`chaos/no-statx.c` is a narrowly scoped compatibility launcher in the test image:
it returns ENOSYS for `statx` in its child process tree, allowing normal Linux
metadata fallback. It does not intercept read, write or sync. Its explicit
check requires both the ENOSYS result and successful ordinary metadata access.
The rest of the Pod/Network matrix uses the binary without this launcher.

A subsequent attempt to change an active IOChaos spec was rejected by admission.
An overlapping-resource probe showed that the pinned
[`ApplyIOChaos`](https://github.com/chaos-mesh/chaos-mesh/blob/v2.8.4/pkg/chaosdaemon/iochaos_server.go)
replaces its existing injector process when the action set changes. Those
observations are retained at `/tmp/kv9-chaos-e2e.3UjiCJ`. The accepted harness uses
one fixed resource per stopped-process/restart cell; it does not claim that an
action update preserves a live FUSE descriptor.

## Scope still required by C01/C02

These are real application-replica I/O errors on a single Kind host. They do not
simulate loss of unsynced power-loss state, corrupted durable media or separate
physical failure domains. Deterministic persistence modeling and cross-host
acceptance remain separate gates. Whole-volume failure (which may also prevent
status-file writes), engine WAL/checkpoint/pending paths, I/O latency under load,
combined failures, expanded network schedules and independent concurrent history
checking remain open. The object-store dependency and all later runtime roles
retain the mandatory no-single-point-of-failure contract.
