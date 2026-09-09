# ReadIndex admission after election

This work addresses the reproducible read-admission defect under [#48](https://github.com/c4pt0r/kv9/issues/48).
The separate intermittent gRPC proposal/apply timeout remains an open diagnosis.

## Failure and implementation

A newly elected leader is observable before its election no-op commits. The pinned
[raft-rs 0.7 implementation](https://github.com/tikv/raft-rs/blob/v0.7.0/src/raft.rs#L2104)
discards a ReadIndex request if the term of its committed log entry differs from
its current term. Returning from `RawNode::read_index` therefore does not establish
that the request was retained. Previously kv9 submitted once and waited for the
exact context until timeout, even after the quorum recovered and committed the
current term.

`RaftPeer::read_index` now returns `Ok(false)` for this temporary admission condition
and `Ok(true)` after submission. Neither result is a read confirmation. Leadership,
current-term commitment and submission are checked under the same peer lock; a
status observation outside that lock cannot authorize a later submission.

`NodeDriver::read_barrier` retains one invocation context and one original deadline
while retrying deferred admission. A deposed leader returns the existing typed
NotLeader result. After admission the existing path still requires the exact
context's quorum receipt, then the unified contiguous applied watermark covering
its index. Only then may the caller acquire its engine snapshot. An admitted read
that subsequently loses its term can still time out; this change does not restart
that read or reinterpret an ambiguous write.

The deterministic regression freezes a real three-peer election after the winning
vote, with commit index zero. A test-only observer proves the read attempted
submission before any further delivery. The same invocation must complete after
the followers acknowledge the election entry. The original implementation times
out at the named assertion despite current-term commit/application reaching index
one. No timing sleep is used to establish the injection cut.

Two additional regressions cover the original deadline and typed leadership loss
while admission is deferred. The existing isolated-leader and frozen-application
regressions still require quorum and application confirmation, respectively.

## Proof contract

`proofs/tla/read-admission/ReadAdmission.tla` models one invocation, arbitrary
positive terms, a monotone committed term, unique own/other contexts, admission,
quorum certification, delayed receipt publication, application coverage, role
changes and a decreasing request budget. Term bounds exist only in the TLC wrapper.
The deductive inventory has 15 declarations and 196 obligations.

The invariants and action properties establish:

- Admission occurs only as leader with a current-term committed entry, atomically
  with the submission decision. At most one submission follows successful admission.
- Deferred attempts retain the invocation context and do not replenish its budget.
- A foreign context cannot establish the invocation. Readiness alone is insufficient:
  successful return requires actual quorum certification and application coverage.
- Under stable leadership, eventual current-term commitment and fair downstream
  progress with sufficient time remaining, the invocation reaches successful return.

The liveness proof has six stages: waiting for current-term commit, ready for
admission, submitted, quorum certified, receipt observed, and application covered,
followed by successful completion. Each stage is preserved or advances, has an
enabled advancing action, and uses the corresponding weak-fairness assumption.
There is no wall-clock latency guarantee and no success claim during perpetual
leadership changes or deadline exhaustion.

The implementation mapping and assumptions are explicit:

| Model boundary | Implementation / assumption |
| --- | --- |
| Atomic admission | `RaftPeer::read_index` holds its peer mutex across role, `commit_to_current_term` and `RawNode::read_index` |
| Deferred invocation | The driver's same context and `Instant` remain live across unsuccessful admission attempts |
| Quorum certification | Pinned raft-rs Safe ReadIndex, authenticated member traffic, and the underlying Raft safety assumptions |
| Exact delivery | Ready publication through `take_read_states`, followed by matching complete context bytes in the driver's receipt ring |
| Application coverage | The unified driver watermark covers the receipt index only after the contiguous batch applies |
| Read result | The establishing caller acquires its snapshot after the barrier |
| Conditional progress | The pump, runtime, network and apply path make progress; the deadline permits completion; the receipt survives ring eviction long enough to be observed |

The model abstracts a quorum certificate and index coverage as facts established
by those existing components. It is not a new proof of upstream consensus, receipt
ring retention, unique context generation across counter exhaustion, Rust mutex
implementation or whole-program refinement. Those assumptions are not discharged
by model checking. In particular, this model does not explain the separate apply
timeout or establish cross-host availability.

The safety composition uses the ordinary Raft current-term barrier argument:
leader completeness preserves earlier committed commands; commitment of an entry
from the elected term establishes the relevant prefix; Safe ReadIndex obtains a
quorum certificate for a fresh invocation context; contiguous application and a
subsequent snapshot prevent serving a prefix below that certificate. This change
adds the missing admission handshake and leaves those obligations intact.

## Reproducible local gates

```sh
cargo test --locked --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/check-read-admission-controls.py --output /tmp/read-admission-controls
python3 scripts/check-read-admission-protocol.py \
  --tlapm /path/to/pinned/tlapm --jar /path/to/tla2tools-v1.7.4.jar \
  --output /tmp/read-admission-protocol
```

Output directories must be new. The six Rust controls each require a compiled,
exactly selected baseline test, the intended assertion failure under one source
mutation, and a passing restoration. The protocol gate checks two finite instances
under two fingerprints, two fair continuations, eight protocol faults through both
TLC and TLAPS, two reachability witnesses, semantic proof audits and ten invalid
output controls. Positive proofs use fresh caches, strict checking and the exact
inventoried obligation count. A legal initial state followed by one invalid
admission can produce a valid two-state counterexample; the checker still requires
both trace states and separately rejects one-state statistics and missing traces.
Existing protocol gates retain their previous exploration thresholds.

Real local Raw KV/MinIO acceptance and actual Chaos Mesh leader-loss/partition
runs with complete histories are required alongside the deterministic regression.
The existing 19-window Chaos matrix supplies broader regression evidence; it does
not force the exact before-commit cut, which is established by the deterministic
regression. Single-host Kind evidence does not prove host-loss tolerance.

GitHub workflows retain these gates for manually triggered pre-release or key
milestone acceptance. Routine development runs locally according to `TESTING.md`.
