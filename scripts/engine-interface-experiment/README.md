# Engine interface diagnostic

This standalone harness measures the unchanged `MemEngine::write_applied`,
`Engine::snapshot`, and captured `ReadView::get` / `get_resident` interfaces.
The candidate dependencies are qualified separately by
[the outlined mutation experiment](../../docs/OUTLINED-MUTATION-PATH.md).
There is no server, WAL, Raft, read barrier, RPC or Redis measurement here.

The retained plan fixes 22 cases and ABBA execution order: four write cases,
two snapshot sizes, and sixteen read cases. Reads separate owned versus borrowed
values, hit versus miss, small versus large maps, and per-call versus whole-pass
timers. Each read epoch uses one snapshot for the first 512 probes, eight untimed
passes and twelve measured warm passes. Twelve fresh engines provide twelve
epochs. This is first-probe timing, not a flushed-cache guarantee.

Write batches are built before timing; the measured call includes internal
clones, batch consumption and applied-position publication. Snapshot acquisition
and dropping the optional old view are outside write timing. Acquisition is
also measured separately. Snapshot destruction is outside its timer.

The read API is selected statically outside each loop. A per-call timer includes
the actual virtual call, result/error check and black box, then stops before
storing the result. A whole-pass timer includes all 512 calls and output stores
in a preallocated vector. Every returned value is retained until validation
after the pass; destruction is outside timing. Copying an owned value is inside
the call. Borrowed results remain tied to the same view. The whole-pass mean
divided by 512 is a pass-derived estimate; its p99 is **not** request p99.

Preparation checks every live prefix and optional prior snapshot against a
separate ordered model, plus applied position/revision and untouched Lock/Write
column families. A Python decoder independently reconstructs full final maps
and seeded probes. Every read epoch subsequently applies a new position and
checks that the old view remains unchanged. Measurement results are never
accepted without successful checks.

The retained experiment root supplies `plan.json`, `groups.bin`, and the unchanged
independent `reference-inputs.py` from the earlier outlined packet. It also points
at the qualified dependency sources. Build and run locally, serially:

```sh
taskset -c 6-15,22-31 python3 scripts/engine-interface-experiment/build.py EXPERIMENT_ROOT v1
taskset -c 6-15,22-31 python3 scripts/engine-interface-experiment/run.py EXPERIMENT_ROOT prepare
# Retain nm -SC and objdump -dC -Mintel output for each copied timing ELF as
# symbols-{baseline,candidate}.txt and disassembly-{baseline,candidate}.txt.
taskset -c 6-15,22-31 python3 scripts/engine-interface-experiment/inspect_codegen.py EXPERIMENT_ROOT
taskset -c 6-15,22-31 python3 scripts/engine-interface-experiment/run.py EXPERIMENT_ROOT measure
taskset -c 6-15,22-31 python3 scripts/engine-interface-experiment/validate.py EXPERIMENT_ROOT final
```

The published run uses the ordinary ThinLTO release profile and jemalloc, CPU 4
on a shared host, with helper processes excluding that CPU and its sibling.
Build sources, dependencies, clean/fresh artifacts, executables, child lifetimes,
all raw samples and independently recomputed statistics are bound in retained
records. Failed checks stop execution; existing output files are not overwritten.
The shared target directory is retained under its build lock. Bulk evidence
belongs under `/mnt/data/kv9-work`.

This diagnostic cannot pass the earlier failed component selection gate or
authorize production promotion. A changed production candidate still needs
material end-to-end benefit, full local correctness, ordinary recovery, actual
Chaos Mesh and a matched three-copy Redis comparison.
