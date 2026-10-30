# Backup and restore of a keyspace

Design packet, 2026-10-29. Follows the operations drill
([OPERATIONS-DRILL-2026-10-29](OPERATIONS-DRILL-2026-10-29.md)), which established that kv9 has
high availability and no backup: [OPERATIONS](OPERATIONS.md) §8 lists the gap, the
[C04 contract](RECOVERY-RETENTION-CONTRACT.md) names "Backup / PITR" as a retention owner that does
not exist yet, and [QUICKSTART](QUICKSTART.md) §4 says the bucket alone cannot rebuild a cluster.
Builds on [SOURCE-CAPTURE](SOURCE-CAPTURE.md), [MIGRATION-AUTHORITY](MIGRATION-AUTHORITY.md),
[REPLICA-ADDITION](REPLICA-ADDITION.md), [RANGE-SPLIT](RANGE-SPLIT.md) and
[RETENTION-LEDGER](RETENTION-LEDGER.md).

> **Status: increments 1 (backup) and 2 (restore into this cluster) implemented, 2026-10-29.**
> Kinds 119–124, the capture subject generalization, `runtime/backup.rs` and
> `runtime/restore.rs`, `kv9-admin backup create / list / describe / verify / retire` and
> `restore create / list / describe`, `task abort` of an incomplete backup or an unpublished
> restore, the Lean models (`scripts/prove-backup.py`, `scripts/prove-restore.py`) and
> `scripts/backup-e2e.sh` on real MinIO (backup, the logical error, the restore), in CI. §12
> records what the implementation settled. Increments 3–5 (another cluster's bundle, the fast
> path, PITR) are still design.

## The problem

Everything the cluster keeps today is a copy of the present. Three to seven replicas survive node
and disk loss; checkpoints in object storage let a replica reopen and a migration install an image;
replica repair and voter replacement put lost copies back. None of that survives a *logical* error:
a bad `delete-range`, a client that overwrote a keyspace, an operator who split and merged the
wrong thing. Replication copies the mistake to every replica within milliseconds, and the images
in the bucket are reclaimed and re-pinned by the operations that made them, not kept for anyone
else. Nor does it survive the loss of the cluster's identity: the root descriptor and Raft logs are
required to read what the bucket holds.

What is missing is a **retention point**: a committed, immutable, self-describing set of range
images that an operator asked for, that nothing reclaims until the operator says so, and that can
be turned back into a serving keyspace — in this cluster or in another one that can read the
bucket. The pieces exist. Capture freezes a group at an exact cut, uploads a verified SST closure
and publishes owners for it (SOURCE-CAPTURE, MIGRATION-AUTHORITY); the retention model already
reserves `ResourceKind::Backup`; split population proves a copy row by row with a chained digest
and publishes a new partition atomically (RANGE-SPLIT §9); `CheckpointReader::restore` turns a
manifest back into an engine. This packet composes them into two primitives, `backup` and
`restore`, driven the way every other operation is driven: committed rows, a reconcile step on the
node whose role it is, stages in the catalog, `kv9-admin` to ask and to watch.

## 0. Decisions

1. **The unit is a keyspace.** An operator thinks in keyspaces; a keyspace is one or more bound
   ranges on data groups. A backup is one anchor per range plus the catalog shape that ties them
   together. The `system` keyspace is not backed up by this packet (its contents are the catalog;
   §8).
2. **Consistency is per range, and that is all kv9 promises.** Each range is captured at its own
   exact cut `(term, index)` under its leader's fences. No atomic batch ever crosses a range
   boundary (RANGE-SPLIT §12), so there is no invariant between two ranges that a per-range cut
   could break. A backup is a set of per-range cuts taken within one window; the window is
   reported, not hidden.
3. **A backup is a retention owner.** Its anchors pin the manifests and SSTs they reference, in the
   same ledger migrations use, at `Published`, under an owner whose kind is `Backup`. The only
   release is a committed retirement by the operator. Elapsed time releases nothing (C04).
