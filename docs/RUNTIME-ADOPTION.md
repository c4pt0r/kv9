# Runtime adoption of installed generations

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D03 [#24](https://github.com/c4pt0r/kv9/issues/24). Builds directly on the
[joint installer](JOINT-SNAPSHOT-INSTALL.md), [source capture](SOURCE-CAPTURE.md)
and [learner attach](LEARNER-ATTACH.md).

## What this increment adds

A selected installed generation now becomes the destination's **live
replica**: the runtime adopts it, restores the driver from the installed
cut, and the learner catches the source leader's retained log tail through
ordinary MsgAppend — with the network-snapshot receive fence unchanged.

1. **One-way adoption receipt** (`adopt_for_runtime`,
   `crates/raft/src/snapshot_install.rs`). Reads the destination's own
   durable group record (KV9INS01) and selector — never a caller-supplied
   description — and requires a selected generation. First adoption verifies
   the complete pair and every remote object exactly like recovery, then
   durably publishes a checksummed `runtime-adopted` marker bound to the
   store binding, generation and image digest. From that marker on,
   `JointInstaller::open` refuses the group: installation is closed. Files
   may then legitimately diverge from their sealed hashes (the log appends,
   the WAL grows), so re-adoption validates the marker and the sealed image
   record only, leaving file validation to ordinary storage/engine recovery.
   The returned paths carry the same per-group lock RegionManager uses.
2. **Validated startup constructors.**
   `RaftPeer::with_installed_storage` is the one path past the
   "snapshot base requires coordinated engine installation" refusal: it
   re-checks that the durable base equals the adopted cut (log starts at
   `cut+1`, term matches, no lease policy) and initializes raft's applied
   index at the cut so the installed prefix is never re-reported.
   `NodeDriver::with_installed_base` restores the driver's unified applied
   position from that cut — the one legitimate restoration, per the
   invariant on `driver_applied` — and refuses an engine watermark behind
   the cut (ahead is a legitimately appended tail; replay re-proves it).
3. **Reconciled adoption** (`RegionManager::adopt_installed`,
   `NodeRuntime::reconcile_adoption`, `crates/server/src/region_manager.rs`,
   `crates/server/src/runtime.rs`). Each reconcile turn, a destination whose
   store identity (node **and** incarnation) matches a committed migration
   adopts at most one installed generation: recover storage and engine from
   the generation directory, validate the range against the committed
   creation and root, start the peer as whatever the image configuration
   says (a learner today), and register the group as Active in the raw
   directory. Errors surface in `group_control_error`; adoption retries on
   later turns.
4. **Tail catchup, snapshots still fenced.** The source leader replicates
   its retained tail to the learner via MsgAppend only. The source's log
   was never truncated past an attach cut in this flow, and the receive-side
   MsgSnapshot drop stays in force — no network image installation exists.

## What it deliberately does not do

The learner never leads, never votes, and refuses reads with a leader hint
like any follower. Nothing here promotes a voter, records committed
destination-install evidence, quiesces or releases a retention pin,
truncates the source log, splits, places, or claims scaling or new QPS.
The [3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md) remains the
only acceptance path for effective horizontal scaling.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting; unit tests
  covering the exact-base refusal shapes of both constructors
  (`crates/raft/src/storage/snapshot/tests.rs`, `crates/raft/src/driver.rs`)
  and real-MinIO adoption fencing: one-way closure, exactness across
  legitimate file growth, sealed-image and marker tamper refusals
  (`crates/raft/src/snapshot_install/tests.rs`).
- Accepted five-node real-process e2e (`scripts/runtime-install-e2e.py`):
  the full chain from committed intent through attach, capture and offline
  install, then destination restart → reconciled adoption as the live
  replica at the exact installed cut → post-install writes at the source
  leader reach the learner through the retained tail → reads at the learner
  refuse with a leader hint → a second restart re-adopts idempotently and
  keeps tracking the tail.
- Fourteen-theorem Lean model
  ([proofs/lean/runtime-adoption](../proofs/lean/runtime-adoption/README.md))
  with eight semantic mutation controls and two proof-policy controls;
  sibling Lean models and the retention TLA/TLAPS model re-accepted against
  the exact working tree.

Validation packet: [docs/runtime-adoption-v1](runtime-adoption-v1/README.md).
