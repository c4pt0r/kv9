# FNV writer source qualification evidence

Experimental runtime source: `12f44d35590ede5f89337fe731dd950162865154`.
This package preserves the complete source-bound proof, first failed controls,
local source checks, source inventories, cache validation and execution receipts.

The archive contains **385 files / 2,329,805 decoded bytes**, compressed to
**449,042 bytes**. Archive SHA-256:
`1d7b7c5d4c6c477570454a3050db132b6735f67127cac8d6e701d1b37df656c1`.
Inventory SHA-256:
`1bc6fe7929123cd6acc112b05c24124158d2b08157ddfc43528e6ce69d7dc4b2`.
Six standalone test executables remain local; exact paths, sizes and hashes are
listed as omissions. Cargo build artifacts remain in the protected local cache;
Cargo JSON and invalidation records identify their actual compilation.

```sh
PYTHONOPTIMIZE=0 python3 verify.py --root . \
  --inventory-sha256 1bc6fe7929123cd6acc112b05c24124158d2b08157ddfc43528e6ce69d7dc4b2
```

The verifier checks every archived byte and gzip EOF without extraction or
execution. This is portable evidence readback, not a repeat of the proof/tests.
Readable top-level result files summarize the archived original records.

Qualified proof: session `42579`, terminal `133027/0`; local source gate:
session `16443`, terminal `3e6304/0`. The earlier full proof-gate attempts exited
with code 1 and remain archived separately. No release, database benchmark or
actual Chaos Mesh execution is included in this source checkpoint.
