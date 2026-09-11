# Owner-read-pump evidence

See [the decision and measurement report](../OWNER-READ-PUMP-SCREEN.md),
[pooled readout](READOUT.md), and [every repetition](PER-REPEAT.md).

The two archive parts reassemble to one exact gzip archive containing 879
retained reporting/source-gate files, 3,167,064 compressed bytes. The inventory
records each original path, length and SHA-256, both parts and the complete
archive. It includes the rejected fixture-level mutation attempt alongside the
corrected semantic control, ordinary recovery histories, source/build receipts,
proof logs and the screen's original reporting inputs. Large database directories,
server binaries and the full 4.56 GB timed runtime inventory remain at the paths
recorded in the original audit; they are not all included here.

Run `python3 docs/owner-read-pump-v1/verify.py` to check exact archive membership
and every retained byte without extraction. This is an integrity check, not a
rerun of TLC/TLAPS, Rust tests, linearizability checking or runtime acceptance.
The source proof runner is `scripts/check-owner-read-pump-protocol.py` on the
candidate commit; original pinned tool paths and invocation logs are retained.

Raw reports distinguish measured attempts from setup routing retries. Quantiles
come from merged histogram buckets; no percentile is averaged. This short
point-read/mixed screen does not establish significance, sustained capacity,
equal durability, the complete point/batch matrix or actual Chaos Mesh acceptance.
