# Resident index named adapter experiment

This isolated harness compares the same rpds index calls under alternate
dependency workspaces. It does not change the production engine. See
[qualification and results](../../docs/OUTLINED-MUTATION-PATH.md).

Both builds use ordinary ThinLTO, one codegen unit and jemalloc. Timing and
allocation counting use separate executables. The retained experiment driver
applies explicit dependency overrides, verifies the resolved graph against the
root lock, cleans affected shared-cache units and copies the resulting binaries
before protocol tests. Never patch the Cargo registry or infer a build from a
previous target artifact.

## Invocation and inputs

The executable accepts `ROOT FRESH_OUTPUT prepare|measure CASE ORDER`.
`ROOT` must contain `groups.bin` and the exact retained `timing-plan.json`.
The plan binds 12 timing passes, one counting pass, ABBA order and the original
106-group / 100,096-mutation corpus. Case numbers cover 18 write cases and
24 read cases across original24, short4 and dispersed0 keys. Preparation checks
1,908 live prefixes and 954 retained old views per arm; an independent Python
reconstruction verifies the maps and seed-71 probe identities before timing.

Writes keep the prior unchanged kernel: overwrite, initial fill and unique
insertion, without/with retained snapshots, one excluded pass followed by
12 measured passes. Inputs are owned and cloned within the measured write
group. Snapshot creation/drop and full-state checks are outside that window.

Reads have two separately retained panels per case. For each of 12 fresh maps,
the first panel times the first 512 probe queries before validating those query
outputs. Construction itself touches nodes; this is not a cache-flush experiment.
The same live map then serves eight untimed probe passes and 12 measured warm
passes. Every map is checked against the independent model before it is dropped.
The counting build uses one map and one pass per panel, with the same eight
untimed warmup passes. No first-pass samples are discarded or pooled into the
warm panel. A map-identity protocol test prevents warmup from using a different
map. Point and predecessor results are borrowed; scan16 owns its results and
drops them outside the timer. Per-call timing overhead remains.

There are 66 comparison cells, 264 rows per build mode and 338 process lifetimes
including preparation. The predeclared screen requires all four original
write means to fall at least 10%, all 18 write means to improve in both orders,
and each of 48 read-panel mean/p99 comparisons to regress no more than 2%.
This component gate cannot establish database QPS, Redis parity, Raft recovery
or Chaos Mesh acceptance.

The named adapter failed the code-generation gate: the same out-of-line
mutation body survives as a `FnOnce::call_once` shim. Its paired executables
and protocol tests were built, but **no timing matrix ran**. Do not run it as
an inlining improvement. The outlined experiment uses the corrected read
protocol with a different, qualified dependency change.
