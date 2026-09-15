# Corrected write-stage capture and observer overhead

The corrected collector completes all four planned loaded-batch cohorts. Both
instrumented cohorts have zero recording loss, all first-fresh snapshots are
valid, and independent outcome, final dataset, drain, lifetime and restoration
checks pass. The [original five-loss attempt](WRITE-STAGE-CAPTURE.md) remains
failed and separate. CRC remains selected; this is diagnostic qualification,
not a new selected-runtime speedup or Redis comparison.

## Exact workload and outcome

Both releases use the same 1,396 clean source files at `3f4d4cf`, Rust/Cargo
1.94, opt3, ThinLTO and one codegen unit. The default ELF SHA-256 is
`18d5da4a5aa8855288088374daa59889c71d39c5332eb58bfdea17e1084dda1a`;
the instrumented ELF is
`14c4d61b3e08f34dc1cba55eaee789901dac2d166cee738abdb4bf23a543f698`.
The latter enables both `write-stage-tracing` and `write-path-diagnostics`.
Both use the already qualified native client
`1b8060eb168610328c10a27480de16bd8b2d6805166d8169638f85ceef0992c4`.
Source, compiler command, Cargo feature and executable readback pass.

Each row measures two seconds of BatchPut(64), c64, 128-byte values, 4,096 keys,
seed 71 and 128 warmups over TonicStream with three tmpfs WAL voters. Client
CPUs are 0–1 and voter CPUs 2–5. The host is shared. Existing sync, quorum,
local application, reply, cancellation and deadline rules are unchanged.

| Order | Build | KV items/s | Mean per batch | p99 per batch |
| --- | --- | ---: | ---: | ---: |
| Default first | Default | 1,080,119.940 | 3.789 ms | 7.864–7.930 ms |
| Default first | Instrumented | 1,040,520.591 | 3.933 ms | 7.864–7.930 ms |
| Instrumented first | Instrumented | 1,035,450.244 | 3.952 ms | 7.209–7.274 ms |
| Instrumented first | Default | 1,054,988.208 | 3.878 ms | 7.602–7.668 ms |

Instrumentation changes throughput by **-3.666% / -1.852%** in the two orders,
with mean latency **+3.797% / +1.894%**. Pooling counts over elapsed time gives
1,067,555.349 default and 1,037,985.550 instrumented items/s (**-2.770%**).
p99 remains per order; percentiles are not averaged. This short diagnostic
does not replace the ten-second performance-selection gate or establish
power-loss durability from tmpfs.

All **131,737 measured calls / 8,431,168 items** succeed in one attempt, with no
dropped slot or failed/unknown/rejected outcome. All four independent dataset
scans, 12 fresh drains, 16 owned process exits and 48 raw boundary files pass.
The exact three original container identities and CPU settings are restored.
Original copy/readback receipts retain **8,418,027,444 bytes** on the data
volume before tmpfs scratch removal. The later audit checks those receipts
and file metadata; it does not rehash the database payload.

## What the trace resolves

Each instrumented leader is node 2 and retains 512 unique compatible
term/index pairs. Followers retain sampled application groups and have no
terminal waiter inspections. All six instrumented voter traces report zero
dropped recordings. Their counters and ring contents remain unchanged from
post-client through drain and independent readback.

The rings explicitly overwrite 1,534 and 1,524 earlier sampled rows per populated
ring. The surviving windows are tails, with deterministic index-modulo-16
sampling. No complete request history or client-p99 membership follows. The
before-to-post-client boundary also includes warmup and client verification;
client exit is not the precise measurement-stop edge.

For group-level timing, deduplicate shared `group_sequence` values. The two
leader tails then contain 339 and 346 distinct groups. Groups with no retained
sampled member are absent; deduplication does not make this an unbiased
whole-campaign group distribution.

| Leader group stage | First tail mean | Second tail mean |
| --- | ---: | ---: |
| Preparation start to locks acquired | 2.438 us | 2.269 us |
| Locks acquired to state-machine apply | 47.045 us | 48.793 us |
| State-machine apply | 549.018 us | 546.521 us |
| Apply return to receipt insertion | 3.993 us | 3.995 us |

Separately, over the 512 sampled positions in each tail, receipt insertion to
terminal inspection averages **33.563 / 24.921 us**, and the inspection itself
averages **0.476 / 0.267 us**. Position-weighted group apply averages
632.222 / 626.510 us; these intentionally differ from deduplicated group means.
One first-tail inspection takes 107.160 us, so its mean is not a typical-case
claim. Stage percentiles cannot be summed into a request percentile.

The sampled state-machine apply region is substantially larger than the
measured lock acquisition and receipt publication/inspection regions. It
includes Raw batch construction, engine WAL work and resident-index publication;
the trace does not yet separate those costs. It also excludes earlier proposal,
replication and waiting intervals. Registration age is not an exact registration
timestamp and cannot safely fill that gap by subtraction.

Existing cumulative leader metrics separately report 2,173 / 2,195 engine WAL
record writes totaling 49.455 / 55.311 ms, and 2,193 / 2,215 successful syncs
totaling 0.238 / 0.238 ms on tmpfs. Namespace publication totals 33.026 / 25.592 ms.
These have different populations and snapshot boundaries from the trace;
subtracting their means from a sampled apply mean would be invalid. They
support investigating CPU work inside apply before assuming this tmpfs workload
is dominated by direct sync calls.

## Next change and evidence

Narrow the remaining application cost with targeted CPU attribution for Raw
batch lowering, WAL encoding/checksum and persistent-map publication. Inspect
existing retained CPU profiles first; reuse them when their recorded scope
answers this question. Reuse these accepted captures and WAL metrics; choose a concrete dominant
operation before another scheduling or data-structure change. The earlier
owned-mutation-buffer candidate was already rejected by its complete comparison;
do not repeat that clone-removal experiment on the strength of an undivided
apply interval. Preserve view atomicity, exact applied positions, ordered fences,
durable quorum acknowledgments and the selected CRC runtime. Candidate promotion
still requires its own strict proof, local correctness/recovery/actual Chaos
acceptance and matched throughput/latency gates.

Matching release terminal: `13353 / 4bd6ff / 0`; independent release readback:
`070a11/0`. Six focused new path/binding checks pass `ca973a/0`; the previous
eight unchanged capture controls are inherited, not rerun. Actual capture:
`78595 / ca7099 / 0`; independent audit: `66bc82/0`; interval/overhead analysis:
`1ee97e/0`. See the [original portable metadata and analysis](write-stage-corrected-capture-v1/README.md).
No code or theorem suite changed after the previously qualified correction,
no new Chaos fault ran, no original industrial checkbox closes, and no hosted
workflow was dispatched. All new bulk output remains in `/mnt/data/kv9-work`.
