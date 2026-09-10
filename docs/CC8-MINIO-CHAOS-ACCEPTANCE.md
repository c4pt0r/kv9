# Exact cc8 MinIO and Chaos Mesh acceptance

This record covers runtime `cc8bc87b6b34d07c76eb2019651aff7574f5fea1` with
its default-debug production features. A fresh real MinIO run and the unchanged
complete 21-window actual Chaos Mesh matrix passed, followed by independent
complete-history and effect checks. This supplies exact-version compatibility
evidence for the scheduling/completion lineage and accepted-socket change.
Runtime promotion and proof composition are tracked separately; this record
makes no broad P0, power-loss or separate-host availability claim.

## Exact source and executing artifacts

The source worktree `/tmp/kv9-chaos-cc8-acceptance` stayed clean at the exact
revision throughout building, execution and independent verification. Every
tracked source hash was rechecked. Cargo used the private target
`/tmp/kv9-chaos-cc8-target`; the selected `kv9` artifact and its fingerprint
both recorded empty enabled features. The admission-pressure example and
persistent client were built separately by the existing fixture.

| Artifact | SHA-256 |
|---|---|
| Source inventory | `b2bb5aa3d5f610bc9b34a08a37f5afebb07af4daff30c197747193b3320c4939` |
| Default-debug KV9 | `8aec1b232489e85da4b3e64326c90a82590f79e1b14bf338898ed18b9ba7766d` |
| Persistent workload executable | `e9c27070e0c05994bfcad8d4b9768ea87760b0fbb50cf2e6a332a1b555ef0784` |
| Admission-pressure executable | `101eba31d9fa2182919ae647c4d132015927277b338b921e1e252297687b2fa1` |
| Local image configuration | `1c487fd85322c767cb61c6784c5219dcddb1ae9470d46eb788422c1676bb47a1` |

The image tag is `kv9-chaos:cc8bc87-20260909-attempt1`. An external observer
read `/proc/1/exe`, the installed executable and the selected segmented WAL
layout inside each of the three running voter Pods. All three actual process
hashes matched the default-debug binary, and their Pod UID, container ID and
image ID observations are retained. Docker image executable hashes matched
the copied private build outputs. These are execution-time observations, not
hashes inferred from a branch name or a later build.

The MinIO observer separately captured ten replica process lifetimes through
`/proc/PID/exe`, with PID start ticks, command lines and CPU affinity. Every
observed executable matched the same retained default-debug binary; before/after
binary hashes also agreed. Retained executable copies are under
`/tmp/kv9-cc8-acceptance-build`.

Root's separate exact-cc8 workspace/process checks use another bracketed debug
artifact, `62dfd1ce29766b42b3281e0f676a379dcd8a72f682d53853393fcd87fd5b0bd9`.
That artifact is distinct from the image/MinIO executable recorded here. The
prior cc8 release benchmark likewise has its own identity and is not reused as
an execution identity for these fault tests.

## Real MinIO result

The unchanged `scripts/minio-kv-e2e.py` passed three-voter service, leader failure,
continued writes, remote checkpoint publication, whole selected WAL segment
reclamation, complete restart from the remote checkpoint plus live tail, and
deletes. It issued 272 acknowledged 60-KiB filler overwrites to force actual
segment rotation. After deleting the filler, checkpoint index 347 covered the
three selected closed prefix files; all three were physically absent. Active
segments remained selected. Final retained voter statuses agreed on term 4,
index 354 with no fatal state.

MinIO used the fixture's pinned image
`quay.io/minio/minio@sha256:14cea493d9a34af32f524e538b8346cf79f3321eff8e708c1e2960462bd8936e`.
The Raw fixture retained the existing rule that an ambiguous write fails the
scenario rather than being retried. Its default 100-ms flush interval and the
existing slow-flush restart tail scenario were unchanged.

Raw evidence is `/tmp/kv9-minio-kv-kvhd75uk`, with log
`/tmp/kv9-cc8-minio-process-attempt1.log`. The separate identity observer record
is `/tmp/kv9-cc8-minio-identities.json`; the final status/layout/reclamation audit
is `/tmp/kv9-cc8-minio-final-audit.json`. The raw file manifest covers 41 files
and 50,770,326 bytes. All ten observed replica lifetimes exited, and no MinIO
fixture container remained after cleanup.

## Actual Chaos Mesh coverage

Only `KUBECONFIG=/tmp/kv9-p0-ci-chaos.kubeconfig`, context
`kind-kv9-chaos-ci-p0-20260908` and owned Kind cluster
`kv9-chaos-ci-p0-20260908` were used. The explicit Kind executable was
`/tmp/kv9-p0-tools/kind-linux-amd64`. The fixture checked Kubernetes and Kind
node sets before creating its unique namespace
`kv9-chaos-1789014035-3578453`.

