# Two-context point-read/mixed screen evidence

See [the decision and results](../READ-WINDOW-SCREEN.md), [pooled tables](READOUT.md)
and [every repetition](PER-REPEAT.md).

`evidence.tar.gz` contains exact selected reporting inputs: all 24 original
client reports, accepted audit and input inventory, frozen preparation/diffs,
smoke/timing/audit receipts, candidate release bindings and the independent
statistical derivation. `inventory.json` binds each original path, byte count
and SHA-256 to a tar member. The archive preserves the inherited generic scope
string in the original audit; actual coverage is 24 point read50/read100 cohorts.

Run `python3 -B verify.py` to check archive membership and every original byte
binding without extracting files. This is an integrity check, not a fresh
runtime audit, proof execution or statistical significance test. The arithmetic
script, unchanged predecessor core and original input hashes are retained for
review. Means use summed nanoseconds/counts; quantiles merge raw buckets. No
percentile is averaged and no repetition is discarded.

Binaries, WAL and most host/resource observations remain in the original local
retention roots. The compact bundle cannot rerun the full original runtime audit.
The screen does not establish batch performance, full-matrix acceptance, actual
Chaos Mesh or promotion of the candidate.
