# Multi-group throughput: single-host baseline and bottleneck ledger

Updated 2026-09-16. D01 [#22](https://github.com/c4pt0r/kv9/issues/22) and
C03 [#13](https://github.com/c4pt0r/kv9/issues/13), following `eddb716`.

This increment demonstrates that the multi-Raft runtime converts added data
groups into added throughput on fixed hardware, and it names every bound it
measured on the way. All numbers come from one host (32 cores, 123 GiB, three
voter processes, tmpfs stores unless stated): **a development diagnostic under
the [benchmark contract](HORIZONTAL-SCALING-PLAN.md), never 3/6/9-host scaling
evidence.** Same-CPU processes are not added nodes; no online expansion, no
p99-budget sustained acceptance, no durability-panel improvement is claimed.

## Harness

[`scripts/multi-group-sweep.py`](../scripts/multi-group-sweep.py) runs one
fresh three-voter cluster per cell, binds G data groups to G fresh Raw
keyspaces (waiting for activation, published routes and a stable leader set),
then drives G concurrent single-keyspace `kv9-workload` processes in
performance mode. Per-run reports keep the existing qualified format; the
cell aggregates rates over per-run measured windows and reports the window
overlap and skew rather than pretending alignment. Per-thread server CPU is
sampled around measurement. A failed cell is recorded and the matrix
continues; failed stores and failure-moment probes are retained.

The `data_workers` node option (default 2, bounded 1..=32, `--data-workers`)
now sizes the shared data-group driver pool that was previously fixed at two.

## Measured scaling (30 s cells, 128 B values, 2048 keys/group, two reps)

Equal per-group load (32 workers/group), admission raised to 1024/node:

| groups | put ops/s | S(g) | success p99 |
| --- | --- | --- | --- |
| 1 | 126,093 (124.3k/127.9k) | 1.00 | 0.5–1.0 ms |
| 2 | 203,566 (203.3k/203.9k) | 1.61 | 0.5–1.0 ms |
| 4 | 270,413 (272.7k/268.1k) | 2.14 | 1–2 ms |
| 8 | 292,167 (272.1k/312.2k) | 2.32 | 2–4 ms |

Fixed aggregate load (64 workers split across groups), default admission,
S(g) peaking at four groups: put 148.5k→202.9k (g2)→177.5k; get
218.1k→318.4k (g4, S=1.46); mixed 163.5k→223.7k (g4, S=1.37). The
single-group put baseline matches the retained 137.9k/s measurement within
tooling variance, validating the harness against prior methodology.

## Bottleneck ledger (each entry evidence-backed, none assumed)

1. **Public admission budget binds first.** Default
   `KV9_PUBLIC_MAX_REQUESTS=64` per node collapsed the 8-group equal-load
   cell (0.55 M refusals in 30 s beside 0.43 M successes); raising it to
   1024 restored clean scaling. Multi-group deployments must budget
   admission with offered concurrency.
2. **Per-group pipeline depth drives batching.** Holding aggregate
   concurrency fixed while adding groups thins each group's pipeline
   (S regresses past four groups at 64 total workers); equal per-group load
   keeps scaling.
3. **Two shared data workers are the right default.** At every measured
   point — under and at saturation (88% worker utilization at
   4 groups × 32) — more workers reduced throughput: at g8/aggregate-64,
   dw1/2/4/8 gave 190.0k/181.3k/161.7k/127.8k; at g8×32/1024-admission,
   dw1/2/4/8 gave 280.0k/292.2k/265.2k/202.1k. Larger pools shrink per-turn
   batches and add wakeups without adding useful parallelism here.
4. **Sixteen full-rate groups exceed this fixture's startup envelope.** All
   six g16 panels failed a sentinel read during fleet initialization,
   insensitive to workers (1–8), admission (64–1024) and 250 ms launch
   stagger. Failure-moment forensics show a healthy cluster: stable terms
   and leaders across all sixteen groups and clean reads seconds later. The
   unlucky group's ReadIndex barrier missed the 2 s deadline during the
   burst, and the workload driver's strict no-retry-unconfirmed rule
   surfaces that honestly. Follow-ups belong to proposal batching/group
   commit ([#20](https://github.com/c4pt0r/kv9/issues/20)) and a bounded
   setup-probe retry policy, not to hiding the window.
5. **Physical-disk durability remains the dominant real ceiling.** On the
   host's RAID5 HDD array with per-append fsync, one group sustains
   0.86k put/s and two groups 0.73k/s — fsync-bound, recorded as a separate
   panel, never pooled with tmpfs numbers.

## Validation and retained evidence

The [portable packet](multi-group-baseline-v1/README.md) retains every cell
of every round — including the aborted early rounds, the harness defects they
exposed (leader-only routability probing, matrix abort on first failure,
store-path collisions between panels) and all g16 failures with forensics.
The full serial workspace suite, strict Clippy, six rechecked Lean models and
the retention TLA/TLAPS model qualify the source after the `data_workers`
plumbing. No proof models the pool size; contracts re-pin the touched files.

Next mainline work stays unchanged: unified-cut source capture, install
evidence and committed release, runtime installation, learner transitions —
and for throughput, proposal batching (#20) before any durability-panel
claim. The 3/6/9-host contract remains the only acceptance path for real
horizontal scaling.
