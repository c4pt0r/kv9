# Read admission credit on CRC: full workload comparison

Tracking: #9, #13 and #20. **Keep the read-credit candidate experimental.** Its c64 GET throughput improves 8.052%, with lower mean and p99 in both repetitions. Isolated GET latency barely changes, and both point and batch mixed workloads have worse c64 p99 in both repetitions. Redis-class read performance remains open.

This is the first accepted full recording of clean candidate `de37c71009e8199859b931e0037818f490e8d3f6` against selected CRC `ca0002c7f8e9ee6f595efcc9f4151085ccce87cb`. Both measurement clients are fixed `0be806d9671e2c50701a64aa7889c8859b7648ba`. The current main branch also contains the build-tool fix `86a689c8b13ae929a2abff2ef8c5794b50ebd639`; that commit changes no database runtime code. Historical `57ff6851` results are not substituted for this candidate.

## Read results

The following are pooled values from both original repetitions. QPS means successful calls per second. BatchGet processes 64 keys per call. Each p99 is the interval containing the quantile in the retained raw histogram, in microseconds; it is not an averaged percentile or a confidence interval.

| Concurrency | API | Version | Calls/s | Keys/s | Mean µs | p99 µs |
| ---: | --- | --- | ---: | ---: | ---: | --- |
| 1 | GET | CRC control | 26,461.156 | 26,461.156 | 37.675545 | 50.176–50.687 |
| 1 | GET | Read-credit candidate | 26,549.591 | 26,549.591 | 37.548538 | 49.664–50.175 |
| 1 | GET | Redis reference | 173,925.004 | 173,925.004 | 5.672621 | 7.232–7.295 |
| 1 | BatchGet(64) | CRC control | 9,664.044 | 618,498.838 | 102.579477 | 130.048–131.071 |
| 1 | BatchGet(64) | Read-credit candidate | 9,623.916 | 615,930.640 | 103.031283 | 131.072–133.119 |
| 1 | MGET(64) | Redis reference | 39,152.163 | 2,505,738.431 | 24.390354 | 28.928–29.183 |
| 64 | GET | CRC control | 344,473.355 | 344,473.355 | 185.664629 | 352.256–356.351 |
| 64 | GET | Read-credit candidate | 372,211.456 | 372,211.456 | 171.819511 | 294.912–299.007 |
| 64 | GET | Redis reference | 513,377.577 | 513,377.577 | 124.555701 | 229.376–231.423 |
| 64 | BatchGet(64) | CRC control | 35,299.670 | 2,259,178.874 | 1,807.797171 | 3,080.192–3,112.959 |
| 64 | BatchGet(64) | Read-credit candidate | 35,887.756 | 2,296,816.375 | 1,778.094738 | 3,014.656–3,047.423 |
| 64 | MGET(64) | Redis reference | 93,042.081 | 5,954,693.193 | 685.840829 | 1,032.192–1,040.383 |

At c64, GET rises from **344,473 to 372,211 calls/s**. Redis records **513,378 calls/s**, leaving a **1.379x throughput gap**. Candidate mean is **171.819511 µs**, versus CRC 185.664629 µs and Redis 124.555701 µs. The two paired throughput improvements are **8.285% and 7.819%**; both p99 intervals improve.

At c1, candidate GET mean is **37.548538 µs**, versus Redis **5.672621 µs**, a **6.619x mean-latency gap**. Pooled candidate throughput improves only 0.334%; the individual throughput changes are +0.843% and -0.172%. Both p99 intervals improve, but this recording does not establish a meaningful isolated-request throughput gain.

BatchGet(64) at c64 reaches **2,296,816 keys/s**, up **1.666%**, versus Redis MGET at **5,954,693 keys/s**. Its **1.778095 ms** mean is for the whole call. C1 batch-read throughput instead falls 0.415% across the two pooled repetitions.

## Full workload tradeoffs

All workloads and both repetitions remain in the [complete quantitative readout](read-credit-crc-performance-v1/READOUT.md) and [publication bundle](read-credit-crc-performance-v1/README.md). The table below summarizes candidate versus CRC; positive throughput is better and negative mean latency is better. `r000`, `r050` and `r100` mean 0%, 50% and 100% reads.

