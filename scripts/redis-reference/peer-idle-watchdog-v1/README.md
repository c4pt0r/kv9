# Peer idle watchdog: executed screening helpers

This directory retains exact helper and evidence bytes for candidate
`f62c08ed9dc59bf56336a64a24c76c2b775f634e`. The candidate source and conditional
watchdog deadline argument are in that commit. Performance selection is against
accepted `5ee897a`, with fixed native client `03c1c776` and Redis client `b8ec38f`.
It does not isolate the notification change against rejected `6707bcc`.

The original preparation is `/tmp/kv9-peer-idle-watchdog-comparison-preparation`.
`matched-driver.py`, `audit.py`, the wrapper, arguments and contract tests were
frozen before execution. Their delta from the preceding direct-peer-body helpers
is only candidate paths, protocol ID and source/build pins; the final full diff
is retained. The inherited six driver and ten auditor tests passed. The earlier
unfinished pin stage remains in the raw preparation's `before-final-pins`.

`inventory.json` records each retained copy's source path, byte count and
SHA-256; this newly authored README and the inventory itself are excluded.
The retained unified diff contains intentional blank context-line prefixes.
Its exact-path whitespace attribute preserves those historical bytes while
ordinary source and documentation whitespace checks remain enabled.
These are retained execution helpers with explicit absolute-path
source/build dependencies, not a claim that copying this directory alone creates
a portable benchmark environment. The source revisions must be checked out and
the pinned releases made available at the paths recorded in `protocol.json`.
Keep `/usr/bin/redis-server` as its lexical invocation path; resolving its
multicall symlink changes its behavior.

The measurement is a short shared-host, three-voter tmpfs-WAL comparison against
standalone Redis with persistence disabled. Both use 128-byte values and batch
size one; Redis does not pipeline. Timing clients use CPUs 0-1 and KV9 voters
share 2-5; the observer and three owned background containers use 6-15,22-31.
Unrelated services remain unconstrained. Original container CPU masks and
historical cluster objects must be preserved/restored. No source gate, fault,
profile or independent audit may overlap timed cohorts.

`source-gates.json`, `source-controls.json` and `source-review.md` bind the 228
Raft tests, Clippy and 14 source-control triples. `process-audit.json` records
379 complete-history calls (346 OK, 33 unknown) with leader loss and restart.
Process recovery is separate from actual Chaos Mesh or power-loss acceptance.
The source argument does not finish machine-checked whole-protocol refinement.

See [the screening report](../../../docs/PEER-IDLE-WATCHDOG-SCREENING.md) and
[the full readout](READOUT.md) for the final selection and all measured cohorts.
