# Original rotation and recovery evidence

This package retains the successful receipt-fenced rotation supplement and
the preceding no-op-gap fixture failure as distinct outcomes. The first
namespace-opt-in failure remains in
[the earlier prefix package](../published-directory-rotation-prefix-v1/README.md).
See [the report](../WRITE-PUBLISHED-DIRECTORY-ROTATION.md) for scope and results.

Run `python3 docs/published-directory-rotation-v1/verify.py` from the repository.
The verifier hashes every package file, streams every member of both original
archives, checks complete inventories and gzip EOF, and binds original runtime
outcomes to their actual terminal records and subsequent cleanup. It never
extracts archive paths or executes archived helpers. This is byte/provenance
verification; it does not rerun the cluster or replace the original independent
history audit contained in the archives.

Both archives contain complete client histories including values, selected
topology bytes and physical headers, fault/process observations and frozen
test helpers. They omit complete raw PVC WAL/SST backups and local ELF bytes,
while retaining original source/build pins. The failed attempt additionally
retains its separate public read-only recovery diagnosis. That diagnosis never
changes the failed runtime into an accepted supplement. Original records that
predate cleanup retain their pending-cleanup fields; later cleanup records
complete that scope without rewriting history.
