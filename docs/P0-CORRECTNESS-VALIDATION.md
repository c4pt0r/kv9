# P0 correctness foundation evidence

Date: 2026-09-08. Tracks the first increment of
[#11](https://github.com/c4pt0r/kv9/issues/11) and
[#14](https://github.com/c4pt0r/kv9/issues/14), under
[#9](https://github.com/c4pt0r/kv9/issues/9). These issues remain open.

## Checked proof scope

Lean 4.33.1 compiled all five theorems in `proofs/lean/Quorum.lean`:
`disjoint_votes_bound`, `majority_intersection`, `unique_election`,
`quorum_after_failures`, and `no_single_voter_dependency`.
The checker inspects their transitive axioms and compiles from fresh source.

```sh
python3 scripts/check-proofs.py --lean /path/to/lean --self-test
```

Observed terminal evidence:
`PASS: 5 theorems checked; 3 invalid controls rejected`.
The controls independently exercise a proof hole, a weakened membership
precondition and an otherwise compilable custom-axiom proof. Their required
failure reasons are checked, not inferred from a nonzero exit status.
Local log: `/tmp/kv9-p0-proof-check.log`.

These are unbounded mathematical lemmas, not a full Raft or Rust verification.
In particular, `unique_election` assumes a fixed per-term vote function. Durable
vote publication, Ready ordering, log matching, leader completeness, metadata
invariants and the refinement from implementation events are still obligations.
See the [proof scope](../proofs/lean/README.md) and
[mandatory gates](CORRECTNESS-GATES.md).

## Actual Chaos Mesh evidence

The revised script ran against Chaos Mesh 2.8.4 on Kind, Kubernetes 1.35.5,
using an explicitly selected kubeconfig. The script verifies that kubectl's
node set matches the requested Kind cluster before creating test resources.
All selectors target only the run's namespace and exact kv9 workload labels.

- A late voter escaped an observed HTTP/2 handshake blackhole and joined.
- A conflicting root reached the identity boundary and was rejected.
- PodChaos killed the selected leader; replacement retained durable identity.
- Sustained PodChaos failure targeted voters 1, 2 and 3 separately. Each cell
  observed victim unavailability, a surviving leader, a successful write and
  correct reads both during the fault and after recovery.
- NetworkChaos isolated the old leader in both directions. Surviving majority
  connectivity and the broken victim edge were observed. The live old leader
  refused writes; a loopback read returned a typed application refusal. The
  majority committed a new value, which remained readable after healing.
- NetworkChaos follower delay produced measured additional TCP latency while
  writes continued. PodChaos container kill increased the restart count, and
  the cluster recovered with the committed value intact.

Local run: `/tmp/kv9-p0-chaos-e2e-r2.log`, exit 0.
Evidence directory: `/tmp/kv9-chaos-e2e.SzS3AY`, including the success scene,
topology, per-victim records, CLI outputs and process logs. The script now keeps
success evidence before deleting its namespace.

The first run failed at the fencing assertion because the harness matched
`not leader` but the CLI emitted the typed `not_leader=true` field. The retained
output established an application refusal; the assertion was corrected to
recognize that exact field. The repaired run additionally checks typed read
refusal. No database behavior was weakened to obtain this result.

The new disposable cluster initially exposed a controller startup failure:
`too many open files`, despite Helm briefly reporting readiness. Its file
descriptor limit was high, while the host's per-user inotify instance limit was
128. Running the unprivileged controller under a distinct UID removed the
failure without changing host limits. The setup script now also requires an
actual server-side dry run through the admission webhooks; the chart has no
controller readiness probe. The initial controller log is retained at
`/tmp/kv9-p0-chaos-controller-failure.log`.

After that setup correction, the complete matrix passed on the new cluster:
`/tmp/kv9-p0-chaos-fresh-r2.log`, exit 0, with evidence at
`/tmp/kv9-chaos-e2e.1K3NSv`. All seven fault resource records were retained before
deletion, alongside the three voter failure cells and typed read/write refusals.

## Availability boundary and remaining work

| Component | Present boundary | Remaining acceptance |
|---|---|---|
| Metadata and Raw KV | Three replicas of one real Raft group; every voter tested unavailable | Durable vote/Ready proof, independent histories, cross-host failure domains |
| Bootstrap/discovery | Root-certified membership and multiple peer contacts | Failure of every discovery/advertising dependency; client endpoint failover without a manual leader selection |
| Storage | Real MinIO checkpoint baseline; local engine and Raft WALs | Full persistence fault model, directory durability audit, IOChaos and checkpoint/pending combinations |
| TSO, placement, transactions, multi-group ownership | The roadmap still owns their distributed runtime and takeover protocols | Replicated authority and recoverable takeover; no indispensable scheduler, coordinator or routing gateway |
| Test infrastructure | One Kind host runs the three database Pods | Separate hosts and fault domains; a one-host test does not prove host-loss availability |

The current matrix has PodChaos and NetworkChaos coverage. IOChaos, additional
network fault families, concurrent independent history checking, complete core
proofs and multi-host availability remain open. Process death is not power loss.

The persistence audit identified an immediate C01 investigation:
`DiskRaftStorage::open` creates the directory and `raft.log` without synchronizing
directory publication, while `write_record` synchronizes file data. The failure
model must test this distinction and error continuation before the durable-vote
assumption is considered established. No power-loss coverage is claimed here.

## Continuous execution

`.github/workflows/correctness.yml` runs proof and actual Chaos Mesh jobs on
pull requests, master pushes, manual dispatch and a daily schedule. It verifies
positive completion markers and publishes logs and scenes on success or failure.
The proof toolchain, Kind binary, Helm binary and Kubernetes image have pinned
checksums/digests; the Chaos Mesh chart has a pinned release version.

`scripts/chaos-mesh-setup.sh` creates a new disposable cluster and refuses to
reuse a name or kubeconfig. It does not install into an ambient cluster.
One controller is sufficient for this disposable test fixture; production
orchestration availability is a separate deployment obligation.

Hosted results for this new workflow must be observed at its published commit;
the baseline CI success alone does not establish the new jobs passed.

The first hosted submission rejected a job-level `runner.temp` expression before
starting either job. Moving kubeconfig selection into a runner step fixes the
unsupported context. The existing `pr-gate` now runs pinned actionlint over both
workflow files. Locally, the linter rejected the original expression at the
expected context error and accepted the corrected workflows. This failure was
workflow validation, not a proof or database test result.
