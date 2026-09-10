# Async write Chaos Mesh acceptance

Exact `23bc58b6d9e15bc46764a30ba4cd3390c36dd281` passed the local 21-window Chaos Mesh matrix, independent complete-history/effect checks, status-writer identity checks, async-apply bounds and final drain. This revision is `a00e39f` plus additive process boot/start identity in exported status. The owned fixture adds one bounded delay observation gate; runtime request/consensus behavior and existing workloads/checkers are unchanged.

This closes this scoped fault-validation increment. It does not establish broad proof composition, P0 completion, performance, cross-host behavior or power-loss acceptance.

## Exact inputs and execution

- Clean private source: `/tmp/kv9-chaos-async-write-status-acceptance`, 416 tracked file hashes verified before and after execution.
- Private Cargo target: `/tmp/kv9-chaos-async-write-status-target`.
- Default-debug server SHA-256: `917ed5ba7e1b3902f3b445adaa38865673acdfc42b172779a146f2218f5c6f39`.
- Same-source persistent correctness client: `765f3d9cead69e10a54285d41735be097d50e3c578a08cbb3b3b86b4129ec289`. This is separate from the retained benchmark client.
- Pressure executable: `e5454cd6134b8defeb23429934cc166707fe4b0f5a57c964c3eed3aec89b9ef1`.
- Prepared and executed Docker image: `sha256:43045bce1d88de3d3d01fad5a1d7e962a1111b25960680ade22f9a594332e3a2`.
- Observed Kind image identity: `docker.io/library/import-2026-09-10@sha256:e5f89d6e45a2ac7772c13470627225963f178f9eeaf94f217af1f84f3bf780db`.

Two complete standalone server → pressure example → correctness client → image build sequences, using the fixture's `umask 077`, preserved every executable and image hash. Retained Cargo JSON explicitly shows empty feature lists for the runtime's root, engine, Raft and server packages. The separate pressure example intentionally uses testing dev-dependencies; its build does not replace the server.

Execution used only `/tmp/kv9-p0-ci-chaos.kubeconfig`, Kind executable `/tmp/kv9-p0-tools/kind-linux-amd64`, and owned cluster `kv9-chaos-ci-p0-20260908`. Host builds/harness ran on CPUs 6–31. Pods were observed on CPUs 0–31 on one physical host, with concurrent proof/background work. No hosted CI ran.

## Faults and complete public histories

The original matrix covered all-voter preformation crashes, registration-seed blackholing, container/Pod replacement, each voter's sustained Pod failure, leader isolation, admission pressure, follower latency, each voter's I/O errors 5 and 28, repeated missing-log starts, independently prepared replacement volumes, retained-volume endpoint migration and collector removal. All 21 required windows contain successful public writes and reads from both unchanged clients. Physical effects and recovery were independently rechecked; resource creation alone was insufficient.

| History | Complete operations | Successful | Unknown | Refused |
| --- | ---: | ---: | ---: | ---: |
| CLI | 5,134 | 4,628 | 496 | 10 |
| Persistent client | 1,318 | 1,303 | 13 | 2 |

Both original and independent CLI searches visited 6,858 states. The persistent checker validated the complete history across all windows. All six independent effect checkers passed, including their existing rejection controls.

CLI unknowns comprise 165 create-keyspace, 53 delete, 39 delete-range, 118 GET, 72 PUT and 49 scan outcomes. Persistent unknowns comprise 2 deletes, 6 GETs and 5 PUTs. Unknown mutations remain uncertain, were not treated as definite failures, and were not retried as successful writes. Complete per-operation/phase/refusal records remain in the raw histories and outcome audit.

- CLI history: 2,904,595 bytes, SHA-256 `3d55eb3deded3fb21725a601633ffc79e2a95a170e830fc73d5fe3a4aa71003f`.
- Persistent history: 967,352 bytes, SHA-256 `d7dd384beed61ea4c9f90c51ee6837fc5948099d4734e542c57f79cdbdbe87e2`.

## Writer identity, fresh delay observations and drain

Every accepted status sample binds both status snapshots' boot ID and process start ticks to the actual live `/proc/PID/stat`, boot ID and executable hash. Pod UID/container identity must remain stable and running. Missing or unavailable writer identity is not attested; a persisted PID 1 record cannot identify a new PID 1 lifetime.

