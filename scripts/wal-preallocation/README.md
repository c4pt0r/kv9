# WAL encoder microbenchmarks

The ignored tests in `crates/engine/src/wal/preallocation_tests.rs` compare the
same byte emitter with zero versus validated initial capacity. Both timed arms
include size validation and buffer destruction. These are CPU microbenchmarks,
not database QPS, RPC latency or fsync measurements.

Prepare a separate source tree with the server's actual Linux jemalloc 0.6.1
global allocator. The helper overlays the reviewed working-tree candidate onto
the recorded committed base, verifies the original corpus, and records hashes.
It never edits the working tree or globally changes `TMPDIR`.

```sh
python3 scripts/wal-preallocation/prepare.py \
  --output /mnt/data/kv9-work/wal-encoder-NEW
cd /mnt/data/kv9-work/wal-encoder-NEW/source
CARGO_TARGET_DIR=/path/to/reusable/target cargo test --release --offline \
  -p kv9-engine --features wal-payload-preallocation --lib --no-run
```

Retain the resulting test executable and record its hash before timing. Run each
comparison once on a recorded CPU without overlapping local builds/tests. The
corpus test performs baseline/candidate/candidate/baseline, 128 passes over
106 original batches in each arm. The small test performs that order for six
fixed synthetic cases: 1/64 mutations and 0/256/4,096-byte values, 20,000 samples
per arm. Its nanosecond results include timer overhead. Neither is a controlled
multi-host production benchmark; do not pool their different workloads or
average per-arm p99 values.

```sh
KV9_WAL_CORPUS=/mnt/data/kv9-work/wal-encoder-NEW/batches.bin \
  taskset -c 4 /path/to/retained/test-executable \
  wal::preallocation_tests::retained_corpus_encoder_microbenchmark \
  --exact --ignored --nocapture --test-threads=1
taskset -c 4 /path/to/retained/test-executable \
  wal::preallocation_tests::small_batch_encoder_microbenchmark \
  --exact --ignored --nocapture --test-threads=1
```

The first corpus run predates the additional small-case test; both original
executables and harness hashes are retained separately. Reproduction does not
require replaying that completed run during ordinary development. Refer to the
[results](../../docs/WAL-PAYLOAD-PREALLOCATION.md) before another experiment.
