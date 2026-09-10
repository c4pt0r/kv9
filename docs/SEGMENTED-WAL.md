# Segmented engine WAL

Tracking: #15 (S01), with the recovery/retention contract in #14 (C04).

## Development status

The first implementation supplies `WalSegment`: exclusively created files,
checksummed stream/sequence/predecessor headers, atomic data/position frames,
streaming recovery, active-tail repair, immutable closed descriptors and
fail-stop append errors. `SegmentedWal` now adds durable topology selection,
rotation, checkpoint publication and whole closed-segment reclamation.
`WalEngine` now recovers and writes that layout, and the production runtime
migrates each verified legacy catalog before starting its Raft peer. Normal
checkpoint adoption publishes the anchor and unlinks covered closed files
instead of copying the surviving tail. Throughput, capacity and latency results
remain unmeasured for this integration.

Daily mainline implementation proceeds alongside the existing fault-environment
closeout. Module regressions run locally as code changes. Deductive protocol
proofs, modeled durability cuts and actual Chaos Mesh acceptance remain required
before closing #15 or accepting P1 performance/capacity claims.

## File and record contract

A stream has a nonzero independently allocated 128-bit local identity. A segment
has a positive sequence and the preceding stream's last exact applied position.
Its 60-byte `KV9SEG01` header checksums identity, sequence, position presence,
term/index and reserved bytes. Recovery compares it with the header selected by
durable topology. A directory entry alone cannot grant recovery authority.

Frames contain a 32-byte header (`KV9R`, payload length, position kind,
reserved bytes, term/index and header CRC-32), the existing batch payload codec,
then a CRC-32 binding the segment header and frame header (excluding their inner
CRC fields) together with the payload. Including an inner CRC would erase its
message's contribution through the CRC's fixed-residue property.
Header integrity is checked before using the length. Complete malformed headers,
payloads and nonmonotonic positions refuse
the entire open. Only incomplete final frames of the selected active segment
can be discarded. Closed segments require their exact sealed length, complete
frames and matching summary. CRC and successful filesystem synchronization are
explicit storage assumptions, not authentication or hardware guarantees.

Each append fsyncs before updating the synchronized summary. I/O failure fences
the writer. Sealing consumes that writer and yields a descriptor; publication
of the descriptor is a separate stream-owner obligation. Streaming replay owns
at most one encoded record plus the decoded batch at a time. The visitor must
discard all unpublished recovery state on error. The existing 64 MiB record
limit remains in force; the complete database still resides in memory.

Applied indexes strictly increase; terms cannot decrease. Index gaps are legal.
Unpositioned writes carry no watermark authority and pin their segment. A
segment straddling a checkpoint stays intact until the checkpoint covers its
last positioned record and every unpositioned write has separate migration
authority. Neither an upload receipt nor a pending flush permits reclamation.

## Stream-owner implementation path

1. Publish a versioned, checksummed topology containing the stream identity,
   active sequence, retained closed descriptors and exact checkpoint anchor.
   The anchor binds the full checkpoint manifest digest/scope and exact
   `(term,index)`. The existing Raft recovery checks must validate the cluster,
   region and committed position before tail replay or deletion.
2. Rotate under the append lock: synchronize the old segment, create/synchronize
   the successor and namespace, then durably publish the topology transition.
   Publish with temporary-file write, file fsync, rename and parent fsync. A
   publication error fences the stream. An orphan successor never becomes
   authoritative merely because it has the largest filename.
3. Adopt an ordered-applied checkpoint by atomically publishing its anchor
   together with the retained segment set. Preserve active and straddling
   segments. Unlink only closed files no longer required by that durable
   topology, then sync the namespace. Recovery may skip covered payloads only
   through validated checkpoint authority; it must reject the same corruption
   in an uncovered replayable record.
4. Migrate the old layout without relying on one full-state WAL record. Stream
   its records into unpublished segments, retain the original layout until
   the new topology is durable, and install a format fence the old writer
   refuses. Large unpositioned prefixes remain pinned until a verified full
   checkpoint accounts for their effects. Migration must not fabricate an
   applied position for individual unpositioned writes.
