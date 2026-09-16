# Protocol snapshot validation packet

See [implementation, record semantics, proofs and remaining scope](../PROTOCOL-SNAPSHOT.md).

- 904 passing workspace tests/doctests, 28 existing ignored; nine current focused tests and strict Clippy/formatting pass. The workspace run precedes a test-only initializer cleanup; production bytes are unchanged afterward.
- 180 actual storage I/O fault/crash schedules, every torn-frame byte boundary, real-file recovery and three rejected Rust defects.
- Eleven new Lean theorem statements, nine semantic controls and two proof-policy controls; preparation and fixed-voter activation proofs rechecked.
- One actual current-binary Chaos Mesh campaign: four routing/failover fault windows, two persistent clients, 46 calls (43 successes, one UnknownWrite, two lookup failures). Complete harness and independent reader exit 0; all submitted mutations read back, three stopped-store archives checked, exact namespace cleanup and historical resources preserved.
- Remote snapshot installation is disabled. No learner attachment, physical Raft log reclamation, complete S05/D03, general concurrent linearizability, split/migration, new QPS, measured scaling or hosted CI claim.

[evidence.tar.gz](evidence.tar.gz) contains **1305 readback-verified files**, 1202326 compressed bytes and 12147318 decoded bytes. SHA-256: `486c589694497baddb5c62d0fb4f173e5d07028e1c8fb906ac2d8d28f2ed2bd4`. [validation.json](validation.json) inventories all members. A separate extraction checked every member and reran the embedded final Chaos reader; [portable-readback.json](portable-readback.json) records that result.

Tested source is based on `1feb64ff20d54c5be50d3d2da970993112ab5014` with the exact current Rust/Cargo/proto source, binary and image identities retained. Default-feature stripped debug binaries remain local, excluded from this packet. Authentication credentials, kubeconfig and GitHub issue snapshots are excluded. Three small stopped synthetic-store tar archives are retained for full-member verification. Original absolute paths record execution provenance; archive paths use `evidence/` and `source/` prefixes.

The first test compile failed on direct RaftState equality, the first proof draft needed a stronger pre-publication invariant, and initial Clippy flagged a fixture initializer. Two Chaos build wrappers refused before compilation: one treated dev-inclusive metadata as production feature selection, and one omitted the workload package. Those failures remain unchanged. The successful third build checks actual first-party compiler-artifact features and unchanged inputs before/after building. Only one runtime fault campaign was launched. Its immutable pre-cleanup result is preserved alongside the accepted final cleanup and reader results.

The reused Chaos harness/checker is byte-identical to the prior scoped-client checkpoint. Its existing offline controls were not rerun and are not counted as new evidence. This campaign checks normal routing under the new snapshot guards, not actual installation or independent physical failure domains.