4. **Restore creates, never overwrites.** A restore produces a *new* keyspace under a name the
   operator chooses, on *new* data groups. It rewrites no identity, bypasses no CAS and touches
   neither the source keyspace nor its groups. "Put it back under the old name" is a restore plus
   the application switching names; an in-place restore would be the one operation that could
   destroy acknowledged state and it is not offered.
5. **Restore copies rows through the new groups' logs.** The first restore path is population: the
   new group's leader reads the anchored image and writes it through its own log in pages, proving
   the copy with the same chained digest the split uses. The result is an ordinary group with an
   ordinary log. The fast path — installing the image as the new replicas' snapshot base — is a
   later increment (§10), because it needs a rebinding step that the install verifier currently
   refuses by design (C04: "wrong root/range/epoch → refuse").
6. **The bundle is self-describing and lives in the bucket.** One object per backup carries the
   format envelope, cluster provenance, the catalog shape and every range anchor with its object
   closure. A cluster that can read the bucket can restore the bundle without the source cluster's
   root or logs. The SSTs are not copied: the bundle references the content-addressed objects the
   capture uploaded.

## 1. The rows

Six TASKS kinds, continuing the sequence after 118 (the first four are built):

```
119  KV9BKP01  backup intent      operation, keyspace (id, tenant), label, sequence,
                                  requested_unix, ranges[]: (region, creation_task, bind_task,
                                  epoch, range capture operation)  — the shape, frozen at intent;
                                  boundaries come from the bind rows, the bundle carries them
120  KV9BKA01  backup anchor      backup operation, range operation, region, cut (term, index),
                                  configuration position, image_digest, manifest_digest, objects,
                                  object_bytes, rows, rows_digest, restorable, owner — one per
                                  range, by its leader, through the shared evidence path
121  KV9BKR01  backup retirement  operation, retired_unix          — the only release
122  KV9BKC01  backup completion  operation, bundle key, bundle digest, bundle bytes, ranges,
                                  completed_unix                   — the bundle was written and
                                  the whole closure GET-verified; `complete` is this row
123  KV9RST01  restore intent     operation, backup task + operation, sequence, the NEW keyspace's
                                  reserved id, tenant, name, requested_unix, targets[]: (source
                                  region, source range operation, target creation_task, target
                                  region, start, end) — the targets are created and activated in
                                  the SAME catalog transaction; a bundle reference instead of a
                                  backup task is increment 3
124  KV9RSE01  restore evidence   operation, source region, target creation_task, rows,
                                  rows_digest — one per target, by its leader; committed only when
                                  rows and digest equal the anchor's; publication needs all
```

`BackupKeyspace(root, operation, keyspace, label)` at the metadata leader reads the keyspace's
committed bindings, refuses if any of them is sealed, TTL-activated or under a live split, merge
or replica plan (the same `live_range_operation` / `live_plan_on_region` family), and commits the
intent with the frozen range set. An identical resubmission confirms; a divergent one refuses.
While a range's anchor is not yet committed, that range refuses a new split, merge or plan: the
capture slot and pins are the plan's own mechanism and one operation binds a group at a time
(REPLICA-ADDITION §1.1). Once anchored, the range is free; the backup keeps its pins regardless of
what the source does afterwards.

`RestoreKeyspace(root, operation, source, name, tenant, voters?)` validates the bundle (§2),
creates one empty group per source range (`plan_child_group` on the given voters, or placement's
least-loaded three active stores), activates them, and commits the intent. The new keyspace is
**not yet visible**: its bindings are inserted by the publication (§5), exactly as a split's
children are.

## 2. The bundle

Object key `backups/<cluster-id>/<backup-task>/bundle` in the configured bucket, magic
`KV9BND01`, written by the metadata leader once every range is anchored, read by any restore.
Its fields follow the C04 anchor table:

| Member | Content |
| --- | --- |
| Envelope | version, required capabilities, counts and length bounds, digest of the whole bundle |
| Provenance | cluster id, root digest, backup task, operation, label, requested/anchored times |
| Catalog shape | keyspace id, tenant, name; ranges in key order with epochs and boundaries |
| Per-range anchor | the kind-120 fields, plus the capture record bytes the installer consumes and the list of object keys with sizes and digests (the resource closure) |
| Retention | the backup owner ids and the ledger generation at anchoring |

