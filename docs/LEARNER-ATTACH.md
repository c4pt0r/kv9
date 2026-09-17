# Learner attach and offline destination installation

Updated 2026-09-17. S05 [#19](https://github.com/c4pt0r/kv9/issues/19) and
D03 [#24](https://github.com/c4pt0r/kv9/issues/24), following `3bfa47d`.

This increment commits the migration destination into the source group's
configuration as a **learner**, and completes the offline half of the
migration flow at the destination's real store. It promotes no voter, starts
no peer, replicates no log, and serves nothing from the installed generation.

## Attach

`attach-migration-learner`, at the group leader, is authorized solely by the
committed migration intent: it refuses an uncommitted operation and a voter
destination, proposes `AddLearner(destination)` through the group's own
ordered log, waits for the applied configuration, then proposes one `Noop`
data command so the engine cut moves past the configuration entry — the next
capture therefore names the learner. Retries confirm idempotently, each
returning a freshly advanced cut. The learner has no store yet: replication
toward it pauses on the existing snapshot fence until installation and
startup land in a later increment.

## Offline installation at the destination

`kv9 install-migration-image --data-dir <stopped-store> --record-file <r>`
is a new offline command beside `init`/`join`: it locks the stopped store,
loads its durable root/identity bundle, derives the exact range from the
record itself, and runs the **unchanged** joint installer. Every check —
root binding, store incarnation, membership, cut/term consistency, object
digests — is the installer's own. The e2e proves both directions at the
destination's real store: the learner-bearing image installs with the exact
captured cut, and a control image whose configuration lacks the destination
is refused by the membership gate.

After a refused or successful offline installation the destination process
restarts healthy: RegionManager isolates the versioned installed generation
(`KV9INS01`) instead of adopting it, exactly as the joint-install increment
specified. Making that generation appendable runtime storage is the next
increment, not this one.

## Checked model and validation

[Attach.lean](../proofs/lean/learner-attach/Attach.lean) proves nine
theorems (attach requires the committed intent and advances the cut; the
destination is never a voter; no serving; learner monotonicity) with seven
semantic and two policy controls. The runtime test covers attach refusals,
idempotent confirmation with fresh cuts, and learner-not-voter status; the
four-process e2e ([packet](learner-attach-v1/README.md)) chains intent →
attach → capture-with-learner → offline install at the real store →
healthy isolated restart, with the one-image-per-operation refusal and the
membership-gate control. Sibling models and the retention TLA/TLAPS model
are rechecked. No Chaos Mesh, no promotion, no serving, no scaling claim.

## Next mainline work

1. Runtime installation: snapshot Ready coordination and the validated
   startup capability making a selected installed generation appendable
   runtime storage under RegionManager, without weakening startup hash
   checks or the group lock.
2. Committed destination-install evidence bound to intent and owner
   subject, then the committed quiesce/release decision for the source.
3. Catchup after source log truncation, promotion from durable evidence,
   removal; then split publication and placement.

The [3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No new QPS or
measured scale-out result exists in this checkpoint.
