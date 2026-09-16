# Scoped-client validation packet

Local checkpoint: 2026-09-16. See [behavior, proofs, actual faults and limits](../ROUTED-CLIENT.md).

- 895 passing workspace tests/doctests, 28 existing ignored; strict Clippy and formatting pass. Seven current routing tests cover bounded discovery, scope checks and write uncertainty.
- 15 new Lean theorems, 11 semantic controls and two proof-policy controls; preparation, activation, control and range proofs rechecked.
- Actual single-host Chaos Mesh: four fault windows, two persistent clients, 48 logical calls (44 successes, one UnknownWrite, three lookup failures), complete point-history/readback checks and three verified stopped-store archives. Full harness and separate final reader exit 0.
- 38 offline reader/parser/archive/lifecycle controls pass. Prior failures, exact source snapshots and owned cleanup receipts are retained. Only `chaos-fourth` is the complete accepted campaign.
- No complete D02, general concurrent linearizability, all-voter/batch/I/O Chaos matrix, split/migration, new QPS, measured independent-host scaling or hosted CI claim.

[evidence.tar.gz](evidence.tar.gz) contains **3860 readback-verified files**, 1869378 compressed bytes and 21391348 decoded bytes. SHA-256: `2098a838eb24688d128573d2e9e753124b686084b6ea07a95cc146585580eba7`. [validation.json](validation.json) inventories every file and identifies the accepted checks. Paths inside retained receipts name the original local workspace; packet members use portable `evidence/` and `source/` prefixes.

The tested source is based on `cfb5561235067789bfc7db7c7f8e635a4c7add0d` with exact source/binary hashes retained. Default-feature stripped debug binaries and the local image are not embedded; this is a correctness qualification, not a release performance sample. Stopped synthetic fixture stores and their original checksums are included. Authentication tokens and kubeconfig are excluded. Full local artifacts remain under `/mnt/data/kv9-work/routed-client-20260916`.

The first run failed on an uncreated tenant; the second failed stale-process sampling during restart; the third failed an outdated reader tenant constant after completing its fault campaign. None of these original wrapper exits is changed to success. The fourth completes faults, history checks, archive verification and exact cleanup, while preserving all eight historical namespaces and their fault identities.

A separate extraction rechecked every member and ran the embedded independent
reader against the extracted campaign, including its final cleanup records;
[portable-readback.json](portable-readback.json) records the exit-0 result.