| Concurrency | Workload | Pooled QPS change | Repeat 0 / 1 QPS change | Mean change | CRC p99 µs | Candidate p99 µs |
| ---: | --- | ---: | --- | ---: | --- | --- |
| 1 | point-r000 | +0.558% | +0.734% / +0.384% | -0.555% | 78.848–79.871 | 77.824–78.847 |
| 1 | point-r050 | +0.253% | +0.180% / +0.326% | -0.253% | 74.752–75.775 | 74.752–75.775 |
| 1 | point-r100 | +0.334% | +0.843% / -0.172% | -0.337% | 50.176–50.687 | 49.664–50.175 |
| 1 | batch64-r000 | +0.141% | -0.320% / +0.603% | -0.140% | 233.472–235.519 | 231.424–233.471 |
| 1 | batch64-r050 | -0.816% | -1.164% / -0.468% | +0.825% | 227.328–229.375 | 229.376–231.423 |
| 1 | batch64-r100 | -0.415% | -0.355% / -0.475% | +0.440% | 130.048–131.071 | 131.072–133.119 |
| 64 | point-r000 | -0.328% | -1.305% / +0.645% | +0.330% | 819.200–827.391 | 819.200–827.391 |
| 64 | point-r050 | +1.980% | +1.799% / +2.161% | -1.941% | 606.208–614.399 | 622.592–630.783 |
| 64 | point-r100 | +8.052% | +8.285% / +7.819% | -7.457% | 352.256–356.351 | 294.912–299.007 |
| 64 | batch64-r000 | -0.751% | +0.334% / -1.828% | +0.754% | 8,650.752–8,781.823 | 8,912.896–9,043.967 |
| 64 | batch64-r050 | +0.116% | +1.101% / -0.859% | -0.118% | 5,636.096–5,701.631 | 5,832.704–5,898.239 |
| 64 | batch64-r100 | +1.666% | +1.477% / +1.854% | -1.643% | 3,080.192–3,112.959 | 3,014.656–3,047.423 |

C64 point mixed throughput improves **1.980%**, but p99 moves from **606.208–614.399 µs** to **622.592–630.783 µs**, worsening in both repetitions. C64 batch mixed throughput changes only **+0.116%**, while p99 also worsens in both repetitions. C1 batch mixed throughput falls **0.816%**. These tradeoffs prevent general promotion based solely on the pure-read gain.

The mixed-load operation breakdown also matters for the read priority: GET gets slower while PUT gets faster, even where the combined mean improves. The following c64 values pool each operation independently; they do not mix read and write latency populations.

| Mixed workload operation | CRC mean µs | Candidate mean µs | Mean change | CRC p99 µs | Candidate p99 µs |
| --- | ---: | ---: | ---: | --- | --- |
| get | 385.141680 | 402.656358 | +4.548% | 622.592–630.783 | 647.168–655.359 |
| put | 360.995980 | 329.054944 | -8.848% | 589.824–598.015 | 540.672–548.863 |
| batch_get | 3,264.467384 | 3,379.250309 | +3.516% | 5,832.704–5,898.239 | 6,094.848–6,160.383 |
| batch_put | 2,939.606702 | 2,816.863467 | -4.175% | 5,373.952–5,439.487 | 5,308.416–5,373.951 |

Point GET mean in mixed load increases **4.548%** and batch GET mean increases **3.516%**. Admission/confirmation waiting is a hypothesis to trace, not a CPU or latency attribution established by these aggregate reports.

C64 point PUT is **123,904 calls/s** versus CRC **124,313** and Redis SET **498,050**. BatchPut(64) is **863,677 keys/s** versus CRC **870,213** and Redis MSET **6,080,939**. KV9 uses three-voter quorum and sync calls on **tmpfs WAL**; Redis is standalone with persistence disabled. These are memory-path diagnostics, not physical-disk durable-write measurements or equal-durability comparisons. Replication and software work remain in this measurement; physical-disk latency cannot explain its entire write gap.

## Recording and acceptance

