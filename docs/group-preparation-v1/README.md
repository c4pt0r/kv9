# Durable group preparation validation

Local [D01b preparation](../GROUP-PREPARATION.md) checkpoint: **865 workspace
tests/doctests passed, 28 existing ignored**; strict Clippy and formatting pass.
Nine focused tests include sixteen publication boundaries and three real
NodeRuntime instances with metadata leader replacement and disk reopening.
Nine single-defect Rust controls fail at their declared assertions.

The Lean component checks nine theorems, four semantic mutants and two
proof-hole/custom-axiom controls. Allowed axiom usage is only `propext`;
`restart_discards_report` uses no axioms. This is not verified Rust extraction
or complete lifecycle/activation acceptance.

[validation.json](validation.json) binds source, toolchain, counts and scope.
[evidence.tar.gz](evidence.tar.gz) retains 64 original logs/control sources,
including initial compilation/fixture/Clippy/proof-runner failures. All archive
members were read back and hash checked. Lean compiled artifacts and temporary
fixture stores are excluded. Archive SHA-256:
`c5ab6c7055a64781406ffa59a565d6513323024fc47aafc13b888b068ef95449`.

No actual Chaos Mesh, physical host failure, new performance or horizontal
scaling acceptance is claimed. #22 remains open. No hosted CI was triggered.
