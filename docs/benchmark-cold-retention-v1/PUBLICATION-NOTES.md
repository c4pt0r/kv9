# Root publication receipts

The frozen metadata selection is preserved in 424 files / 13,136,741 decoded
bytes, one 1,586,531-byte archive part. Its copied files include the first failed
metadata-report draft. `verify.py` verifies byte identity without extraction or
payload revalidation. This does not repeat original stage, eviction or restore.

After the completed performance run, metadata packaging stopped twice before
creating output because its additional publication-space guard was unsatisfied:
receipts `282605/1` and `f23eda/1`. The new auxiliary evidence worktree was made
sparse, retaining its new evidence while releasing duplicate tracked files.
That alone was insufficient. Root then invalidated reproducible first-party
release cache using the existing BuildCache lock, and the Tokio release cache
under the same lock. The measured retained server, control, native client and
recovery workload hashes remained unchanged. Original WALs and histories were
untouched. Future builds must rebuild the invalidated dependency.

The first-party cleanup completed as `e949ef/0`; the bounded Tokio cleanup as
`d427a3/0`. Metadata packaging then passed as `27ebb2/0` without lowering its
guard. Exact failure observations, cleanup commands/results and package script
are copied under `root-publication/`. This post-run housekeeping is not new
benchmark evidence and did not overlap measurement.

The phase's 96 GiB staging/benchmark preflight floor remains unchanged. The
benchmark has a separate lower runtime floor. New measurements require their
own capacity preflight; this publication does not promise space for another
full campaign or simultaneous restoration of all cold members.
