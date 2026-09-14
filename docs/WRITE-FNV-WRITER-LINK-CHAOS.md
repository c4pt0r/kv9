# FNV writer: client-link and quorum-loss acceptance

The exact default FNV writer at `12f44d35590ede5f89337fe731dd950162865154`
passes the separate eleven-window client-link, stream-reset and quorum-loss
campaign. Independent complete-history, packet-effect and source checks pass.
This supplements the [21-window Chaos Mesh campaign](WRITE-FNV-WRITER-CHAOS.md);
it provides no new database throughput or latency measurement. CRC main remains
the selected runtime.

## Results and scope

The complete native history contains **2,126 operations: 1,835 OK, 63 unknown
and 228 refused**, covering point Get/Put/Delete and BatchGet/BatchPut. The
independent checker finds a valid history while accounting for unknown effects.
An initial search that excluded unknown effects was inconclusive; it was not
reported as a successful check or discarded from the evidence.

The eleven windows are baseline, client delay, delay healed, 30% client packet
loss, loss healed, client partition, partition healed, exact TCP reset, reset
healed, quorum loss and quorum healed. Delay, loss and partitions use actual
Chaos Mesh faults. The exact TCP reset uses `SOCK_DESTROY` and is recorded as a
separate mechanism.

After the prescribed quiescence period, all **37 fully contained calls** in the
client-partition window end unknown, with no success. All **136 fully contained
calls** in the quorum-loss window are refused, with no success. Calls spanning
fault installation or healing remain in the complete history; the negative
window assertions do not apply to those boundary calls. Each recovery window
resumes successful traffic.

Independent raw packet-counter readback verifies **203 dropped packets** on the
same netem leaf and network namespace. Parent and child counters are not added.
The full fixture's batch-read counters record 638 inline completions and eight
blocking submissions; these include setup and verification and cannot be
attributed to particular responses or fault windows.

This is continuous mixed traffic; the fixture does not guarantee that a chosen
write is pending at the instant a fault is injected. It runs on one Kind host
with volatile tmpfs storage and does not establish power-loss durability,
cross-host behavior, every in-flight cancellation case, or an fsync-stall result.
It preserves the default Raft quorum, synchronization and response fences.

## Execution and retained evidence

The existing qualified image is reused without rebuilding. Input verification
binds all 887 source files, default production features and the retained server
SHA-256 `d84b0ec8e466a9dc3f4953751605d91c90a90a66df1b03b57a3412e43e42603e`.
Five preparation control suites pass before runtime. A preparation-only binding
failure is retained; no runtime attempt is repeated.

Actual runtime session **69956** ends at **d310bf/0**. The independent full audit,
netem-leaf readback and final source verification all pass in session **38527**,
ending **a8923f/0**. The runtime retains the original owned-data archive and
readback, stops its writers, records three fresh empty replica drains and
completes owned cleanup. All five owned container lifetimes end. The owned
namespace is absent and all eight historical namespace UIDs match their pre-run
values. A fresh post-run check also confirms the protected historical PodChaos
UID is unchanged. Original histories, command outputs, source bindings,
terminal records and failed preparations remain available locally.

Matched point/batch c1/c64 performance testing still needs
[additional retention capacity](WRITE-FNV-CAPACITY.md). No original industrial
roadmap work package closes at this checkpoint. All validation ran locally;
hosted CI was not dispatched.
