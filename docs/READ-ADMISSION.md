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

## Accepted local evidence — 2026-09-09

The failing regression is retained in `7dcca7a`; the implementation and proof gates
are in `dcc307893ea9bb93e9fee4baeaa81b5193d9e676`. Runtime acceptance used a clean,
detached worktree at that exact implementation commit. Proof and Rust mutation
artifacts were independently bound to its source hashes.

| Gate | Accepted evidence |
| --- | --- |
| Original regression | Exactly one compiled test fails at the lost-read assertion before the fix and passes after it |
| Workspace | 532 passed, zero failed, 22 ignored; all-target build, warnings-denied Clippy, formatting and actionlint pass |
| Rustdoc | All eight workspace crates freshly documented with broken and private intra-doc links denied |
| Rust source controls | Six isolated baseline / intended failure / restored sequences, with exit codes 0 / 101 / 0 |
| Protocol | 32 TLC cases, 31 proof/audit cases, 21 fresh positive strict proof runs; 15 theorems and 196 obligations per positive run |
| Validator compatibility | All 64 retained endpoint/route model cases accepted under their unchanged exploration thresholds |
| Real process E2E | Six scripts pass: phase-1 acceptance, dynamic membership, Raw KV, quickstart, root trust and partition-read refusal |
| Real MinIO | Three replicas, failover, remote checkpoint, reclaimed WAL, live tail and deletes pass |
| Actual Chaos Mesh | Full 19-window matrix passes; complete histories independently accepted for 4,108 CLI operations and 1,148 persistent-client operations |

Chaos coverage includes faults on every voter, leader partition, public admission
pressure, six actual EIO/ENOSPC failures, repeated missing-log refusal and three
independently prepared replacement-PVC refusals, followed by original-store
recovery. Formation, storage-loss, replacement, latency and persistent-history
auditors were rerun against an independent copy of the successful scene. The
persistent collector was removed before a final database read/write probe passed.
The matrix uses single-host Kind and local WAL mode; MinIO has its separate E2E.

Retained archive: `target/correctness-evidence/2026-09-09-dcc3078-read-admission.tar.gz`,
113,869,460 bytes, SHA-256
`032a946ce4536431b6bb050fa126814541b0b061e397651dd2cb5e2031090759`.
Its manifest binds 3,290 entries, including exact source archives, raw logs,
histories, proof inventories, source controls and independent audit scripts.

Failed attempts remain visible in the archive. The first two protocol runs exposed
TLC wrapper/adapter errors; the third complete run is the accepted result. A
temporary E2E wrapper used the wrong partition-read terminal marker and exited one,
although all six scripts exited zero. The retained independent marker audit checks
the unchanged logs against the exact committed workflow. The first independent
persistent audit supplied a short SHA and correctly failed revision matching; the
corrected invocation uses the full SHA and preserves the original certificates.
None of these corrections reran or rewrote the successful runtime histories.

The original hosted Raw KV failure and local initial gRPC apply timeout are also
retained. Three subsequent runs of the original 118-test Raft binary passed, which
does not explain the initial apply timeout. Listener and apply-wait diagnostics
are now more informative; no specific transport cause is established. Issue #48
remains open for that diagnosis, #47 remains open for production endpoint migration,
and this increment does not close P0. No hosted workflow was dispatched.

## Further #48 diagnosis: listener fixture ownership

Commit `60fa59aa695a512c4c25cc4eb15040b6bdcf9a2f` removes a deterministic
test-environment defect from the two three-voter gRPC fixtures. They previously
called `free_addr`, which binds and immediately drops a listener, then passed the
released address to an asynchronously started server. Another concurrent listener
could claim a voter endpoint in between. A new contender assertion in the existing
failover test fails on the original fixture: the competing bind succeeds before
the server starts. This demonstrates the allocation gap; it does not reproduce or
explain the historical initial proposal/apply timeout.

Both fixtures now retain all three bound listeners and transfer the actual sockets
to tonic's `serve_with_incoming`. The shared minimal-server helper also reports
bind failure synchronously to its test caller. A current-thread-runtime regression
checks exclusive binding after allocation and again after asynchronous handoff,
before any server task can poll, then verifies discovery from all three actual
gRPC endpoints. An isolated source mutation restores release-and-later-rebind;
the new regression rejects it specifically at the second ownership boundary,
with baseline / mutant / restored exits of 0 / 101 / 0.

Local validation: 533 workspace tests passed, zero failed, 22 ignored; formatting
and warnings-denied all-target Clippy passed. Four executions of the exact
workspace-built Raft binary at the fix each passed all 122 tests with default
concurrent scheduling. The preceding `4de7a6e` binary also passed four executions
of all 121 tests. Neither series establishes the cause of the earlier failure.
The original failure, deterministic allocation regression, isolated handoff
control, both binaries, resolved build metadata and complete logs are retained.

Every source change is inside `#[cfg(test)] mod tests`; the 53,439-byte production
gRPC prefix is byte-for-byte identical to `4de7a6e` (SHA-256
`9669eca4e802eb124190ccfadace331e87f4cb0bb2886bfb5a74997db94f1807`).
This test-fixture increment changes no protocol, client outcome, deadline or apply
assertion. It does not claim a fresh Chaos/MinIO or proof run; the separately
scoped production evidence above remains attributed to `dcc3078`. GitHub CI was
not dispatched. The initial apply timeout remains an open #48 diagnosis; #47
production endpoint integration remains the next independent development path.

Listener evidence archive:
`target/correctness-evidence/2026-09-09-60fa59a-grpc-listeners.tar.gz`,
75,125,727 bytes, SHA-256
`ca83c636af118c83ca0d0b3a0c356de588dc80b5a93c142ab468c8a1813ffaca`.
All 36 manifest entries were independently checked for size and SHA-256.
