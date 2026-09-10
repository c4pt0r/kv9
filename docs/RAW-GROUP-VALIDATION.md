# Raw engine group-commit fault validation

This report accepts the local fault-test increment, not the complete runtime candidate. Parameterized group-composition and implementation-refinement gates remain open. Main runtime is unchanged.

Revision: `fe650ed814757e8172fb03eced102894e35d4a4d`.
Source SHA-256: `1e77891287e905e805fa04e4455b7f9896e16c30d6bf7067e2cb8961bb6b2de5`.

The first actual MinIO run and first complete 21-window actual Chaos Mesh run passed. Both complete histories and six additional fault-effect validators passed again on an independent copy. No source, runtime, harness or shared validator changed, and no hosted workflow or GitHub mutation occurred. This is new exact-fe evidence; no b3 fault result is relabeled.

## Exact identity and environment

- Clean source: `/tmp/kv9-chaos-fe-acceptance`; private target: `/tmp/kv9-chaos-fe-target`.
- Default production debug binary SHA-256: `3fb57e1618ba8b79bd05045ec0bf69836cd69d426c4a12d31e99687e1e4840bf`.
- Workload SHA-256: `2421a0ce841cf92a44383a1b4041adbae2e00bf1c8a60ff172eff2bb8e4653bf`.
- Admission helper SHA-256: `0991e6d3f74ae272083a3ba2e13c92f9a1000653b3496b17938fb4cec1582fee`.
- Image: `kv9-chaos:fe650ed-20260909-attempt1`; ID `sha256:ad7bd9d614bf2a5469aca4d0dc44f11a127d1393856103fab9612421cae9379d`.
- All three actual voter pods independently returned the expected binary hash and segmented WAL topology header.
- Only `KUBECONFIG=/tmp/kv9-p0-ci-chaos.kubeconfig`, `KIND=/tmp/kv9-p0-tools/kind-linux-amd64`, and owned cluster `kv9-chaos-ci-p0-20260908` were used.
- One Kind control-plane node on one shared physical host. Shell/Cargo affinity was CPUs 6-31; actual pods reported CPUs 0-31. This does not establish independent host failure domains or replace group-commit proof/new source-cut gates.

## Actual MinIO

Log: `/tmp/kv9-fe-minio-process-attempt1.log`; raw: `/tmp/kv9-minio-kv-mtdm06sv`.
Terminal: `COMMAND_EXIT=0 LOG_EXIT=0`.

Three replicas passed failover, remote checkpoint, physical closed-prefix reclamation, full remote restart, live tail and deletes. 272 acknowledged overwrites of a 60 KiB value forced real rotation. The fixture captured one closed prefix file per voter, deleted the filler, checkpointed through index 344, and verified all captured files were unlinked. Final state agreed at term 4/index 351. The pinned actual MinIO image digest and local image metadata are retained in the inventory.

## Actual Chaos Mesh and complete outcomes

Log: `/tmp/kv9-chaos-fe-attempt1.log`; raw: `/tmp/kv9-chaos-e2e.ympLno`.
Terminal: `COMMAND_EXIT=0 LOG_EXIT=0`.

All 21 windows contain both CLI and persistent client progress. Actual effects include all-owner formation failure, first-seed blackhole, container/Pod replacement, sustained failure of each voter, leader partition and fencing, admission saturation, network delay, EIO and ENOSPC reaching Raft on each voter, repeated missing-log startup refusal, independent replacement-PVC refusal and retained-PVC endpoint migration.

| History | Paired operations | Confirmed | Conservative unknown | Proven pre-execution refusal |
|---|---:|---:|---:|---:|
| CLI | 5,110 | 4,607 | 491 | 12 |
| Persistent | 1,269 | 1,254 | 11 | 4 |

Unknown outcomes remain unknown; they are not definite write failures. Persistent unknowns consist of four mutations and seven reads. Full per-operation, phase and reason counts are in `/tmp/kv9-chaos-fe-outcomes.json`; original histories retain all invocation/return records and complete responses/attempt details.

CLI SHA-256: `c35356ce67c2b69d8f040df456b8ab591ad334ed7cfc42a0dd67e6813ff56895`.
Persistent SHA-256: `6c7c81d90e0e0ede7c294568b6404edb6936cd8d9c941fcbb0e970ea3ce9c6b4`.

Original and independent CLI checks both searched 6,972 states and returned valid. The independent persistent check requires the exact fe revision and all 21 fault windows. Formation, log-loss, replacement-PVC, endpoint-migration, latency and admission-effect validators (including their invalid-evidence controls) also passed on the independent copy.

## Retained attempts and prior process evidence

The private build, actual MinIO run, full Chaos matrix, complete-history rechecks and effect rechecks all passed their first attempts. During an extra layout/hash observation, the third voter was intentionally unavailable under sustained Pod failure; eight unsuccessful diagnostic attempts precede its successful ninth observation. All attempts remain in `segmented-layout-observation-attempts.json`. They did not alter the matrix verdict.

Root's earlier three-process run at `/tmp/kv9-raw-group-process-first` remains a source-attested pass at term 2/index 14. Its Cargo transcript selects the shared-target executable from the clean fe worktree; root reports no executable rebuild between build and fixture. No execution-time binary SHA was captured, so this report makes no retrospective hash claim for that prior run. The new private MinIO/Chaos runs provide the stronger executable identity. The prior process data/status/layout evidence was rechecked without rerunning the fixture.

## Cleanup and inventories

The owned namespace `kv9-chaos-1789009866-1686731` was deleted after scene collection; all eight preexisting namespaces remain. Fixture child processes exited. The Kind cluster and images remain available.

- `/tmp/kv9-chaos-fe-inventory.json`: exact source, executable/features, image and cluster identities, full commands and prior process-attestation boundary.
- `/tmp/kv9-chaos-fe-final-audit.json`: independent complete histories and acceptance markers.
- `/tmp/kv9-chaos-fe-independent-effects.json`: six independent effect-check commands and exit statuses.
- `/tmp/kv9-chaos-fe-file-manifest.json`: 1,137 raw Chaos files, 156,657,022 bytes, individually hashed.
- `/tmp/kv9-fe-process-final-audit.json`: original-process and MinIO final-state/layout/reclamation audit.
- `/tmp/kv9-fe-acceptance-build`: retained default debug database/admission executables and build provenance; workload executable/provenance is under the raw Chaos `persistent-build` directory.
- `/tmp/kv9-chaos-fe-cleanup-verification.json`: explicit-context cleanup verification.

## Retained archive

The complete source, binaries, original and independent histories, effect audits and failed diagnostic observations are retained in `target/correctness-evidence/2026-09-09-fe650ed-minio-chaos.tar.gz`: 2,794 entries, 161,993,043 bytes, SHA-256 `303304ec0721f03dd2223345940b6c518c557ba2a29365c93d9cd596189096da`. Every archive entry was independently read back and compared byte-for-byte against its original and manifest hash. Adjacent manifest and verification sidecars preserve the scope and identity limits described above.
