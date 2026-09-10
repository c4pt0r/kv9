# Resident read: exact local Chaos Mesh acceptance

The second unchanged full local matrix passed on resident runtime
`11cae977f15df0912c2a35561480d45447bae660`, with both complete public-client histories
independently checked, all original fault-effect checks repeated, and a separate
PID-aware observer confirming resident GET activity during actual faults and recovery.
This is one-host correctness evidence. It does not establish performance, cross-host
resilience, power-loss durability, full proof composition, or acceptance of later runtimes.

## Exact inputs

The clean private source is `/tmp/kv9-chaos-resident-acceptance`; private Cargo target
is `/tmp/kv9-chaos-resident-target`. All 412 recorded runtime/build inputs remained
unchanged, aggregate SHA-256 `34b6fcebd05d90f799d912425dd64e247ffe758e8ae88bbb7274670ace648ef3`.
The default-debug server has Cargo features `[]` and SHA-256
`f687329a34304d75737c8c9b0c0ae303aa5685a700e55d5ece5b885352ac51ce`.
The independent persistent correctness client was built from the same exact revision,
SHA-256 `e78386c1e321b6ffb7b9a5b64d1451325c851f601fe05e4e0888334ffeb9df1f`;
it is separate from the frozen Ready performance client.
The admission-pressure executable SHA-256 is
`dcf4ae3cda619a072b2958124c072e975e3b6100da8337e5f380d26e684f5b70`.

Image tag `kv9-chaos:resident11cae-20260910-attempt2` resolves to Docker image
`sha256:e724c5fa88ef323b57b6dda6ec22f343dd84a791090c6864df79f3c27f533173`.
Observed Kind container image identity is
`docker.io/library/import-2026-09-10@sha256:fbd1fe538472524211769ee5c921fa65e8f0be080d8d162bace9489e06d0a8ef`.
The observed runtime `/proc/PID/exe` hashes match the default server above.
The persistent client's live PID 15/start ticks 131710698 was independently hashed
while its history was running, with Pod UID `edd457e0-6e56-4772-ab09-8eee256f7e13`.

Only the explicitly selected kubeconfig `/tmp/kv9-p0-ci-chaos.kubeconfig`, Kind CLI
`/tmp/kv9-p0-tools/kind-linux-amd64`, and cluster `kv9-chaos-ci-p0-20260908` were used.
The original matrix, workloads and shared validators were unchanged. Host work used
CPUs 6–31; observed database Pods and persistent client used CPUs 0–31 on one host.
Proof/background work continued. This was not a timed performance comparison.

## Complete histories and positive effects

The unchanged matrix required concurrent CLI and persistent PUT/GET progress in all
21 windows: seed blackhole; each voter's sustained Pod failure; partition; public
admission pressure; follower delay; each voter under EIO and ENOSPC; each voter's
missing-log refusal; each voter's independent replacement-PVC refusal; and pending
and recovered endpoint migration. Additional gates covered all-voter pre-formation
crashes, container and Pod replacement, original-store recovery, and continued reads
and writes after removing the persistent collector.

All six IOChaos windows reached the intended voter's Raft persistence path, caused
exit, and recovered with majority service. Missing-log starts were rejected twice
per voter; replacement PVCs carrying old identity material were rejected before
original-store recovery. The retained fault objects, per-voter observations, effect
probes, complete histories and negative evidence controls passed again in the
independent copy. The same live isolated old replica returned typed `not_leader=true`
after majority progress; this is post-deposition evidence, not a pre-deposition claim.

| Second-run history | Invocations = returns | OK | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI | 5,109 | 4,650 | 453 | 6 |
| Persistent | 1,405 | 1,378 | 23 | 4 |

Unknown outcomes remain uncertain in the complete checker input; they are not
classified as definite failed writes. The persistent unknown population includes
7 PUTs and 16 GETs. All ten explicit refusals above were count-admission refusals.
No workload or checker criteria were relaxed. The original and independent CLI
checkers each explored 5,273 states. Histories were copied byte-for-byte:

- CLI SHA-256: `30663103302ffa949be479b7d8f71057f8feec0935fe9b8da73d66dd16ab7cde`.
- Persistent SHA-256: `f10c85ef7ef93632b1c2953966186b65c220b8a2fb9e543fde82473acaf96354`.

## Resident-path observation and retained failures

