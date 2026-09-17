# Unified-cut source capture

Updated 2026-09-17. S05 [#19](https://github.com/c4pt0r/kv9/issues/19) and
D03 [#24](https://github.com/c4pt0r/kv9/issues/24), following `2aefab0`.

This increment captures a live data group's offline image at one exact,
defensible cut, under the group driver's own leadership — the description the
[joint installer](JOINT-SNAPSHOT-INSTALL.md) consumes and the
[migration owners](MIGRATION-AUTHORITY.md) pin. It moves no replica, starts
no peer, admits no snapshot message and releases no pin.

## The cut and its configuration

`plan_capture` freezes the group's engine at its durable applied position:
the frozen view supplies the data, the authoritative `DataRange` (and so the
manifest scope) at the same instant, and the cut's own `(term, index)`. The
attached Raft configuration comes from `configuration_at_committed` at
that exact cut — the durable history refuses, with typed reasons, a compacted
or ambiguous history and any configuration entry committed at-or-before the
cut but not yet indexed. A configuration committed **past** the cut is
correctly excluded: the live-path tests prove a learner added after the cut
does not appear in the image until a later data command moves the cut past
the configuration entry, and that the attached configuration names its own
commit position. Planning is I/O-free; the manifest — and therefore the
owner subject and complete SST closure — is final before any upload.

## Pin before upload, across leaders

The capture flow decouples group leadership from metadata leadership:

1. `plan-migration-image` (group leader): plan only; returns the canonical
   manifest and cut.
2. `bind-migration-image` (metadata leader): commit both migration owners for
   exactly that manifest, unchanged from the previous increment.
3. `capture-migration-image` (group leader): re-plan, then verify from LOCAL
   applied ledger state that both owners are Published with subject equal to
   this manifest's digest — local visibility implies commitment, and absence
   refuses in the safe direction — then upload (PUT plus verified GET per
   object) and return the canonical `KV9RSN01` record.

A cut that advances between binding and capture changes the subject and
refuses: **one image per operation, by construction**. Re-capturing an
unchanged cut is idempotent. The record claims the cut and the configuration,
nothing about elections (vote is empty); the installer's transition rules
apply unchanged.

## Closing the loop

The component loop test captures from a live single-voter group with a
learner committed in its configuration, uploads through real MinIO, installs
the exact record with the unchanged `JointInstaller` at the learner's
prepared store, recovers, and reads back every value. Capturing before the
learner exists is refused by the installer's existing membership gate —
which is precisely why learner attach (the next increment) precedes any
runtime installation: a destination outside the image configuration has no
defensible base.

## Checked model

[Capture.lean](../proofs/lean/source-capture/Capture.lean) proves the
finite-trace projection in 11 theorems: the attached configuration is
at-or-before the cut, upload requires committed pins, the receipt requires a
pinned upload of the exact cut, the frozen cut is immutable, one image per
operation, retry is a pure receipt, and no step grants serving or voting.
The runner checks nine semantic mutations and two proof-policy controls.
Ledger atomicity and installer validation are premises from their own
models; the eight sibling Lean models and the retention TLA/TLAPS model are
rechecked against the exact working tree.

## Validation

Live-path unit tests cover cut/scope/configuration binding, the
configuration-lag cases and ownershipless refusal; the real-MinIO component
loop covers capture → install → recover → value readback. The four-process
e2e ([packet](source-capture-v1/README.md)) covers leader-only capture with
typed follower refusal, unpinned and uncommitted refusals, canonical record
framing, committed owners before upload, idempotent re-capture and the
second-image refusal after the cut advances. No Chaos Mesh campaign, no
serving, no learner, no scaling claim.

## Next mainline work

1. Learner attach: commit the destination into the group configuration, then
   capture (the configuration now names the destination) and install; define
   snapshot Ready coordination and the validated startup capability with
   RegionManager so an installed generation becomes appendable runtime
   storage.
2. Committed destination-install evidence and the committed release decision
   for the source owner — never from a local receipt.
3. Catchup after source log truncation, promotion from durable evidence,
   removal; then split publication and placement.

The [3/6/9-host benchmark contract](HORIZONTAL-SCALING-PLAN.md) remains the
acceptance criterion for effective horizontal scaling. No new QPS or measured
scale-out result exists in this checkpoint.
