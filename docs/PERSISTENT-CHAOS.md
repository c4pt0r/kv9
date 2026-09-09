# Persistent workload under Chaos Mesh

The complete 19-window matrix passed locally at
`371163bb2b77ad3ed24f3cffb826c6eb6a7ff8ef` on 2026-09-09. Independent audits
accepted all 4,133 CLI operations and 1,151 persistent-client operations,
including three independently prepared replacement PVCs, three missing-log
cells and six Raft I/O fault cells. Each replacement retained a fresh lifecycle
while carrying the old root/store bundle; real initialization and two actual
runtime starts refused its incarnation before Raft or a listener opened. The
surviving quorum served during each rejection, and the exact original PVC
recovered through the majority receipt. Six replacement-evidence controls,
five missing-log controls and the existing fault-evidence controls were rejected.
Collector removal left the database able to serve reads and writes.

Retained evidence:
`target/correctness-evidence/2026-09-09-371163b-pvc-history.tar.gz`
(128,633,289 bytes), SHA-256
`eeea1e1708a9628d9c92b3442c29d4fbff8f0bee89272db1ac43e474d82fbf07`.
It includes exact sources/builds, both local 19-window runs, the original failed
hosted history, independent audits, 38 history tests and 14 isolated source
controls. The revised checker independently replays the complete previously
inconclusive 4,063-operation hosted history within the same 60-second budget;
its original hosted result remains failed. See [history checking](HISTORY-CHECKING.md).
These are local single-host results, not host-loss or throughput acceptance.
Hosted results are tracked in #9. Broader #42 acceptance and authorized endpoint
migration [#47](https://github.com/c4pt0r/kv9/issues/47) remain open.

The 16-window baseline passed locally at
`8ed7dec7dc35f10fbecebe20f917e923f8d08c13` on 2026-09-09. Its independent
checks accepted a 3,240-operation CLI Raw/catalog history and an 869-operation
persistent-client history, including all three missing-log cells and all six
Raft I/O failures. Each original voter refused two fresh missing-log starts
and recovered through a majority receipt after restoring its exact log. All
five new corrupted-evidence controls were rejected. The original failed
attempt (Pod replacement preceding old-owner lock release) is also retained.

Evidence archive:
`target/correctness-evidence/2026-09-09-8ed7dec-store-log-loss.tar.gz`
(107,650,595 bytes), SHA-256
`a86e104701791a5a3dd077e7d48dd034bdb5b2abe8ce9005e187ee5c5eb5945d`.
It includes the exact source/builds, successful and failed scenes, raw logs,
histories and audits. This is local single-host acceptance; hosted run status
is tracked in #9. This is the earlier baseline, before the replacement-PVC matrix.

The existing Chaos Mesh acceptance matrix now runs both its CLI-generated
Raw/catalog history and a separate persistent public-gRPC workload through every
selected fault window. Both complete histories must pass independent model
checking and witness replay. The original fault injection, actual fault-effect,
majority progress, exact receipt, recovery, registration and server latency gates
remain required.

This topology uses three voters with persistent local WALs on a single Kind host.
It does **not** configure `KV9_STORAGE=minio`; the separate workload executable
E2E in [WORKLOAD-RUNNER.md](WORKLOAD-RUNNER.md) verifies real MinIO. These are
distinct acceptance scopes. Neither test is a multi-host throughput result.

## Run the gate

Use an explicitly owned Kind cluster and its kubeconfig. The runner compares the
Kubernetes node set with the named Kind cluster before creating a namespace or
fault. It requires a real Chaos Mesh installation and the pinned tools installed
by [chaos-mesh-setup.sh](../scripts/chaos-mesh-setup.sh).

```sh
export KUBECONFIG=/path/to/owned-kind.kubeconfig
export KV9_KIND_CLUSTER=owned-kind-cluster
scripts/chaos-mesh-e2e.sh
```

The runner builds the default server, admission fixture and actual workload
executable. It retains the workload build manifest, complete source inventory and
binary outside the source tree, then copies real files into the Docker context.
An external shared `CARGO_TARGET_DIR` is supported without relying on Docker to
follow symlinks outside its context. Hosted CI requires the clean build revision
to match `GITHUB_SHA` during independent artifact validation.

The persistent client is one `restartPolicy: Never` Pod outside the database
fault selectors. It has two workload workers, two runtime workers, a 2-CPU limit
and a 256-MiB memory limit. Four mutable keys plus a sentinel use 128-byte values,
a fixed seed of 40, and a 50/40/10 Get/Put/Delete mix. Each worker pauses 500 ms
between completed calls. The run reserves at most 6,000 logical operations and
64 MiB of full history; each logical call has a 1.5-second deadline and at most
six attempts under the client refusal rules. A 30-minute issuance cap prevents
unbounded traffic. Duration or operation-cap exhaustion before the coordinator
finishes the matrix is a failed acceptance run.

A fresh persistent keyspace is created before the original CLI history starts
and included in its initial catalog. The two workloads use separate Raw keyspaces
and retain independent complete histories for their observed operations. No
external unrecorded allocation is hidden from the original catalog's initial
uniqueness state. The persistent runner establishes absence and acknowledges its
initial dataset before measuring; mutable-key traffic cannot modify its sentinel.

## Fault windows and evidence

| Window | Required effects in the existing gate |
| --- | --- |
| Registration seed blackhole | First seed blocked; joiner reaches Serving with an exact applied receipt through surviving seeds |
| Pod failure, voter 1/2/3 | Selected voter unavailable; surviving majority serves before healing |
| Leader partition | Minority fenced; connected majority elects and serves |
| Public admission pressure | Live partition, real count refusals, bounded reservations and majority progress |
| Follower delay | Measured TCP delay while the fault remains injected |
| Voter 1/2/3 × errno 5/28 | Actual Raft WRITE failure, fail-stop exit and recovery through a majority-acknowledged position |
| Missing Active log, voter 1/2/3 | PodChaos kills the original owner; two fresh starts refuse the missing log; surviving voters serve; the original log restores the same store |
| New PVC, voter 1/2/3 | PodChaos kills the original owner; a separately prepared PVC carrying the old root/store bundle refuses initialization and two actual starts; the original PVC restores service |

Each common phase callback atomically changes the persistent Pod's phase file.
It then waits for acknowledged Put and Get calls from **both** histories while
the fault is still present. Phase progress snapshots and observation timestamps
are retained before the original caller rechecks the actual effect and heals
the fault. Phase labels have fixed cardinality and do not grow client state.

Each persistent snapshot includes the live Chaos resources, their injection
conditions and selectors. I/O windows additionally retain the selected Pod's
metadata **during that window**, so the verifier can bind the selector to its
actual namespace, voter label, name and UID. The UID must match the recorded
fail-stop process evidence. The verifier also checks that the fault targets
`/data/raft/raft.log` WRITE calls at 100 percent with the expected errno, and
that the recovered voter applied at least the exact majority receipt index.
A resource reporting `AllInjected` alone does not establish these process and
recovery facts.

`check-persistent-chaos.py` first uses the independent whole-run verifier from
`workload_report.py`: full terminal accounting, hashes, lifecycle/populations,
logical/attempt histograms, initialization, sentinel and a legal history witness
are mandatory. It then requires all 19 phase snapshots to contain positive
Put/Get progress. Each snapshot's reported success counts must be supported by
actual terminal events at or before its monotonic cutoff. The fault resource
must still be injected and not deleting, must target the owned database namespace
and expected voter, and must agree with the retained I/O process evidence.

For each missing-log cell, `chaos-mesh-store-loss.sh` waits for PodChaos to kill
the original owner and for a new Pod UID held by a fixture shell without a
database listener. The fixture then moves the stopped Raft log out of the Raft
directory and retains its exact bytes. Before moving it, the real idempotent
`store-prepare` command must acquire the existing store's exclusive lock and
return the unchanged incarnation: Pod API deletion alone does not prove that
the original process has finished exiting. File loss is a fixture operation;
PodChaos supplies the actual process failure. This does not model power loss.
Two distinct database child PIDs must exit with the missing-file Raft recovery
error, without recreating the log or updating the old runtime status. The
surviving majority must acknowledge writes and reads while both histories make
progress. Restoring the exact saved file must recover the original PVC,
lifecycle, root and incarnation, and apply through the majority's receipt.

`check-store-loss-chaos.py` independently binds the fault's original victim to
the before/after Pod and PVC identities, repeated child exits, stopped endpoint
probes, retained log hashes, client phase times and recovery receipts. It also
replays the complete KV/catalog history witness. Five evidence controls remove
the victim, accept a startup, recreate the log, reuse a child PID, or restore
different bytes; each must fail its specific check.

`chaos-mesh-store-replacement.sh` then tests an independent replacement PVC for
each initial voter. A provisioning Pod mounts the original PVC read-only and
a newly created PVC writable on the same owned Kind node. The new directory
mints its own Prepared incarnation. Even with the correct old bootstrap
credential, `init` must refuse that incarnation before writing a root bundle
or creating Raft state. The fixture next copies only the old root descriptor
and store identity files, leaving the new lifecycle untouched, and mounts this
PVC in the actual voter deployment. Two distinct child processes must refuse
the mismatched prepared identity without opening Raft, publishing status or
listening. Both histories and explicit surviving-majority receipts must show
progress while the replacement remains refused. Switching back to the original
PVC must recover its original identity and apply through the majority receipt.

`check-store-replacement-chaos.py` verifies distinct PVC/PV UIDs, claim bindings
and physical paths, independently decodes and checks lifecycle checksums, binds
the copied bundle to the original bytes, and checks the actual init/process
refusals and recovery evidence. Six controls reuse the old PVC or lifecycle,
accept initialization or startup, remove the actual victim, or substitute a
different bundle. This tests a newly prepared disk carrying a copied identity
bundle; it does not claim to detect a complete bit-for-bit clone of a disk or
provide hardware-bound identities. Address migration is a separate #42 task.

The checker rejects three isolated corruptions of real artifacts: missing Put
progress, a cutoff preceding the claimed completions, and an uninjected fault.
It rechecks the original window after each rejection and retains the corrupted
inputs and exact rejection reasons. Missing phase-time Pod evidence fails; a
post-run Pod read cannot replace it. Histories preserve every unknown operation;
an inconclusive model search fails acceptance.

## Completion and retained artifacts

After the complete matrix, the coordinator creates the stop file, waits for the
runner to drain and verify its final dataset, and copies its artifacts before
deleting the Pod. Successful exit, `stop.reason=stop_file`, a complete independently
checked history and all fault windows are required. A fresh client then writes
and reads after collector removal, verifying that this external workload is not
a mandatory proxy or quorum participant.

The run's artifact directory contains `persistent-build/`, `persistent-run/`,
`persistent-history-checker.json`, the client Pod/resource description, runtime
CPU clock-tick rate, per-window progress/fault/Pod snapshots, corrupted evidence,
workload log/exit status and post-collector receipts. The original history,
witness, metrics, fault manifests and diagnostics remain alongside them. On
failure, available client artifacts and the live namespace are preserved for
inspection; missing or partial reports never become a passing result.

The client retry proof and database protocol/durability proofs are unchanged and
remain required CI gates. The #40 reproducible measurement protocol, client
calibration and trial variation are described in [BENCHMARKS.md](BENCHMARKS.md). Fault-test
operation counts or this paced functional fixture are not server capacity data.
