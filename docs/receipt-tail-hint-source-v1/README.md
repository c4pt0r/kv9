# Receipt tail-hint source qualification evidence

This small, uncompressed reporting bundle preserves the exact source gate,
proof outputs, semantic audits, finite counterexamples, preparation lineage,
original draft proof and actual terminal records. All original files remain
local. No database workload or performance measurement is included.

Runtime/proof source is `a6ac335ef4567f2e1a2b33da1a723d694cd5b7d9`.
The proof ran before commit; `data/source-root/committed-proof-binding.json`
records exact Git-blob equality for all 12 proof-bound inputs. The source gate
then qualifies a clean 869-file checkout with 793 tests/doctests (23 existing
ignored), 49 included driver tests, formatting, Clippy and experimental-feature
compilation. Formal validation passes 18 theorems / 158 fresh obligations,
finite gap/certificate counterexamples, semantic and output controls.

Run `python3 verify.py` here for independent size/SHA readback of every copied
file. This checks byte integrity, not semantic re-execution. The original paths
and hashes are in `inventory.json`. Default release, recovery, actual Chaos Mesh
and matched database throughput/latency remain separate, uncompleted gates.
