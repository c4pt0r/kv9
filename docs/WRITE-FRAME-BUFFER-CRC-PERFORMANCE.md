# Single-buffer Raft frames: measured write results

Measured 2026-09-14 UTC. **Keep the integrated CRC baseline.** The single-buffer
candidate does not establish a general write improvement: loaded point Put
changes **-0.517%**, while BatchPut(64) changes **+0.379%** with a worse pooled
p99. Both loaded workloads reverse throughput direction between the two orders.
Candidate `e9249f2` remains experimental and is not promoted.

All eight two-second smokes and sixteen ten-second timed cohorts pass independent
acceptance. All **7,247,954 measured calls** succeed on one attempt, covering
**65,011,016 input items**, with zero errors, unknown writes or dropped slots.
The separate smokes contain 810,022 successful calls and 8,246,542 input items.
No cohort was repeated or omitted.

## Throughput and whole-call latency

The control is the original integrated CRC main executable at `bd42e60`; the
candidate is the single-buffer version at `e9249f2`. Each row pools the two
opposite complete orders. Calls/items per second divide summed successful
counts by summed elapsed time. Means use integer latency sums and population
counts; p99 intervals come from merged raw histograms. Batch latency covers
the entire 64-item call.

| c | API | Role | Calls/s | Items/s | Mean us | p99 us |
| ---: | --- | --- | ---: | ---: | ---: | --- |
| 1 | Put | CRC main | 19,463.837 | 19,463.837 | 51.277 | 65.536–66.559 |
| 1 | Put | Frame buffer | 19,424.073 | 19,424.073 | 51.382 | 65.536–66.559 |
| 64 | Put | CRC main | 139,188.639 | 139,188.639 | 459.683 | 737.280–745.471 |
| 64 | Put | Frame buffer | 138,469.020 | 138,469.020 | 462.072 | 745.472–753.663 |
| 1 | BatchPut(64) | CRC main | 6,217.406 | 397,914.000 | 160.734 | 204.800–206.847 |
| 1 | BatchPut(64) | Frame buffer | 6,252.179 | 400,139.454 | 159.832 | 202.752–204.799 |
| 64 | BatchPut(64) | CRC main | 16,651.252 | 1,065,680.142 | 3,842.594 | 6,946.816–7,012.351 |
| 64 | BatchPut(64) | Frame buffer | 16,714.393 | 1,069,721.142 | 3,827.746 | 7,143.424–7,208.959 |

Loaded point throughput changes **-1.799% / +0.786%** in the forward/reverse
orders; loaded batch throughput changes **+2.321% / -1.544%**. Their mean and
p99 directions also reverse. Pooling does not erase the worse individual order.
At c1, batch throughput improves 0.559% with lower p99 in both orders, while
point throughput falls 0.204% with the same p99 bucket. This does not support
general selection. Two short runs on a shared host establish neither statistical
significance nor a universal performance ranking.

