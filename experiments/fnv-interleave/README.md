# Four-frame FNV kernel experiment

The first root execution now completes all 108 rows. See the
[results and integration contract](../../docs/WRITE-FNV-INTERLEAVE-KERNEL.md).
The exact source prototype is `c00b57452360464d8d67acd055fb92280ba834b9`;
actual session 17991 ends at 326c39/0. The commands below reproduce the fixed
protocol in a fresh directory after source/proof review.

`bench.rs` includes the exact production candidate module with
`#[path = "../../crates/raft/src/storage/fnv.rs"]`; no checksum implementation is
copied into the benchmark. Matching `#[inline(never)]` wrappers expose scalar
four-call and interleaved four-lane variants through the same function-pointer
signature. Both receive `std::hint::black_box` input and return black-boxed output.
Buffer allocation, deterministic filling and a scalar/interleaved equality check
occur outside timing. Thirty-two warmup groups precede each timed row.

The fixed shapes are `[0;4]`, `[32;4]`, `[205;4]`, `[206;4]`, `[10600;4]`,
`[10601;4]`, `[65536;4]`,
`[205,205,10600,205]` and `[0,205,10600,32]`. Three repetitions each run the complete
18-job list forward and then completely reversed: **108 measured rows**. Nonempty
rows use `clamp(floor(64 MiB / group_bytes), 256, 1_000_000)` groups; empty rows use
1,000,000 groups. Work counts do not adapt to observed speed. Each row retains
lengths, group count, exact input bytes, elapsed nanoseconds, ns/group and MiB/s;
zero-byte MiB/s is unavailable. The binary has a 60-second wall budget, and the
runner allows 65 seconds before bounded TERM/KILL cleanup.

The retained header sample at
`/tmp/kv9-raft-fnv-interleave-kernel-20260914-first/record-length-sample.json`
motivates earlier point 204/205-byte and batch 10599/10600-byte body sizes.
The later `record-length-complete-headers.json` in that directory finds mature
206-byte point and 10601-byte batch bodies; both mature sizes and the earlier
width boundaries are retained here. Neither header traversal shows that four
bodies occur together in a Ready or establishes production group frequency.
This benchmark reuses warm synthetic buffers and measures only the
checksum kernels. It cannot establish database QPS, storage latency, workload
percentiles, Ready grouping or an end-to-end speedup.

Root runs the following after terminal proof/source checks, substituting the
installed compiler path and using a fresh output directory. No compile, test or
timing ran during source preparation; the subsequent root execution is recorded
above. The module and benchmark hashes below are the measured source versions.

```sh
env PYTHONOPTIMIZE=0 PYTHONDONTWRITEBYTECODE=1 taskset -c 6-15,22-31 \
  /usr/bin/python3 scripts/benchmark-fnv-interleave.py \
  --output /tmp/kv9-raft-fnv-interleave-microbenchmark-20260914-first \
  --rustc /ABSOLUTE/INSTALLED/TOOLCHAIN/bin/rustc \
  --expected-module-sha256 a836a03afbf5080eea9d35e040896dd823b66954ed5db356a7de88cfc0d1945d \
  --expected-bench-sha256 9e58d740e85048be56901d18a0645d1911386db7b45fc42c2b378f711d6e36cb --cpu 6
```

Run from this candidate source worktree. The runner compiles once using Rust 2021,
opt-level 3, ThinLTO, one codegen unit, unwind panics, no debug assertions/overflow
checks/debuginfo, and no CPU ISA overrides. It retains the actual compiler `-Vv`,
compiler/source/binary hashes, current Git revision/status, uname, CPU identity and
every original command, stdout/stderr, child identity and exit. Selected child and
compiler affinity is one explicit helper CPU. Compilation is bounded to 120 seconds;
each metadata child to 15 seconds; retained output to 512 MiB and logs to 4 MiB,
with a 96 GiB available-space floor. These microbenchmark limits do not replace
or reduce any source-build or database performance gate.

Raw rows and each direction are retained. The Python readback validates exact
ordering/counts and byte/time arithmetic, then pools by total work/time, never by
averaging throughput or ratios. It reports no latency percentiles. A failed
compile, incomplete matrix, changed source or child cleanup failure remains failed;
there is no retry, recalibration, output overwrite or silent row filtering.

Header metadata pin: `/tmp/kv9-raft-fnv-interleave-kernel-20260914-first/record-length-sample.json`, SHA256 `4655bfb8a3c28eecbc183a72542e86a487f4498ac28cac22efa4aa25327d95f1` (5081 bytes). No original WAL body was read by this preparation.

Header metadata pin: `/tmp/kv9-raft-fnv-interleave-kernel-20260914-first/record-length-complete-headers.json`, SHA256 `3ae3723e41cb95588f439c31b7981673298e0556800c9d11e8ad1770fa76551d` (5053 bytes). No original WAL body was read by this preparation.
