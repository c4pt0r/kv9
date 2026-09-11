# Write CPU attribution and CRC source proof

See [the report](../../../docs/WRITE-APPLY-CPU-PROFILE.md) for interpretation.
`index.json` binds exact copies from the original local recording and proof
directories. The source fields identify those retained originals. Large raw
perf data, WALs, full runtime inputs and earlier failed proof attempts remain
local. This compact selection is not a self-contained rerun environment.

The two `measured-samples.json` files contain the selected CPU population.
`crc-hotspot.json` and the exact-binary disassembly identify the CRC loop;
`analysis-summary.json` keeps the remaining CPU categories and their limits.
The recording uses baseline 5ee897a, not the CRC candidate. CPU percentages
are not end-to-end latency fractions or a claimed performance improvement.

The original `root-stage-gate-first.json` predates proof acceptance and retains
its pending status. `proof/result.json` is the separate accepted fourth gate,
with 22 statements, 256 actual compiled entries and five rejected controls.
The three earlier failed gates have not been relabeled as passes. The proof
covers CRC computation with an explicit source-operation map; end-to-end
performance, process recovery and Chaos Mesh acceptance remain separate.
