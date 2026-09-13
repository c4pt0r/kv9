# CRC slicing8 complete native regression: preparation plan

The existing fixed v3 client and native fixture support the requested scope. Prepare a fresh24-smoke/48-timed campaign comparing selected11113f68 with CRC e748620a, retaining the original binaries. This directory contains a plan and metadata arithmetic only: no runnable derivative, test, source validation, workload, codec or new performance result. Runtime remains blocked by capacity and helper qualification.

| Workload cell | Read API | Write API | Read percentage | Batch size |
| --- | --- | --- | ---: | ---: |
| point-r000 | point_get | point_put | 0 | 1 |
| point-r050 | point_get | point_put | 50 | 1 |
| point-r100 | point_get | point_put | 100 | 1 |
| batch64-r000 | batch_get | batch_put | 0 | 64 |
| batch64-r050 | batch_get | batch_put | 50 | 64 |
| batch64-r100 | batch_get | batch_put | 100 | 64 |

Each cell runs c1 and c64, old then new. Smoke is one full forward list of24 cohorts at2seconds. Timing is the24-cohort forward list followed by its full reverse at10seconds:48 cohorts,24 paired comparisons across both orders and12 pooled comparisons. Preserve4096 mutable keys plus sentinel,128-byte values,seed71,warmup128,max10M calls,closed-loop load,run_id p{repeat}{concurrency:04d}, tonic_stream, max_attempts6,deadline1500ms and retry_backoff5ms. Read50 means the existing deterministic per-call mix, not exactly half the realized calls. Read-only cells still have startup writes and final verification; their retention is not zero.

`protocol-plan.json` records every proposed descriptor, inherited settings and expected accounting:96 smoke plus192 timed exited lifetimes,72 smoke plus144 timed fresh drains,144 timed writer/listener bindings. These are required future counts, not observed results. Redis has no trial or role in this comparison.

The native client remains0be806d9 / SHA8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957. The accepted full72 campaign used these exact client bytes for all six cells. Its ThinLTO server SHA matches selected11113's dd028cb2; that establishes client/artifact compatibility, not new CRC read/mixed acceptance. Current old/new retained binaries and manifests come from the accepted write16 preparation; no build is needed unless root's fresh identity checks fail. This plan only stat-checked executable presence and retains historical hashes.

Minimal environment adaptation:

1. Derive from `/tmp/kv9-write-crc-slicing8-ab-preparation-first`, keeping its native-only role binding, corrected native smoke cleanup schema, allocator provenance, source-aware default Cargo graph checks and original runtime lifecycle. In `matched-driver.py`, change WORKLOAD_CELLS, protocol/path labels, make_plan's8/16 count to24/48, and scope text. `make_plan` already implements the API/mix mapping and full reverse order (lines233–254); `native_trial` and generic validators remain unchanged. Keep the existing source-bound RESP/Redis report helper imports used for API pairing; they do not launch a Redis trial.
2. In `audit.py`, retain independent expected_descriptor, shared_config, validate_phase_apis, dataset_check, operation_statistics, histogram, resource and lifetime predicates. Expand WORKLOADS, all complete-smoke/timed/retention/paired count assertions, exact paths and new helper hashes. Require48 timed cases,24 smokes and both exact orders. Preserve all outcomes and the distinction between valid accounting and healthy one-attempt/all-success comparisons. The historical mixed logic is already present; do not restore obsolete three-role/Redis post-loop assumptions.
3. Derive both `compressed_retention.py` and `retention_audit.py` with only COHORT_ROOTS changed. Preserve every function and limit. Reuse `isolate-and-run.py` with exact new driver/argument bindings; its retained host/container/UID observations must still match before any intervention. Preserve clients CPUs0–1 and all three voters CPUs2–5 for timing; smoke uses the existing helper placement. Retain separate process start/boot/executable identities, outer restoration and post-reap retention between cohorts only.
4. Use `/tmp/kv9-thin-lto-full-performance-statistics-preparation-first/core.py` unchanged for read/write/merged histograms and phase accounting. Its aggregate function handles both operation populations; the newer write16 summarize.py intentionally rejects read_percent!=0 and must not be reused unchanged. Adapt only reporting gates/roles/counts/paths plus the native16 per-process CPU readback and directional bucket labels. Require the new full audit and exact report hashes, report each order and pooled sums/time, separate GET/PUT and BatchGet/BatchPut call/item rates, count-weighted means and merged p50/p95/p99 bounds. Empty populations remain unavailable. No batch latency division by64, averaged percentiles, historical performance pooling or automatic promotion.

