# Migration abort: the stranded-destination recovery (D03)

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D03 [#24](https://github.com/c4pt0r/kv9/issues/24). A destination store
incarnation lost mid-migration wedged its operation forever; the
committed abort settles it and makes the region re-migratable.

## The wedge

A migration destination that dies after attach/publish/capture but
before adoption can never emit install evidence — evidence names the
EXACT destination incarnation. Without evidence: the source pin sticks
at Published (quiesce requires committed evidence), truncation stays
blocked, the stranded learner stays in the source configuration, and
the region can never plan another migration.

## What this increment adds

1. **The committed abort row** (kind 108, `KV9ABT01`,
   `crates/meta/src/data_groups/abort.rs`): immutable, idempotent,
   cross-validated against the committed migration; the subject is
   NEVER caller-supplied — `record-migration-abort` reads it from the
   operation's PUBLISHED source pin. Evidence and abort are mutually
   exclusive per operation, permanently, in both directions.
2. **Ledger settlement**: the retention quiesce fence accepts
   evidence-matched OR abort-matched owner pairs (same
   subject-and-operation-digest discipline, same applied-view reads).
   The source pin then quiesces and releases through the UNCHANGED
   verbs. The abandoned image's destination pin never drops — a
   documented cost until object-store reclamation exists.
3. **Learner detach** (`detach-aborted-learner`): removes the stranded
   learner from the source configuration at its leader, gated on the
   committed abort; a voter destination refuses loudly (abort authority
   never removes voters); idempotent when already absent.
4. **Region re-migration**: a committed abort settles its operation, so
   the one-live-migration-per-region rule skips settled predecessors —
   in planning AND in every reader (readback stays whole with the
   aborted history committed; one bad row freezing all readers is the
   cascade-hang defect class, avoided here). Attach and runtime
   adoption refuse aborted operations permanently.
5. **Node re-provisioning** (the operational half): `revoke-admission`
   exposes the operator decommission path; a fresh PENDING admission
   authorizes a NEW store incarnation to take over the node id at
   registration (the identity gate still refuses every non-Pending
   path), with the stale metadata-learner progress reset at takeover.
   The raft step layer now DROPS heartbeats whose commit floor exceeds
   the local log end — progress tracked for a lost incarnation
   otherwise panics a re-provisioned peer; raft is lossy-safe and the
   sender re-probes through the clamping append path.

## Known limits, deliberately out of scope

Source-log truncation for ABORTED operations (the truncation decision
still requires committed evidence; the aborted source keeps its log),
object-store reclamation of abandoned images, automatic abort (the verb
is an explicit operator decision — the system cannot distinguish a slow
destination from a dead one), voter takeover (refused by design), chaos
campaigns, performance claims.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting; new
  catalog tests cover idempotent abort planning, both directions of the
  evidence/abort exclusion, readback rebinding refusals, the view-level
  probe, and re-migration unlock (with whole readback and a live
  successor still blocking a third).
- Accepted five-process e2e (`scripts/migration-abort-e2e.py`): a real
  stranding (published+captured, never installed, destination killed),
  refusals before settlement, the committed abort (mutation then
  confirmation), attach refusing permanently, quiesce→release through
  the unchanged ledger verbs, learner detach (then idempotent confirm),
  and a FRESH incarnation re-provisioned onto the same node id
  (revoke-admission → re-admit → takeover registration) completing a
  new migration of the SAME region end to end: install, adoption,
  retained-tail catchup, evidence, quiesce, release. Five retained
  failures document the road: two admission/registration walls (the
  revoke verb and the takeover authority now exist because of them),
  two stale-heartbeat panics at the re-provisioned peer (the step-layer
  clamp exists because of them), and one replication-lag flake (bounded
  attach retry).
- Thirteen-theorem Lean model
  ([proofs/lean/migration-abort](../proofs/lean/migration-abort/README.md))
  with five semantic mutation controls and two proof-policy controls;
  twenty-six sibling Lean models and the retention TLA/TLAPS model
  re-accepted.

Validation packet: [docs/migration-abort-v1](migration-abort-v1/README.md).
