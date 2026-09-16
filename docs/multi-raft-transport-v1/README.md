# Multi-Raft transport validation

Local validation for [D01a](../MULTI-RAFT-TRANSPORT.md): 856 workspace tests/
doctests pass, 28 pre-existing ignored; formatting and strict all-target Clippy
pass. Five focused tests pass before and after five independent single-defect
controls, each failing at its declared assertion.

[validation.json](validation.json) binds exact source hashes, toolchain, commands
and outcomes. [evidence.tar.gz](evidence.tar.gz) contains original logs, retained
initial compilation failure, exact control sources and the control runner.
All 20 archive members were read back and hash-checked. The archive SHA-256 is
`ee0d65f3462390802c0caba8ba7c61b60f30f17d761dfacbfb113db02cc28723`.

This is local loopback/in-memory transport acceptance, not durable multi-group
recovery, formal lifecycle acceptance, actual Chaos Mesh or scaling measurements.
The complete #22 issue stays open. No GitHub CI was triggered.
