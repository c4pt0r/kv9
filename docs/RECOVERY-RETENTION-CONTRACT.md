# Recovery anchors and retention ownership

Status: C04 design increment, 2026-09-15. Tracks [#14](https://github.com/c4pt0r/kv9/issues/14)
under [#9](https://github.com/c4pt0r/kv9/issues/9). Source audit: `d4a7ab782e5a057b40c6ea1099ee93059bdfc77e`.
This defines the implementation contract for the remaining storage work. The
common anchor codec, replicated retention ledger, deductive composition proof
and new fault scenarios are still to be implemented. C04 remains open.

The [per-resource record increment](RETENTION-RECORD.md) now implements bounded
pin transitions and their recovery codec, with a checked parameterized component
proof. It does not yet implement the enclosing durable ledger or complete anchor.

The [configuration-at-cut component](CONFIGURATION-AT-CUT.md) now supplies the
actual full membership at an exact retained committed cut, with refusal for
missing or ambiguous authority. The [local checkpoint publication validator](CHECKPOINT-PUBLICATION.md)
now consumes that input during startup and matches an actual winning apply batch
to its exact committed manifest command. Complete portable anchor identity,
retention binding and bounded online snapshot capture remain open.

## Authority already implemented

Recovery currently composes several records; there is no independently
restorable protocol anchor in an SST manifest. These are different authorities:

| Record / component | What it establishes | What it does not establish |
| --- | --- | --- |
| Root descriptor, store identity and lifecycle | Cluster/root binding, local store incarnation, exclusive cooperating writer, activation before Raft | A replacement disk's right to reuse a lost voter's identity |
| `DiskRaftStorage` | Durable protocol entries, term/vote, commit and indexed configuration changes | Remote state-machine snapshot installation or safe protocol-log truncation |
| Positioned engine WAL | Applied data and its exact `(term,index)` in one durable record | Consensus commit without the Raft protocol state |
| `CheckpointManifest` | Full-state cut, cluster/region/epoch, exact SST references and content hashes | Root digest, receiving store identity, `ConfState` or a complete backup |
| `catalog.pending` | The original prepared attempt and predecessor generation survive restart | Applied manifest authority or permission to reclaim WAL/SSTs |
| Segmented topology | Selected active/closed files and the locally adopted checkpoint | Independent certification of that checkpoint by Raft |
| Retained manifest generations | Exact historical CAS winner or positive effect evidence | A negative verdict from absent history |

The source boundaries are [root identity](../crates/common/src/root.rs),
[store lifecycle](../crates/common/src/store_lifecycle.rs),
[Raft storage](../crates/raft/src/storage.rs),
[checkpoint codec](../crates/engine/src/checkpoint.rs),
[pending journal](../crates/engine/src/flush_journal.rs),
[WAL topology](../crates/engine/src/wal_stream.rs),
[manifest state machine](../crates/raft/src/state_machine.rs) and
[reconciliation](../crates/region/src/manifest.rs).

Startup in [runtime](../crates/server/src/runtime.rs) checks the selected
checkpoint's cluster/region and configuration at its exact committed cut before
restoring SSTs. It observes the retained uncovered WAL, requires an atomic
winning manifest pair and descriptor, and matches their publication position to
the exact committed Raft command. Only successful complete engine recovery and
final history checks return a local publication observation. The runtime also
checks the recovered applied term against committed history before serving.
The old `has_committed_checkpoint` helper only establishes proposal presence;
a committed CAS loser passes it. It is no longer startup authority. This local
validator still relies on ordered apply and retained history; it is not a
portable certificate for a new store or a replacement for full anchor binding.

The [remote worker](../crates/server/src/remote_storage.rs) adopts on every
replica; upload scheduling runs on leaders. Pending recovery validates
cluster/region and the exact committed term, rechecks remote bytes and resumes
the same attempt. SSTs and manifest history currently remain retained. Global
GC cannot use the local pending journal as an enumeration of all owners.

## Format and compatibility inventory

These are current readers/writers, not a promise that old binaries understand
new formats. Bounds apply before allocation where supported by the codec.
Checksums detect the corruption covered by their assumptions; they do not
authenticate a malicious object store or prove device persistence.

| Format | Current encoding and integrity | Compatibility / refusal boundary |
| --- | --- | --- |
| Root descriptor | `KV9ROOT\0`, version 1, canonical binary encoding; SHA-256 root digest is bound by store records | Validate version, fields, complete consumption and root/store agreement; no implicit new root during recovery |
| Store identity | `KV9STOR\0`, fixed cluster/node/incarnation/root-digest tuple | No standalone checksum or explicit version field; exact layout and lifecycle/root cross-checks are required |
| Store lifecycle | `KV9LIFE2`, SHA-256 over the complete payload; reader also accepts `KV9LIFE1` | Existing writer-format fencing precedes activation; malformed or missing activated state is not fresh state |
| Legacy engine WAL | `KV9W`, records v1/v2; CRC-32 over version through payload, including v2 kind/term/index/length; 64 MiB payload bound | v1 has no applied pair; v2 distinguishes positioned/unpositioned writes; unknown version/kind and complete valid-CRC invalid positions refuse open |
| Segmented topology | `KV9W\x03SEG1`; SHA-256 over preceding bytes; 4 MiB bound and at most 4,096 closed descriptors | Old single-file readers reject version 3; topology selects the chain; highest filename never selects a successor |
| Segment / frame | `KV9SEG01` header; `KV9R` frame; header CRC plus frame CRC binding stream/header/payload, excluding inner CRC fields | Exact selected identity/sequence/predecessor; complete corruption refuses; only an incomplete active final frame is repairable |
| Full checkpoint | `KV9CHECKPOINT\x01` plus JSON; 1 MiB descriptor bound; 48 MiB referenced serialized state bound | Descriptor itself has no trailing checksum; pending/topology/Raft containers provide integrity. Decode checks scope presence, position and ordered nonoverlapping content-addressed SST references |
| Pending flush | `KV9PENDING\x01`, LE generation, checkpoint bytes, trailing SHA-256 over all preceding bytes | 1 MiB + 128 byte file bound; exhausted generation, malformed/truncated/corrupt published state refuses; a temporary file is not a pending slot |
| SST | `KV9S`, version 1; CRC-32 over all bytes preceding the checksum; SHA-256 in object key/reference | Full fetch and parse; validate CF, unique sorted keys, bounds and footer; missing/corrupt bytes are errors, never an empty table |
| Raft log | BE length, FNV-1a over kind + body; kinds 1–5 include indexed `ConfState` and experimental lease epoch | No outer version/capability envelope; 64 MiB body bound; unknown checksum-valid kind/invalid protobuf refuses; old builds refuse unknown kinds |

Two existing boundaries need explicit preservation during migration. Legacy
non-strict WAL replay can treat bad magic/length/checksum as a tail; strict
migration refuses complete corrupt records. The Raft log parser also treats a
bad checksum/length as a tail, including one in the middle of a file. Those
are existing failure-model limits, not the stronger segmented-frame corruption
guarantee. A new common codec must not silently inherit them for anchors.

Checkpoint JSON currently accepts unknown fields through serde. Consequently,
adding a security-, membership- or retention-critical field to v1 is unsafe:
an old reader could ignore it. The new anchor must use a separately versioned
envelope and explicit required capabilities. Define canonical bytes, maximum
size/count for every collection, checksum coverage of the complete header and
payload, checked arithmetic and rejection of unknown required capabilities.
Unknown optional extensions must remain in the byte identity even if a reader
does not interpret them. No decoder may re-encode away fields and then certify
that it validated the original object.

WAL migration remains an offline writer transition under the store lock. Keep
the original layout authoritative until the new files and their selecting
namespace are durable. Preserve exact data/position equality, reject conflicting
or decreasing positions, and permit legal index gaps. Large legacy states that
cannot fit one positioned record retain their unpositioned prefix after the
verified marker transition. Segmenting those bytes does not remove that pin.
A future frozen local-base conversion needs its own source/image authority and
proof; an ordinary Raft watermark cannot invent positions for those records.
Rolling upgrade, downgrade and format-4 local-base acceptance remain separate.

## Common recovery anchor: required semantic interface

The following names define data and validation obligations, not an already
exported Rust API or a final binary layout. Keep serializable descriptors
separate from private capabilities minted by validation/publication.

| Field / bundle member | Required binding |
| --- | --- |
| Format envelope | Version, required capabilities, length/count bounds, complete-byte digest and canonical identity |
| Cluster root | Cluster ID and exact root digest; retrieve and validate the matching root descriptor |
| Group scope | Region ID, configuration/data epoch and complete owned key range; a group identifier alone is insufficient after split/merge |
| State cut | Exact applied `(term,index)` and state-image digest, including all metadata needed to replay the tail |
| Configuration at cut | Full `ConfState` at the cut, including incoming/outgoing voters, learners, next learners and `auto_leave`; bind the conf-change index, which cannot exceed the cut |
| Manifest authority | Manifest generation, predecessor/change identity, descriptor digest and evidence of successful ordered apply; preserve the publication position separately from the image cut |
| History evidence | Durable winner/effect records and their exact generation/attempt scope, plus the retention owners requiring them |
| Resource closure | Immutable SST identities, sizes/hashes, version roots and all recursively required resources; no missing child can mean an empty state |
| Protocol continuation | Snapshot boundary and the retained Raft suffix needed after it, with committed term/configuration agreement; restore cannot lower durable term/vote |
| Local installation | Destination node/store incarnation, local WAL stream identity, installation generation and exact selected anchor digest |
| Transfer authority | For a new store or group, a committed admission/ownership transition binding source scope to destination; a copied directory is not that transition |
| Retention authority | Durable pin IDs, owner generations and the ledger view that makes the bundle restartable and transferable |

The portable group anchor and destination installation record are separate.
An anchor must be usable on an authorized surviving/new replica; embedding the
donor's store incarnation as the only acceptable destination would prevent
takeover. Record provenance separately. Bind the destination incarnation in
the install record and verify its admission before giving it a writer.
Similarly, the random segmented stream ID is a local file identity, not a
Raft membership credential.

There are at least three distinct positions: the state cut `c`, the manifest
publication `p`, and the configuration-change position `k`. A checkpoint can
freeze at `c` and be proposed/applied later at `p`; its state at `c` cannot
contain its own later publication. Validate exact terms at each position using
the corresponding authority. Do not label `manifest_at`'s stored checkpoint
watermark as the Raft position where the manifest won. Carry an independent
publication certificate or retain/replay the certifying log. Likewise,
`k <= c` alone does not prove the supplied `ConfState`: replay the committed
configuration transitions or validate a certified snapshot boundary.

Historical epochs/configurations are valid at their certified cut. Recovery
must validate the retained transition chain to the epoch/configuration under
which it will serve; it must not reject every older checkpoint merely because
the live group advanced. Conversely, comparing only numeric epoch maxima
cannot certify a different range, root or ownership lineage.

An anchor validation result has three outcomes: `Validated`, `Unavailable`
(required bytes/evidence could not be obtained), or `Invalid` (contradictory,
unsupported or corrupt input). Neither of the latter two yields a serving
engine or a reclaim token. An unavailable object does not authorize rollback
to a checkpoint that loses acknowledged state.

## Publication and crash protocol

Local recovery continues to hold the store guard. A future portable-anchor
publication additionally requires replicated retention authority before a
resource can become visible to a GC-enabled namespace:

1. Freeze one exact state/configuration cut under the existing apply/view
   fences. Construct and validate immutable resource descriptors outside apply.
2. Register a durable upload/build owner before creating resources that GC may
   discover. This owner pins source views/history and reserves exact destination
   object instances. Current upload-before-`catalog.pending` is permitted only
   while object deletion is disabled.
3. Upload and verify each object. Remote I/O remains outside ordered apply.
   Failed/ambiguous PUT retains the owner; a listing or ETag is not preparation.
4. Persist the exact pending attempt before proposing. Publish references only
   through a committed operation that validates owner tokens, epoch/generation
   and descriptor identity. A committed losing proposal grants no manifest.
5. Observe successful local ordered apply. Transfer/add pins for the published
   version and required winner/effect evidence before releasing the build owner.
6. Write and fsync the local candidate install record; atomically select it and
   fsync the parent namespace. Bind the anchor and retained file set in the
   same selected generation. Preserve the old root until this succeeds.
7. Only then expose the installed view or reclaim covered local files. On
   ambiguous publication fence the writer and recover against a complete
   permitted root. Do not continue writing via a possibly obsolete inode.
8. Keep active/straddling/unpositioned segments and every externally pinned
   resource. Engine WAL coverage is not permission to truncate Raft history.
9. Release the old owner after durable successor ownership/settlement. Cleanup
   is idempotent housekeeping; a failed DELETE or unlink does not undo the
   installed anchor. Retain enough metadata to retry safely after restart.

| Crash / invalid-input cut | Authoritative recovery state | Forbidden inference or action |
| --- | --- | --- |
| Unknown required version/capability, malformed anchor | Refuse before namespace mutation or serving | Ignore the new field and open as v1 |
| Valid digest, wrong root/cluster/store/range/epoch | Refuse the incompatible installation | Invent a new root or reuse a dead voter's identity |
| Valid image, wrong term/configuration/winning transition | Refuse; preserve original inputs | Treat descriptor presence in any committed command as successful CAS |
| Upload before completion | Old anchor plus durable build owner | Missing object means empty table; orphan age means deletion permission |
| Objects verified, pending not yet published | Old anchor plus build owner | Resume a different attempt and drop the original resources |
| Pending durable, propose result unknown | Same pending identity and all its pins | Timeout, replaced proposal or later generation proves failure |
| Manifest applied, pending clear interrupted | Applied version plus old recoverable attempt | Drop history before that attempt can settle |
| Candidate install fsynced, selection not durable | Complete old or new selected root; both covered | Select the largest orphan filename |
| Selection durable, cleanup incomplete | New anchor plus possibly extra obsolete files | Require cleanup to succeed before the new anchor is valid |
| Pin transfer interrupted | Old owner or both owners retained | Zero-owner interval between removing old and adding new |
| Snapshot installed, Raft truncation interrupted | Certified snapshot and retained legal suffix | Discard term/vote or configuration state with the application WAL |
| Delete intent committed, worker paused/restarted | Permanently retired object instance | Re-reference that instance before a delayed DELETE arrives |

Successful file fsync establishes file-content durability under the declared
filesystem/device model. It does not establish a newly created directory entry.
Successful rename followed by the required parent/ancestor directory syncs
establishes namespace selection. Unsynchronized rename/unlink outcomes may
revert across a crash. A failed sync can be ambiguous, so callers lose authority
instead of treating the failure as a rollback. These premises need deterministic
filesystem cuts and actual IOChaos/process evidence; tmpfs and process kills
alone do not qualify physical power-loss behavior.

## Retention owners and transitions

Pins name immutable resource identities, not mutable paths or just a watermark.
A resource identity includes cluster/root, resource kind, stable object or
stream instance and content identity. A durable `OwnerId` includes its kind,
scope, operation identity and monotonically fenced owner generation. Local
owners additionally bind node/store/process lifetime as appropriate. No PID,
wall-clock timeout, current leader flag or object listing supplies ownership.

| Owner | Resources retained | Release / restart condition |
| --- | --- | --- |
| Upload/build attempt | Frozen source version, planned object instances and reconciliation history | Durable transfer to pending/published owners or proven abort; restart recovers original operation identity |
| Pending manifest attempt | Exact prepared references, predecessor window and sufficient winner/effect evidence | Typed durable settlement and transfer to any surviving version owner; `Unknown` keeps the complete closure |
| Published version / current manifest | All referenced SSTs, version metadata and required catalog state | Durable successor publication plus all reader/transfer/backup owners detached |
| Reader/view | Exact immutable version and transitively its resources | Last reader leaves after admission is closed; a paused reader remains a reader |
| Local checkpoint installation | Selected anchor and retained active/closed WAL chain | Durable replacement selection with proven coverage and independent pins preserved |
| Raft snapshot / transfer | State image, exact boundary/configuration, transfer identity, protocol and reconciliation evidence | Receiver's durable authorized install, or durable fenced abort; donor disappearance alone does not release |
| Store/range migration and split/merge | Source view, destination objects, route/epoch transition and reconciliation evidence | Committed ownership handoff plus all transfers/readers settled; no filename-based adoption |
| Backup / PITR | Sealed anchor set, required log interval, root/identity metadata and all snapshot references | Explicit committed retirement of the backup/retention point; elapsed wall time alone is not release |
| Legacy migration | Original source and any unpublished replacement image | Durable validated selection with complete source coverage; positioned maxima do not cover unpositioned bytes |
| Delete intent | Object-instance tombstone and durable work identity | Execution/retry metadata may compact only when no delayed operation or ID reuse can invalidate the tombstone |

A reader need not commit a Raft record per GET. S03 may aggregate local read
guards beneath a durable replica/version owner. Acquisition and retirement
must serialize: acquire the guard before a view can be retired; keep its
durable parent until admission closes and all guards drain. Process-crash
cleanup needs proof that the old lifetime can no longer use the view. Network
partition or suspicion of death supplies no such proof. A missing replica may
delay reclamation; it must not prevent an available quorum from serving.

The shared ledger interface must implement the following semantic operations:

| Operation | Precondition and durable result |
| --- | --- |
| `Acquire(owner, resources)` | Idempotent exact request; refuse conflicting owner reuse or retired resource instances; success means every member is pinned before use |
| `Publish(owner, descriptor)` | Validate the acquired resource closure and ordered CAS; atomically install the version/history owner or retain the prepared owner on refusal/unknown |
| `Transfer(from, to, resources)` | Acquire destination first, or use one atomic transition; interruption cannot leave zero owners |
| `Settle(attempt, evidence)` | Evidence binds the exact attempt/window; persist the verdict and required ownership transfer before clearing the pending slot |
| `Release(owner, generation)` | Match the exact current owner generation and prove its use ended; stale/duplicate release cannot decrement another owner's reference |
| `Retire(resource, ledger_version)` | In one serialized authority domain, verify no pins/references, reject future acquire, and commit an irreversible delete intent |
| `Recover(scope)` | Restore owners, verdicts and tombstones before admitting publication or GC; absent/unsupported evidence returns unavailable/invalid, never zero references |

Refcounts are a derived acceleration, not the sole authority. Enable GC only
after a consistent backfill from all manifests, versions, pending/build owners,
readers, snapshots, transfers and backups, and after fencing writers that do
not register owners. Catch up mutations during backfill before activation.
Backfilling just the current manifest loses the other owners.

Before S07 introduces distributed ownership sharding, serialize shared-object
references and deletion through the existing replicated metadata authority.
An owner label may name another region; that region's isolated refcount cannot
certify the global absence of owners. Sharding this authority later needs an
explicit pin-transfer protocol and proof. It must not introduce an indispensable
external coordinator or a designated database node.

### Delayed deletion and reuse

A committed delete intent alone is insufficient if the same object key can be
referenced again. Counterexample: GC commits deletion of unreferenced `K`, its
worker pauses, a new manifest references `K`, and the old DELETE then succeeds.
Checking leadership again cannot recall an already issued request.

Therefore retirement permanently fences the exact object instance against new
references. A new upload of identical content uses a new object instance/key
generation; the content digest remains its integrity identity. Existing v1
hash-only keys cannot be reused after retirement. S07 must either retain their
tombstones indefinitely or migrate new references to versioned object keys.
Do not garbage-collect the tombstone on a timeout while stale PUT/DELETE work
or old writers can still address that instance. This is an additional enabling
condition for the older deletion contract in [OBJECT-STORAGE.md](OBJECT-STORAGE.md).

## Historical settlement across snapshots

The exact attempt is `(scope, expected_generation, change_id, descriptor)`.
For predecessor `g`, a certified winner at `g + 1` can prove this attempt won
or can never win. A latest pair beyond that window cannot decide it. Positive
effect evidence may settle the desired outcome without asserting the original
proposal won. Those verdicts stay distinct.

Before snapshotting away a generation, retain a durable owner for every
unresolved attempt and carry sufficient certified winner/effect evidence into
the replacement closure. This is a ledger of semantic facts, not just a list
of currently live SSTs. A local pending file on an unavailable voter cannot
be discovered by scanning the leader's current manifest; all attempts must be
registered before history pruning is enabled.

Unknown/unavailable evidence preserves the attempt and its object/history pins.
History cannot be deleted on the assumption that its absence will later prove
the attempt failed. Releasing an attempt owner does not release an independent
snapshot, reader or backup owner of the same bytes. Compaction's replacement
of SSTs also cannot reinterpret current `covers_effect` subset matching as
evidence that arbitrary destructive changes are equivalent.

## Proof obligations and implementation refinement

The existing inventory is split by component. Reuse the actual results linked
below; this design adds no new checked-theorem or Chaos acceptance claim.

| Boundary | Existing specification / evidence entry | Remaining composition |
| --- | --- | --- |
| Durable vote and Ready publication | [correctness gates](CORRECTNESS-GATES.md), [Ready publication](READY-PUBLICATION.md), [Lean inventory](../proofs/lean/theorems.json) | Bind restored term/vote and installed snapshot to the same Ready authority; do not lower protocol state |
| Root/store authority | [store lifecycle](STORE-LIFECYCLE.md), [root formation](ROOT-FORMATION.md) | Portable anchor admission, destination incarnation and ownership transfer |
| Metadata constraints and planning | [metadata planning](METADATA-PLANNING.md), [TLAPS inventory](../proofs/tlaps/README.md) | Shared pin ledger atomicity, reference CAS and global deletion fence |
| Atomic data/position frames | [segment proof](SEGMENT-PROOF.md) | Full image at the exact cut and independently certified manifest publication |
| Local file selection / reclaim | [WAL topology proof](WAL-TOPOLOGY-PROOF.md) | Validated anchor premise plus external reader/snapshot/backup pins |
| Pending/winner/effect reconciliation | [object-storage contract](OBJECT-STORAGE.md), manifest state machine and seam above | Durable settlement transfer and preservation of facts across snapshot/history pruning |

The new TLA+ model must distinguish visible/durable local selection, applied
ledger transitions, active owners, unpublished uploads, unresolved attempts,
retired instances and arbitrarily delayed physical deletion. State safety is:

1. Every published reference and permitted live use has a durable owner.
2. Every acknowledged prefix has a valid selected recovery closure, including
   metadata/configuration and the required protocol continuation.
3. Every unresolved attempt retains resources and enough evidence to preserve
   its unknown status or produce a sound later settlement.
4. A physically deleted instance has no owner or future acquire authority.
5. Restart/transfer never grants a second writable owner or lowers term/vote.

The induction must cover acquire-before-publish, overlap during transfer,
generation-checked release, zero-owner retirement, delayed execution and crash
recovery. The deletion step relies on both absence of owners at retirement and
the permanent future-acquire fence; omitting either admits the counterexample
above. Local publication additionally composes the old/new-root durability
proof, rather than assuming rename makes a new root durable immediately.
Model missing evidence explicitly; it cannot be modeled as a negative verdict.

Conditional progress requires an available quorum, reachable required objects,
successful eventual I/O, fair execution and eventual release/settlement of the
owners in question. Without the last condition, retain data and report debt;
do not promise bounded disk during an indefinitely pinned backup or stalled
reader. Database quorum progress must remain possible when an upload/GC worker
or one replica fails. The object store remains the only allowed service
dependency exception.

TLC explores finite failure schedules; TLAPS must prove the parameterized
invariants and conditional progress. Keep explicit source mappings for codec
validation, minted capabilities, apply transitions, install publication and
deletion. The written induction requirements here do not discharge those proofs.

## Implementation order and acceptance

| Increment / owner issue | Concrete work and necessary controls |
| --- | --- |
| C04 next | Implement bounded common descriptors and capability validation. Test unknown required capabilities, valid-digest wrong root/range/term/configuration, distinct cut/publication positions, destination admission and exact-byte identity. Define ledger operations without enabling deletion. |
| C04 proof | Add retention/anchor TLA+ state and TLAPS induction with source mappings. Remove acquire-before-publish, transfer overlap, generation checks and retirement fence separately; retain actual counterexamples/failed obligations. |
| S01 / #15 | Preserve segmented publication and unpositioned pin rules. Add external-pin coverage before any new reclamation authority; existing engine-only topology proof does not authorize protocol truncation. |
| S03 / #17 | Integrate immutable versions and aggregate reader pins, admission/drain ordering, upload owners and bounded backpressure. Crash/restart must preserve durable parents of live references. |
| S05 / #19 | Install data, exact configuration, historical evidence and local identity through one recoverable selection; retain correct term/vote and log suffix. Interrupt each install/truncation boundary and refuse incompatible anchors. |
| S07 / #21 | Backfill and fence legacy writers, implement global owners, irreversible per-instance delete intent, idempotent workers and pin-aware history pruning. Delay DELETE across leadership change and attempt key reuse. |
| D03–D05 / #24–#26 | Transfer the full owner/evidence closure through learner admission, migration, split and merge; certify destination scope/epochs before ownership changes. |
| O01 / #31 | Seal and restore the complete multi-group anchor/log/identity closure. Bucket-only SST reconstruction is not backup acceptance. |

Actual Chaos Mesh acceptance must exercise owner takeover on every voter,
pending settlement after the majority advances, receiver/donor failure during
snapshot transfer, partitions plus delayed deletion, and EIO/ENOSPC at local
publication. Record successful, refused and unknown operations in complete
histories; observe the injected effects and retain all original failures.
Use deterministic filesystem cuts for file-versus-directory outcomes Chaos
cannot precisely select. Qualify physical disk and failure domains separately.

This documentation increment changes no runtime, wire format, proof program or
test harness. Local validation checks links, referenced source symbols, original
roadmap checkboxes/dependencies and repository snapshot limits. It does not
rerun or expand the already accepted performance/Chaos results. The complete
write-candidate comparison remains pending its original storage envelope.