A bundle is **complete** when the leader has `GET`-verified every referenced object against its
digest after writing the bundle; only then does the backup reach `complete`. A bundle that fails
verification keeps its pins and reports `incomplete` with the missing key; the operator aborts
or retries the range (§3). `backup verify <id>` repeats the closure check at any time: a backup
whose closure no longer verifies is a finding, never silently fine.

The bundle references the SSTs where capture put them; nothing is copied. This is why the backup
owner must pin them (decision 3): the same objects may also be referenced by a migration or a
live manifest, and each owner releases independently.

## 3. The backup driver

Stages shown by `backup describe` and `group describe` (`backups[]` on each range's group):
`intended → capturing → anchored → complete | incomplete | aborted | retired`.

Each reconcile pass, on the node whose role it is, one step, derived from committed state:

| Where | Step | Reuses |
| --- | --- | --- |
| range leader | `plan_image`: freeze the cut, record the configuration at the cut, plan the manifest | `plan_capture` with a backup subject instead of a migration destination (`BackupOwners` beside `MigrationOwners`) |
| metadata leader | `bind`: publish the backup owner's pins for the planned manifest and its SSTs | the migration bind path, owner kind `Backup` |
| range leader | `capture`: upload and verify the closure; scan the frozen view once for `rows` and `rows_digest` | `upload_migration_published`; the split's `digest_half` over the frozen snapshot |
| range leader | `anchor`: commit kind 120 locally or over the internal evidence RPC | `RecordSplitEvidence`'s shape |
| metadata leader | `bundle`: when every range is anchored, write the bundle object, verify the closure, mark complete | `CheckpointReader` for the `GET`s |

Faults follow the plan driver's rules: a crashed node resumes from committed rows; capture and
install-class steps run once per process; a leader change moves the step to the new leader, which
reads the same rows. `KV9_RANGE_DRIVER=0` pauses this driver too. The fault matrix gains
`backup:<step>:before|after` points.

Abort (`task abort <backup>`, kind 117 with a new target) before `complete`: owners are released,
nothing is published, the row stays as history. After `complete` the verb is `backup retire`.

## 4. Retention

Owner id `H(root, "backup", operation, region)`, resource kind `Backup`, phase `Published` from the
bind step on. It pins the manifest and every SST of the anchor. The release is kind 121,
committed by `backup retire <id>`; it moves every owner of the backup to `Released` in one
transaction and tombstones the bundle. Objects are reclaimed by GC when it exists
(OBJECT-STORAGE §7.4–7.6); until then retirement is bookkeeping, which is exactly what C04 asks
for before deletion can be enabled: every reference has an owner the retention authority can see.

A backup whose source keyspace is later split, merged, migrated or deleted keeps its pins: it
references objects, not groups. A retired source group's objects stay until the backup is retired.

## 5. The restore driver

Stages: `intended → populating → populated → published | aborted`.

| Where | Step | Reuses |
| --- | --- | --- |
| metadata leader | validate the bundle: envelope, provenance recorded, closure `GET`-verified; create and activate the target groups; commit the intent | `plan_child_group`, `plan_activation` |
| each target leader | `populate`: open the anchor's image (`CheckpointReader::restore(manifest)` → an engine over the frozen rows), scan it in pages of 512 rows, write each page through the target's own `raw_batch_put` (its fence admits exactly the new keyspace and range), then digest the target row by row | RANGE-SPLIT §9 population, the split's `scan_page` and `digest_half` |
| each target leader | `evidence`: commit kind 123 with `rows` and `rows_digest` | `RecordSplitEvidence`'s shape |
| metadata leader | `publish`: when every target's evidence equals its anchor's `rows`/`rows_digest`, one catalog transaction inserts the new keyspace and its bindings and REGIONS rows; routable at once | `plan_split_publication` |

Until publication the new keyspace does not exist for clients: a restore is invisible or whole.
Abort before publication retires the target groups (sealed-binding authority is not involved;
they are empty groups with an aborted intent, reclaimed as a split's children are after an abort).
A population that restarts recopies its range: the target is empty until publication and the
fence refuses nothing twice, so the copy is idempotent.

Restoring from **another cluster**: `restore create --bundle <cluster-id>/<backup-task>` reads
the bundle under that prefix with this cluster's object-store credentials; provenance (source
cluster id and root digest) is recorded in the intent and shown by `keyspace describe`. Nothing in
the source cluster is contacted. Restoring into the **same** cluster takes `--backup <task>` and
resolves the bundle from the committed row.

Size: `CheckpointReader::restore` materialises an image in memory under the checkpoint image
budget. Increment 2 inherits that bound per range (a range larger than the budget refuses with a
typed reason at `validate`); increment 4 streams SSTs through borrowed views
([RAW-TTL-SST-VIEWS](RAW-TTL-SST-VIEWS.md)) and lifts it.

## 6. The verbs

```
kv9-admin backup  create  --keyspace <name|id> [--label <t>]        → backup task, ranges, stage
kv9-admin backup  list                                               task, keyspace, ranges, bytes, stage, age
kv9-admin backup  describe <task>                                    per-range anchors, bundle key, closure verdict
kv9-admin backup  verify  <task>                                     GET-verify the closure now
kv9-admin backup  retire  <task>                                     the only release of the pins
kv9-admin restore create  (--backup <task> | --bundle <cluster>/<task>) --as <name> [--tenant <id>] [--voters a,b,c]
kv9-admin restore describe <task>                                    per-target rows/digest, stage
kv9-admin task    abort   <task>                                     backup before complete; restore before published
kv9-admin keyspace describe <name>                                   gains: restored_from (cluster, backup, anchored_at)
```

SDKs gain the same calls; exit codes are the table's (0, 1 refused, 4 partial when a bundle's
closure verification is partial, 6 on `--wait` deadlines). `backup create --wait` waits for
`complete`; `restore create --wait` for `published`.

