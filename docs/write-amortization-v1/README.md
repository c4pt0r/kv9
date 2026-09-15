# Existing write batching: event accounting

This 2026-09-15 offline analysis uses the original before/after metrics from the
[accepted CRC CPU diagnostic](../WRITE-CRC-MAIN-CPU-PROFILE.md). It executes no
database workload, profiler or codec and supplies no new QPS result.

| Workload | Voter | Command apply events | Engine WAL write events | Engine WAL sync events | Commands / engine write event |
| --- | ---: | ---: | ---: | ---: | ---: |
| Point Put | 1 | 653,662 | 43,656 | 43,662 | 14.973 |
| Point Put | 2 | 653,662 | 51,395 | 51,401 | 12.718 |
| Point Put | 3 | 653,662 | 42,426 | 42,432 | 15.407 |
| BatchPut(64) | 1 | 80,870 | 4,272 | 4,323 | 18.930 |
| BatchPut(64) | 2 | 80,870 | 5,099 | 5,150 | 15.860 |
| BatchPut(64) | 3 | 80,870 | 4,201 | 4,252 | 19.250 |

These are ratios of successful event-count differences over the full original
capture intervals, including setup and warmup. They are not measurements over
only the nominal five-second workload. A BatchPut call contributes one command,
not 64 command events. Metric snapshots are coherent individually, not one
simultaneous snapshot across all metrics.

The ratio is **not a Raw apply-group average or histogram**. Engine write events
also include segment-header writes; sync events additionally include segment
sealing. An append event encloses three `write_all` calls, so write-event counts
are not syscall counts either. The source already groups an eligible committed Raw prefix and sends
one composed batch to `write_applied`. These counts support investigating the
existing amortization, rather than assuming one engine-WAL synchronization per
logical write or adding a second implementation of group commit.

The command-apply timer has another important interpretation: the driver starts
one timer for each command before applying the group, then finishes every timer
after the shared apply succeeds. These command intervals overlap. Their duration
sums must not be interpreted as exclusive CPU time or exclusive engine wall time.
This behavior is intentional per-command latency accounting, not evidence that
the shared work ran repeatedly.

Source mappings are pinned in `provenance.json`: the selected `bd42e60` driver,
Raw group implementation, segmented engine WAL and monotonic timer match the current files
byte-for-byte. In particular, see the driver group/timer loop, the single
`engine.write_applied` call in `apply_raw_group`, and the writer's `create`,
`append` and `seal` metric sites.

`result.json` retains all six rows, input SHA-256 identities and interpretation
limits. Reproduce the arithmetic, with the original local input files present:

```sh
env PYTHONDONTWRITEBYTECODE=1 PYTHONOPTIMIZE=0 python3 -B docs/write-amortization-v1/derive.py \
  --profile-root /tmp/kv9-crc-main-current-profile-preparation-20260914-first \
  --output /tmp/kv9-write-amortization-recomputed.json
```

The output path must be absent. The script checks process/exporter continuity,
schema, capture order, valid and unsaturated counters, bucket/count consistency,
monotonic deltas and unchanged non-success/export-failure counts. Original metric
payloads remain in the accepted diagnostic evidence; this small report does not
duplicate them. All four input identities match the
[accepted portable inventory](https://github.com/c4pt0r/kv9/blob/13cdb54175e170eed6493d4187dab746a95c9ee8/docs/crc-main-current-cpu-v1/inventory.json),
SHA-256 `fa32fdc7d5894129c396fee58a7b44bdcaf0af74417043347d100a0c970befe3`.
Actual analysis terminal: `7a4a9c/0`; independent six-row arithmetic and source
review: `5b8699/0`. The original runtime/perf decoder and separate portable-byte
readback remain distinct, as recorded in the provenance.

No duplicate-key rate, group-size distribution, queue age or speedup follows from
these counters. Those would need bounded observations of actual groups before
justifying a new grouping or map algorithm. The receipt candidate's unchanged
matched screen remains the next runtime step after capacity recovery. Existing
rejected owned-buffer and worker experiments remain rejected.
