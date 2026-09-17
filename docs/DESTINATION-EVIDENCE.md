# Destination-install evidence and the committed release decision

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D03 [#24](https://github.com/c4pt0r/kv9/issues/24), retention seam
[#19](https://github.com/c4pt0r/kv9/issues/19). Builds directly on
[runtime adoption](RUNTIME-ADOPTION.md) and the
[migration authority](MIGRATION-AUTHORITY.md) retention owners.

## What this increment adds

The migration retention fence — "migration pins cannot quiesce without
committed install evidence" — now has exactly one key: a **replicated
destination-install evidence row**, and a committed release decision that
consumes it.

1. **Committed evidence rows** (TASKS kind 104, `KV9EVD01`,
   `crates/meta/src/data_groups/evidence.rs`). One immutable row per
   migration operation binding root, operation, migration task, region,
   destination node **and** store incarnation, adopted generation, image
   digest, the retention owner subject (sha256 of the canonical manifest)
   and the exact adopted cut. The planner validates the receipt against the
   committed migration row; an identical resubmission is a confirmation;
   any divergence refuses. A receipt cannot supply its own task id.
2. **The destination replays durable facts.** `adopt_for_runtime` now also
   returns the image's manifest subject; `RegionManager` keeps a per-region
   adoption receipt, and a new `EmitInstallEvidence` RPC (+ CLI
   `emit-install-evidence`) replays the canonical receipt from the
   destination's own adopted state. Read-only; no capability.
3. **Commit with a published-pin cross-check.** `RecordInstallEvidence`
   (+ CLI `record-install-evidence`) commits the row at the metadata
   leader through the serialized, term-fenced catalog transaction — after
   verifying the receipt's subject equals the **published source pin's**
   subject (owner IDs re-derived from the shared
   `migration_operation_digest`). A wrong claim refuses instead of
   permanently binding the operation to a false image.
4. **The ledger fence consults committed rows only.**
   `QuiesceAfterTransfer` for Snapshot/Migration pairs now accepts exactly
   when a committed evidence row — read from the SAME applied view the
   plan runs against — matches both owners' derived operation digest and
   subject (`evidence_matches_owner_pair`). Absence or mismatch keeps the
   refusal. `Release` still requires the Quiesced phase, so the whole
   settlement chain is: committed migration → published pins → adoption →
   committed evidence → quiesce → release.
5. **The destination pin never drops.** Quiescing the destination owner
   still requires a published successor and stays refused; releasing it
   refuses outright while the replica lives.

## What it deliberately does not do

No source log truncation, physical deletion (still disabled ledger-wide),
promotion, removal, split, placement, chaos campaign, or scaling claim.
The [3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md) remains
the only acceptance path for effective horizontal scaling.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting; catalog
  unit tests for the evidence planner/readback/defect matrix and the
  view-level probe; retention ledger tests for both fence directions
  (no/mismatched evidence keeps refusing; the matching committed row
  settles quiesce then release with the destination pin intact).
- Accepted five-process e2e (`scripts/destination-evidence-e2e.py`, first
  attempt): the full qualified chain through adoption and tail catchup,
  then quiesce/release refusals **before** evidence, receipt emission with
  the exact installed image and cut, a divergent-subject commit refusal,
  idempotent recording, committed quiesce → release of the source pin,
  destination-release refusal, and continued tail tracking afterward.
- Thirteen-theorem Lean settlement model
  ([proofs/lean/install-evidence](../proofs/lean/install-evidence/README.md))
  with nine semantic mutation controls and two proof-policy controls;
  sibling Lean models and the retention TLA/TLAPS model re-accepted.

Validation packet: [docs/destination-evidence-v1](destination-evidence-v1/README.md).
