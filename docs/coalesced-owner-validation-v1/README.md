# Coalesced owner validation evidence

Run `python3 docs/coalesced-owner-validation-v1/verify.py` to verify every
portable original member against `manifest.json` without extraction. This is
an integrity check, not a rerun of the recorded gates. The manifest preserves
each original path and SHA-256; the gzip/tar archive uses deterministic metadata.

Included: the original audited proof gate, two incomplete development proofs
and the successful corrected derivation, local source gates and Cargo cache
transaction, clean default-release provenance, ordinary three-voter recovery
histories/WALs, and the independent history audit. Original executable bytes
remain in the retained release directory, bound by the included manifests.

Scope: candidate `42e0117b13bed9671b672436c6b36bfe63c462af`, 14 new theorems /
64 obligations plus unchanged scheduling dependency 33 / 294; 438 default
Raft/Server tests/doctests (one existing ignored), 214 overlapping Raft testing
tests/doctests, formatting and both all-target Clippy configurations. Ordinary
streaming/unary recovery checks 369 operations: 341 OK and 28 unknown, including
point/batch overlap, leader loss and restart from original directories.

These records do not establish candidate performance, actual Chaos Mesh,
independent host failure, full Rust refinement or full database correctness.