## 7. Invariants

- **B1 — a complete backup is immutable and pinned.** Every object a bundle references has a
  `Published` owner of kind `Backup` from bind until a committed retirement; no reclamation path
  may delete a pinned object (C04 ledger `Retire` refuses while pinned).
- **B2 — an anchor is exact.** Its cut is a frozen view of the range's leader at a durable applied
  position with the configuration at that position; its `rows_digest` is computed over that view
  and nothing later.
- **B3 — restore writes only new identities.** No row of the source keyspace, its groups, bindings
  or owners is modified by a restore; the target groups and keyspace are created by the restore's
  own transactions.
- **B4 — a restore publishes whole or not at all.** The new keyspace's bindings appear in one
  catalog transaction after every target's evidence matches; readback's partition rule refuses a
  partial shape.
- **B5 — restored content equals the anchor set.** Per range, `rows` and `rows_digest` of the
  target equal the anchor's before publication is admitted.
- **B6 — abort leaves nothing visible.** A backup aborted before `complete` publishes no bundle
  and holds no pins; a restore aborted before `published` leaves no keyspace and retires its
  groups.
- **B7 — the shape is frozen at intent.** Later splits, merges or moves of the source change
  neither the backup's range set nor its anchors.

## 8. What it does not do

- **Point-in-time recovery.** PITR needs the per-range log interval after an anchor archived
  somewhere (WAL segments are local files today). The anchor's exact cut is the base PITR will
  attach to; the owner table in C04 already lists "required log interval" under Backup / PITR.
  Increment 5.
- **Rebuilding a cluster from the bucket.** A restore needs a serving cluster to restore into;
  forming a new cluster from bundles alone (new root, new store identities, every keyspace)
  is a sequence of restores once a cluster exists, and is deferred with the identity questions it
  raises.
- **A cross-range consistent point.** Not a property kv9's data model has; not promised here.
- **Restore in place.** Decision 4.
- **The `system` keyspace and the catalog.** The bundle carries the catalog *shape* for routing;
  metadata recovery stays the metadata group's checkpoint and logs.
- **Scheduling and expiry.** `backup create` is one verb; cron and retention policy live outside,
  as drains and repairs do. `backup list` shows age so a policy can act.
- **Encryption and cross-bucket copies.** The bundle is as protected as the bucket.

## 9. Evidence

