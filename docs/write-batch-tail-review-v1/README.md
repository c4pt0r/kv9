# Loaded BatchPut(64): retained evidence review

This is an offline review of four accepted ten-second timed cohorts and eight
accepted two-second observer cohorts, all at concurrency 64. No experiment,
codec, WAL readback, control replay, build or production edit was performed.
The result supports retaining the CRC selection. It identifies a useful missing
timing boundary; it does not establish the cause of a particular slow call.

## Populations remain separate

| Population | Server sources | Client ELF | Measured duration |
| --- | --- | --- | --- |
| Requalified matched screen | CRC `bd42e60` versus upper-bound `e2e23cc`, both default | separately qualified `1b8060eb...` from `0be806d` | 10 seconds per cohort |
| First observer | `9317e63`, matching default/diagnostic builds of the selected CRC algorithm | historical `8da9af46...` from `0be806d` | 2 seconds per cohort |
| Upper-bound observer | `e2e23cc`, matching default/diagnostic builds | historical `8da9af46...` | 2 seconds per cohort |

The first observer is not a measurement of the held receipt-tail optimization.
Schema 1 has 17 driver distributions; schema 2 adds the actual upper-bound
skip population. Missing schema-1 skip fields cannot be interpreted as observed
zero skips. The default observer arms export no diagnostic distribution.

All use the retained closed-loop BatchPut(64) protocol, 4,096 keys, 128-byte
values, seed 71, 128 warmups, three loopback voters, client CPUs 0–1 and voter
CPUs 2–5. Active WALs are volatile tmpfs. The later matched screen additionally
uses data-volume retention and a root-space observation in both arms. None of
these results measures physical-disk or power-loss performance. Nothing is
pooled across captures, source revisions, client ELFs or durations.

## Batch tail variation

Rates use summed successful items / summed actual elapsed time. Whole-call
means use summed integer nanoseconds / calls. Percentiles merge the original
3,776-bucket native histogram; intervals below are inclusive bounds. No batch
latency is divided by 64 and no percentile is averaged.

| Ten-second order | CRC items/s | Upper-bound items/s | Change | CRC p99 ms | Upper-bound p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| CRC first (ordinals 6,7) | 1,044,746.316 | 1,008,834.336 | -3.4374% | 7.208960–7.274495 | 9.043968–9.175039 |
| Upper-bound first (8,9) | 1,016,710.156 | 1,023,380.194 | +0.6560% | 8.388608–8.519679 | 8.257536–8.323071 |
| Pooled work/time | 1,030,729.153 | 1,016,106.575 | -1.4187% | 7.798784–7.864319 | 8.650752–8.781823 |

Pooled mean increases 3.973→4.030 ms (+1.4474%). The pooled p99 increase is
bounded by +10.0000% to +12.6050%, solely from the two quantile intervals, not
a confidence interval. The first-order p99 increase is +24.3243% to +27.2727%;
the reverse-order change is -3.0769% to -0.7813%. The loaded batch result is
order-sensitive and does not support unconditional promotion.

The two observer comparisons also differ. First-observer diagnostic throughput
changes -1.6702%/+0.4681% by order and p99 worsens in both, with pooled p99
8.192–8.258→8.520–8.651 ms. Upper-bound diagnostic throughput changes
+0.1204%/+0.7719% and p99 improves in both, with pooled p99
8.651–8.782→7.406–7.471 ms. These are separately observed instrumentation
effects on short runs; they neither establish negligible overhead nor explain
the later uninstrumented CRC/candidate contrast.

The matched screen's sampled total voter RSS at the last measurement sample
is 6.400/6.176 GiB (CRC/upper first order) and 6.219/6.256 GiB (CRC/upper reverse
order). There is no consistent candidate RSS increase in these samples. RSS
includes workload-dependent resident state and is not an allocation or pause
trace. All per-voter samples summarized by the accepted reporter remain in
`summary.json`.

## Group formation and queue age

These are diagnostic **whole-capture** deltas for node 2, leader at both
endpoints. Continuous leadership is not proven. All three nodes are retained
in the machine result. Endpoints cover about 7.05 seconds around each 2-second
measurement, including setup/warmup/drains/readback within the endpoints.

