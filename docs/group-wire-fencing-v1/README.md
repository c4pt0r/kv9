# Data-group RPC fencing validation

The [transport increment](../GROUP-WIRE-FENCING.md) passes **872 workspace
tests/doctests**, with 28 existing tests ignored. Formatting and strict
all-target workspace Clippy pass. Seven new HTTP/2/ownership tests and five
existing multi-group tests cover the changed transport paths.

Six single-defect Rust controls fail at their intended test assertions. The
Lean model checks 15 theorems, rejects six semantic defects and rejects two
proof-policy violations. Its standard axiom inventory and explicit source
mapping are retained; this is not verified Rust extraction.

[validation.json](validation.json) binds sources, toolchains, test counts and
scope. [evidence.tar.gz](evidence.tar.gz) retains 51 source/log/result members,
including development failures, six mutated Rust sources, Lean model controls
and the final restored-source workspace run. Every member was read back and
hash checked. Archive SHA-256:
`04266342cbb5c9a40cdeb1a8f5418325e18f81e5af0e62a75d8ac4dbbe9ad85f`.

The legacy dispatcher is an emulator, not a released old executable. No actual
Chaos Mesh, complete D01, new QPS, or measured horizontal scaling is claimed.
No hosted CI ran. Original roadmap checkboxes remain open.
