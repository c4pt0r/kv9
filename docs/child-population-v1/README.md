# Child-population validation packet

See [implementation, population semantics and remaining scope](../CHILD-POPULATION.md).

- 930 passing workspace tests/doctests in the serial qualifying run, 35
  ignored, zero failures; strict Clippy/formatting pass; 8 real-MinIO
  focused installer tests.
- Eleven checked Lean theorems for the child-population model with six
  semantic mutation controls and two proof-policy controls; nineteen
  sibling Lean models and the retention TLA/TLAPS model re-accepted
  against the exact working tree.
- One accepted five-process e2e (`e2e-second`, covering the exact final
  source; `e2e-first` failed on a wrong fixture expectation — the harness
  assumed a probe key that was never written — and is retained): the
  committed intent and durable fence flow, a population refusal BEFORE
  the seal, both halves copying into their children through the child
  logs in bounded committed batches with independently recomputed equal
  digests (distinct across halves, exact row counts), reruns converging
  including after a child leader restart, and the parent staying fenced
  with the catalog directory untouched throughout.

[evidence.tar.gz](evidence.tar.gz) contains **929 readback-verified
files**, 838226 compressed bytes and 5102121 decoded bytes. SHA-256:
`fd8436b309ada57f934c914fe8c34f8e9927be8899a68fd9e3fba71dbf35114c`.
[validation.json](validation.json) inventories all members. A separate
extraction re-verified every member and the e2e/proof/retention receipts;
[portable-readback.json](portable-readback.json) records that result.

Tested source is based on `9f67e8d` with the exact current
Rust/Cargo/proto source retained under `source/`.

Nothing here republishes the range directory, reroutes a key, unseals
anything, reclaims storage, or claims scaling or new QPS. This increment
ran no Chaos Mesh campaign. The independent 3/6/9-host benchmark contract
in [HORIZONTAL-SCALING-PLAN.md](../HORIZONTAL-SCALING-PLAN.md) remains
the acceptance criterion for effective horizontal scaling. No hosted CI
was dispatched.