The first full matrix at `/tmp/kv9-chaos-e2e.mseyCn` passed its original and independent
history/effect checks: CLI 5,071 complete operations (4,631 OK/435 unknown/5 refused),
persistent 1,348 (1,324 OK/19 unknown/5 refused). Its additional observer incorrectly
assumed the runtime remained PID 1. Later fixture Pods use Bash PID 1 and separately
executing kv9 children, so the first run has incomplete later-child execution
attestation. It is preserved with that limitation, not relabeled as the second run.

Three owned audit failures and their exact sources/logs remain in that first root:
`resident-inline-audit-first.*` used the benchmark request cap; `-second.*` used the
benchmark byte cap as the absent-override default; `-third.*` exposed the PID 1 gap.
The actual primary Pod override is 8 requests/2 MiB; absent overrides use the exact
runtime defaults of 64 requests/64 MiB. Async read admission is 128. None of these
repairs changed production code, Pod configuration, workload or shared validators.

Before repeating the matrix, a bounded real Bash-parent/child observer check accepted
the correct executing child and rejected wrapper PID, wrong hash, PID disagreement,
start-tick disagreement and dead PID. Both owned test processes exited. The fresh
observer binds Pod UID/container identity before and after sampling, exported status
PID, process start ticks, and `/proc/PID/exe` hashes before and after. Wrapper pidfiles
only corroborate that identity. Limits come from the exact Pod spec and source defaults.

The second observer retained 407 batches and 1,411 attempted samples; 1,259 matched,
including 468 wrapped-child samples. There were 35 sampled runtime lifetimes. Transient
failed observations remain visible and cannot support positive claims. This is
sampled serving-lifetime attestation, not a claim to hash every brief rejected start.
The strict audit passed on its first second-run execution: counter monotonicity,
14 positive fault/recovery envelopes, and drained final ledgers on all four replicas.

| Required envelope | Serving PID | Inline before → after | Contained successful persistent GETs |
| --- | ---: | ---: | ---: |
| Actual partition | 1 | 6 → 26 | 10 |
| Actual follower delay | 1 | 7,236 → 7,241 | 4 |
| Endpoint migration recovered | 43 | 298 → 304 | 4 |

Each envelope stays within one Pod UID/PID/start-tick lifetime, has the same owned
Chaos resource `AllInjected` at both boundary observations, and contains checked
successful persistent GET completions. Counters include every public client and
export/sampling timing: the deltas are positive activity evidence, not attribution
of a particular response. Legal blocking fallback is not prohibited. Final public
in-flight/running/queued/bytes and async queued/active/groups/in-flight counters are
zero on all four Serving replicas, without fatal or stopped-read state.

## Retention and cleanup

Second raw root: `/tmp/kv9-chaos-e2e.wQx9D1`; independent copy:
`/tmp/kv9-chaos-resident-independent2`. First raw and independent copy remain intact.
The matrix and observer terminated successfully (session 49528); fresh complete-history
checks (41009), effect rechecks (15191), and the strict inline audit all passed.
Both owned matrix namespaces were removed; all eight preexisting namespace UIDs
were preserved. Original files and cached immutable images remain available.
No GitHub CI or runtime/shared-validator source change occurred.

Detailed records:

- `/tmp/kv9-chaos-resident-preparation2/plan.json`
- `/tmp/kv9-chaos-resident-final-audit2.json`
- `/tmp/kv9-chaos-resident-independent2-effects.json`
- `/tmp/kv9-chaos-e2e.wQx9D1/resident-inline-audit.json`
- `/tmp/kv9-chaos-resident-cleanup-verification2.json`
- `/tmp/kv9-chaos-resident-outcomes2.json`
- `/tmp/kv9-chaos-resident-evidence-inventory.json`

Earlier 2cb read-group evidence and later a00e async-write inputs remain separate.
This completes the exact resident full-matrix fault increment; it does not by itself
close the remaining proof/composition or broader industrial acceptance work.

The final inventory contains **5,091 files / 1,215,889,500 bytes**, all independently
reopened and hash-checked against their originals; inventory SHA-256
`5b325aac2d4b3fe17dffb3b75ebf7384eb5a969cf0bce7530a0b8d8f6a2574fd`. Readback evidence is
`/tmp/kv9-chaos-resident-evidence-inventory-verification.json`. The committed human
report is separate from the inventory to avoid recursive manifest hashing.
