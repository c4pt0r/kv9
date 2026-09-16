# Joint engine/protocol installation validation packet

See [implementation, storage semantics, proofs and remaining scope](../JOINT-SNAPSHOT-INSTALL.md).

- 907 passing workspace tests/doctests in the serial qualifying run, 33 ignored,
  strict Clippy/formatting pass. A post-campaign takeover recheck reproduced the
  identical 907/0/33 result. Default-parallel and two-thread runs hit six
  pre-existing runtime leader/timing and formation-planning flakes, retained in
  their logs; the serial run is the qualifying evidence.
- Three local unit tests plus five real-MinIO focused tests: install cuts across
  ten post-operation states, explicit old-selector power-loss emulations,
  missing/corrupt selected files, foreign restored images, term/vote/cut/
  membership/scope refusals, missing or corrupt real SSTs, and eight retained
  partial attempts before an explicit budget refusal with idempotent retries.
  Three isolated Rust defect controls fail at their intended assertions.
- 13 checked Lean theorem statements for the installation ordering/recovery
  model, ten semantic mutation controls and two proof-policy controls. The
  protocol-snapshot, group-preparation, group-activation and data-range models
  are rechecked against the exact working tree.
- One accepted Chaos Mesh campaign (`chaos-fourth`): seven bounded non-serving
  install cells on one persistent destination Pod/PVC, six actual container-kill
  faults — one at each pause point (after-engine, after-protocol,
  before-selector, after-selector-rename, after-selector-sync and a repeated
  before-selector) — each followed by exact whole-pair old-or-new selection,
  identical-destination checks and object readback. Historical namespaces and
  faults are preserved; exact scoped cleanup passes. The independent reader
  exits 0. Three failed launches are retained with their outputs: a containerd
  image-name normalization refusal, a transient pid-0 restart race and a kubelet
  crash-backoff timeout after repeated kills. All three repairs are
  fixture-only; production bytes, the probe ELF and the campaign policy are
  unchanged.

[evidence.tar.gz](evidence.tar.gz) contains **7957 readback-verified files**,
2564619 compressed bytes and 33750418 decoded bytes. SHA-256:
`9b5b8848bf3ae9eb1650d3678607c2d12803a33fbb5bb82acc557a741dab5743`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and reran the embedded independent chaos
auditor; [portable-readback.json](portable-readback.json) records that result.

Tested source is based on `5417f8c808e8867159cd7c9eb7e4e786c1e5bbea` with the
exact current Rust/Cargo/proto source retained under `source/`. Debug/probe ELF
binaries, MinIO private credentials, `.minio.sys` metadata, kubeconfig material
and GitHub issue snapshots are excluded. Original absolute paths record
execution provenance; archive paths use `evidence/` and `source/` prefixes.

Nothing here authorizes a remote sender, source pin release, retention-owner
transfer, runtime peer startup from an installed generation, learner catchup or
promotion, physical protocol-log reclamation, split/migration or any
horizontal-scaling claim. Process kills cannot establish hardware power-loss
behavior, quorum availability or concurrent linearizability. No new QPS or
measured scaling exists; the independent 3/6/9-host benchmark contract in
[HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion. No hosted CI was dispatched.