| Diagnostic capture / order | Mean commands/apply group | Singleton groups | Mean entries/nonempty persistence Ready | Mean requests/nonempty service | Mean resolved age ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| Schema 1 / default first | 15.1621 | 14.1938% | 6.7130 | 36.5778 | 2.72749 |
| Schema 1 / diagnostic first | 15.1328 | 13.3702% | 7.3627 | 36.8266 | 2.79086 |
| Schema 2 / default first | 14.7515 | 14.5303% | 6.4573 | 36.3191 | 2.64569 |
| Schema 2 / diagnostic first | 14.5622 | 14.0870% | 6.6613 | 36.5577 | 2.69408 |

Apply-group p99 is 32–63 commands in all four leader deltas. Observed maxima
are 64,64,63,64 commands; encoded-byte maxima are 677,824,677,824,667,233,677,824.
The existing limits are 128 commands and 1,048,576 encoded bytes. These captures
do not show those limits being reached and give no basis to raise them or add
a batching delay. A command can represent 64 input items; group counts are not
item counts, unique-key counts or WAL syscall counts. Persistence Ready groups
and committed/application groups are different populations. All four leader
LightReady committed-entry sums are zero.

Terminal-inspection age p99 remains the coarse 4.194304–8.388607 ms bucket in
all four deltas. Inspection-age means are 1.36148/1.39097 ms in schema 1 and
1.30475/1.34352 ms in schema 2. The age clock starts at registration after
proposal and is sampled before inspecting a receipt. A pending request can
contribute repeatedly; resolved age samples only terminal inspections, which
generally include errors/replacements as well as success. It is neither
exclusive queue occupancy nor receipt-search time nor whole RPC latency.

Observed comparisons/lookup are 1,008.689/1,010.085 in schema 1 and
136.939/139.623 in schema 2. All schema-2 recorded misses in these rows are
upper-bound skips; successful hits still average about 996.8 comparisons.
This establishes the logical skipped work, not a CPU-time saving. Similar
group/age scales across these different captures do not establish that the
skip caused the batch behavior. Zero-apply successful turns resolving requests
number only 1/2 in schema 1 and 3/3 in schema 2; the joint table records
co-occurrence and cannot pair a command to a particular resolution.

## What the writer timers actually show

The ten-second cohorts' metric endpoints span 15.095–15.146 seconds, not just
the nominal measurement. The following success populations are node-2 deltas:

| Timed ordinal / role | Command apply mean us | Engine record write mean us | Engine record sync mean us | Namespace publication count / mean us |
| --- | ---: | ---: | ---: | ---: |
| 6 / CRC | 610.225 | 24.491 | 0.165 | 206 / 331.598 |
| 7 / upper | 644.757 | 24.419 | 0.113 | 200 / 947.774 |
| 8 / upper | 611.216 | 26.515 | 0.110 | 202 / 390.912 |
| 9 / CRC | 612.429 | 24.851 | 0.148 | 202 / 638.213 |

Engine record-write p99 is 0.131072–0.262143 ms in all four. Command-apply p99
is 1.048576–2.097151 ms except ordinal 7's 2.097152–4.194303 ms. Namespace
publication p99 is 4.194304–8.388607 ms in all four. The slowest candidate order
therefore has larger command-apply and namespace-publication aggregates despite
nearly identical record-write aggregates. A rare namespace event could lie on
a request's critical path, but these records do not identify such a request.
Tiny tmpfs sync means do not prove physical-storage sync is cheap.

Source boundaries at exact `e2e23cc`:

- `crates/raft/src/driver.rs:398`: `raft_pump_service` wraps `step_inner`.
  Completion notification, `complete_pump`, its transport sends, asynchronous
  receipt service and read completion follow outside that timer. Some earlier
  transport work occurs inside `step_inner`, so total transport time cannot be
  inferred by subtracting timer sums.
- `driver.rs:538`: the first Command decode precedes the applied/state-machine
  locks (lines 553–554); subsequent prefix decoding/group formation holds both.
  Per-command apply timers start only at line 604. They exclude those prior
  stages, overlap for every command in a group, and finish before receipt-ring
  publication at lines 634–635. Their summed durations count shared apply work
  repeatedly and are not exclusive CPU time or wall time.