5. Integrate this owner into `WalEngine`, replace checkpoint's O(tail) copy with
   descriptor publication/unlink, and measure lock time, disk usage and recovery
   across repeated write/flush cycles. Then run the complete proof, persistence
   and actual fault-history acceptance. Engine WAL reclamation does not grant
   Raft protocol-log truncation; S05 retains that separate scope.

The stream topology is local replica durability metadata. It introduces no
database coordinator and no additional service dependency. Required replicated
checkpoint authority remains in the existing Raft group.

## Log architecture decision

Retain the present separate Raft protocol log and positioned engine WAL while
implementing S01. The former owns votes, terms, membership and committed-log
recovery; the latter atomically publishes applied data and its exact position.
Segmenting engine storage removes tail copying without changing either role.

The unified shared log described in DESIGN section 6.4 remains the target for
multi-group batching. Adopting it requires a recoverable per-group index,
group-aware retention/pins, commit/application ownership and an explicit format
migration. Replacing two logs before those interfaces exist would transfer
their obligations without implementing the required protocol. S01 must expose
segment/position summaries that this later owner can use, while retaining
independent truncation authority in the meantime. This decision does not
complete C04's wider snapshot, backup and pending-history retention contract.

## Local implementation checks

The engine library passes 117 tests, including nine segment regressions:
round-trip/position gaps and unpositioned pinning; every partial byte cut of an
active frame with subsequent append; complete corruption and valid-checksum
nonmonotonic positions; identity/predecessor refusal; rejected appends and
visitor failures; position/payload swaps and cross-stream frame transplantation;
valid-checksum malformed batches; actual write failure; and actual sync failure.
Warnings-denied engine Clippy and rustdoc, formatting and diff checks pass.

Independent review identified the need to bind payload, position and stream
identity in one frame checksum. The first swap regression also exposed the
fixed-residue mistake of including nested CRC fields. Both unpublished failed
attempts are retained with the final passing logs. These module checks are not
new Chaos Mesh or deductive-proof acceptance. The segment proof and stream
publication/reclamation integration remain work in progress under #15.

## Stream owner increment

`wal_stream::SegmentedWal` owns one selected topology and active writer. Its
bounded, SHA-256-checked topology contains the active header, up to 4,096 closed
summaries and the complete checkpoint reference. The `KV9W` version-3 prefix
makes the old single-file writer refuse before editing this format. Segment
files are selected by that topology under the stream's identity directory;
unpublished successors cannot select themselves through filenames.

Rotation consumes the old writer, synchronizes a successor and its namespace,
then publishes the next topology by file fsync, rename and parent fsync. A
failure fences subsequent writes until recovery. The exact previous applied
position is carried through empty and entirely unpositioned segments.

Checkpoint adoption checks the caller's ordered-applied reference against local
progress and known segment position bounds. Cluster/region identity stays fixed;
checkpoint epochs may advance monotonically. The caller still certifies exact
Raft and object authority. A rejected older checkpoint has no side effects, and
unpositioned history keeps its reclaim ban. Successful topology publication
removes only completely covered closed descriptors. Physical unlink follows
publication and can be retried independently; straddling and active files are
never rewritten. A housekeeping error can therefore follow a durably adopted
checkpoint, and callers must preserve that unknown/completed distinction.

`RecoveryPlan::read` is read-only. Recovery first invokes the caller's checkpoint
validation/restore callback, verifies that the selected topology has not changed,
and stabilizes it. It then streams retained records and repairs only the selected
active tail. Failed restore never repairs files. Covered files absent from the
published retained set are not opened, so a non-durable unlink or covered body
corruption cannot force replay of obsolete data. Missing or corrupt retained
files still refuse the whole open.

