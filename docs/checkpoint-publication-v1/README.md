# Checkpoint publication evidence v1

Actual local validation on 2026-09-15, based on `61180ff` plus the source pinned
in this packet. [Implementation, proof premises and next steps](../CHECKPOINT-PUBLICATION.md).

- [evidence.tar.gz](evidence.tar.gz): **3,280 regular members**,
  **79,173,576 decoded bytes**, **2,308,764 gzip bytes**.
- SHA-256: `17f689cbf98fa73c1e37f2d4de8e0e32a565403cf17a699b060fb2a0720a36ce`.
- [inventory.json](inventory.json) lists each original path, size and SHA-256.
  Every member was decoded after archive close and compared byte-for-byte with
  its retained original. Known fixture credentials are absent.
- Packet creation and full original readback: **75505 / 69d5b2 / exit 0**.

## Accepted evidence

| Gate | Actual result | Archive location |
| --- | --- | --- |
| Libraries, Clippy, format | 137 engine / 262 Raft / 207 server library tests pass; two ignores reported separately; all commands exit 0 | `runs/kv9-c04-checkpoint-publication-development-20260915-fourth` |
| Strict formal runner | 9 theorems / 36 obligations; two positive models, five actual counterexamples, three proof rejections | `runs/kv9-c04-checkpoint-publication-protocol-20260915-second` |
| Compiled source controls | Five baseline tests pass; three separately compiled faults fail their exact selected test with exit 101 | `runs/kv9-c04-checkpoint-publication-source-controls-20260915-second` |
| Actual MinIO component | Explicit ignored test passes: winner acceptance and committed-loser refusal in both legacy and segmented layouts | `runs/kv9-c04-checkpoint-publication-minio-20260915-second` |
| Three-node process recovery | Physical reclamation after 272 filler writes, leader failover, full restart and checkpoint plus durable tail; all three nodes show publication logs | `runs/kv9-c04-checkpoint-publication-process-20260915-first`, `process-fixture` |
| Actual Chaos Mesh | **17738 / fd294e / exit 0**; 296 successful serial operations, 280 filler writes, actual container-kill, remote checkpoint restart and owned cleanup | `chaos` |
| Independent Chaos audit | **204b50 / exit 0**; complete serial history replay, actual fault/container evidence, selected SST hashes and all-voter fresh-write application | `publication/chaos-independent-audit.json` and `publication/audit-chaos.py` |

The positive TLC instances finish with **37 generated / 33 distinct** and
**25 / 22** states, with empty queues. The deductive proof is parameterized by
an arbitrary positive finite prefix; these small models are additional checks.
The source-control baseline overlaps the library suite. Neither the counts nor
the isolated test fixture are a complete Rust refinement or snapshot proof.

The actual Chaos fault is UID `798fda12-73d4-470c-acfb-79b56f18caf5`.
Voter 2 exits 137 and returns in a new container with the same Pod, PVC and store.
Its recovery reports generation **210**, image cut **499**, publication **500**;
the fresh write is acknowledged at **501** and applied on all three voters.
The history contains one keyspace creation, 285 puts, two deletes and eight reads,
including a real same-key overwrite and four post-recovery reads. All operations
succeed on their first attempt. This is serial history coverage, not a general
concurrent linearizability campaign.

The owned namespace UID `f7c23c6d-51a2-4dad-af8f-1ce20c1dd6e5` is removed.
All eight protected namespaces and the older fault retain their identities.
Maximum measured filesystem decrease is **978,882,560 bytes**, within the
separate 1 GiB cell budget and above its 8 GiB free-space floor. Original
preparation capacity is preserved across retries.

## Original failures and scope

Original Rust, SANY, Python, MinIO and dependency-preparation failures are listed
in `publication/development-failures.json`. The three Chaos setup failures remain
under `chaos-prior`: invalid Python file-open arguments, provisioner termination
grace/deletion mismatch, and a missing image CA trust store. None ran client
workload or injected a fault. Failed startup stores were captured before their
owned namespaces were removed; the packet retains archive hashes/inventories,
while original stopped-store payloads remain local.

The successful image adds a retained CA bundle to the exact tested binary image.
The CA's hash/path and image lineage are retained; its bytes are not published.
The final published Chaos helper changes only the selected-SST inventory
inclusion in the evidence selector. Both executed and final copies, their
one-line diff and the original metadata-selector assertion failure are retained.
The first packet attempt also stopped before archive creation because its secret
scanner mistook nonsecret MinIO settings for credentials. The corrected scanner
checks actual access/secret fields and every Chaos fixture secret.

The packet excludes credentials, CA/image inputs, executables, Cargo targets,
proof caches, TLC state files and bulk stopped-PVC payloads. Exact selected remote
SST bytes and their hashes are included. Isolated MinIO data is memory-backed:
these runs prove actual S3/process recovery behavior, not object-store power-loss
durability. This single-host Chaos cell does not replace full21, cross-host,
concurrent-history or performance acceptance.

C04 remains open. Complete portable root/range/destination anchors, durable owner
ledger and snapshot installation remain next work. No hosted CI ran, no original
industrial stage closes, and the selected CRC write baseline is unchanged.
