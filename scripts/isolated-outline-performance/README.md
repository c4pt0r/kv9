# Isolated shared-clone performance screen

The [completed result](../../docs/ISOLATED-OUTLINE-PERFORMANCE.md) fails the
material/read gates while passing correctness and allocator accounting.

The [qualification](../../docs/ISOLATED-OUTLINE-QUALIFICATION.md) binds the original
registry archery/rpds baseline and candidate with only triomphe extraction.
This screen reuses its exact ordinary-release timing binaries and adds a
separate [allocation-only companion](../engine-interface-counting/README.md).

The prospective plan fixes 22 cases, ABBA timing order, CPU 4, twelve write
passes, twelve read epochs, eight same-view warmup passes and twelve warm passes.
Snapshot, owned/resident, first-probe/warm and per-call/whole-pass units stay
separate. Whole-pass p99 is **not** individual request p99. Helper CPUs exclude
the benchmark CPU and its sibling; the host remains shared.

Create a fresh experiment root with the declared and execution plans, original
corpus and independent decoder. The only clarification to the published plan
removes an inherited sentence that incorrectly denied allocation evidence;
cases, thresholds, binaries, process counts and timing scopes are unchanged.
Then run locally and serially:

```sh
taskset -c 6-15,22-31 python3 scripts/isolated-outline-performance/build_counting.py ROOT
taskset -c 6-15,22-31 python3 scripts/isolated-outline-performance/run.py ROOT prepare
taskset -c 6-15,22-31 python3 scripts/isolated-outline-performance/validate.py ROOT prepare
taskset -c 6-15,22-31 python3 scripts/isolated-outline-performance/run.py ROOT counting
taskset -c 6-15,22-31 python3 scripts/isolated-outline-performance/validate.py ROOT counting
taskset -c 6-15,22-31 python3 scripts/isolated-outline-performance/run.py ROOT timing
taskset -c 6-15,22-31 python3 scripts/isolated-outline-performance/validate.py ROOT final
```

The complete screen has four preparation processes, 44 allocation processes and
88 timing processes. Their 76 allocation rows and 152 timing rows are checked
separately. The counter gate requires exact per-window equality between arms,
plus independent expected read-value and snapshot allocation counts. No timing
process starts before that gate passes. All raw samples and terminal records
are retained; failed checks stop the run without overwriting evidence.

The material write and no-regression thresholds are fixed before execution.
Passing this component screen would not promote production: full local
correctness, ordinary recovery, actual Chaos Mesh and material matched database
benefit against three-copy Redis remain separate requirements. The earlier
combined candidate's failed gate remains failed.

Do not replay this unchanged screen after it completes. Keep bulk artifacts under
`/mnt/data/kv9-work`, reuse the locked shared Cargo target and preserve each
experiment's source/binary/corpus identities.
