# Measure the Raft request-body handoff before changing scheduling

The completed confirmation-queue diagnostic leaves one important interval
unmeasured: time from offering a coalesced batch to `batch_tx.send` until the
request-body stream yields it to tonic. Existing admission measurements end
when the bounded channel accepts a batch. They do not measure its consumer's
polling delay, HTTP/2 flush or peer delivery. Neither the Append-payload screen
nor inbox vector reuse produces a useful overall read gain.

## Bounded next implementation

1. Branch independently from selected CRC behavior. Wrap the existing
   `ReceiverStream` with an observer that returns exactly the same `Poll`
   results and batch values. Keep the 16-slot channel and original select arms.
2. Carry an optional, bounded sample owned by the already existing batch. Start
   immediately before the original send/select, and stop when `poll_next`
   returns that batch. Name the interval **batch offer to request-body poll**:
   it includes any admission wait and must not be mislabeled pure residence.
   Do not replace `send` with `reserve` to obtain a cleaner clock boundary.
3. Sample on a fixed cadence with checked counters and fixed memory. Preserve
   explicit cancellation, dropped samples, RPC failure and counter exhaustion.
   Dropping observation must never keep a batch, route or RPC alive. Do not
   add a flush timer, unbounded channel, task or wait. Instrumentation must not
   authorize a request or claim that a stream yield is a network ACK.
4. Account for mixed message kinds within each batch. Retain whole-client
   envelope and measurement-only client totals separately. Do not subtract
   means from different populations or sum queue means into GET latency.
   Preserve the existing bounded export cap and validate reachable worst cases.

## Acceptance before collecting observations

Document removal of observation as a projection to the selected transport:
queue decisions, batch bounds, route generation, select/cancellation behavior,
keepalive and progress timeout remain unchanged. Cover Pending/Ready/EOF,
sampled versus unsampled batches, cancellation, route replacement and full
channels with focused contracts. Run relevant source checks and ordinary
recovery before collecting fixed c1 GET and c64 mixed cells.

Use original retained builds and local CPU isolation. Only root runs Cargo or
runtime. Reclaim known rebuildable caches before a recording if required;
retain original evidence and leave storage guards unchanged. The first complete
recording and its failed attempts, if any, must be independently checked before
interpretation. This diagnostic is not a new Redis comparison.

## Decision after evidence

If this interval materially occupies the confirmation path, prototype removing
one producer-to-body handoff while preserving route ownership, bounded queued
memory, immediate idle-message delivery and stalled-reader reconnection. A
replacement must retain a progress watchdog even when HTTP/2 PINGs succeed but
the remote application stops reading. Prove cancellation cannot deliver an old
route's message to a new destination. Then check recovery and compare a clean
uninstrumented candidate using the complete c1/c64 GET/mixed protocol.

If this interval is small, keep the transport structure and identify the next
specific owner/HTTP2/socket boundary. Do not repeat the same coarse confirmation
diagnostic, broaden held candidates, or introduce DPDK without NIC/cross-host
measurements. Useful changes require applicable full point/batch, core-proof
and actual Chaos Mesh qualification before promotion. The selected runtime
remains CRC; dynamic multi-Raft and automatic splits remain after the read gate.
