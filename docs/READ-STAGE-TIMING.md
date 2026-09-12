# Asynchronous read stage diagnosis

This branch adds an opt-in `kv9-raft/read-stage-timing` observer to the selected
CRC runtime. It does not include the experimental coalesced owner notification
change. A diagnostic result is not a new uninstrumented performance result.

## Why this boundary

The accepted notification screen retained before/after server metrics for all
client phases. In its four isolated GET cells, leader ReadIndex establishment
means are 23.852–23.969 us, within public backend means of 25.892–25.986 us.
These snapshots cover initialization, warmup, measurement, verification and
readback. They are not matched to measurement-only client calls; subtracting
them from client latency does not measure network latency. Resident GET awaits
the existing asynchronous ReadIndex ticket and then reads its fenced memory
snapshot without entering the blocking pool in the normal ready path.

## Observation contract

The feature adds six fixed histograms, using the existing seven outcomes and
65 logarithmic buckets, to driver snapshots. The default metric inventory stays
at 26; the diagnostic inventory has 32 metrics. There is no per-key label,
unbounded trace, new queue, background task or file write on the read path.

For each successfully received read reply, five adjacent intervals and their
total are recorded from that same request's monotonic timestamps:

| Metric suffix | Begin | End |
| --- | --- | --- |
| `registration` | Existing read start | Request construction immediately before queue insertion, under the queue lock |
| `owner_queue` | That construction timestamp | Immediately before the successful sealed group's ReadIndex call |
| `quorum` | That call boundary | First exact-context confirmation observed by the owner |
| `completion` | That confirmation | Immediately before sending the selected successful reply |
| `resume` | That send boundary | Receiver branch resumes after receiving the reply |
| `total` | Existing read start | That receiver resume timestamp |

All names start with `raft_async_read_`. Registration includes sequence minting,
channel allocation and registration locking. Owner queue includes publication,
notification, owner scheduling, filtering and any deferred admission attempts.
Quorum includes peer admission, local pumping, transport and remote peer work;
it is not network RTT alone. Completion includes the remainder of the successful
pump and its apply fence. Resume includes the actual send operation, publication,
wakeup and executor scheduling; it is not an after-send timestamp. The total
excludes histogram recording and subsequent memory view/lookup/RPC encoding.

Deferred attempts do not create admitted timestamps. A group's first exact
confirmation stamps its surviving members; later or unrelated confirmations do
not overwrite it. Errors, dropped tickets and undelivered replies do not produce
successful stage samples. A missing or reversed boundary on an otherwise
successful response records six observer errors and leaves that response intact.
Existing whole-call/outcome accounting remains mandatory, so success-only stage
populations cannot hide failures or substitute for full latency histograms.

The six records execute synchronously in the receiver branch without an await.
Snapshots remain coherent per metric and independent between metrics. Only
fresh quiescent snapshots, bracketed by the existing admission/async waiter
drain predicates, can establish equal stage populations and exact sum-of-sums
conservation. Live snapshots cannot justify those cross-metric assertions.
Percentiles of adjacent stages cannot be added.

## Correctness boundary

No timestamp participates in read admission, context lookup, quorum decisions,
apply coverage, timeout, cancellation, stop or read-view authorization. The
existing queue limits, sealed membership, first exact confirmation, successful
whole-pump completion fence, original deadline and cancellation-close ordering
are unchanged. The feature carries a timestamp envelope alongside the existing
oneshot result; it does not change the result or introduce another sender.
With the feature disabled the channel payload remains the original result type,
and timestamp fields and recording code are absent at compile time.

The feature changes observation and scheduling cost, so measurements must be
labelled instrumented. Existing proof assumptions and runtime correctness tests
remain applicable; this is not a new proof of the full Rust implementation.
New tests check exact stage conservation, rejection of missing/reversed timing,
and real sealed-group confirmation/apply/cancellation/delivery behavior.

## Execution gate

Root runs formatting, default and diagnostic Raft/Server tests and Clippy locally,
using the serialized build-cache invalidation helper. A retained release must
bind its exact clean source, compiler, opt-in feature and first-party artifact
rebuilds. Two fixed diagnostic fixtures cover c1 and c64 point GET with the
retained v3 clients, 4,096 keys and 128-byte values. Before/after exporter identity,
all outcomes, bounds/saturation, monotonic deltas, complete phase accounting,
fresh drains and CPU/storage/source/process provenance remain required.

Do not promote this instrumented branch as a performance optimization. Use its
largest measured boundary to choose the next bounded runtime change, then prove
and test that change and compare uninstrumented throughput and latency.
