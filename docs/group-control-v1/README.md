# Group-control validation packet

Local checkpoint: 2026-09-16. See [behavior and limits](../GROUP-CONTROL.md).

- 882 passing workspace tests/doctests; 28 existing ignored; strict Clippy and formatting pass.
- Three independent default-feature kv9 processes pass online creation, autonomous activation, leader loss, minority outage, all-process restart and single-group recovery refusal.
- 12 new Lean theorems, nine semantic and two policy controls; prior 15 activation and nine preparation theorems/controls rechecked.
- No new QPS, scaling gain, Chaos Mesh or hosted CI claim.

[evidence.tar.gz](evidence.tar.gz) contains 245 readback-verified members (255895 bytes), including source snapshots, compiler transcripts, local checks and process command/status evidence. SHA-256: `94b395ca7b1e70a06221774d2341578769a186f395c71d1ffe8422d486c3a3f0`. [validation.json](validation.json) lists each member's size/hash and the precise checks. The fixture intentionally uses synthetic credentials and identities.

Evidence was collected from the working tree based on `fb4964e22d7d83817954601c7c622ec55f011c61` before this checkpoint commit. Source hashes identify the tested implementation. Development failures are retained and separated from accepted results; their resolutions are explained in GROUP-CONTROL.md. Full local disk fixtures remain under `/mnt/data/kv9-work/group-control-20260916`; binaries and WAL payloads are not embedded here.