The matrix retained actual fault effects and successful independent CLI and
persistent-client read/write progress inside every required window:

| Window family | Count | Observed effect |
|---|---:|---|
| Registration seed blackhole | 1 | Joiner reached Serving through healthy seeds with an exact receipt |
| Sustained voter Pod failure | 3 | Each voter failed in turn while the majority continued service |
| Two-way leader partition | 1 | Isolated authority fenced while the majority continued service |
| Public admission pressure during partition | 1 | Bounded admission refused pressure while quorum history progressed |
| Network delay | 1 | Delayed follower and subsequent recovery |
| IOChaos EIO / ENOSPC | 6 | Both errors reached Raft on every voter, caused exit, then recovered with majority service |
| Activated Raft log loss | 3 | Each voter refused two missing-log starts, then recovered its original store |
| Independently prepared replacement PVC | 3 | Each voter refused the old bundle on a foreign store, then recovered its original PVC |
| Endpoint migration pending / recovered | 2 | Retained-PVC replacement obtained exact endpoint confirmation and preserved both histories |

Additional stages killed all original owners before catalog formation,
verified original-store formation recovery, replaced a Pod, and proved a killed
container restarted in its original Pod/store with a live endpoint. After the
persistent collector was removed, database reads and writes still succeeded.

## Complete histories and independent checks

Both histories contain every invocation and paired return across all 21
windows. A fresh copy of the raw artifacts was checked independently; original
history bytes were compared with the copy. Only prior derived persistent-check
outputs were excluded so the independent checker could create new results.

| History | Paired operations | Successful | Unknown | Refused | SHA-256 |
|---|---:|---:|---:|---:|---|
| CLI / catalog | 5,001 | 4,510 | 480 | 11 | `d01797850565647a0685b6efce0f57e0737fbfd57024e1ea6befc7b25312785e` |
| Persistent SDK | 1,294 | 1,277 | 14 | 3 | `ee75cb728af5ba67ce103c14d13a66116ce7a3040a0ba09c41f897b3745a0202` |

The CLI unknown population contains 292 mutations, including catalog creation,
and 188 reads/scans. The persistent population contains eight unknown writes
and six unknown reads. An unknown write is not a definitely failed write, and
unknown operations were never retried. Full operation/phase/reason breakdowns
are retained in `/tmp/kv9-chaos-cc8-outcomes.json`.

The original and independent CLI checkers each explored 7,512 states and
returned valid. The independent persistent checker accepted the exact revision
and all required phases. Six additional independent effect auditors passed:
formation, log loss, replacement stores, endpoint migration, latency metrics
and admission pressure. They retained their existing invalid-evidence controls:
five log-loss, six replacement, five endpoint, three latency and four admission
controls; the persistent checker also rejected three invalid fault-evidence
controls. These observer controls accompany actual injected faults rather than
substituting for them.

## Retention, cleanup and limits

The fresh default build, MinIO run, Chaos run, process observers and independent
checks all passed their first attempts. No runtime source, shared validator or
fault-matrix repair was needed. Earlier failed development attempts remain in
their original b4a/cc8 archives; they were not relabeled by this fresh run.

The shell/Cargo affinity was CPUs 6–31. Actual voter Pod affinity was 0–31 on
one owned Kind control-plane node, sharing one physical host. This demonstrates
application-replica effects, not independent machine or storage failure domains.
Chaos used local WAL storage; MinIO was verified by the separate executable
scenario above. Neither scenario proves complete disk power-loss behavior.
No hosted workflow was dispatched.

The raw Chaos directory is `/tmp/kv9-chaos-e2e.uEc9tg` (1,132 files,
156,753,157 bytes); log `/tmp/kv9-chaos-cc8-attempt1.log` ends with
`COMMAND_EXIT=0 LOG_EXIT=0`. The fresh independent copy is
`/tmp/kv9-chaos-cc8-independent`. Principal records are:

- `/tmp/kv9-chaos-cc8-inventory.json`: exact source/build/image identities and every attempt.
- `/tmp/kv9-chaos-cc8-final-audit.json`: all phases, original/fresh history hashes and source audit.
- `/tmp/kv9-chaos-cc8-independent-effects.json`: all six independent effect commands and terminal outcomes.
- `/tmp/kv9-chaos-cc8-file-manifest.json`: original raw file lengths and hashes.
- `/tmp/kv9-chaos-cc8-cleanup-verification.json`: owned namespace absent; all eight preexisting namespace names and UIDs preserved.

Original data, complete histories, process logs and scene snapshots remain on
disk. Cleanup affected only the invocation's owned resources. This exact cc8
result does not turn older f2, b4a or later b3/fe/Ready fault evidence into a
result for another runtime.
