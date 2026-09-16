# Shared-prefix index evidence

Portable originals for the [rejected candidate](../PACKED-PREFIX-EXPERIMENT.md).
`evidence.tar.gz` contains source snapshots, plans, the count-only diagnosis,
qualification/build logs, failed proof/build attempts, passing proof and controls,
full prepared read keys/probes, all 252 timing and 252 allocation rows, independent
input/statistics analysis, and execution receipts. `members.json` records every
uncompressed member. `verification.json` records full streaming readback and
archive identities, plus local-only artifacts.

The retained corpus SHA-256 is
`d978ba9e49131f6277f975ecd333c33854845cc03bd14ee57a22fff915a08350`.
Its large `groups.bin`, retained executables and compiled Lean modules stay local;
the original corpus is also in the earlier resident-index experiment packet.
All new bulk outputs live under `/mnt/data/kv9-work`.

`analyze.py` and `validate-inputs.py` reconstruct the nine final maps and 24 query
sets and recompute statistics from raw samples. Neither the archive integrity
check nor the conditional prefix lemmas imply full index, engine or distributed
correctness. No production, recovery, Chaos Mesh, database-QPS or Redis-parity
claim is made. Do not repeat this unchanged failed screen.
