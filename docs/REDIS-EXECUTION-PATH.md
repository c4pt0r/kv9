# Redis's short execution path and KV9's optimization target

Tracking: #9, #13 and #20. The reference here is the benchmark's Redis 7.0.15,
with one I/O thread, standalone memory operation, persistence disabled and
no client pipelining. This analysis does not claim equivalent durability or
fault semantics. KV9 retains three-voter Raft and linearizable reads.

## What the reference actually does

Redis's [GET implementation](https://github.com/redis/redis/blob/7.0.15/src/t_string.c#L300-L316)
looks up the key, checks its type and appends a bulk reply. Those few lines are
not the entire request cost: parsing, expiration handling, dictionary lookup,
accounting and network I/O still execute. They illustrate how little command
coordination a successful memory lookup requires.

Command execution has a single main-thread owner. This avoids transferring
each ordinary GET between independently scheduled command and storage workers.
It also avoids concurrent command writers contending over the same dictionary.
Redis can use other threads for I/O and background work; describing the entire
process as single-threaded would be inaccurate. Keeping ownership local is the
useful architectural property, not a rule that fewer threads always win.
See the [official latency explanation](https://redis.io/docs/latest/operate/oss_and_stack/management/optimization/latency/).

The [reply path](https://github.com/redis/redis/blob/7.0.15/src/networking.c#L361-L418)
uses the connection's existing buffer and reply blocks. It avoids creating
extra string/Redis objects when the existing storage suffices. Before returning
to the event loop, [pending writes](https://github.com/redis/redis/blob/7.0.15/src/networking.c#L1893-L1922)
are attempted directly; a writable-event handler is installed when output
remains. This is a concrete example of removing a scheduling step from the
common path, while still handling backpressure.

Redis also amortizes transport overhead. Its
[pipelining documentation](https://redis.io/docs/latest/develop/using-commands/pipelining/)
explains that multiple commands and replies can share read/write system calls.
Explicit MGET/MSET additionally amortize command and response envelopes. Those
are distinct from one-at-a-time latency: the current reference has one
outstanding command per connection and does not use pipelining. Pipeline
benchmark numbers therefore cannot explain its advantage in this comparison.

## The consistency cost and the implementation cost

The conceptual successful read paths are:

```text
Standalone Redis:
socket -> parse -> lookup -> reply buffer -> socket

Current KV9 safe read:
RPC -> register read -> Raft owner -> fresh quorum confirmation
    -> applied-index fence -> memory lookup -> RPC completion
```

This is an ordering sketch, not a timing decomposition. KV9's confirmation
contains outbound and inbound transport, peer processing and local scheduling.
The [Raft paper, section 8](https://raft.github.io/raft.pdf) explains why a
leader must establish authority before serving safe reads and why a lease
alternative introduces timing assumptions. KV9 must preserve its successful
confirmation and local apply checks. Read-only confirmation does not require
appending a new log entry for every GET.

Redis's [default replication](https://redis.io/docs/latest/operate/oss_and_stack/management/replication/)
is asynchronous, and the measured standalone reference has no replica at all.
A Redis memory SET acknowledgment is consequently not equivalent to KV9's
commit-and-apply write acknowledgment. This difference establishes additional
required work; it does not establish that today's implementation is optimal.

For both reads and writes, batching can amortize required distributed work.
It must preserve ordering and completion authority: a later read cannot borrow
an earlier group's already-started confirmation. Batching also trades waiting
time against amortization, so both mean and tail latency remain selection
criteria. Cross-host network latency and storage durability will impose limits
that a loopback, volatile-WAL comparison cannot quantify.

## What the existing measurements say

The latest completed [wake-coalescing screen](WORK-SIGNAL-SCREENING.md) retains
four repetitions of its read-credit parent `57ff6851` and same-recording Redis
GET controls. The parent is an experimental read path, not a general promotion:

| Concurrency | KV9 GET/s | Redis GET/s | KV9 mean us | Redis mean us |
| --- | ---: | ---: | ---: | ---: |
| 1 | 26,536.505 | 172,609.493 | 37.555366 | 5.717858 |
| 64 | 374,812.531 | 512,144.140 | 170.629087 | 124.854313 |

QPS is pooled across all four repetitions; mean latency is weighted by calls.
The c1 turnaround gap is approximately 6.6x, whereas the c64 throughput gap is
approximately 1.37x. The unselected wake-coalescing candidate reaches 377,805
GET/s at c64 but slightly worsens c1 mean; its full tail results remain in the
screen. The general `5ee897a` baseline's earlier measurements remain in the
[idle-watchdog experiment](PEER-IDLE-WATCHDOG-SCREENING.md).
Closed-loop concurrency overlaps waiting across calls;
higher throughput does not demonstrate that an individual request path has
become short. QPS and mean latency are related in this workload and are not
independent causal evidence.

The [accepted-source c64 CPU diagnostic](RPC-PAIR-READ-LIFECYCLE.md) attributes
17.19% of GET on-CPU samples to scheduling/synchronization, 15.53% to
RPC/framing/buffers, 13.02% to allocation/copy/comparison and 2.10% to
engine/storage. These sampled categories support examining coordination and
representation overhead before redesigning the memory lookup. They are not
end-to-end latency fractions, and removing a category cannot be assumed to
produce an equal percentage speedup.

The separate [c1 lifecycle diagnostic](LOW-CONCURRENCY-READ-LIFECYCLE.md)
places 20.997 us of a sampled 24.448-us barrier between ReadIndex admission
invocation and observed confirmation. That includes software and quorum work;
it is not a measured irreducible network RTT. The instrumented stage cannot
be subtracted from the uninstrumented 38-us client latency as an exact budget.

## Consequences for development

1. Test quorum amortization independently. Accepted-source c64 endpoints show
   only about two members per admitted read group. Candidate `57ff685` bounds
   locally submitted, unconfirmed ReadIndex requests using the actual Raft
   pending queue. Idle admission remains immediate; waiting reads can form a
   subsequent sealed group. Its [completed screen](READ-GROUP-CREDIT-SCREENING.md)
   now measures about 8% higher c64 throughput with better mean/p99 and roughly
   15 members per group in full fixture envelopes. Its initial c1 point GET
   regresses slightly. The separate [four-repeat longer follow-up](READ-CREDIT-C1-FOLLOWUP.md)
   measures 26,447.329 GET/s and 37.680573 us mean, versus Redis's 172,509.024/s
   and 5.721245 us. Candidate pooled throughput/mean improve slightly against
   the control, but p99 improves in two repeats and worsens in two. Both studies
   remain retained, and general promotion remains open. This supports treating
   amortization and isolated request latency as separate optimization targets.
2. Shorten the confirmation path using evidence about individual handoffs.
   Record queue publication, body polling, peer receipt/owner observation and
   reply delivery before choosing the next structural change. Preserve source
   and sample identity, and separate instrumentation from performance selection.
   Previous queue-removal and scheduler experiments regressed, so their
   intuitively shorter code paths are not evidence of lower elapsed cost.
3. Keep state ownership and buffer reuse explicit when changing execution.
   Avoid needless intermediate representations and task handoffs. A future
   per-shard owner is compatible with multiple Raft groups; simply setting the
   existing runtime to one worker already failed its paired screen. Writes,
   mixed traffic, sustained load and large batches still need separate checks.
   The [inbox-vector candidate](https://github.com/c4pt0r/kv9/blob/c3131800b665f6f160a11c804f542237e77ab83a/docs/RAFT-INBOX-DRAIN.md)
   removes a redundant production vector allocation and message move. Local
   source gates and process E2E pass, but the [completed matched screen](RAFT-INBOX-DRAIN-SCREENING.md)
   shows only small pooled gains and mixed throughput/p99 directions. It is
   not selected as the next performance increment. This is evidence against
   assuming fewer allocations automatically yield a useful end-to-end gain.
   The subsequent [wake-coalescing screen](WORK-SIGNAL-SCREENING.md) improves
   c64 GET by 0.798% but regresses c1 GET mean and several p99 pairs. It also
   remains unselected. Refresh actual point-write/mixed and larger-batch
   behavior before continuing isolated read-only micro-optimizations.
4. Evaluate kernel bypass only after a real NIC experiment identifies the
   kernel/network path as the limiting cost. Redis's measured reference uses
   ordinary sockets. The current loopback profile neither proves a NIC limit
   nor predicts a DPDK benefit; DPDK would not remove quorum or task queues.

The practical target is fewer instructions, allocations and ownership
transfers per successful operation while preserving the distributed protocol.
Measure low-concurrency request turnaround separately from high-concurrency
amortization: a larger group can improve throughput while leaving isolated GET
latency unchanged. An ownership transfer can incur queueing, wakeup and cache
movement even when its queue is lock-free. This is an architectural hypothesis
to test with stage measurements, not an attribution of every remaining
microsecond. The current write path also dispatches blocking preparation before
asynchronous completion; the new point-write/mixed clients allow measuring
that path explicitly before changing it. A per-shard owner should eventually
allow parallel independent ranges while keeping each range's critical path
short; that architecture still requires proof and measured validation.
Redis-class read/write throughput and latency, complete proof composition,
Chaos Mesh acceptance and automatic splitting remain open requirements.
