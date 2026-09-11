# Version 3 point and batch workload comparison

Protocol: `kv9-v3-workloads-c1-c64-v1`. This refreshes actual point GET/PUT and
64-key BatchGet/BatchPut at 0/50/100 percent reads. It compares the unchanged
general baseline `5ee897a`, read-credit candidate `57ff6851` and standalone
Redis. Both native and Redis measurement clients are clean `0be806d9` releases.
No server or client implementation is changed for this recording.

The exact protocol and independent descriptors specify c1/c64, 4,096 keys plus
sentinel, 128-byte values, seed 71, 128 warmup calls, 1,500-ms deadline,
ten-million-call cap and closed-loop traffic. SDK max attempts stays six;
all outcome and attempt populations are retained. Healthy one-attempt results
are distinguished from accounting-valid cohorts with failures.

The first correctness smoke covers one forward list of all 36 cells, each for
two seconds. It is excluded from performance selection. After reviewing smoke,
the timed matrix covers the full 36-cell list followed by its exact reverse,
ten seconds per cohort. There is no per-cell retry or discarded unfavorable
result. If a validation or resource guard fails, preserve the first partial
recording and diagnose before deciding a new protocol.

Timed clients use CPUs 0-1; all three KV9 voters, or the Redis server, share
2-5. Helpers and three exact owned background containers use 6-15,22-31.
Smoke clients use 6-7 and servers 8-15,22-31. The unchanged outer wrapper
restores the owned containers' original configured/effective 0-31 masks and
preserves historical namespace and fault identities. The host is shared and
unrelated services remain unconstrained. No build, test, fault, profile or
independent audit may overlap timed execution.

KV9 retains normal quorum and sync calls on tmpfs; Redis has no replicas,
save/AOF disabled, one I/O thread and one outstanding command per connection.
No pipeline, equivalent durability, real-NIC capacity, sustained-load or
production-readiness claim follows from this diagnostic.

The preserved lifecycle, source/build, PID/start/boot, listener, allocator,
fresh-drain, complete raw-retention and resource checks remain in the driver
and independent auditor. Exact selected v3 APIs and deterministic final
value/write-key membership replace the previous pure-read nonce-zero scope.
The actual issued nonce set and full linearizable histories are not recorded;
final scans are not substituted for those independent correctness gates.

Batch results report calls/s and input keys/s separately. Whole-call means and
p50/p95/p99 are never divided by batch size. Raw histograms preserve each
operation and outcome, including inactive populations. Pooled means are
weighted by calls; quantiles use merged counts, not averaged percentiles.

Space preflight requires 32 GiB free tmpfs and 96 GiB free retention storage.
The common resource sampler records both filesystems for every arm and stops
on observation failure or free space below 16/64 GiB. The inherited tmpfs
fixture's 32-GiB startup and 8-GiB snapshot checks also remain. These guards
do not prove a worst-case resource bound from the ten-million-call cap.
Smoke WAL growth informs operational release of the fixed timed recording;
extrapolation must remain explicitly conditional and cannot certify capacity.

Root owns the wrapper, arguments, launch and storage release. `stream_review`
owns the driver and seven passing preparation contracts;
`replacement_acceptance` owns the independently defined auditor and fifteen
passing contracts. Audit execution requires the root's actual terminal timing
session and exit code. The auditor inventories the six existing endpoint
snapshots per native cohort for subsequent source-bound metric analysis;
endpoint metrics have independent populations and overlapping intervals.
