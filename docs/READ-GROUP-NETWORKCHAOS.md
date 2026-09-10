# Exact sealed read-group NetworkChaos evidence

Exact runtime `2cbbe26a3d4273c6d265fa40c8b56c548b1a59ec` passed one actual
Chaos Mesh leader-isolation and recovery scenario with observed multi-member
read groups. During the installed partition, the majority admitted **241 read
members through 120 groups**, with a maximum admitted group of **17**. A finite
public correctness client reached **32 concurrent requests**. The isolated
former leader refused a point read after the majority acknowledged a newer
value, and recovery returned that newer value with all queues drained.

This is one scoped fault increment. It is not the full 21-window matrix,
a pre-deposition point-read phase test, a distributed proof, a throughput result,
or production/composition acceptance. Earlier exact-1ad evidence is not reused
as group acceptance, and this report does not apply to later resident-read code.

## Exact runtime and environment

| Artifact | Identity |
|---|---|
| Runtime source | `2cbbe26a3d4273c6d265fa40c8b56c548b1a59ec` |
| Default debug executable SHA-256 | `e538a53b7dc44a0eb608ba2ee581f88066d96151b265e47b5fea048e7b4edbd8` |
| Docker image ID | `sha256:309cd31ccef8ce9b9596e63ff3aa404a38c5530911ff33cc60399a7cc51affa3` |
| Running Kind image digest | `sha256:ea6a370802bcbe2e0fadf4b2aa80f8a01d9693b384b423e04063240fc61564cc` |
| Fixed persistent client source | `892b2a178450309859113c942f1738c070130eb5` |
| Fixed persistent client SHA-256 | `b47d5f408cd9a36ce7a8117917eb0fba530b0616125b4fff38980a255ddd12dc` |

The private worktree was `/tmp/kv9-group2cb-chaos-read-source`; the private Cargo
target was `/tmp/kv9-group2cb-chaos-read-target`. The locked default-feature
build completed once, with features `[]`, on CPUs 6–31. Its exact executable
was reused unchanged across all three fixture attempts. No production source or shared validator was modified. No shared Cargo target
or hosted CI was used.
All 410 tracked source inputs are retained and hash-verified in the final
attempt's `source/` directory. Default debug identity is distinct from the
release executable used in the separate performance diagnostic.

Only `/tmp/kv9-p0-ci-chaos.kubeconfig` selected the owned Kind cluster
`kv9-chaos-ci-p0-20260908`, with Kind executable
`/tmp/kv9-p0-tools/kind-linux-amd64`. The one Ready node ran Kubernetes v1.35.5;
Chaos controller, daemon and DNS were Running. Host fixture/build processes
used CPUs 6–31; observed Pod process affinity was CPUs 0–31. This shared,
single-host fixture does not establish independent physical failure domains.

The image includes the default runtime and the separately identified unchanged
Ready persistent client. Baseline, partition, recovery and cleanup snapshots
hash both `/proc/1/exe` and `/usr/local/bin/kv9` for every voter. Pod UIDs,
container IDs and executable hashes stayed unchanged, with no restart. The
persistent client's executing `/proc/<pid>/exe`, PID/start identity and status
were also captured before it exited; its complete original build/provenance
files are retained. Runtime and client revisions are deliberately distinct.

## Actual fault and multi-member reads

The NetworkChaos selector cut old leader **n3** from **n1/n2**, in both
directions. All three victim records show successful injection. Before and
after client fault-window observations, all four cut TCP directions were
blocked and both surviving-majority directions were reachable. A separate
public client Pod remained outside the database fault selector.

The targeted history first acknowledged `k=v1`. After partition, majority
leader **n1** acknowledged `k=v2`. The same live n3 Pod then returned exactly
`not_leader=true leader_node_id=unknown` for a point read of that key. It did
not return a stale value. A separate dead-port transport failure was rejected
by the typed-refusal predicate. The n3 observation was post-deposition; it
does not establish a pre-deposition point-read quorum-timeout case.

While the actual partition remained installed, an unchanged persistent client
ran in correctness mode with 32 workers, four keys, 128-byte values, a
256-operation cap and complete bounded history. Its generated main operation
mix was GET100; setup wrote five fresh values and verification checked them.
It completed **246 operations: 241 GET and 5 setup PUT**, all successful, with
231 GETs in its main work stage. No throughput rate is used here. Client
configuration was read back byte-for-byte before execution; explicit NotLeader
routing was allowed, while uncertain writes were not retried.

The majority's before/after counters increased by 120 admitted groups and 241
admitted members; the largest group observed was 17, within the turn bound of
64. Members exceeding groups proves that at least one admitted group had
multiple members during this observed interval. Counter snapshots surround the
client's full setup, concurrent reads and verification and can include the
concurrent CLI workload; the ratio is not attributed exclusively to timed
reads. The entire persistent process execution was bounded by host timestamps
strictly inside the observed injection/deletion interval.

