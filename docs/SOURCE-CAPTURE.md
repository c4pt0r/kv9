# Unified-cut source capture

Updated 2026-09-17. S05 [#19](https://github.com/c4pt0r/kv9/issues/19) and
D03 [#24](https://github.com/c4pt0r/kv9/issues/24), following `2aefab0`.

2026-10-05 update: production planning/capture now use budgeted, independent
anonymous SST spools and the existing V2 format for large images. Capture also
checks the actual driver/engine owner and drains local upload jobs. See the
[current implementation and validation](RAW-TTL-CAPTURE-SPOOL.md); the historical
real-MinIO evidence below does not qualify this new path for production.

2026-10-05 follow-up: both production endpoints now use a
[durable fixed capture slot](RAW-TTL-DURABLE-CAPTURE.md). The same source store
reuses the original cut/configuration across writes and restart; committed
owners remain required before upload.

2026-10-05 subsequent update: the [replicated capture catalog](RAW-TTL-CAPTURE-CATALOG.md)
independently observes that slot before metadata Raft registers its descriptor.
A successor data leader can verify the original objects with GET only and return
the exact record after source loss. Complete retention/recovery authority and
mixed-binary deployment qualification remain open.

The subsequent [V8 store/service floor](RAW-TTL-CAPTURE-UPGRADE.md) now excludes
older writers and RPC paths that can bypass the catalog. Existing plans retain
their bytes and pins across upgrade. This keeps the stop-all-nodes upgrade
contract; it does not add rolling-upgrade support or qualify real deployment.

The subsequent [durable payload transfer](RAW-TTL-CAPTURE-TRANSFER.md) makes Plan
success wait for full payload copies on majorities of the image's voter sets.
A successor can retrieve the exact original file from those voters and finish
an incomplete upload. Failure before sufficient copies exist can still leave
the operation without a recoverable payload; unresolved PUTs remain protected.

The subsequent [migration Publisher lifecycle](RAW-TTL-MIGRATION-PUBLISHER.md)
now registers each process generation before capture object I/O and persists
its actual drained PUT counts before finishing that local owner. Lost receipts
block later local generations until the original durable report is acknowledged.
Unknown writes remain recorded under the unchanged long-lived image protection.

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
commit position. Planning performs no object-store I/O (the production path writes
a local file, registers it through metadata/source RPC and copies it to voters); the manifest — and therefore the
owner subject and complete SST closure — is final before any upload.

## Pin before upload, across leaders

The capture flow decouples group leadership from metadata leadership:

1. `plan-migration-image` (group leader): atomically persist or load the fixed
   local plan, register its exact descriptor through independent source observation
   and metadata Raft, copy the full payload to the image configuration's voter
   majorities, then return its canonical manifest and cut. No object I/O.
2. `bind-migration-image` (metadata leader): commit both migration owners for
   exactly the catalog-registered manifest.
3. `capture-migration-image` (group leader): load the same plan, then verify from LOCAL
   applied ledger state that both owners are Published with subject equal to
   this manifest's digest — local visibility implies commitment, and absence
   refuses in the safe direction — then confirm atomic entry of an independent
   Publisher generation before object I/O. Upload (PUT plus verified GET per
   object) from the original local file, or verify the catalog's original SSTs
   with GET only on a successor. If objects are missing, a successor retrieves
   an exact durable voter copy and uploads the original bytes. Return the
   canonical `KV9RSN01` record only after every object verifies; if no complete
   original payload is available, refuse without replanning. Actual object jobs
   must drain and their original PUT report must be durably acknowledged before
   this call succeeds or a later local generation can begin.

A source cut that advances between binding and capture leaves the durable
plan unchanged. Retries reuse the original image, including after reopening
the same source store. A changed binding or corrupt plan refuses. The record claims the cut and the configuration,
nothing about elections (vote is empty); the installer's transition rules
apply unchanged.

## Closing the loop

The current offline CLI requires a metadata leader address and the exact image
owner IDs in addition to the record and stopped store. Its old-image checks,
new-image restore and final verification now use retained GET-only Readers;
see [installation Reader lifetimes](RAW-TTL-INSTALL-READERS.md). Replacement
requires owners for both images. The lower-level installer and runtime adoption
remain separate integration boundaries.

The historical component loop test captures from a live single-voter group with a
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
The historical runner checks nine semantic mutations and two proof-policy controls.
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
second-image refusal after the cut advances in that earlier implementation.
It does not validate the new durable slot. No Chaos Mesh campaign, no
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
