# Multi-Raft group retirement and local resource reclamation

Updated 2026-09-27, on baseline `4aa8c4b1a082bda8ea016baa87d8cb03e2d302e8`.

**2026-10-03 follow-up:** [task-history pagination](TASK-HISTORY-PAGINATION.md)
removes the independent 255-row catalog ceiling and removes the fixture pruning
described in this historical record. New churn tests retain the complete catalog.
The original logs and proof results below remain unchanged; safe history GC,
bounded maintenance and full production acceptance remain open.

This change fixes local lifecycle ownership. It does **not** close the production
capacity gate: the metadata task catalog still has a separate 255-row total
budget, and no safe task-history GC protocol is implemented here.

## Resulting behavior

- `DriverPool::try_unregister` stops an exact driver and returns `false` while
  a worker owns its turn. Slots have stable indices and become reusable only
  after the worker releases its reference. Unrelated groups keep their slots.
  A stale unregister request cannot remove a replacement driver.
- The last `GroupTransport` owner unregisters its inbox. The queue is closed
  and cleared, including sender clones obtained before router removal. Unknown
  wire traffic still cannot create groups or fall back to metadata.
- `RegionManager` withdraws serving/capture handles and saves a pending
  retirement before attempting durable publication. Publication failures keep
  the group lock and retry state. Subsequent reconciliation advances retirement
  without waiting in a loop for outstanding readers.
- After the durable `Retired` fence and pool detachment, the manager uses
  `Arc::try_unwrap` to destroy the driver, peer and engine in that order. A live
  request/capture keeps retirement pending. Successful destruction, rather than
  a weak-reference count, establishes that destructors completed before payload
  deletion. Status includes `state: "retired", draining: true/false`.
- Drained `Retired` and `Reclaimed` tombstones retain their disk records but no
  payload owner or open lock FD. Ready, quarantined and draining groups still
  count against the local 255-slot admission bound. Recovery no longer counts
  historical terminal directories as live capacity; it validates terminal
  store/root identity and refuses reactivation even before discovery runs.
- Reclamation takes a temporary lock and writes `Reclaimed` before deletion.
  A deletion failure after that write can retry in the same process. Shutdown
  also clears the shared group-handle and adoption-receipt registries.

The on-disk record encoding and committed split/removal authority are unchanged.
The runtime still requires the enclosing exclusive store guard. Inbox/worker
slot reuse alone never authorizes reuse of a retired region ID.

## Verification and limits

Retained logs and source hashes are in
[the validation record](group-retirement-validation-20260927/manifest.json).

- Two tests failed on the pre-fix implementation: the retired driver remained
  alive after stop, and payload was deleted while an external engine handle
  still existed. Both pass with the change.
- All 14 RegionManager tests pass, covering six retirement publication failure
  cuts, retry and restart, retained engine/peer handles, and reclamation retry.
- The local churn fixture keeps one active group while creating/retiring 260
  other groups. Half retain their payload; half reclaim it. Per-store Linux FD
  counts return to the same baseline after each lifetime and after recovery;
  shutdown reaches zero group FDs. Old identities cannot reopen. **This component
  fixture explicitly deletes completed creation rows from its test catalog to
  isolate local lifecycle capacity. No production code does this.** The first
  unmodified-catalog attempt hit `creation task capacity reached` at 255 rows;
  its failure log is retained as evidence of the remaining independent bound.
- 269 Raft library tests pass with 8 pre-existing ignored tests and 37 network
  module/tests filtered out. Pool tests cover an in-flight blocked worker,
  progress of another group, 265 repeated slot lifetimes with actual single-voter
  apply receipts, and stale-owner rejection. Transport tests cover inbox closure
  and more than 255 cumulative registrations.
- Strict Clippy for the changed crates and all their targets, and workspace
  formatting checks, pass.
- The initial broader network runs failed at socket creation/bind with
  `Operation not permitted` in this sandbox: eight group/gRPC tests, then three
  TCP tests. These are unexecuted protocol checks, not passing fault evidence.
  Actual multi-process/Chaos Mesh acceptance and physical power-loss testing
  remain open. No new QPS, RSS-soak or cross-host result is claimed.

## State model and refinement boundary

[Retirement.tla](../proofs/tla/group-retirement/Retirement.tla) specifies dispatch,
stop, durable fence, detach, external-reference release, payload close, unlock,
delete, crash and recovery for one immutable ID. Safety includes: a running slot
remains registered; external owners retain open payload; an open payload retains
its lock; deletion requires a durable fence and closed owners; a durable fence
survives recovery and prevents new dispatch. Existing in-flight work may finish
after stop, and files remain owned until it does.

[Retirement.lean](../proofs/lean/group-retirement/Retirement.lean) proves the
invariants by induction over arbitrary modeled traces, plus a finite enabled
continuation after workers and readers drain. All 10 statements compile under
Lean 4.33.1, their axioms are audited, and five semantic defects plus two proof
policy defects are rejected. This is an abstract proof with the following mapping:

| Model transition | Implementation / assumption |
| --- | --- |
| dispatch / finish / unregister | Registry mutex, stable `Option<Slot>` indices, `running` flag and release of the worker's Arc before clearing it |
| stop / publish | Exact committed split/removal authority at the caller; directory removal, driver stop, fsync/rename/directory-sync |
| borrow / release / close | Rust Arc ownership, issued Raw/capture handles, and synchronous final destruction via `drop_exclusive` |
| unlock / delete | Terminal local record, drained payload owners, temporary deletion lock, and retryable durable deletion intent |
| crash / recover | Process death releases volatile owners; successfully synced fences survive and are checked before opening payload |

The TLC runner parsed the model, then its fingerprint-store RMI listener was
denied by the same sandbox restriction before exploration. The combined proof
gate remains **unaccepted**; only the Lean sub-gate passed. `Safety.cfg` and
`Drain.cfg` retain the intended finite safety and fair-draining checks, along
with five counterexample controls in the runner:

```sh
python3 scripts/prove-group-retirement.py \
  --lean /path/to/lean-4.33.1 --jar /path/to/pinned/tla2tools.jar \
  --output /fresh/validation-directory
```

The explicit `--lean-only` mode records `lean_accepted: true` while leaving
`accepted` and `tla_accepted` false. It cannot discharge the combined gate.
Progress assumes finite worker/storage service, release of issued references,
successful eventual fence persistence, and recurring maintenance turns. The
model does not establish those assumptions, verified Rust extraction, allocator
behavior, wire consensus, full storage-GC coverage or catalog-history retention.

## Remaining production work

The next capacity task is a metadata history protocol: distinguish active work
from permanent creation/dedup/ownership evidence, add bounded indexed or paged
access, and authorize compaction without breaking old retries, recovery or
split/migration references. Merely raising `MAX_CREATION_TASKS` would retain the
lifetime ceiling and whole-history scan cost. Local tombstone memory and discovery
time also remain proportional to history; this change only removes their active
slot and persistent FD cost. Installed snapshot generations and object-store
retention require their own complete cleanup coverage.

The remaining production review items, including concurrent backpressure,
cross-topology maintenance, TTL and merge, remain open.