Root should qualify only changed planning/audit/reporting closure:24/48 exact order and API mapping; read0/read100 unused-operation refusal; read50 separate populations and final deterministic nonce membership; missing/duplicate/reordered cohort and smoke count refusal; both changed retention root allowlists; operation-specific pooling and empty-population handling. Existing unchanged source, codec/EOF and fixture predicates can reference their accepted contracts. Preserve the latest native-smoke schema repair and all original failures. No contracts were executed here.

Storage is the concrete blocker. `capacity-scenario.json` uses accepted metadata, without reading WAL/object payloads:

| Scenario component | Bytes |
| --- | ---: |
| Current accepted pure-write16 timing +8 smoke compressed objects | 48,501,030,853 |
| Two copies of selected-identical-binary historical read/mixed volumes | 32,541,557,738 |
| Combined empirical compressed volume | 81,042,588,591 |
| Original96GiB retention floor | 103,079,215,104 |
| One16GiB cohort restore/transient reserve +1GiB metadata scenario margin | 18,253,611,008 |
| Required available space for that scenario | 202,375,414,703 |
| Observed available /tmp space | 121,498,157,056 |
| Additional scenario headroom needed | 80,877,257,647 |

The corresponding logical volume is115,712,043,863bytes, leaving21,726,909,609bytes under the unchanged128GiB combined smoke/timing logical cap. The largest current write cohort is10,656,683,223bytes, below16GiB. Neither observation bounds future growth. Faster writes or a different mixed schedule can exceed these values; the10M call cap is not a storage proof. The full72 historical native-only volume was76,153,708,812 compressed bytes, and is kept as a separate historical comparison.

Preserve8GiB/member,16GiB/cohort,128GiB/campaign,600seconds/codec,1800seconds/cohort,64MiB decode memory and8MiB retention metadata. Preserve32GiB tmpfs/96GiB disk preflight,16GiB tmpfs/64GiB measured runtime guards, and the stricter96GiB retention/restore floor and per-member temporary reservation. The1GiB metadata allowance is a planning margin, not proof of maximum reporting allocation. A cap/floor failure remains failed and retained; no narrowed acceptance or automatic retry. Current space is authoritative and already includes all completed archival effects. Zero credit is taken for any unachieved64GiB planning target or hypothetical cache/data deletion.

`prospective-commands.json` gives the existing root smoke/isolation/audit CLI shape with proposed fresh paths and an explicit pending new-driver hash. The referenced runtime helper files deliberately do not exist. Root must first prepare/freeze those finite derivatives, execute relevant controls, revalidate exact source/build/client/tool/host identities, establish full-run capacity, then run all24 fresh smokes and the complete48 timing campaign. Publisher/build/test/proof/audit/codec work must be terminal during measured intervals. Original benchmarks and retained objects stay unchanged.

The result scope remains shared-host volatile tmpfs WAL with normal sync/quorum semantics; no power-loss durability, sustained-capacity or new Chaos inference. CPU uses source-bound observed per-process intervals, not exclusive request service time. RSS/HWM includes setup; voter numbers need not denote the same leader. Complete dataset/sentinel checks retain deterministic bytes and allowed write membership, not an exact concurrent operation history.
