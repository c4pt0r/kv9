# Published-directory matched write results

Completed locally on 2026-09-15 UTC. Candidate `483b8c3` improves batch
throughput and tail latency in both execution orders, while loaded point
writes regress slightly in both orders. **Keep CRC main `bd42e60` selected**;
retain the directory candidate as a batch improvement for later evaluation.
This screen does not justify replacing the general baseline or claim Redis
parity. The [rotation/recovery gate](WRITE-PUBLISHED-DIRECTORY-ROTATION.md)
and the existing 21-window Chaos baseline both pass on the exact candidate.

All eight smoke and sixteen timed cohorts pass independent acceptance.
The timed population contains **7,252,679 successful one-attempt calls /
64,934,408 input items**, with zero refused calls, unknown writes, read/client
failures or dropped slots. Full final datasets, 48 fresh drains, 48 voter
writer/listener bindings, 64 timed lifetimes and 32 smoke lifetimes pass.
All owned test processes exit and the background containers' CPU settings
are restored exactly. Unknown or failed work was not retried or discarded.

| Concurrency / API | CRC main throughput | Directory throughput | Change | CRC mean / p99 | Directory mean / p99 |
| --- | ---: | ---: | ---: | --- | --- |
| 1 / Put | 19,354.182 calls/s | 19,300.239 calls/s | -0.279% | 51.567 / 68.608–69.631 us | 51.710 / 68.608–69.631 us |
| 64 / Put | 139,457.016 calls/s | 138,734.094 calls/s | -0.518% | 458.798 / 745.472–753.663 us | 461.186 / 745.472–753.663 us |
| 1 / BatchPut(64) | 393,952.625 items/s | 402,267.054 items/s | +2.111% | 162.346 / 215.040–217.087 us | 158.987 / 206.848–208.895 us |
| 64 / BatchPut(64) | 1,056,344.501 items/s | 1,076,849.048 items/s | +1.941% | 3.877 / 7.537–7.602 ms | 3.803 / 6.554–6.619 ms |

Throughput divides summed successful counts by summed cohort elapsed time.
Means are count-weighted, and p99 comes from merged original whole-call
histograms; ranges are histogram bucket bounds. Batch latency is per complete
64-item call. The candidate's loaded batch rate is 16,825.766 calls/s.
Smoke, historical experiments and Redis runs are excluded from these totals.

The per-order throughput changes retain the same conclusion:

| Workload | Forward order | Reverse order |
| --- | ---: | ---: |
| c1 Put | -0.589% | +0.032% |
| c64 Put | -0.820% | -0.215% |
| c1 BatchPut(64) | +2.613% | +1.613% |
| c64 BatchPut(64) | +2.166% | +1.717% |

Loaded batch p99 improves in both orders: 7.602–7.668 to 6.619–6.685 ms,
and 7.471–7.537 to 6.554–6.619 ms. Loaded point p99 moves in opposite
directions between orders, with unchanged pooled bucket bounds. The small
point regressions and modest batch gains are shared-host diagnostic results,
not a statistical confidence claim.

The fixed native v3 client `0be806d` uses 4,096 keys, 128-byte values, seed 71,
128 warmup calls, one in-flight call per worker and ten-second closed-loop
measurement windows. Both servers use default features and ThinLTO. Three
voters retain the original Raft quorum, append sync, durable apply and reply
fences. WAL storage is explicitly volatile tmpfs: these are CPU/protocol
measurements, not physical power-loss or cross-host qualification. Redis was
not rerun; the [earlier WAIT 1/2 references](WRITE-REDIS3-BASELINE.md) have
different confirmation and durability semantics.

Actual terminals: smoke `24363/c3d554/0`, smoke readback `5664a6/0`, timing
and restoration `16255/bf3870/0`, independent audit `62866/c1c5d3/0`.
The initial launch `bb381b/1` failed Git ownership checks before creating the
smoke output or starting a workload. The next launch adds command-local
`safe.directory` entries for the three exact source repositories, with no
global Git configuration, source, driver or workload change. The original
launch failure remains retained. Two stale v2 literals in the smoke reader
were corrected to the qualified v3 helper/floor before launch; eight focused
controls pass in addition to the original 71 environment controls.

Independent decoding covers **74,033,997,777 logical bytes** across all 24
cohorts. Their combined physical retention is **52,208,918,528 bytes**;
compressed objects preserve original bytes after temporary raw paths are
removed. No codec overlaps measured work. Fresh pre-smoke and remaining
capacity checks pass under the separately versioned v3 policy; observed
post-timing available space is 27,476,590,592 bytes. Any future restore or
campaign still requires fresh capacity. See [the prior capacity work](LOCAL-CAPACITY-RECOVERY.md).

[Original reports, configs, datasets, audit records and byte verifier](published-directory-performance-v1/README.md)
make the performance population reviewable. The package omits compressed
WAL objects and is not sufficient to rerun the complete local byte-retention
audit. Full objects and original failures remain local. The first derived
summary used misleading nested p99 field names while its values were in us;
the corrected summary explicitly names the enclosing us unit. Both summaries
are retained; original measurements and audit results were unchanged.

The subsequent [receipt-tail matched screen](WRITE-RECEIPT-TAIL-PERFORMANCE.md)
now passes after its separate default release, recovery and actual full21 Chaos
gates. It improves point writes in both orders but has batch tradeoffs; CRC
remains selected. Keep these candidates separate until their interaction has
independent evidence.
Read optimization remains held; dynamic multi-Raft, routing, recoverable
membership and automatic range splits follow the write phase. All testing
was local, no hosted CI ran, and no original industrial roadmap checkbox closes.
