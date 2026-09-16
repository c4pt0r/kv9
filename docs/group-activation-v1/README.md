# Durable activation and shared-worker validation

The [activation increment](../GROUP-ACTIVATION.md) passes **878 workspace
tests/doctests**, with 28 existing ignored, plus formatting and strict all-target
Clippy. Six new focused tests cover six activation-publication cuts, ten invalid
recovery cases, shared-worker ownership/wakeups and a real three-runtime
durable multi-group scenario with leader loss and all-store reopening.

Eight single-defect Rust controls fail at their intended test assertions.
The new model checks 15 theorems, eight semantic defects and two proof-policy
controls. Nine existing preparation theorems and their controls are rechecked
under an explicit preparation projection. Source mapping is inspected and
pinned; this is not verified Rust extraction.

[validation.json](validation.json) binds sources, toolchain and exact scope.
[evidence.tar.gz](evidence.tar.gz) retains 108 source/log/result members,
including initial compilation failures, both runtime-fixture failures, model
development, mutation sources and final restored-source checks. Every archive
member has been read back and hash checked. Archive SHA-256:
`104e50a7319be79654ce0301c6cdd9d6d42964e218fc3c43683279fa0fbbe35f`.

No actual Chaos Mesh, independent-host, complete D01, public multi-range API,
new QPS or horizontal-scaling acceptance is claimed. No hosted CI ran.
