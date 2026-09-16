# Static callback component screen

This standalone rpds harness compares baseline archery 1.2.3 with the
[qualified static callback patch](../static-pointer-callback/README.md).
The production workspace and dependency selection are unchanged.
See the [completed evaluation](../../docs/STATIC-POINTER-CALLBACK-PERFORMANCE.md)
for all measured results, read-pass limitations and the held selection gate.

Build the identical source in two isolated workspaces, overriding archery with
the respective checksum-verified qualification source. Use ordinary ThinLTO,
one codegen unit and jemalloc. Build latency and `allocation-counting` binaries
separately. The retained execution helpers serialize builds, verify exact
dependency paths and preserve each executable before another build.

The write/read kernels, corpus decoder, key transformations, fixed-seed probe
selection and allocation counter are copied unchanged from the previous
resident-prefix harness. Packed-tree backends are removed. The new driver
executes one rpds case per process so baseline/candidate executables can be
interleaved in ABBA order. It does not pool earlier benchmark results.

```text
kv9-static-callback-experiment ROOT FRESH_OUTPUT prepare 0 ORDER
kv9-static-callback-experiment ROOT FRESH_OUTPUT measure CASE ORDER
```

`ROOT` holds the retained `groups.bin` and `timing-plan.json`. `ORDER` is 0..3
for baseline, candidate, candidate, baseline; the supervisor must select the
corresponding pinned executable. The executable label alone does not establish
its dependency identity. Each case gets one excluded warmup followed by 12
timing passes, or one counting pass without a warmup. Every result is written
to a fresh file.

Cases 0..17 are the three key distributions × overwrite/initial-fill/unique
insertion × unpinned/pinned snapshots. Cases 18..41 are the three distributions
× small/large final maps × GET hit/GET miss/predecessor/scan16. Query probes use
the predeclared seed 71, full Fisher-Yates permutation and 512 keys without
replacement. An independent Python reconstruction checks exact maps and probe
identities before any timing.

Writes include owned key/value clones; old snapshots are created/dropped
outside each measured group. Reads time borrowed point/predecessor answers and
owned scan output; output destruction is outside the window. Per-call timer
overhead remains. Counting reports allocation calls and requested bytes, not
RSS or allocator fragmentation. All maps must release their requested bytes.

Selection thresholds remain fixed before execution: at least 10% lower pooled
mean in original overwrite and unique insertion with/without snapshots, every
write mean lower in both orders, and every read pooled mean/p99 within a 2%
regression bound. This is an offline component experiment, not database QPS,
recovery, actual Chaos Mesh acceptance or a production promotion.
