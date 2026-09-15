# Initial recovery anchor evidence v1

Actual local validation on 2026-09-15, based on `1f3e450` plus the exact source
and binary inputs retained here. [Implementation, format, proof premises and
remaining work](../RECOVERY-ANCHOR-ENVELOPE.md).

| Packet | Members | Decoded bytes | Gzip bytes | SHA-256 |
| --- | ---: | ---: | ---: | --- |
| [Evidence](evidence.tar.gz), [inventory](inventory.json) | 2,996 | 76,171,467 | 1,729,758 | `a309c291e576dffb51e68f5d7202d5bf753b7c75d006532e68fd5795c9aed284` |
| [Process protocol history](protocol-history.tar.gz), [inventory](protocol-history-inventory.json) | 4 | 50,368,468 | 203,083 | `6a2083bb77363c0ad5ad0d33bd66f4a7d6dee2a5332d8fa7a398d065e4cad8fa` |

Both packets were completely decoded after close and matched to every retained
original byte-for-byte. Known MinIO and Chaos fixture credentials are absent.
Creation/readback terminals: **693f47 / 0** and **d87ab3 / 0**. The second packet
adds all three actual process Raft logs and its creation script; the first
packet's explicit process selector covered metadata and topology, so those
protocol logs are supplied separately rather than implicitly claimed present.

## Accepted gates

| Gate | Actual terminal / result | Evidence location |
| --- | --- | --- |
| Libraries, privacy and Clippy | **80744 / a56a5f / 0**; common 44, engine 138, metadata 32, Raft 263, server 207: **684 passed**, four existing ignored fixture tests; private-constructor compile-fail passes; all-target warnings-denied Clippy passes | `runs/kv9-c04-anchor-envelope-development-20260915-first` |
| Default binary | **62665 / 5be3b8 / 0**; freeze **bc5c53 / 0** | Development and runtime-input directories |
| Anchor protocol | **99613 / 96a61e / 0**; **7 declarations / 26 strict obligations**, five TLC configurations (45, 30, 36, 36, 28 states), seven model counterexamples, four proof rejections | `runs/kv9-c04-anchor-binding-protocol-20260915-first` |
| Affected composition | **70232 / e7efc4 / 0**; configuration 11 / 95, base 7 / 27, publication 9 / 36, each with its existing positive models and negative controls on current pins | `runs/kv9-c04-anchor-composition-proofs-20260915-first` |
| Actual Chaos Mesh | **68008 / 162975 / 0**; new-binary container kill and checkpoint recovery | `chaos` |
| Independent Chaos audit | **bf0ca2 / 0**; complete serial history, actual injected/container identities, lifecycle command outputs, PVC identity, SST bytes and fresh all-voter progress | `publication/audit-chaos.py`, `publication/chaos-independent-audit.json` |
| Three-voter MinIO/process recovery | **29359 / 644343 / 0**; reclamation, leader loss/failover, complete restarts, live tail, overwrite/delete and fresh writes | `process`, `process-fixture` |
| Independent process audit | **90b8d9 / 0**; actual terminal/resources and all recovery logs; shared image identity agrees across observed replicas/restarts | `publication/audit-process.py`, `publication/process-independent-audit.json` |

The exact default-feature binary is 45,846,960 bytes, SHA-256
`19a2cd9bdf352c9ae8c11dd88c11eeff23e76ee0d303c2315e8a2e2b8e53a0b6`.
The current source/build inventory and all four proof source maps were checked
along with formatting and `git diff --check`: **7ba753 / 0**. Executable and
container payloads stay local. Source pins are correspondence inputs, not an
automatic proof of Rust refinement.

Chaos runs **296 successful first-attempt serial operations**: one keyspace
creation, 285 puts (including 280 segment-rotation filler writes), two deletes
and eight reads. Voter 2 exits 137 and recovers generation 211, cut 500,
publication 501 with anchor
`80c90d535ffaf74ca89562d96de1301b9379331abcf470d59824fc4aba29a41b`.
The Pod, PVC and durable store identity remain unchanged; fresh writes reach all
three voters. Scoped cleanup removes the owned namespace and preserves eight
protected namespaces and the older fault. Anchor-suffix audit **2ee302 / 0** and
selected-byte readback **82bd0e / 0** are also retained.

The cell's original available-space baseline is **9,673,428,992 bytes**, only
1,363,968 bytes above its unchanged launch threshold. Maximum measured decrease
is **474,476,544 bytes**, within the separate 1 GiB budget; minimum available is
**9,198,952,448 bytes**, above the 8 GiB floor. No preparation retry or relaxed
guard was needed. This is a completed bounded gate, not reserved future capacity.

The distinct process gate observes peak local allocation **101,101,568 bytes**,
minimum available **9,207,762,944 bytes**, and confirms every owned process exited.
Its three voters log **2, 3 and 2** checkpoint recoveries. Voters 1 and 3 share
generation 74/cut 354/publication 356 and anchor
`97395a7926516e7544db9013b646ea680074d6a051c1aa5dfba0adfa57ab31c9`;
the same selected checkpoint keeps that identity across observed restarts.
The original process anchor audit/readback is **32399c / 0**.

## Preserved failures and limits

Four initial proof attempts leave the encoded-size obligation unproved; the
remaining protocol obligations pass. A separate arithmetic diagnostic preserves
the generated SMT input and the terminal with child exit 10. The corrected
formula groups the same 232 fixed header bytes, reducing deeply nested untyped
arithmetic. Its bound remains 1,147,128 bytes. All seven theorem statements remain,
and the runner rejects the deliberately smaller false bound. No proof cache,
admitted theorem or custom axiom supplies acceptance.

This packet excludes credentials, executable/image/CA bytes, Cargo/proof caches,
TLC state stores and bulk stopped Chaos PVC archives. Selected SST bytes and
the full public serial history are included. Actual S3 uses a memory-backed
object store: these are process-failure recovery tests, not power-loss durability,
general concurrent linearizability or the full21 Chaos matrix.

C04 remains open. Transferable retained-history authority, ownership transitions,
the replicated outer retention ledger and destination installation remain next.
No original industrial checkbox closes, no write candidate is promoted, no new
performance result is claimed, and no hosted CI is dispatched.
