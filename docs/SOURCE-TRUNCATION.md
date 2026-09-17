# Committed source log truncation

Updated 2026-09-17. Parent [#9](https://github.com/c4pt0r/kv9/issues/9),
D03 [#24](https://github.com/c4pt0r/kv9/issues/24), retention/log seam
[#19](https://github.com/c4pt0r/kv9/issues/19). Builds directly on
[destination evidence](DESTINATION-EVIDENCE.md).

## What this increment adds

The source group's retained log — kept whole until now precisely so the
learner could catch up — can finally be compacted, under committed
authority only, with the tail preserved.

1. **Committed truncation decisions** (TASKS kind 105, `KV9TRC01`,
   `crates/meta/src/data_groups/truncation.rs`). One immutable decision per
   operation, bound to the committed evidence row, with its floor bounded
   by the evidence cut: the destination's installed image already carries
   every entry at or below that cut, so no entry a migration learner could
   still need is ever authorized away. Recording it at the metadata leader
   additionally requires the source pin to be Released. Idempotent
   confirmation; divergence refuses.
2. **A dedicated prefix-compaction seam.**
   `DiskRaftStorage::compact_retained_prefix` writes a durable
   `REC_COMPACTION` record (floor, decision digest, configuration at the
   floor) and compacts the in-memory mirror while KEEPING the tail —
   `install_protocol_snapshot` remains a forward-install seam and is
   untouched. Replay re-performs the same transformation at the same file
   position. No disk space is reclaimed yet; no snapshot is served;
   the network MsgSnapshot receive fence is unchanged.
3. **Leader-side execution.** `RaftPeer::truncate_retained_log` (RPC
   `TruncateSourceLog`, CLI `truncate-source-log`) re-checks leadership,
   local application of the floor, term exactness, the configuration
   committed at-or-before the floor, and that **every tracked peer —
   voters and learners — has matched the floor** before compacting: an
   entry below `first_index` can never be replicated again.
4. **Restart under committed authority only.** A compacted base at startup
   is accepted exactly when a committed truncation decision matches its
   region and floor (`RegionManager::start_group` over
   `committed_truncations`); anything else keeps the blanket refusal, and
   the peer/driver start through the same validated installed-base
   constructors as runtime adoption.
5. **Two pre-existing restart gaps found and fixed by the e2e** (three
   failed launches retained): `validate_fixed_group` refused ANY source
   voter restart after learner attach — it now authorizes membership the
   group's own committed log evolved from the creation voters
   (`ConfigurationHistory::evolved_from`; voter set must still equal the
   creation voters, no joint transition in flight); and the engine WAL
   replay cross-check probed batch positions below the compacted floor —
   positions at or below a durable compacted base are now vouched for by
   the REC_COMPACTION record itself (`position_in_history`).

## What it deliberately does not do

No physical disk reclamation, no follower-side compaction (the leader-only
seam is the one exercised and proven), no stranded-learner recovery, no
promotion, removal, split, placement, chaos campaign, or scaling claim.
The [3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md) remains
the only acceptance path for effective horizontal scaling.

## Evidence

- Serial qualifying workspace run plus strict Clippy/formatting; catalog
  unit tests for the decision planner (no evidence refuses, floors beyond
  the evidence cut refuse, idempotent confirm, one floor per operation)
  and the readback defect matrix.
- Accepted five-process e2e (`scripts/source-truncation-e2e.py`,
  `e2e-fourth`; three failed launches retained as evidence): the full
  settled chain, then compaction refusal without the decision, floor-
  beyond-cut refusal, exact compaction to floor+1 (idempotent), restart of
  the compacted voter under committed authority, acknowledged data below
  the floor still served, and the learner still tracking the tail.
- Thirteen-theorem Lean model
  ([proofs/lean/source-truncation](../proofs/lean/source-truncation/README.md))
  with eight semantic mutation controls and two proof-policy controls;
  twelve sibling Lean models and the retention TLA/TLAPS model re-accepted.

Validation packet: [docs/source-truncation-v1](source-truncation-v1/README.md).
