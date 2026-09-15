# Receipt tail hint: original default release and recovery evidence

Exact source: `a6ac335ef4567f2e1a2b33da1a723d694cd5b7d9`.
The detached source reproduces all 869 source-gate hashes and all 12 proof-bound
Git blobs. Reversing only the specified receipt adapter changes reconstructs
the selected parent driver byte-for-byte.

The archive retains the original release manifests, Cargo/codegen logs, shared
cache records, root preflight and actual terminal, independent release readback,
all ordinary recovery fixture files, both original complete histories, checker
outputs, independent audit, and frozen operational environment preparation.
Release executables are excluded and remain retained locally; their actual
hashes are in the original manifests. The small ordinary recovery WAL files
are included. This package does not contain or claim a Chaos campaign.

Actual local terminals:

- Default release: session `18954`, terminal `924704/0`.
- Independent release readback: `9ade34/0`.
- Ordinary recovery: session `57860`, terminal `f5f864/0`.
- Independent recovery/history/process audit: `23d005/0`.

Run `python3 -B docs/receipt-tail-recovery-v1/verify.py` from the repository.
The verifier hashes every package file and archive member, reads through gzip
EOF, checks the original result/terminal/binary bindings and independently
recomputes the two history populations: 359 complete calls, 329 OK and 30
unknown. It performs no extraction or execution of archived helpers. Population
recomputation is not a replacement for the retained full linearizability audit.

`input-inventory.json` records original paths, archive names, sizes and SHA-256.
Original bytes remain unchanged. Operational policy v3 uses an 8 GiB continuous
host floor and the unchanged 16 GiB maximum available-space decrease, with an
8 MiB launch allowance, five-second samples and 1200-second outer timeout.
These are sampled resource guards, not reserved capacity. The separate prepared
Chaos policy is not evidence of executing or passing Chaos Mesh.