The observer retained 411 batches and 1,420 attempted samples. The independent audit accepted 1,268 writer-bound samples across 32 observed lifetimes, including 462 wrapped-child samples; 152 transient/unattested samples remain visible. This is sampled coverage, not a claim that every short-lived failed startup was observed. No accepted sample violated status bounds. Async-apply occupancy and peak were bounded by 128, observed peak was at most 6, and peak never decreased within an attested lifetime. All four final replicas had empty public/read/apply ledgers and stopped=false.

The owned delay gate has one global 20-second bound. Before choosing its first qualifying endpoint, it requires two separately observed server export-success advances under the same active fault and boot/start lifetime. It then requires two serial client progress advances, a complete successful GET, a second exact-lifetime capture and positive inline growth. The original physical delay probes and minimum hold remain; timeout or missing evidence fails.

The gate completed in 5.794 seconds. Independent raw-probe and final wall-anchor checks confirmed persistent GET 528 was contained between fresh captures, with voter 2 inline count increasing by 8. Physical TCP delay was 0 ms before injection, 152 ms after injection and 241 ms after history/gate progress. The separate observer found 13 positive inline phase envelopes, including required partition, delay and recovered endpoint migration. Inline growth is envelope-scoped; it is not attributed to a particular response.

The strengthened probe passed 12 preflight cases. The final gate passed 13 original controls plus 7 added stale-export/freshness controls. Earlier gate versions remain retained; no monotonicity or effect criterion was waived.

## Preserved failed attempts

1. Initial preparation selected a package that did not contain the pressure example. Its exact Cargo failure remains retained. A subsequent combined workspace server/example build enabled transitive engine/Raft testing features despite root features `[]`. The first actual `a00e` launch rebuilt a different standalone server and was terminated as identity-invalid. Raw evidence: `/tmp/kv9-chaos-e2e.0x5iOl`.
2. The corrected exact-`a00e` matrix passed original and independent histories/effects, but its added observer failed. A new PID 1 read the preceding lifetime's persisted PID-only status, producing an apparent peak decrease. Its delay phase also lacked two qualifying observer boundaries. These failures remain unchanged at `/tmp/kv9-chaos-e2e.5qdh24`; that run is not relabeled as `23bc58b` acceptance.
3. On the accepted run, two owned independent-audit adapters initially failed: one treated the history header as an operation; another referenced the preceding attempt's log filename. Both original scripts/logs were preserved. Only those parser/path defects were corrected; the completed fixture was not rerun or modified.

A separate retained review confirmed the older resident server's standalone Cargo dependency graph and executable hashes were unaffected by the combined-build mistake.

## Evidence and cleanup

- Accepted raw run: `/tmp/kv9-chaos-e2e.1ECL6E`.
- Curated combined record: `/tmp/kv9-chaos-e2e.1ECL6E/acceptance-record.json`.
- Fresh independent copy: `/tmp/kv9-chaos-async-write-status-independent2`.
- History/effect records: `/tmp/kv9-chaos-async-write-status-final-audit2.json` and `/tmp/kv9-chaos-async-write-status-independent2-effects.json`.
- Freshness argument: `/tmp/kv9-delay-observation-gate-freshness.md`; exact fixture diff and input hashes: `/tmp/kv9-chaos-status-observation-gated-fixture`.
- Inventory: `/tmp/kv9-chaos-async-write-evidence-inventory.json`, SHA-256 `c4ee4c87f2b6b2406cfccd8010283ed87eac3878ee45bb19ef4785987feef652`.

All 6,443 inventoried files (3,441,059,597 bytes) were reopened and hash-checked; 227 owned helper symlinks were rechecked. The verification sibling is `/tmp/kv9-chaos-async-write-evidence-inventory-verification.json`. Original raw data, failed preparations, build/feature records, executables, source inputs, controls and audits remain available; this human report is excluded to avoid recursive hashing.

Fixture and observer exited 0. Namespace `kv9-chaos-1789056589-1159591` was removed, matrix PID 1159591 and observer PID 1160020 exited, and all eight preexisting namespace UIDs were preserved. The cleanup record is `/tmp/kv9-chaos-async-write-status-cleanup-verification2.json`.