Local engine checks pass 128 tests, including eleven stream tests for rotation,
position gaps, empty successors, unpositioned pins, checkpoint epochs and term
contradictions, old-writer refusal, corrupted authority/retained identities,
pre-publication I/O failures, post-publication unlink failure, and comparable
within-frame covered/tail corruption. Engine Clippy with warnings denied passes.
The checkpoint unit fixture explicitly simulates the caller's committed-object
premise; it is not real MinIO or Raft certification. Full modeled file/directory
sync cuts, particularly failure after topology rename, and actual Chaos Mesh
acceptance remain in the integrated S01 gate.

## Engine and runtime integration

`WalEngine::open_with_uploader` recognizes the selected version-3 topology and
streams recovered batches directly into the visible index under construction.
Its `EngineReplay` result contains aggregate replayed/covered record counts and
discarded tail bytes, avoiding a second retained collection of all replayed
batches. Legacy v1/v2 opening remains available for offline upgrades. Engine
opening refuses complete legacy checksum/magic corruption instead of repairing
it as a torn tail; the old format still lacks an independent length checksum.
Missing or shorter-than-format roots beside an existing segment directory fail
closed instead of creating an empty legacy database.

The production runtime reads the selected checkpoint through
`WalEngine::checkpoint_reference`, certifies its exact encoded manifest against
the local committed Raft history, and then opens/restores the engine. It verifies
legacy marker authority and the recovered exact applied term before calling
`enable_segmentation`, all under the retained exclusive store guard and before
peer startup or Serving. The version-3 prefix rejects older single-file writers.
This local format transition adds no database service or singleton dependency.

Offline migration streams each valid legacy record into an independently named
segment stream through `catalog.migration`; both staging and final root resolve
to `catalog.segments/<stream-id>/`. The original `catalog.wal` remains selected
while copied segments rotate and synchronize. The final rename plus parent
fsync publishes the complete new layout. A failed transition fences the current
engine's writes, applied-position reports and new freezes until reopen. A crash
before final publication recovers the old layout; a visible complete new topology
is stabilized and recovered on reopen. Unreferenced streams from interrupted
migration are not elected or replayed; their garbage collection is still pending.

A migrated checkpoint is embedded with its exact position as the first segment's
predecessor, including the case of an empty surviving legacy tail. Any legacy
`catalog.checkpoint` sidecar becomes obsolete after this transition; subsequent
anchors live only in the selected topology. Recovery requires real MinIO restore
for that selected anchor before segment repair. Legacy unpositioned records are
preserved, not assigned invented positions, and continue to pin reclamation.
Large marker upgrades that retain an unpositioned prefix therefore still need
an explicit full-checkpoint migration authority before that prefix can be freed.

The integrated acceptance batch must still exercise runtime migration and remote
recovery, sustained actual segment rotation/reclamation, post-rename directory
sync ambiguity, and actual Chaos Mesh histories. The single-file segment proof
is a separate, narrower obligation from topology, migration and reclamation
proofs. This implementation increment does not close #15 or C01/C04.

Local integration validation: 577 workspace tests pass, 23 external-environment
cases remain ignored in that ordinary run, and workspace/all-target Clippy with
warnings denied passes. Four explicitly selected engine tests pass against an
isolated real MinIO container, including remote-object failure, straddling active
frames and empty-checkpoint-tail migration. The three-process raw KV fixture
passes leader death, new-leader reads/writes, deletes and old-node restart, with
version-3 topology and segment files observed on all three data directories.
Those process tests are local fault/restart evidence, not actual Chaos Mesh
acceptance or measured S01 performance results.

## Empty-source interruption fix

Before staging a segmented migration from a zero-length legacy WAL, the engine
now durably writes an empty legacy framing record. A checkpoint-backed tail
keeps its exact position; an initially empty source gets no invented progress.
This distinguishes the valid old root from a truncated new topology after a
failed migration. Migration omits only empty unpositioned batches so that the
framing no-op does not create a permanent reclaim pin. The failure and its
[syscall-cut regressions](SEGMENT-PUBLICATION-FAULTS.md) cover migration retry,
new writes, segment pinning and real MinIO checkpoint-backed empty tails.