- **72 timed cohorts**: c1/c64 × point/batch64 × read/write/mixed × CRC/candidate/Redis × two repetitions. Each cohort measures 10 seconds; the second repetition reverses the entire first 36-cohort order. A separate 36-cohort smoke passed first.
- Closed loop, 4,096 keys plus a sentinel, 128-byte values, seed 71, 128 warmup calls, and the original request/deadline/admission limits. Tonic streaming remains selected. No pipelining is enabled for the Redis reference.
- Measurement clients use CPUs 0–1, servers 2–5, and helpers/owned containers 6–15,22–31. This is a shared-host diagnostic, not an exclusive-host capacity or cross-host NIC measurement. No builds, tests, faults, profiles or audits ran during timing.
- **81,041,621 measured calls**, all successful in one attempt, with zero refused calls, unknown writes, read failures, client rejections or dropped slots. Initialization separately records **48 extra NotLeader attempts**, all retained; measurement has none.
- All **240 recorded process lifetimes** exited. The audit verifies **144 qualifying drains**, **144 voter/writer/listener bindings**, **2,352 source-file checks**, resource observations and exact restoration of the three owned containers.
- The original local runtime inventory contains **4,311 files / 94,130,468,368 bytes**, including retained WAL evidence. The repository bundle publishes reporting inputs and their original-byte mappings; its verifier recomputes statistics, not the entire runtime audit.

Terminal runtime: root session **34148**, exit 0, receipt **4fa070**. Corrected independent audit: session **34565**, exit 0, receipt **d72289**. Statistics: exit 0, receipt **e57c20**. All validation was local; no hosted workflow was dispatched.

## Preserved build and audit corrections

The first candidate release incorrectly reused the preceding slicing candidate's executable through the shared Cargo cache. It was excluded **before any candidate runtime or benchmark**. Root explicitly invalidated first-party release artifacts and rebuilt every project library and executable. The measured candidate server is the corrected retained binary with SHA-256 `5464adee210f7cebbce7cd57d6e01bfe5a1ace42936edad24f57e63ff9cfbdd7`; its original build manifest is `164b7999386b042ad2df814fea44fe93724ae15396e9f0cae0356a926fc3d1de`. The [build helper fix](https://github.com/c4pt0r/kv9/blob/86a689c8b13ae929a2abff2ef8c5794b50ebd639/docs/BUILD-CACHE-SAFETY.md) now prevents this cooperating-builder cache reuse.

The first frozen auditor failed before per-cohort checking because it retained the preceding experiment's arguments hash `6f1497da…`. Actual argv, wrapper and driver matched the reviewed configuration. The correct arguments hash, `163b9cb4f137dee027e361f6060f15fe86af9fb6bc8cc887e0ce11020a424a83`, was already bound by the pre-run readiness and frozen inventory. The original failure remains retained. A separate auditor changes **only that one hash literal**, with all 15 existing contracts passing and a new static check against the pre-run pin. No runtime result was rerun or changed. Corrected auditor SHA-256 is `9a549e1fb45f703e5bde12914211b46240f70d3b9df0b4185c60711fb8ebeb94`; statistics changes only its audit/gate paths, retaining the original arithmetic core.

## Decision and next development step

Keep CRC as the selected runtime behavior and preserve `de37c71` as a read experiment with scoped proofs and source/process tests. Its current-source gates pass 716 workspace tests/doctests (23 ignored), formatting, warnings-denied Clippy, 14 compiled semantic-control triples, and the unchanged 37-case formal gate with 28 distinct theorems / 265 baseline obligations. Those proofs cover the stated admission abstraction, not full Rust/Raft composition. Ordinary three-voter recovery passes; no exact-source Chaos Mesh acceptance is claimed for this reapplication.

The next read experiment should shorten an identified confirmation-path handoff while preserving fresh quorum authorization, sealed admission, cancellation bounds and apply-before-read. The present change mainly amortizes quorum work under concurrency; it does not remove the isolated GET turnaround gap. Use the existing [read lifecycle evidence](LOW-CONCURRENCY-READ-LIFECYCLE.md) to choose one bounded change, qualify correctness and fault behavior, then compare c1 and mixed tails alongside loaded GET throughput.

Durable writes should be evaluated against equivalent persistence and quorum requirements. Continue measured batching/pipelining and CPU improvements, while prioritizing the remaining read-path latency. Dynamic multi-Raft and automatic splits remain the next product sequence after the read-performance milestone; this recording does not close that milestone.