- `crates/raft/src/state_machine/raw_group.rs:45`: a bounded committed Raw
  prefix composes one engine batch and synchronously waits for `write_applied`.
  `crates/engine/src/persist.rs:594` takes the WAL lock, appends, then updates
  the memory index. An apply interval includes more than record-write time.
- `crates/engine/src/wal_segment.rs:244`: validation, batch encoding, framing
  and CRC occur before the record-write timer. Its body covers the header,
  payload and checksum writes; sync has a separate timer. Segment creation
  also records header writes and syncs. Event-count ratios are not exact
  application-group sizes. Legacy `wal.rs:486` has the same encoding/timing
  distinction. No dedicated writer-thread CPU interval is present.
- `crates/raft/src/rawnode.rs:566`: Ready and LightReady persistence finish
  before queued messages/committed entries are published. The three Ready
  distributions count successful populations, not persistence durations or
  quorum/network latency.
- `crates/raft/src/async_apply.rs:137,197,232`: registration starts the age
  clock; terminal age is sampled before inspection and before sender delivery.
  It excludes proposal submission and subsequent RPC/client completion.

The same source files at CRC `bd42e60` and diagnostic `9317e63` are independently
read from their committed Git objects and hash-pinned in `summary.json` (23
file/revision combinations). This retains exact-source distinctions without
copying or modifying any production source.

## Single next bounded instrumentation question

For a terminally resolved loaded batch, **does the long interval occur before
its committed group's apply starts, inside apply, or between apply completion /
receipt publication and its exact terminal inspection?**

A finite default-off group trace can answer this boundary question: timestamp
entry before the first decode/locks, lock acquisition/group formation, apply
start, apply return and receipt publication; retain exact first/last term-index
and group count, then join terminal-inspection events by exact proposal identity.
Do not infer contiguous command indexes across no-op/barrier entries. Preserve
finite capacity, dropped-event/coverage accounting and observer-overhead
qualification. This is a proposed measurement, not implemented or accepted.

That trace still does not cover proposal-to-Ready/quorum scheduling before
group availability, or inspection-to-sender/RPC/client delivery. It cannot be
joined to a particular native p99 call without corresponding exact receipt
identity and call timestamps. Current endpoint histograms cannot supply that
missing join; their means and quantiles must not be subtracted as a latency
decomposition.

## Evidence and execution

`extract.py` reads only the 12 selected native reports, their before/after
metrics/status maps, accepted analysis/audit authorities and committed source
blobs. It verifies existing exact input SHA/length pins, process/exporter
continuity, monotonic unsaturated integer buckets/counts/sums, and rederives
the loaded native rates/means/quantiles. This is a reporting extraction, not a
replacement campaign audit. All 36 per-node metric deltas and all diagnostic
node populations are retained in `summary.json`; `input-hashes.json` names
68 exact local inputs. It does not access WAL objects, executables or archives.

Accepted native report authority:
`/mnt/data/kv9-work/upper-bound-requalified-preparation-20260915-first/results-first/input-inventory.json`,
bound by final audit SHA
`7f60d63edbfaa3580160b3e80bf5ed97ba01aafe4faeb8e1342a6ba82daedb4f`.
Observer authorities are the `input-hashes.json` adjacent to
`/tmp/kv9-write-diagnostics-analysis-20260915-first/results-first/result.json`
(SHA `11311dee4d8a31ff20dbc18f5d52038296ae2a4677591f2a00d699af81e928de`)
and `/tmp/kv9-upper-bound-observer-preparation-20260915-first/analysis-first/result.json`
(SHA `3cc9e8f56593ca4f58eff29da7640b8b68dfca29b90169ecdb847a73f4a776a9`).

Actual successful extraction: direct tool `c704de/0`, helper CPUs 6–15,22–31,
`PYTHONOPTIMIZE=0`, `PYTHONDONTWRITEBYTECODE=1`, `python3 -B`.
The initial reporting-only attempt `1a6d42/1` incorrectly expected the metrics
`reset` field to be boolean. It is the stable string
`node_component_construction`; `extract-initial.py` and `execution.json`
preserve this error and the exact one-line correction. No data or original
acceptance changed. No output from the failed attempt was overwritten.
