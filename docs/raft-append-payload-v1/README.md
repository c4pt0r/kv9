# Bounded Append payload experiment evidence

This bundle supports [the report](../RAFT-APPEND-PAYLOAD-PERFORMANCE.md) for
candidate `74b958a8bcdf25252ab55ba6149876a1cddc0637`. It contains 1,504 original
files / 366,860,935 decoded bytes in nine parts totaling 17,432,807 bytes.
The [readout](READOUT.md) and [per-repeat table](PER-REPEAT.md) are exact aliases
of the original statistics output.

Only the complete second 24-cohort attempt supplies those statistics:
49,675,313 measured calls succeed in one attempt. The first attempt stops on
the unchanged 96-GiB retention preflight guard after 18 completed cohorts.
It is retained as incomplete, including its raw reports and terminal failure;
none of its results are resumed, pooled or accepted as a complete comparison.
Root reclaims only rebuildable Cargo dev cache under the shared lock, verifies
preserved source/release identities, then starts a separate whole attempt
without changing descriptors, binaries, order, guards or acceptance predicates.

The selection includes original source gates, clean release/cache provenance,
complete ordinary recovery histories, six driver and 17 auditor contracts,
smoke terminal records, frozen preparation and derivation code, both timing
attempts' reports/histograms, storage repair, independent audit and root tool
terminal records. Recovery covers 357 calls, 327 OK and 30 unknown. The second
timing session 26382 exits 0 (`2a5775`); audit 95004 exits 0 (`bc3235`);
statistics exits 0 (`524dd8`). CPU settings and namespace identities restore
exactly. No hosted CI runs.

The independent second timing audit retains 636 files / 4,562,369,306 bytes,
a different population from this publication. Original WALs, executables and
bulky host/container observations stay local under their existing inventories.
This bundle cannot rerun full runtime acceptance on its own. Process recovery
is not actual Chaos Mesh, physical durability or a whole-implementation proof.
The candidate remains held, with CRC selected.

Run `python3 verify.py` here to verify bounded archive parts, safe member paths,
original byte hashes and top-level aliases. This checks integrity only; it does
not rerun tests, timing, the independent audit or the statistics.
