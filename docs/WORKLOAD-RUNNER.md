# Persistent workload executable

`kv9-workload` composes initialization, warmup, measured traffic, drain and final
verification over the bounded persistent client in [PERSISTENT-CLIENT.md](PERSISTENT-CLIENT.md).
It supports point operations on a fresh, unsplit Raw keyspace. Tokens come from
`KV9_CLIENT_TOKEN`; they are excluded from configuration, reports and CLI arguments.

## Build and run

Build an actual retained executable outside the source tree:

```sh
python3 scripts/build-workload.py --output /tmp/kv9-workload-build
# Add --release for an optimized binary; keep the profile fixed across trials.
/tmp/kv9-workload-build/kv9-workload \
  --config /tmp/workload-config.json --output /tmp/kv9-workload-run \
  --build-manifest /tmp/kv9-workload-build/build.json
python3 scripts/check-workload-report.py /tmp/kv9-workload-run \
  --build /tmp/kv9-workload-build --output /tmp/kv9-workload-checked.json
```

The run directory and checker output must be new. The build helper snapshots the
tracked and non-ignored untracked source inventory before and after a locked
Cargo build, rejecting concurrent source/revision changes. `sources.json` retains
the individual hashes, revision, dirty flag, build command and selected compiler
environment inputs. `build.json` binds that inventory digest, profile, `rustc -vV`
and copied executable SHA-256. The executable hashes itself before issuing work;
the independent checker hashes the retained binary again. This records provenance
from a trusted local builder, not compiler attestation. Use `--expected-revision`
on the checker or E2E script to require a particular clean commit.

Create a fresh Raw keyspace with an existing authorized admin client first and
substitute its ID/name and all configured replica addresses. Example configuration:

```json
{
  "version": 1,
  "client": {
    "version": 1,
    "peers": [
      {"node_id": 1, "address": "127.0.0.1:24001"},
      {"node_id": 2, "address": "127.0.0.1:24002"},
      {"node_id": 3, "address": "127.0.0.1:24003"}
    ],
    "keyspace_id": 1,
    "epoch_conf_ver": 1,
    "epoch_version": 1,
    "max_in_flight": 4,
    "max_attempts": 6,
    "deadline_ms": 1500,
    "retry_backoff_ms": 5
  },
  "mode": "correctness",
  "run_id": "trial-001",
  "keyspace_name": "trial-001",
  "seed": 40,
  "workers": 4,
  "keys": 8,
  "value_bytes": 128,
  "mix": {"get": 50, "put": 40, "delete": 10},
  "warmup_operations": 32,
  "max_operations": 2000,
  "measure_ms": 10000,
  "interval_ms": 10,
  "history_bytes": 67108864
}
```

See [WORKLOAD-CORE.md](WORKLOAD-CORE.md) for resource limits. Performance-only mode
requires `history_bytes: 0` and never claims a complete checked history. Seeds
determine the generated nonce stream; concurrent scheduling can reorder its
invocations. Initialization and traffic use disjoint nonce ranges. Dataset size
is bounded independently of operation count and remains below the current
checkpoint boundary.

## Lifecycle and stop semantics

1. Initialization performs bounded distinct reads to find an available endpoint
   and establish an absent sentinel. It checks absence and acknowledges one write
   for each mutable key, then writes the immutable sentinel. Any unacknowledged
   initialization write fails the experiment without replaying it.
2. Sequential warmup uses the same channels and requires acknowledged outcomes.
   Setup has a 300-second issuance budget; an issued call still gets its one
   configured logical deadline. Setup and warmup are outside measured metrics.
3. Workers run closed loops with an optional per-worker interval. The first
   duration, operation-budget or explicit stop-file observation ends admission
   of new work. Operation slots reserve all final reads before measurement.
   The configured duration is an issuance budget, not a hard process timeout.
4. All worker tasks drain. A worker already between its stop check and invocation
   may record an invocation during drain; it remains in the same measured cohort.
   Unknown writes retain their invocation, terminal result and attempt evidence.
   No stop condition implies backend cancellation or permits ambiguous replay.
5. Final verification reads the sentinel and every mutable key. Each key permits
   at most one distinct read per configured peer, allowing routing past unavailable
   endpoints. Every read is recorded. The 60-second verification issuance budget
   cannot turn an unfinished read into success. Mutable-key legality comes from
   the full independent history, and the immutable sentinel must match exactly.

`--stop-file PATH` provides explicit graceful stopping. It is observed before
each new measured call and by a 100-ms coordinator. `--phase-file PATH` reads at
most 64 bytes per operation and accepts only the fixed measurement/fault labels;
it cannot relabel traffic as initialization, warmup or verification. Publish phase
changes with an atomic rename. `ready.json` marks acknowledged setup completion;
`progress.json` reports bounded counts and phase successes while measuring and
draining. The coordinator flushes/syncs a progress replacement every 100 ms when
active. That filesystem overhead is part of this runner and must be measured in
client-capacity experiments.

Unrecoverable I/O errors, worker failure, protocol violations, missing terminal
records, zero measured work and unsuccessful final reads invalidate the run.
Completed failure reports set `complete=false`; an earlier artifact error or
SIGKILL can leave no final report. Missing reports are failures. SIGKILL does not
drain and is never represented as complete. The runner is an external client;
its loss does not supply or remove a database quorum member or mandatory proxy.

