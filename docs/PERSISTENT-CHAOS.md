# Persistent workload under Chaos Mesh

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
are mandatory. It then requires all 13 phase snapshots to contain positive
Put/Get progress. Each snapshot's reported success counts must be supported by
actual terminal events at or before its monotonic cutoff. The fault resource
must still be injected and not deleting, must target the owned database namespace
and expected voter, and must agree with the retained I/O process evidence.

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
