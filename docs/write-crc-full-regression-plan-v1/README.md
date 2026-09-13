# Full CRC regression preparation

This is a plan, not an executed campaign. The accepted write-only screen is
published in [WRITE-CRC-PERFORMANCE.md](../WRITE-CRC-PERFORMANCE.md).

The next comparison retains selected server `11113f6`, CRC candidate `e748620`
and the same native v3 client. It covers point and batch APIs, read percentages
0/50/100, concurrency 1/64 and two complete opposite orders: **24 smoke cohorts
and 48 timed cohorts**. The fixed client already exercised all six workload
cells in the accepted historical full72 campaign. That establishes compatibility,
not acceptance of the unrun CRC read/mixed workloads.

[PLAN.md](PLAN.md) maps the finite driver, audit and reporting adaptations.
[protocol-plan.json](protocol-plan.json) contains every proposed descriptor,
source/executable pin and required future count. Reuse the existing generic
native lifecycle and full72 mixed-operation arithmetic; the write-only summary
reader intentionally cannot accept read/mixed populations unchanged. Fresh
controls must cover the changed matrix, API/outcome accounting and report gates.

[capacity-scenario.json](capacity-scenario.json) projects 81,042,588,591
compressed bytes and 115,712,043,863 logical bytes from accepted metadata.
With the original 96-GiB floor, a 16-GiB restore reserve and 1-GiB metadata
margin, the empirical reservation needs **202,375,414,703 available bytes**.
The recorded observation has **121,498,157,056 bytes** free, a shortfall of
**80,877,257,647 bytes (80.88 GB)**. This is a volume scenario, not an upper
bound. Faster writes or another mixed schedule can exceed it. Every original
cap and floor remains enforced, and no unachieved historical recovery target
or hypothetical deletion is credited.

Only this larger performance campaign is gated on capacity and helper
qualification. The separate Raft frame-buffer experiment can continue exact
release and recovery qualification under its own original guards. Its runtime
changes remain separate from CRC until independently accepted.

The [conditional command shapes](prospective-commands.json) reference future
helper files and a pending driver hash; they are deliberately not runnable yet.
Prepare and freeze the finite derivatives, pass relevant controls, verify exact
source/build/client/host identities and capacity, then launch the complete
smoke/timed protocol. Preserve all attempts, tail latency and unknown outcomes.
Do not rerun failed cohorts or claim promotion from partial coverage.

[inventory.json](inventory.json) preserves eight exact planning artifacts under
SHA-256 `56e47d918789acff8ae52c0699ad61e0b191f9c4c095506c64bbc63151764177`.
The original metadata planning receipt is `9688b8/0`. The [input hashes](input-hashes.json)
and [artifact observation](retained-artifact-observation.json) identify the
retained evidence used. No source validation, tests, build, workload, codec,
WAL payload hashing or cleanup was executed for this preparation. This README
is an editorial guide; original planning bytes remain unchanged.
