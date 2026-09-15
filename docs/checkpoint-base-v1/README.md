# Historical checkpoint identity evidence v1

Actual local validation on 2026-09-15, based on `007338a` plus the source pinned
in this packet. [Implementation, proof premises and remaining work](../CHECKPOINT-BASE-IDENTITY.md).

- [evidence.tar.gz](evidence.tar.gz): **2,840 regular members**,
  **126,288,171 decoded bytes**, **1,910,060 gzip bytes**.
- SHA-256: `fc7af84ed529be38ec65d1ac52e414f11888cfd8182ccca6e42f77818b8b8360`.
- [inventory.json](inventory.json) lists every member's original path, size and
  SHA-256. Complete archive decoding matched each retained original byte-for-byte
  after close. Known MinIO and Chaos fixture secret values are absent.
- Creation and original readback: **59812 / 64b7ad / exit 0**.

## Accepted results

| Gate | Actual result | Packet location |
| --- | --- | --- |
| Local libraries | Engine 138, metadata 29, Raft 262, server 207 pass; four ignored fixture tests reported separately; **64097 / 2a8052 / 0** | `runs/kv9-c04-checkpoint-base-development-20260915-first` |
| Real MinIO components | Two applicable ignored tests explicitly run and pass against actual S3; both WAL layouts; correct old epoch accepted and wrong historical root refused before tail repair | Same development directory, `engine-minio` and `metadata-minio` logs/terminals |
| Clippy and default binary | All-target Clippy with warnings denied passes; corrected default-feature build **51747 / 73d8f1 / 0**; formatting passes | Development logs and runtime inputs |
| Base protocol | Seven statements / 27 strict obligations; three positive TLC models (92, 59 and 82 distinct states), five counterexamples and three proof rejections; **21195 / dbbb93 / 0** | `runs/kv9-c04-checkpoint-base-protocol-20260915-first` |
| Publication composition | Existing nine statements / 36 obligations, two models, five counterexamples and three proof rejections rechecked against changed sources; **7757 / 33da8b / 0** | `runs/kv9-c04-checkpoint-base-publication-protocol-20260915-first` |
| Actual Chaos Mesh | New-binary container-kill and checkpoint recovery, **34615 / b84937 / 0** | `chaos` |
| Independent Chaos audit | Every serial operation, both lifecycle command outputs, actual injected fault/container identities, selected SST bytes and all-voter fresh progress; **28c93d / 0** | `publication/audit-chaos.py`, `publication/chaos-independent-audit.json` |
| Three-voter process gate | Reclamation, leader loss/failover, complete restart, checkpoint plus durable tail, overwrites/deletes/fresh writes; **69955 / 4a6bb4 / 0** | `runs/kv9-c04-checkpoint-base-process-20260915-first`, `process-fixture` |
| Process log audit | All three voters show at least two successful checkpoint recovery logs with distinct image/publication cuts; resource and actual terminal checks; **151d08 / 0** | `publication/process-independent-audit.json` |

The exact default-feature binary SHA-256 is
`70a61b09c72d0e3e79717ee7f84f7d87c7c77ad6b347013aa8678ad656ce3966`.
The 165-file source/build inventory and both proof source maps still match at
final readback **596159 / 0**. Executables remain local; source hashes are input
bindings, not an automatic proof of Rust refinement.

The new Chaos history contains **296 successful first-attempt serial operations**:
one keyspace creation, 285 puts (280 filler writes), two deletes and eight reads.
Voter 3 exits 137 and restarts with the same Pod/PVC/store. Checkpoint generation
210 has image cut 499 and winning publication 500; the fresh write at 501 is
applied on every voter. One exact selected SST is retained in the packet.
Fault UID: `180e99b5-28b3-481e-9037-70490ab5aadd`. Owned namespace UID:
`18423a94-9b61-4ea6-b674-54f029d203d0`; scoped cleanup removes it and preserves all
eight protected namespaces and the older fault.

Maximum filesystem decrease is **431,124,480 bytes**, within the unchanged
1 GiB cell budget; minimum available space is **9,260,679,168 bytes**, above the
8 GiB floor. Image preparation retries preserve the original baseline. The
separate process gate peaks at **95,707,136 bytes** of local allocation and
confirms all owned processes exited.

## Failures and scope

The packet retains the initial test compile refusal for constructing a MinIO-only
uploader with a memory store, the incorrect package name in the first binary build,
and the first proof's two unproved obligations. The corrected proof uses explicit
Boolean initialization and a stronger reachable-phase invariant; all seven
statements remain. Rejected controls are additional evidence, not successful
database workload counts.

Two Chaos preparation failures precede all fixture/workload actions: the source
pin map used a different field name, then Docker interpreted a local image ID as a
registry name. The final helper requires an exact nonempty source map, validates
the binary size/hash, and verifies a retained image tag against its pinned ID.
The failed-image continuation carries the original baseline and exact binary/input
binding. Both failed selectors and sources are retained under `chaos-prior`.
The final published helper exactly matches the executed source:
`7806cd034bd80b8beba78742c35fbd5f7fb6b955c41a08697ccfd9a3abc3139f`.

The packet includes the three process fixture Raft logs, selected process
identity/topology files, exact selected Chaos SST bytes, source, proof and command
records. It excludes credentials, executable/image/CA payloads, Cargo/proof caches,
TLC state stores and bulk stopped Chaos PVC archives. Original runtime and image
inputs remain retained locally. Actual S3 uses a memory-backed object store;
these are process-recovery tests, not object-store power-loss durability.
The serial history is not general concurrent linearizability or full21 acceptance.

C04, complete portable anchors, destination admission, ownership transition chains,
and the replicated retention ledger remain open. No new performance comparison,
hosted CI, original industrial checkbox closure or CRC candidate promotion occurs.
