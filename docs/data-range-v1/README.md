# Initial data-range validation packet

Local checkpoint: 2026-09-16. See [behavior, upgrade requirements and limits](../DATA-RANGE-ROUTING.md).

- 888 passing workspace tests/doctests; 28 existing ignored; strict Clippy and formatting pass.
- Three default-feature production processes pass public multi-group data routing, data/metadata leader loss, minority outage, all-process restart and one-group recovery refusal.
- Actual previous V2 and current V3 binaries pass offline upgrade, both RPC version refusals, legacy data preservation and downgrade refusal.
- 16 new Lean theorems, eleven semantic and two policy controls; prior preparation, activation, control and wire proofs rechecked.
- No new QPS, measured scaling, complete D02, Chaos Mesh or hosted CI claim.

[evidence.tar.gz](evidence.tar.gz) contains 622 readback-verified members (602127 bytes). SHA-256: `e615c76578b4fd6f9e4b9482825196600501989e11e1745694185c608ff4bfb3`. [validation.json](validation.json) lists each member and the exact accepted checks. Synthetic fixture credentials/identities are intentional. Source hashes identify the tested working tree based on `318a4ab3e67cc39a14a743a9dd99856460681375`.

Development failures are retained separately from the named accepted runs. In particular, a failed initial upgrade attempt used an unqualified stale binary; the accepted previous binary was built in its own fresh Cargo target, and the runner now refuses identical old/new binary hashes. Full local fixtures remain under `/mnt/data/kv9-work/range-routing-20260916`; executable and WAL payloads are not embedded.