## Report contract and independent acceptance

`report.json` version 1 is capped at 2 MiB. It retains canonical configuration and
build hashes, optional full-history hash, actual runtime worker count, monotonic
lifecycle spans, a wall/monotonic anchor pair, stop reason/time, operation and
attempt counts, phase-separated latency histograms and recorder overhead. All
history and lifecycle timestamps share one recorder epoch. The wall anchor is
correlation evidence; it is not a synchronized distributed clock.

The measured numerator includes every terminal operation in the measured/fault
phases, including failures and completions during drain. The denominator is
`drain.end_ns - measurement.start_ns`. `cohort_terminal_ops_per_second` counts all
terminal outcomes; `cohort_success_ops_per_second` counts acknowledgements only.
Neither includes setup, warmup or final reads. These are closed-loop cohort
rates, not open-loop arrival rates, corrected coordinated-omission estimates or
steady-state server-capacity claims. The configured operation cap may stop a
trial before its requested duration; `stop.reason` records that fact.

Linux `/proc/self/stat` CPU ticks and `/proc/self/status` current/peak RSS are
captured before setup and after verification. RSS fields are approximate Linux
accounting samples, including `VmHWM`; they are not checked as monotonic counters.
The two raw samples are retained even if the reported high-water value decreases.
See [BENCHMARKS.md](BENCHMARKS.md) for the observed case and kernel documentation.
Unsupported resource readings are null. They describe the process, including
transport/runtime/recorder overhead;
the E2E harness retains the runtime system's `SC_CLK_TCK` for interpreting ticks.
They are not per-measurement CPU percentages. Peak in-flight operations and total
recorder-call time expose additional client-side pressure. Recorder time includes
lock waits and can overlap across workers.

The separate Python validator imports no kv9 implementation. It rejects duplicate
JSON fields and bounded-input violations, verifies hashes and lifecycle/accounting
relations, checks histogram buckets/extrema/percentile intervals and recomputes
cohort rates. In correctness mode it requires one invocation and one terminal
per ID, no overlapping calls by one worker, bounded observed concurrency, valid
attempt routing, justified refusals and positive write receipts. It reconstructs
all logical/attempt counters and histograms from the full retained events, checks
the initial dataset and final sentinel, and invokes the existing independent
history model with a separately replayed witness. An inconclusive search fails.
Performance-only mode checks artifact/internal accounting consistency and clearly
reports `full_history_independently_checked=false`; it cannot reconstruct absent
samples or establish full-history correctness.

## Verification and remaining acceptance

```sh
cargo build --locked --bin kv9
python3 scripts/workload-e2e.py --output /tmp/kv9-workload-e2e \
  --build /tmp/kv9-workload-build --server target/debug/kv9
python3 scripts/check-workload-report-controls.py --e2e /tmp/kv9-workload-e2e \
  --build /tmp/kv9-workload-build --output /tmp/kv9-workload-report-controls
python3 scripts/check-workload-drain-control.py --output /tmp/kv9-workload-drain-control
```

The E2E script owns its MinIO container and three local database processes,
uses the pinned MinIO digest, and removes only those processes/container. It
checks complete correctness history, performance-only graceful stop, a killed
initial seed, invalid phase rejection and independent database progress after
generator/collector SIGKILL. Final status and remote checkpoint presence are
retained. This is a functional single-host fixture, not Chaos Mesh evidence or
a multi-host performance result.

The functional E2E uses a 100-ms refusal backoff, six attempts and the same
1,500-ms absolute logical deadline. This leaves 500 ms between the first and last
possible attempts instead of consuming the hop budget in a few tens of
milliseconds. It is a fixture configuration, not an election-time guarantee:
repeated refusals, unknown writes and unacknowledged warmup still fail the run.
The benchmark protocol retains its separately declared 5-ms backoff; previously
published measurements are unchanged. Progress waits check the specific workload
process before accepting a file, so an exited generator is reported immediately.

When warmup returns a non-success call report, it writes bounded `warmup-failure.json`, including
its typed outcome, stop reason, elapsed time and each attempted node/refusal hint.
This diagnostic is retained in performance mode without enabling a full history.
It contains no request keys/values, token or arbitrary server error text. The run
remains `complete=false`; the diagnostic does not make failed evidence acceptable.

An additional real-gRPC test holds an already-applied write's reply, requests
stop, and requires the runner to remain draining until the reply is released as
unknown. An isolated source mutation that aborts tasks at stop must compile,
select exactly this test, fail its behavioral assertion and pass after restoration.
Twelve artifact controls corrupt terminals, counts, phase populations, raw
durations, throughput, drain denominator, provenance, report presence, unknown
classification, sentinel observations and performance history claims. Every
original run is independently accepted again after the isolated corruptions.

CI retains the executable/build inventory, raw E2E reports/histories, independent
witnesses, corrupt artifacts and drain control logs. The persistent workload also
runs in the actual [19-window Chaos Mesh gate](PERSISTENT-CHAOS.md). #40 was accepted
at `ef6e90e` after all exact-revision CI/Correctness gates and independent artifact
checks passed, including the [measurement matrix and client calibration](BENCHMARKS.md).
The broader #13 work and #41 scheduling investigation remain open. Existing client
protocol proofs and database proof/Chaos gates remain required for later changes.
