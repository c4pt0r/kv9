# Takeover validation record

Date: 2026-09-08. Local Linux, Rust/Cargo 1.94.0. Audit base: `739ca1d`.
These results came from actual execution against the takeover worktree, not inferred GitHub Actions status.

## Published implementation checks

Implementation commit [ec50678](https://github.com/c4pt0r/kv9/commit/ec506780f273663da9b1d37fe9357495d549bf7c)
was pushed to `master`. Its [GitHub Actions run](https://github.com/c4pt0r/kv9/actions/runs/34274338807)
completed successfully: all nine jobs passed, including `pr-gate`, formatting, the real MinIO job
and six process-level acceptance jobs. The local counts and failure-control logs below remain separate evidence;
they are not claims that hosted CI executed every local mutation control.

## Normal checks

| Command | Result |
|---|---|
| `cargo test --workspace` | 413 unit/integration tests passed, 0 failed; 21 real MinIO tests intentionally ignored here and run separately; 20 doctests passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo clippy --workspace --all-targets --features checkpoint-testing -- -D warnings` | Passed; crash gates require an explicit nondefault test feature |
| `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` | Passed for all 8 workspace packages |
| `cargo fmt --all --check` | Passed |
| `git diff --check` | Passed |
| `scripts/apply-store-tripwire.sh` | 0 ObjectStore references in Raft; 57 engine positive-control hits |
| `scripts/manifest-key-tripwire.sh` | 0 escapes; 2 expected in-owner hits |
| Four instrument selftests | Mutation guard 39/39; unlanded check 14/14; apply-store tripwire 7/7; manifest-key tripwire 8/8 |

Counts are sums of Rust harness results. Zero-test binaries and ignored tests are not counted as passes.
The 413 include 23 catalog tests, both legacy-upgrade sizes, pending journal/history reconciliation and two
production bootstrap regressions. The 20 doctests include compile-fail capability boundaries and positive controls.
Normal tests do not prove MinIO availability; the following runs provide separate evidence.

## Real MinIO

Pinned image:
`quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e`.

Tests used dedicated containers/buckets and generated credentials, not pre-existing unrelated MinIO data.
With all four `KV9_OBJECT_STORE_*` variables configured:

| Command | Selected / passed / failed / ignored |
|---|---|
| `cargo test -p kv9-engine --test minio_backend -- --ignored --test-threads=1` | 18 / 18 / 0 / 0 |
| `cargo test -p kv9-engine --test minio_checkpoint -- --ignored --test-threads=1` | 3 / 3 / 0 / 0 |

Checkpoint assertions include:

- Overwrite and delete after a frozen cut; restore must combine full SSTs with the later WAL tail across all CFs,
  including `ff ff` boundary keys.
- Absorbed records actually leave `catalog.wal`; complete local replay cannot masquerade as remote recovery.
- Missing required objects cause `missing SST`; incorrect bytes cause `checksum or size mismatch`.
  Both failures preserve the WAL. Restoring the original object permits reopen.
- Immutable-object protection remains enabled. Corruption repair deletes the injected object before putting
  correct bytes; it does not bypass production immutability checks.
- Pending close/reopen preserves predecessor generation and canonical identity. Recovered capabilities require
  actual remote GET verification; missing data refuses recovery and leaves the original pending file intact.

## Eight end-to-end acceptance paths

Every script returned zero and emitted its own completion marker. Database nodes were separate OS processes
on one host.

| Script | Behavior established by its marker |
|---|---|
| `scripts/quickstart-smoke.sh` | Three-node readiness, keyspace creation and public KV operations |
| `scripts/raw-kv-e2e.sh` | Replicated reads/writes, failover, delete/delete-range and original-directory restart |
| `scripts/phase1-final-acceptance.sh` | Quorum fencing, three-voter metadata bootstrap, leader kill and durable catch-up |
| `scripts/dynamic-membership-e2e.sh` | Three voters to admitted/registered learners, promotion, five-voter failover and full restart |
| `scripts/root-trust-e2e.sh` | Explicit root, durable incarnation, discovery fences, credentialed learner and promotion |
| `scripts/partition-read-e2e.sh` | Typed isolated-leader read refusal; transport errors excluded; majority service and healing |
| `scripts/minio-kv-e2e.sh` | Remote checkpoint, real catalog WAL reclamation, failover, cold restart, unflushed tail and no delete resurrection |
| `scripts/minio-pending-e2e.sh` | Crash after durable prepare/before propose and after apply/before pending cleanup; majority advances multiple generations; original identity settles; corrupt/wrong-term pending refuses startup; later flush progresses |

The MinIO script passed both with `KV9_MINIO_EXTERNAL=1` against the dedicated test service and in standalone
mode creating and cleaning up its own container. Its terminal marker is:

```text
PASS: minio-kv-e2e (3 replicas, failover, remote checkpoint, reclaimed WAL, live tail, deletes)
```

Session artifacts:

- Standalone run: `/tmp/kv9-minio-kv-ipigazt1`.
- Final workspace: `/tmp/kv9-takeover-final-workspace.log`.
- Final E2E runs: `/tmp/kv9-takeover-final-<script>.log`.
- Restored historical-lookup control: `/tmp/kv9-pending-e2e-final.log`, artifacts
  `/tmp/kv9-minio-kv-q6w2qv6x`.
- Final pending E2E including the bootstrap fix: `/tmp/kv9-minio-kv-k5yuo7p0`.
  The subsequent ordinary MinIO E2E rebuilt the default binary.
- Object suites: `/tmp/kv9-real-minio-backend.log` and `/tmp/kv9-journal-real-minio.log`.
- Standalone MinIO E2E: `/tmp/kv9-minio-e2e-standalone.log`.

These are local session paths, not portable repository artifacts. Reproduce elsewhere using the commands above.

## Executed failure controls

| Control | Selected scope | Observed failure | Restored control |
|---|---|---|---|
| Add metadata regressions before fixing the original implementation | 19 catalog tests | 3 failures: `a failed statement must preserve its original index`, `deleting a referenced parent must preserve the FK`, `attempt to add with overflow` | Final catalog suite 23/23 |
| Replace only caller `wait_reply(reply_rx, remaining)` with the full `self.op_deadline` | One `caller_charges_elapsed_time_without_the_worker_hard_cap_masking_it` test | `caller must spend only the remaining original deadline`; actual 60s, expected 0ns; 0 passed / 1 failed | Both deadline barrier tests pass |
| Replace only durable `append_applied` with ordinary `append`, leaving the in-memory position update | One `legacy_upgrade_rewrites_all_state_and_leaves_only_positioned_records` test | `new positioned writes must remain durable after migration`; reopened position 17 instead of 18 | Both legacy-upgrade tests pass |
| Disable only the superwindow historical-query branch | One `superwindow_settles_only_from_exact_retained_transition_evidence` test | `retained historical winner must settle the applied attempt` | Targeted test passes |
| Skip only production worker `journal.stage` | Real three-process pending E2E | `prepared gate requires a durable pending journal before propose` | Full E2E passes |
| Disable only historical lookup and hold recovery until local catch-up completes | Real three-process pending E2E | Timeout at `original identity settles from retained history`; the transient single-generation catch-up window cannot mask the defect | Full E2E passes |
| Add two regressions against the original bootstrap path | Two `initialization_` tests | Duplicate planning with apply frozen; recreating an already certified root fails | Both tests pass after repair |

Each mutation introduced one defect and failed at the stated assertion or wait condition; all were reverted.
Additional logs: `/tmp/kv9-history-mutation-red.log`, `/tmp/kv9-stage-mutation-red.log`,
`/tmp/kv9-history-e2e-mutation-red.log`, `/tmp/kv9-bootstrap-before.log`,
and `/tmp/kv9-bootstrap-after.log`. No claim is made that every new test received mutation control.

## Limits of this evidence

These runs do not establish a complete power-loss/fsync/rename matrix, a long-running concurrent history checker,
cross-machine throughput, multiple Raft groups, snapshot installation/log truncation, or remote GC acceptance.
The real MinIO test service was not an independently validated multi-disk, multi-failure-domain storage cluster.
[ROADMAP.md](ROADMAP.md) orders these remaining deliverables.