- Lean model `proofs/lean/backup-restore`: ranges, anchors, owners, bundles, targets; steps plan /
  bind / capture / anchor / bundle / retire / populate / evidence / publish / abort; B1, B3, B4, B6
  as theorems; controls (publish with one target's digest differing, retire releasing a pin still
  referenced by a live restore, abort after complete, bundle written before every anchor).
- `scripts/backup-restore-e2e.sh` on real MinIO: a keyspace split into two ranges and filled;
  `backup create --wait`; then the logical error (`delete-range` over half the keys and 200 new
  writes); `restore create --as orders-20261029 --wait`; the restored keyspace holds exactly the
  anchored rows, the source keeps the damage; a second cluster on the same bucket restores from
  `--bundle`; `backup verify` after the source groups were drained and reclaimed still verifies;
  `backup retire`; abort of a backup mid-capture and of a restore mid-populate leave B6 true.
- Fault matrix cells `backup:*:before|after` and `restore:*:before|after`; a drill step after
  §9b of `scripts/ops-drill.sh`.

## 10. The increments

1. **Backup.** Kinds 119–121, `BackupOwners`, the backup driver, the bundle, `backup create /
   list / describe / verify / retire`, the model, the e2e without restore (pins and closure are
   the claims). This alone turns "the images are in the bucket" into a retention point.
2. **Restore into this cluster.** Kinds 122–123, the restore driver by population, publication,
   `restore create --backup … --as …`, the logical-error e2e, the drill step.
3. **Restore from another cluster's bundle.** `--bundle`, provenance in `keyspace describe`, the
   two-cluster e2e.
4. **Fast restore.** Image install as the target replicas' snapshot base with a rebinding record
   (new root, region, keyspace) that the install verifier accepts under a committed restore
   intent; streamed SST views; lifts the image budget.
5. **PITR.** Per-range WAL segment archiving between anchors under the backup owner, `restore
   --to <index | time>` replaying the interval onto the populated base.

## 12. As built (increment 1)

