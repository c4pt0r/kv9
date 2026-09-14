# FNV writer default release and ordinary recovery evidence

Runtime/source revision: `12f44d35590ede5f89337fe731dd950162865154`.
The package retains full release provenance, independent readback, both complete
recovery histories, original recovery payloads, the independent audit and
execution receipts. It also preserves the failed documentation-commit preflight
and the exact-source worktree resolution, without changing inventory limits.

The archive contains **167 files / 4,171,798 decoded bytes**, compressed to
**418,076 bytes**. Archive SHA-256:
`68248a3732845f8ac9b72fefaafac816febf01e636ac2d1c637cd1c98654025e`.
Inventory SHA-256:
`4ed52163ab0e26e9c591443cfc38e7b00b12ecb130497db952edc7b84b7d2f01`.
Two executables remain local with exact paths, lengths and hashes in the
inventory. No other original selected input is omitted.

```sh
PYTHONOPTIMIZE=0 python3 verify.py --root . \
  --inventory-sha256 4ed52163ab0e26e9c591443cfc38e7b00b12ecb130497db952edc7b84b7d2f01
```

The verifier checks all archive bytes, path/member bounds and gzip EOF without
extracting files or executing their contents. Actual readback `c89ed5/0` passes.
This validates portable evidence integrity, not a repeated runtime test.

Release session `53741` ends at `b9574b/0`; independent readback `b71ed8/0` passes.
Ordinary recovery session `10589` ends at `bc3b35/0`; the unchanged independent
auditor passes at `103df6/0`. The earlier preflight `7a74e0/1` did not start a
build. Actual Chaos Mesh and database performance remain separate pending gates.
