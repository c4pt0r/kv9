# Read layout diagnosis

These tools inspect and compare the exact executables from the outlined mutation
screen. They do not change the database or select an optimization. See the
[report](../../docs/READ-LAYOUT-DIAGNOSIS.md) and its retained evidence.

The fixed experiment reuses the two ordinary timing binaries and builds the
same source/dependency pair with `-Cllvm-args=--align-all-functions=6`. It runs
only original small-map GET hit, GET miss and pinned unique insertion. A
palindromic eight-execution sequence per case gives two orders per configuration:
26 processes including two preparations, 40 result rows and 10 comparisons.
The original first-probe/same-map warm protocol and every sample are preserved.
This is a layout intervention, not a profiler and not a new selection gate.

Given fresh `EXPERIMENT` and the previous `QUALIFIED` evidence roots, retain the
exact plan/corpus from the packet and use:

```sh
python3 scripts/read-layout-diagnostic/inspect_elf.py ORDINARY_BASELINE EXPERIMENT/codegen-ordinary-baseline
python3 scripts/read-layout-diagnostic/inspect_elf.py ORDINARY_CANDIDATE EXPERIMENT/codegen-ordinary-candidate
taskset -c 6-15,22-31 python3 scripts/read-layout-diagnostic/build.py --experiment EXPERIMENT --qualified QUALIFIED
python3 scripts/read-layout-diagnostic/compare.py --experiment EXPERIMENT
taskset -c 6-15,22-31 python3 scripts/read-layout-diagnostic/run.py --experiment EXPERIMENT --qualified QUALIFIED
python3 scripts/read-layout-diagnostic/analyze.py --experiment EXPERIMENT
```

The inherited plan describes the complete 42-case namespace; `diagnostic_cases`
and `diagnostic_sequence` declare the executed subset. Only timing binaries are
built here. Both protocol tests run after the retained binary copies. Independent
analysis reconstructs the original corpus, verifies every result and recalculates
all quantiles. Freshness and dependency hashes are required; the Cargo registry
is never modified. Bulk output goes to `/mnt/data`, with the shared build cache
serialized by `scripts/build_cache.py`.

`inspect_elf.py` decodes the exact x86-64 ELF, resolves the six-entry operation
jump table and checks the GET `memcmp` relocation. It retains full function
assembly. Equality after removing addresses/RIP displacements does not establish
identical executed addresses, physical cache placement or a performance cause.

The separate `capture-gdb.py` stops each exact ELF at the first GET entry and
reads one 4,096-node tree and the resolved runtime `memcmp` target. Its offsets
are private to these recorded binaries, not a Rust ABI guarantee. Use the
retained `capture-layout-v3.py` supervisor to reproduce its four captures in a
fresh root. `analyze-layout.py` independently verifies all keys/values, strict
ordering, red-black invariants, complete graph coverage and probe identity
before comparing relative virtual layouts. It reads `capture-summary-v3.json`
and the corresponding `layout-v3-*` outputs. Debugger timings are excluded from
performance analysis. This first-map observation makes no later-epoch or
physical-memory claim.

The first decoder used an ArcInner-relative color offset against a Node pointer;
it was rejected before acceptance. Corrected captures and all failed logs are
retained. A subsequent inspector rename avoids shadowing Python's standard
`inspect` module. `retained-tool-bindings.json` binds the exact pre-rename build
helper to its immutable snapshot; all Rust source, ELF, plan and timing identities
stay unchanged. This is an explicit tooling-source relocation, not a benchmark
rerun or a relaxed source check.