After NetworkChaos deletion, all six TCP directions were reachable and a fresh
quorum read returned v2. Every voter reached **term 2/index 192**. Final async
active groups, active members, queued and in-flight requests were zero; public
in-flight/queued/running/encoded-byte occupancy was also zero. Public admitted
and completed counts balanced. No fatal state was recorded.

## Independent histories and outcomes

The independent continuous CLI history contains **347 complete operations**
(309 ok, 38 conservatively unknown); the separate targeted history contains
**8** (7 ok, 1 unknown). Fresh copies of both, and their merged 355-operation
history ordered by their common host monotonic clock, passed the unchanged
history checker and witness replay. Corrupting the recovered v2 result into a
stale v1 success was rejected as invalid.

The persistent client's separate **246-operation** full history passed the
unchanged workload validator, complete dataset/accounting checks and independent
history checker with witness replay. Its relative clock origin differs from
the host CLI coordinators, so it was not sorted into their timeline. Its initial
keyspace identity is linked to the targeted CLI's acknowledged fresh-keyspace
creation. All **601 original public operations** have terminal records:
**562 ok and 39 conservatively unknown** across these histories.

The 39 unknown classifications retain original responses: 22 typed NotLeader
responses (19 GET, 3 DELETE), 3 quorum-unconfirmed SCAN responses, 2 catalog
not-leader responses, and 12 catalog unique-name conflicts. They are not
relabeled as definite write failures or pre-execution admission refusals. The
quorum-unconfirmed SCANs do not establish the grouped point-read timeout path.
All original receipts, stdout/stderr and invocation/response order remain.

Within the actual injection-observation/deletion-invocation boundaries, the
CLI histories contain 265 fully completed operations (244 ok, 21 unknown),
including 52 successful PUT, 40 GET and 48 SCAN operations. The entire separate
persistent run also occurred inside that fault interval. Phase labels are
supplementary; actual fault timestamps and TCP effects determine coverage.

## Retained failed attempts and cleanup

All attempts retain the same exact runtime and fixed persistent client:

- `/tmp/kv9-group2cb-chaos-read-attempt1`: the new fixture passed its JSON
  configuration to `kubectl exec` without `-i`. The client rejected invalid
  configuration before producing a workload report/history. The repair added
  explicit stdin forwarding and a byte-identical Pod-side read-back. Its
  147 complete CLI operations (126 ok, 21 unknown) remain independently checked
  as a partial failed scenario, not accepted recovery evidence.
- `/tmp/kv9-group2cb-chaos-read-attempt2`: the persistent client completed its
  246-operation history, but the local observer rejected missing `sources.json`
  because only two of its retained build files had been copied. The complete
  run then passed unchanged validation against the original full frozen build;
  that follow-up is retained separately from the initial rejection. The repair
  copied all five original build/provenance files. Its 148 CLI operations
  (127 ok, 21 unknown) and 246 successful persistent operations remain retained;
  the aborted recovery path is not accepted as the final scenario.
- `/tmp/kv9-group2cb-chaos-read-attempt3`: the complete scenario and independent
  final audit passed. No runtime mutation or uncertain-write retry was used to
  repair either earlier fixture defect.

Each attempt's fault, namespace, Pods and PVCs were deleted after evidence
collection; all eight preexisting namespace UIDs were preserved. Host history
coordinators exited, and the persistent client was waited to completion before
cleanup. The owned images, private source/target and original artifacts remain
for reproduction. Readable PVC tar captures are diagnostic evidence, not a
claim of a power-loss-consistent snapshot.

Final evidence root: `/tmp/kv9-group2cb-chaos-read-attempt3`.
`independent/audit.json` SHA-256 is
`4908382984cb566959fe605276d5df7d063a2a88867d6ad9d9072d58a5074eff`.
`independent/persistent-group-audit.json` records full persistent validation and
every voter's group-counter deltas. `final-resource-audit.json` separately
checks injection records, strict final ledgers, source copies and cluster
cleanup. Full command argv, status, timestamps, stdout/stderr, manifests,
process identities, all original histories and every failed attempt remain.

Complete inventory: `/tmp/kv9-group2cb-chaos-read-inventory.json`, **2,307 files /
587,996,797 bytes**, SHA-256
`e0f4ed145eb7d73c09de1ed14b557fec8fd37c80c85b166d54f0674984b337df`.
Every inventoried original was independently reopened and its byte length and
hash rechecked; the result is
`/tmp/kv9-group2cb-chaos-read-inventory-verification.json`. All three attempts,
exact executable/source copies and the complete original client build remain.
