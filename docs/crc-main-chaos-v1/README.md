# Main CRC Chaos Mesh evidence

The default release of `bd42e60f84657e22e36e34924a5c80a08eac623a` passes the
actual 21-window Chaos Mesh matrix and independent complete-history acceptance.
The three histories contain 11,316 operations: 10,683 OK, 602 unknown and
31 refused. Unknown writes remain unknown. Four final replica lifetimes publish
fresh drained states; all 31 observed server lifetimes exit. The run's namespace
and fault objects are removed after full archive readback; all eight historical
namespace UIDs are preserved.

The matrix covers seed unavailability, each voter's sustained pod failure,
leader partition, overload, delay, six actual EIO/ENOSPC injections, missing-log
refusal and original-store recovery for each voter, replacement-PVC refusal
and original-PVC recovery for each voter, and retained-PVC endpoint migration.
This is local correctness evidence. It adds no throughput result and does not
establish dedicated client-link/quorum-loss, cross-host or power-loss acceptance.

`summary.json` binds the exact source, binaries, image, complete histories,
original archive and final cleanup. The independent audit's earlier
`cleanup_complete=false` remains intact; subsequent cleanup receipts supplement
it. The original runtime namespace retains the adapter's `vectored` prefix;
source and executable hashes bind this CRC integration, with no vectored change.

The full local archive was read back before cleanup: 4,208 regular files,
1,062,174,968 decoded bytes, 100,471,419 compressed bytes. Its SHA256 is
`1b2abda397dd8d6505894d0e2f1125bd936b9b6879379e37e3bceedc60fa5935`.

This portable selection retains complete histories, faults, observers,
source/build bindings, exact pressure-feature controls, independent acceptance,
archive catalogs and cleanup records: 2,291 files, 499,798,357 decoded bytes,
25,140,016 compressed bytes in 12 parts. Compiled production binaries, WAL
payloads and duplicate archive/copy payloads remain local; their original
catalogs and hashes remain published. Literal symlinks are JSON metadata.

Run `python3 -B verify.py` in this directory to verify all selected file and
part bytes without extracting. This checks publication integrity, not a fresh
live test or the residency of excluded payloads. The original inventory SHA256
is `49e7c43036b233f8a597e7354dd398d99e9a35d8c6366ca748f7b52c83418882`.

Actual runtime session 94647 exits 0 (`b9c075`); post session 85751 exits 0
(`4ad876`) after all six phases. Selection exits 0 (`f42fe3`), package session
36897 exits 0 (`eb1349`), and independent published-part verification exits 0
(`f27f49`). Publication receipts and a pre-execution newline correction are
retained beside the original inventory. No workload was rerun for reporting.

The preceding [source/proof/recovery evidence](../crc-main-integration-v1/README.md)
retains the separate 789-test, 47-statement and 353-operation populations.
All work ran locally; no hosted CI was dispatched.
