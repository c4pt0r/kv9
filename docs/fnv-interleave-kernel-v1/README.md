# FNV interleaving kernel evidence

This archive retains the original source-bound Lean/test gate, all kernel
microbenchmark rows, source/tool/child receipts, actual WAL header-length
metadata and the earlier failed proof-development attempts. Scope and results
are in [the report](../WRITE-FNV-INTERLEAVE-KERNEL.md).

The archive contains **107 files / 285,321 decoded bytes**, compressed to
**52,232 bytes**. Its SHA-256 is
`ea6a1dc0f65ff4dfb42b824ff2db1f1217b77343460c8d7feca9341d95d33bc5`.
The exact [inventory](inventory.json) SHA-256 is
`b68eb08f56676b94e48a98a984f8599ef05630efb50abc1ac0828c25bd62eaba`.
Three original executables stay local with their path/size/hash declarations
in the inventory. No database payload is included or needed for portable
byte verification.

Readable [proof results](proof-result.json), [kernel measurements](microbenchmark-summary.json)
and [execution receipts](execution-receipts.json) accompany the archive.
Actual proof session 81919 ends at 6e45e3/0; measurement session 17991 ends
at 326c39/0. Kernel source is c00b57452360464d8d67acd055fb92280ba834b9.
The complete benchmark is kernel-only, with no writer integration or database
throughput acceptance.

Verify every archived byte without extracting or running the proof/benchmark:

```sh
PYTHONOPTIMIZE=0 python3 -B verify.py --root . \
  --inventory-sha256 b68eb08f56676b94e48a98a984f8599ef05630efb50abc1ac0828c25bd62eaba
```

Proof and benchmark replay use their separately committed scripts and toolchain
contracts. Portable byte verification is not a new proof run, database
correctness run or performance result. Hosted CI was not dispatched.
