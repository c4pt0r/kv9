# Independently checked rotation prefix, failed fault setup

This package preserves the first rotation supplement's failed outcome and its
valid pre-fault observations. It does not establish leader-kill recovery,
post-recovery rotation, complete supplement acceptance, or performance.

Run `python3 docs/published-directory-rotation-prefix-v1/verify.py` from the
repository root. It hashes all packaged files, streams every original archive
member against the full inventory, and consumes gzip EOF without extracting
any archive path. This checks the copied evidence; it does not rerun the
database or replace the original semantic auditors.

The archive retains 821 members / 112,252,546 decoded bytes in 586,169 compressed
bytes. SHA256:
`4b204632bcbc7c0fffcf8642ce573b57f73ba037d8adae6a993236db8e11f8f2`.
It includes complete client history values, selected topology and physical
headers, original commands, namespace/controller failure observations,
frozen helpers and actual failure receipts. Original source paths in the
inventory are provenance, not a claim those paths exist after download.
Raw PVC WAL/SST backups and retained ELF copies are explicitly excluded.

Separate copied records establish the valid pre-fault prefix and subsequent
archive verification and owned cleanup. Original runtime `74027/00a0c3/1`
remains failed. Prefix audit/capture `79959/b310b4/0`, archive/independent
readback `925892/0` and exact cleanup `86820/e74200/0` passed locally.
The original three voter containers and one collector exited; the native
workload child had already exited. All eight historical namespace UIDs remain.