Loaded three-voter CPU is 3.562 versus 3.558 sampled cores for point Put, and
3.361 versus 3.359 for BatchPut(64). Client rates are approximately 0.431/0.429
and 0.481/0.481 cores respectively. These are process samples over their observed
intervals, not an additive request-latency breakdown or proof of a bottleneck.
Complete per-voter CPU, p50/p95/p99, all outcomes and individual orders are
retained in the [portable results](https://github.com/c4pt0r/kv9/blob/6d52e3cefc0d7ff7f462be5bb3b2069b430eb680/docs/frame-buffer-crc-performance-v1/README.md).

## Configuration and qualification

- CRC main source: `bd42e60f84657e22e36e34924a5c80a08eac623a`; server SHA256
  `106f517c84fabe790ea139f3c18661e14447abd9e257fdd6ba82bfabe6796eb9`.
- Candidate source: `e9249f2cbd069dcdc44312be826a68494cf694db`; server SHA256
  `96bdb92ea90f131c6434033f912193c24531bc17b7bfaf8207e4c46902aa1714`.
- Fixed native v3 client: `0be806d9671e2c50701a64aa7889c8859b7648ba`; SHA256
  `8da9af469f962a938027d1970141bbe4622f7d42b2b795f720e288fb3f8d5957`.

Both roles use clean default releases with the original compiler provenance,
ThinLTO, opt-level 3 and one codegen unit. Three voters share CPUs 2–5, the client
uses 0–1, and helpers/owned background containers use 6–15 and 22–31. Each cohort
uses 4,096 keys plus sentinel, 128-byte values, seed 71, 128 warmup calls and
closed-loop concurrency 1 or 64. The second repetition reverses the complete
eight-row order. The original client, source checkouts and build manifests
remain unchanged.

Raft quorum, WAL synchronization, durable apply and response fences remain
enabled on **volatile tmpfs WAL**. This is a shared-host loopback CPU/protocol
diagnostic, not a disk, power-loss, cross-host or equal-durability Redis panel.
Redis was not rerun. The [three-copy Redis reference](WRITE-REDIS3-BASELINE.md)
retains its own WAIT 1/2 and persistence interpretation. These are new exact-main
write measurements; the [earlier e748 full read/write matrix](WRITE-CRC-FULL-REGRESSION-PERFORMANCE.md)
remains a separate campaign, including the latest full read/mixed results.

The candidate's [correctness qualification](WRITE-FRAME-BUFFER-CRC-QUALIFICATION.md)
passes 790 tests/doctests, frame/CRC proofs, ordinary recovery and actual 21-window
Chaos Mesh acceptance. The [published correctness evidence](https://github.com/c4pt0r/kv9/blob/fdebb0bd6fba798c67b2d5e49e500238ac0c2aad/docs/frame-crc-correctness-v1/README.md)
preserves the original histories and cleanup chronology. The performance run
does not add another Chaos history or establish read/mixed regression acceptance.

## Independent acceptance and storage

Runtime preparation passes 28 controls; reporting passes five inherited arithmetic
controls. Actual timing session **96921** ends at **e49a51/0**, independent audit
**54774** at **5c1dec/0**, and reporting at direct **821d81/0**. The audit verifies
64 timed and 32 smoke lifetimes exited, 48 fresh timed drains and writer/listener
bindings, 2,306 role/source checks, 3,070 resource samples and exact environment
restoration.

All **74,191,158,465 logical WAL bytes** are independently decoded and hash-checked:
65,811,419,545 timed plus 8,379,738,920 smoke. Combined retained allocation is
**52,317,179,904 bytes**. Compression starts after each cohort's writers exit
and finishes before the next measured cohort. No build, profiler or unrelated
codec overlaps timing. Original retention caps and runtime floors remain intact.

Before this campaign, precise removal of inactive Cargo build artifacts increased
available space by **59,290,816,512 bytes**, from 114,679,197,696 to 173,970,014,208.
Only affected compiler fingerprints were retired; their metadata was retained.
Protected KV9 executables remained hash-identical. Sources and prior runtime
evidence were preserved. Fresh smoke availability 173,969,911,808 exceeded the
170,152,267,776-byte empirical whole-campaign reservation. After smoke, fresh
availability 168,015,204,352 exceeded the remaining 164,764,512,256-byte reservation.
These reservations retain the 96 GiB floor, 16 GiB restoration reserve and 1 GiB
margin; they are empirical sizing, not worst-case fit guarantees.

The portable performance package contains 2,291 files / 319,523,066 decoded bytes
in 19 parts totaling 38,290,587 compressed bytes. Original performance WALs,
compressed objects and executables remain local with their inventory references.
Package **7627/9d19e5/0** and independent portable readback **d4d29f/0** pass.

## Next implementation decision

Keep CRC main as the runtime baseline and retain this experiment for comparison.
Do not spend a full regression campaign promoting this screen's small, unstable
loaded changes. The earlier vectored and owned-buffer experiments also remain
under their original measured conclusions.

Next collect bounded CPU attribution on the exact current CRC main release.
The retained post-CRC profile used the older byte-table implementation; it does
not describe today's slicing-by-eight/ThinLTO binary. Reuse its qualified active
prefix, point/batch c64 protocol, sample coverage, zero-loss and independent
decoding requirements. Preserve Raft synchronization and success fences. Select
the next implementation from current instruction/task/ownership costs, with c1
latency and loaded tails as well as throughput. Instrumented recordings remain
separate from these timing results. Dynamic multi-Raft and automatic splits
follow the write phase under the [existing development order](WRITE-PERFORMANCE-NEXT.md).

All work ran locally. No hosted CI was dispatched and no original industrial
work-package checkbox closes for this experiment.