- **The capture subject.** Every path that resolved a capture operation to a migration — the
  capture catalog (`capture::plan_record`), the ledger's owner-binding check and the capture
  recording on the metadata leader (`migration_capture/catalog.rs`), the source slot's
  `with_migration_capture`, the bind path and `MigrationOwners::new` — now resolves it through
  `data_groups::CaptureSubject`: a migration intent, or one range of a backup. A backup range's
  capture names a **synthetic destination** (node id `u64::MAX`, incarnation derived from the
  range operation), because a capture request needs a destination distinct from its source and
  the request format is durable; admission refuses that node id. The capture slot of a group
  (`capture.plan`) is reusable after a backup once the range's anchor names the same image
  (the slot's `superseded` rule), proven by the e2e's replica plan after the backup.
- **Owners.** A backup range publishes two owners through the migration bind path: the source
  snapshot owner (`kv9-backup-source-v1`) and the `Backup` owner (`kv9-backup-v1`, salted by the
  region). Release (kind 121) is a committed fact; the ledger has no release path in this
  increment for migrations either — GC, when it exists, consumes both.
- **Rows and digest from the image.** After the upload the range leader restores the image
  under the checkpoint image budget (`RemoteUploader::restore`) and digests it with the split's
  row digest. An image the budget refuses is anchored with `restorable: false`, rows 0; the
  backup is kept and `backup describe` says so (§11).
- **The bundle** (`KV9BND01`, `backups/<cluster-id>/<task>/bundle`) carries provenance, the
  keyspace's shape with boundaries, and per range the anchor and the committed capture
  descriptor (request, image cut and configuration, manifest with the SST closure). Completion
  is committed only after a `GET` of every referenced object verifies; `backup verify` repeats
  that check and the bundle digest.
- **Exclusivity.** `live_range_operation` answers `backup` for a range an unanchored backup
  holds; the split, merge and migration planners refuse with the backup named. A backup refuses a
  keyspace whose range is under a live split, merge or plan, and skips sealed bindings (the
  published history of a split or merge; publication seals and binds in one transaction, so a
  sealed row always has live successors). One live backup per keyspace: a second `create`
  confirms it; sequences advance once it is complete, aborted or retired.
- **Abort.** `task abort <backup>` is a kind-117 row with target `backup`, refused once the
  backup is complete (retire instead). Capture refuses permanently afterwards; a later backup of
  the keyspace is a new sequence with new range operations.
- **Driver.** `runtime/backup.rs`, status line `backup=`, steps `plan_image` (range leader),
  `bind` (metadata leader), `capture` (range leader: upload, digest, anchor receipt), `bundle`
  (metadata leader); fault points `backup:<step>:before|after`; `KV9_RANGE_DRIVER=0` pauses it.
- **Evidence.** `scripts/backup-e2e.sh`: a two-range keyspace of 300 rows; with the drivers
  paused a held backup refuses a split and a plan, confirms, aborts; with the drivers on and a
  crash armed after `bind`, `backup create --wait` completes with 150 + 150 rows anchored, a
  verified bundle, `verify` ok, the committed rows by kind; a replica plan then runs on the
  anchored range; `retire` retires and `list` hides it. Lean `proofs/lean/backup`: ten theorems,
  seven semantic controls.

### Increment 2, as built

- **The keyspace id is reserved at intent** (`SequenceKind::Keyspace`) and the keyspace row,
  REGIONS rows and bindings are inserted at publication, in one transaction. Between the two
  the targets carry descriptors for the reserved id (proposed once per term by each target's
  leader, as a split child's half is), so their fences admit the copy while no client can
  route to them: the name resolves to nothing until publication.
- **The copy** restores the source range's anchored image under the image budget
  (`RemoteUploader::restore` from the committed capture descriptor), scans it page by page
  under the SOURCE keyspace id, writes each page through the target's own `raw_batch_put`
  under the NEW keyspace id, then digests target and image with the split's row digest; the
  receipt carries the target's rows and digest, and `plan_restore_evidence` refuses it unless
  both equal the anchor's (B5). The whole range is copied in one worker step (images fit the
  budget by construction: an unrestorable anchor refuses the intent).
- **Names.** A restore refuses a name any keyspace has or a live restore is taking; the same
  backup under the same name confirms the live restore; after abort the next sequence is free;
  after publication the name is taken for good.
- **Retirement waits.** `backup retire` refuses while a restore of the backup is live, because
  the targets read its anchors and images (B1); a published or aborted restore no longer holds
  it.
- **Abort.** `task abort <restore>` before publication; the targets are abandoned empty groups
  that the retirement pass retires like an aborted split's children (B6). A published restore
  does not abort: its keyspace exists.
- **Evidence.** `scripts/backup-e2e.sh` phase D: `delete-range` over half of `orders` plus
  200 new rows; `restore create --backup <task> --as orders-restored --wait` publishes a
  keyspace holding exactly the 300 anchored rows while `orders` keeps the damage; the source
  name is refused; the live restore confirms; a second restore blocks `backup retire`, aborts,
  and its targets retire everywhere. Lean `proofs/lean/restore`: ten theorems, seven controls.

## 11. Decisions taken (2026-10-29, with the owner)

- **A range with an unanchored backup excludes replica plans too**, not only splits and merges.
  Capture slots are per operation and two captures on one group may well coexist, but nothing
  has verified it; a backup holds a range for tens of seconds, which a plan can wait. Revisit
  when a workload needs both.
- **Restore targets default to placement's least-loaded three active stores**, and `--voters
  a,b,c` overrides. "The source range's current members" is not a default: it stacks load on the
  same stores and is meaningless once the source moved or was drained.
- **Retirement tombstones the bundle; nothing is deleted.** Kind 121 marks the backup retired,
  `backup list` hides it unless `--all`, the bundle object stays for audit. Object deletion is
  GC's job, under C04's delayed-delete rules, when GC exists. The prefix is
  `backups/<cluster-id>/<backup-task>/`.
- **Increment 1 does not refuse large keyspaces.** A backup whose range exceeds the checkpoint
  image budget is still taken (its data is pinned and its anchors exact) and `backup describe`
  reports `restorable: no (range … exceeds the image budget; restore needs the streamed path of
  increment 4)` so that no operator mistakes "kept" for "restorable". A backup that keeps the
  data is always better than none.
