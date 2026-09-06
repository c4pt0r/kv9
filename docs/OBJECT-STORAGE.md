# Object storage — the durable layer

**Status:** contract for the Phase-2 disaggregated storage vertical slice. Approved by EdHuang
2026-09-05 ("达成共识就开始开发", "object store 的部分使用 minio"). Task #39.

This document is the agreed shape of the object-storage path *before* it is built. It exists
because the design had been settled in chat over several days and **not one word of it was in the
repository** — four people re-derived the plan order from memory on 2026-09-05, which is what this
file is meant to stop happening again.

Sources: `DESIGN.md` §6.4, §6.5, §5.1; `docs/ROADMAP.md`. Consensus-side half supplied by Rafa,
interface/security review by Tess, acceptance discrimination by Cindy.

Read `DESIGN.md` §6.5 first. This document does not restate it; it fixes the decisions §6.5 leaves
open and records the contracts that round one must honour.

---

## 1. Scope of round one

| In | Out (named so absence is deliberate, not forgotten) |
|---|---|
| MinIO `ObjectStore` backend (S3 API) | Real S3/GCS/Azure; TLS; production credentials |
| Immutable SST writer/reader, checksum, key range | Compaction; multi-level LSM |
| `PreparedSst` handoff value | GC / delete-intent execution |
| WAL record-format upgrade + segmentation + reclaim | Cross-region or cross-group recovery |
| `ManifestChange` propose/apply/query seam | `split`/`merge` manifest attach |
| Recovery: authoritative manifest + WAL tail | Backup / PITR / branch / clone |
| — | **Refcounts** (§7.4): no delete path, so none maintained; objects leak by design |

`LocalDirObjectStore` is **not** built. It was scaffolding standing in for a real object store;
MinIO replaces it. `MemoryObjectStore` survives **only** as a unit-test fixture and is never
evidence that object storage works (§8).

**Round one is done when the §8 story passes against a real MinIO process — not before.** A green
SST unit suite retires the *SST-format* risk alone; it does not make the slice half finished.

---

## 2. What is authoritative, and what merely holds bytes

`DESIGN.md` §6.5 invariant 1. Two mutable/immutable boundaries, and it is easy to collapse them:

```
immutable, on object storage    SSTs — write-once objects, never updated in place
mutable, in raft-replicated     the manifest: file-id list + LSM structure + watermark
   region state
```

**The manifest must be carved out explicitly.** Object storage sees only immutable *creates* and
*deletes*; every mutation and every ordering decision is raft-committed. This is exactly what
sidesteps object-store consistency weaknesses. If someone writes "durable state goes to object
storage" without excluding the manifest, the mutable pointer eventually lands on S3 and reimports
the problem the design was built to avoid.

### 2.1 What must never go to object storage

Ack-path consensus durability is local fsync, and this is **safety, not a durability preference**
(`crates/raft/src/storage.rs` already states it):

1. **HardState (term + vote)** — fsynced *before the reply leaves the node*. A restart that forgets
   a vote lets the node vote twice in one term: two leaders.
2. **Log entries** — a commit ack requires the quorum's local WAL to hold the entry. The ack does
   not wait for object storage, not for one millisecond.
3. **ConfState pairing record** — membership change plus its log position, same batch.

Object storage is a network round-trip with eventual-consistency semantics. It satisfies neither
the latency nor the ordering requirement above.

### 2.2 The trade-off this forces, written down rather than discovered later

Because the ack does not wait for object storage, acknowledged-but-not-yet-drained data lives only
in local WALs until it lands. Losing **one** node's disk is recoverable from the surviving replicas —
that is ordinary raft durability. What object storage does *not* cover is narrower and worth stating
with its quantifiers intact:

1. **Every replica holding the acked-but-undrained tail loses its local disk at once.** Only then is
   that window unrecoverable, because object storage never had it.
2. **Loss of a node's durable identity** (NodeId / incarnation), after which the identity can be
   wrongly reused — a failure object storage has no bearing on at all.

*(An earlier draft asserted flatly that "object storage does not cover single-node disk loss". That
is false under multi-replica raft, and the absolute phrasing hid the quantifier that carries the
whole meaning.)*

---

## 3. Call direction — fixed, and it never reverses

Ruled by Tess 2026-09-05, confirmed by Rafa:

```
engine            builds the SST and uploads it durably, yielding PreparedSst
region runtime    holds BOTH the engine and a propose-only handle; proposes a
                  content-derived ManifestChange
ordered apply     installs the authoritative manifest + watermark
only then         the WAL range it subsumes may be reclaimed
```

**`crates/engine` must not depend on `kv9-raft`.** The engine *declares what it needs* as a trait;
the server stitches the two together; raft never learns the engine exists. Dependency direction is
a standing constraint, not a preference of the moment.

The propose-only handle is **the deliverable of task #5** (Rafa, branch
`rafa/propose-consume-split`, based on `469f151`). Until #5 lands, the manifest proposal path has
no legitimate seam — this is the one *real* dependency between our two lanes. SST production does
not depend on #5 and starts immediately.

### 3.1 `apply` never touches object storage

The whole `engine → propose → apply` chain is synchronous and contains no tokio; tokio exists only
at the public gRPC edge, which bridges into the synchronous core. Confirmed by Rafa 2026-09-05:
`RaftGroup::propose` is a synchronous `fn`, the driver pump is a dedicated OS thread, and
`drive_apply` runs on that pump thread.

**Uploading completes before the proposal is made. That is part of the call-direction contract, and
it has a consequence worth stating as a rule rather than as background:**

> **The apply path must never perform object-storage I/O.** It installs references that are already
> durable. Every deadline and every blocking risk stays on the engine's upload thread.

Written in refusable form deliberately: when someone later proposes to fetch or upload inside
`apply`, the grounds for refusing it are this sentence, not an oral tradition.

**What actually enforces it today, stated exactly** — because "the invariant holds" and "something
holds it up" are different claims:

```
today            the Engine trait exposes no object-storage entry point, so the handle apply
                 holds cannot reach a store  ...  plus review discipline
                 (kv9_engine::ObjectStore IS re-exported and in scope in that crate; nothing
                  prevents a direct use, or threading a store in from elsewhere)
after task #9    apply is narrowed to a capability surface that does not include a store, with a
                 grep tripwire landing in that card's first commit
```

`ObjectStore` currently occurs **0 times** across `crates/{raft,server,txn,meta,region,common}` and
17 times in `crates/engine` alone. **That 0 is the invariant holding because nothing has had reason
to break it — not because something guards it.** A count of zero looks identical whether a rule is
enforced or merely unviolated, which is exactly why it should not be read as evidence of the former.

### 3.2 `ObjectStore` stays synchronous; the backend owns one worker

Ruled by Tess 2026-09-05. `Engine` and the server core are synchronous contracts, isolated behind
`spawn_blocking`/blocking-backend boundaries. Making `ObjectStore` async would push the async
boundary all the way into `Engine`, exceed the slice, and still require bridging back for the
synchronous read path.

**The MinIO backend owns one dedicated worker thread holding a Tokio runtime.** Synchronous methods
hand operations to it over a bounded channel and wait for the response. Do **not** build a runtime
per call, and do **not** `block_on` on the calling thread. A mistaken call from an async worker is
then merely observable blocking rather than a nested-runtime deadlock.

`Handle::try_current()` is explicitly **not** the guard: it can succeed inside `spawn_blocking`, so
it would reject the one legitimate route while claiming to protect it.

Channel and operation both carry explicit deadlines, and a timeout fails loudly.

---

## 4. Coordinates — the correction that cost us a design defect

**`new_watermark` is a replicated applied position — the `(term, index)` coordinate system of the
unified driver watermark. It is never a local WAL byte offset.**

This is already what `DESIGN.md` §6.5 says: the manifest change carries "the new file-ids **and the
range of log indices it subsumes**".

It is recorded here as a correction because the first version of this design proposed
`PreparedSst.covers_through: WalPosition` — a node-local byte offset published into a
raft-replicated structure. On another replica that offset addresses entirely different content, so
that replica would reclaim the wrong amount: **not merely meaningless elsewhere — actively
destructive.** Caught by Tess before any code existed.

Two things worth keeping, because the fix is cheap and the habit is not:

- **Never put a coordinate that is only meaningful in one frame into a structure shared across
  frames.** A replicated value must be expressed in a coordinate system every replica shares.
- The correct answer was already written in `DESIGN.md`. The error came from a *summary* of §6.5
  that had blurred "range of log indices" into "the matching WAL truncation watermark". **A summary
  that drifts from its source is worse than no summary: it is consulted with the confidence owed to
  the original.**

Each replica privately maintains its own `applied position → local WAL segment` mapping. That
mapping is **local, never replicated, and never part of a `ManifestChange`.**

The manifest change is **quorum-committed**, not unanimously held: a commit requires a majority, and
a given replica may not have applied it yet. **Each replica reclaims its own segments only after its
own local ordered-apply has reached the watermark.** The authority is shared; the act of reclaiming
is local and independently decidable. (An earlier draft of this section said "authority is
unanimous", which is simply wrong about raft — and wrong in the same direction as the
`WalPosition` defect above: describing something local as though it were global.)

---

## 5. `PreparedSst` — the engine→runtime handoff

```
object_key       content-addressed key (the content hash)
content_id       hash of the SST bytes — anchors ManifestChange identity (§7)
cf               ColumnFamily. The Engine trait is a flat (cf, key) space (lib.rs:147),
                 so an SST must state which CF it belongs to
key_range        (smallest, largest) — range-filtered reads; later, the range bound of a
                 meta-only split
size / count     statistics
checksum         verified on read; a corrupt SST fails closed (§8), it never returns data
covers_through   the replicated applied position this SST absorbs (§4) — NOT a WAL offset
```

### 5.1 Durability promise — the state that has no recovery

**A `PreparedSst` value may exist only once its bytes are durably readable by a different process.**
Not "the write call returned Ok".

This exists because of a failure mode absent from the original crash matrix:

> **The manifest commits while the SST bytes are not durable → a dangling reference.**

Every other crash point in §8 is recoverable — an orphan is garbage, a double apply must be
idempotent, a torn segment must be detectable. This one is not, **and replication makes it worse
rather than better: once the change is committed, every replica applies the same manifest, so all of
them come to agree on a reference to an object that does not exist.** The dangling reference is not
one node's corruption that a healthy peer can repair; it is the agreed state.

So in the chain `durably upload → propose manifest → apply → reclaim WAL`, **the first arrow is as
load-bearing as the third.** Both are enforced:

- **MinIO/S3:** a `PreparedSst` requires an acknowledged PUT. A PUT that times out or loses its
  connection leaves existence *indeterminate* — a failure local disks do not have. Content
  addressing resolves it: `HEAD` the content-hash key to settle whether the object exists, and
  re-upload is naturally idempotent because the key is derived from the bytes. **The same decision
  taken for change-id idempotence (§7) closes this too.**
- **Any file-backed backend:** fsync the file **and its parent directory**. Going from one file to
  many makes the directory entry itself persistent state — the classic omission.

---

## 6. The WAL: record format and segmentation

### 6.1 Today's shape, and why it cannot express what round one needs

```
record          MAGIC | VERSION | len | payload | crc          (wal.rs)
Replay          { batches, discarded_tail_bytes }              — batches only, no positions
recovery        replay() seeks to 0 and reads every record     (wal.rs:234)
checkpoint      none
prefix-delete   no primitive exists. The only production set_len calls drop a torn TAIL
                (wal.rs:224; and raft/src/storage.rs:162 on the raft-log side)
contract        lib.rs:30 states the log "may only be truncated to the extent the state
                machine has actually landed its data" — written, never implemented
```

**A record does not know which applied position it belongs to.** So "cold start skips the prefix
already absorbed, by watermark" is not a change to recovery logic — it is a **record-format
change**, and it is a round-one item.

The header carries a `VERSION` byte, so the change is expressible, and **no deployed data exists to
migrate. This is the cheapest this will ever be**; it stops being cheap the moment anything ships.

### 6.2 Segments, not in-place prefix deletion

The WAL becomes a sequence of segments with a checkpoint. Recovery skips records at or below the
manifest watermark; closed segments are deleted whole and late.

This is Tess's design and it is **structurally** better than the alternative, which is worth
recording because the difference is not stylistic:

- The rejected approach answered "manifest applied, WAL not yet reclaimed" with *replay must be
  idempotent — raw put naturally is*. **That is a property of the current operator set, not an
  invariant.** `delete_range` already breaks it and MVCC will. It hangs correctness on a premise
  later work would quietly overturn, and the symptom would be silent.
- Segments skip by watermark instead of replaying and hoping the result matches. Whole-segment
  deletion makes "crash midway through reclaiming" **impossible rather than detectable**.

**Two of the five crash points stop being failure modes that need tests and become states that
cannot occur.** Prefer that trade wherever it is available.

### 6.3 Reclamation predicate

```
delete a segment  iff  it is CLOSED  and  segment.max_applied_position <= watermark
never delete      the active tail
never delete      a segment straddling the watermark
```

Segment size therefore sets reclamation granularity. Creating a segment creates a file: fsync the
file and the parent directory (§5.1).

---

## 7. `ManifestChange` — identity, outcomes, refcounts

### 7.1 Identity is content-derived; history is ordered by an explicit generation

Identity is a hash of canonicalized content, never an allocated id. Content-derived identity is what
lets a proposer *recompute* the identity after a crash rather than having to have durably remembered
one.

**Stable identity enables reconciliation. It does not authorize a retry.**

**`region_epoch` is the fence and only the fence.** A change carrying the wrong epoch is rejected;
the epoch does **not** double as a manifest sequence number.

An earlier draft made it do both, arguing that a file-id enters a region at most once per epoch so
any legitimate re-add must cross an epoch bump. **That argument does not hold, and the reason
matters: the region epoch is a routing/membership generation.** Nothing in its contract promises it
advances when the manifest's file set changes, so relying on it to serialise manifest history
borrows a guarantee the epoch never made. Even where no path re-adds a file within one epoch today,
that is an accident of the current operator set rather than something enforced — the same shape as
§7.5's clone problem, where a planned feature voids an unstated premise.

**Manifest history is therefore ordered by an explicit `generation`,** carried in the change and
advanced by apply (§7.2). The epoch keeps fencing; the generation keeps order; neither borrows the
other's guarantee.

> **Upstream defect, not resolved here.** The epoch-as-nonce argument originates in `DESIGN.md` §6.5
> and this document inherited it. Correcting only this file would leave the two disagreeing, with the
> error in the more authoritative document that readers reach first. `DESIGN.md` wording belongs to
> the API/compat owner and is tracked separately — this note exists so the fix is not assumed to have
> happened here.

### 7.2 Propose outcomes are three states, divided by result semantics

Not by error source:

```
NotLeader { leader }   KNOWN not applied. Only if refused before proposing, or proven by
                       authoritative reconciliation. Observing a leadership change AFTER the
                       proposal went out is NOT enough — it may still commit under the new
                       term. That case is Unconfirmed.
Unconfirmed            UNKNOWN. Deadline, lost response, driver dropped mid-wait. May yet
                       commit. Must not drive any reclamation decision, and must not be
                       blindly re-proposed — reconcile first (below).
Failed(Error)          KNOWN failed, and not a leadership change (queue full, driver closed).
```

Collapsing `Unconfirmed` into `Failed` guarantees callers treat *unknown* as *known-failed*.
**`Failed(Error::NotLeader)` and `Failed(timeout)` are both forbidden.**

#### Reconciliation needs durable state, not a lookup in the current manifest

"Query the authoritative applied manifest for this change-id" is **not executable on its own**, and
the reason is easy to miss: the manifest's file set keeps evolving. A change can be applied and then
superseded, after which the current manifest no longer contains its effects — **so absence proves
nothing.** Any scheme resting on absence silently treats *applied-then-superseded* as *never
applied*, and re-proposing on that basis is exactly the double-apply this variant exists to prevent.

The seam therefore carries durable ordering state (design owned by Rafa, task #9):

```
payload        { change_id (content-derived), expected_generation, changes, new_watermark }

apply, in order:
  current_generation == expected_generation  → apply; generation += 1;
                                               record (generation, last_change_id)
  change_id == last_change_id                → idempotent repeat: typed already-applied.
                                               MUST NOT be reported as newly accepted.
  otherwise                                  → typed stale, rejected

reconcile      on an unknown outcome, read (current_generation, last_change_id) and compare
               against the (expected_generation, change_id) proposed. Three states are
               decidable: applied · superseded by another proposer · never reached.
```

**One in-flight change per region** is what keeps a single `last_change_id` slot sufficient — the
in-flight window *is* the retention window for the criterion. A durable applied-id ledger is
deliberately not built: its value appears only with multiple in-flight changes, which round one does
not need, and adding it later is a pure extension.

**That single-in-flight property is enforced structurally, not assumed** (task #9). The seam holds
one proposal slot per region and is the only entry to propose; a second proposal while the slot is
occupied is a typed refusal, neither queued nor silently accepted. The slot is cleared only by a
*settled* reconciliation — applied, preempted, or refused; "still unknown" does not clear it — and
restart takes the same path, reconciling the previous in-flight change (whose `change_id` is
recomputable from the engine's durable prepared state) before the slot can be granted again.

**Why it cannot rest on convention:** the whole three-state criterion is only sufficient while the
property holds, and its degradation is silent in both directions. With two concurrent proposers, a
change that was *preempted* reconciles as *never reached*; worse, a change that **succeeded** can
reconcile as *preempted*, because a following change overwrites `last_change_id` and the original
proposer then sees an advanced generation with an id that is not its own.

**Ordering consequence: without this state, the drain worker cannot start** — `Unconfirmed`'s
contract would be unexecutable and the variant decoration. So the generation/`last_change_id`
criterion plus its query seam is built *before* the proposer.

The same criterion serves crash point 3 (§8.2): when WAL replay re-presents a record already
absorbed by an SST, the second row above is what stops the watermark/receipt path from reporting a
re-apply as newly accepted. **One criterion, two faces, no second channel.**

### 7.3 Why change-id exists — name the direction, or it gets deleted as redundant

```
Live -> Retired   absolute assignment  => state-idempotent (Retired->Retired is a no-op)
+ref              replay => over-count => leaked object      => SAFE direction
-ref              replay => UNDER-count => premature delete   => DATA LOSS   <-- the danger
```

Reconciliation is **universal** (any `Unconfirmed` needs it); exactly-once is **`-ref`'s additional**
requirement. `DESIGN.md`'s "crashes only leak over-counts" protects against *ordering* skew; a
replay performs one decrement twice, which ordering cannot police.

**A `ManifestChange`'s change-id does not supply that exactly-once.** They are two different
operations in two different places: the manifest change is committed in the region's raft group,
while the refcount mutation is a `CatalogTxn` against the system keyspace. A stable identity for the
first does not deduplicate a retry of the second. When `-ref` is eventually built it needs its own
durable dedupe — a stable `op_id`, or an absolute/CAS transition carrying a generation — or it must
ride inside the same ordered apply that already has the generation criterion (§7.2).

### 7.4 Round one does not maintain refcounts — and why that is the safe choice

**Round one builds no delete path (see §1), so it maintains no refcount.** Objects leak; nothing is
reclaimed. This resolves what would otherwise be a contradiction between the scope table and the
ordering rule below.

The choice is deliberate rather than merely convenient:

```
no delete path  ⇒  nothing ever decrements, and nothing ever deletes
                ⇒  over-count is harmless (a leaked object)
                ⇒  under-count CANNOT cause data loss, because no deletion consumes the count
                ⇒  -ref's exactly-once problem has no carrier in round one, and needs none
```

**Enabling condition for Phase 3, written now so it is not rediscovered then:** before GC is switched
on, refcounts must first be **backfilled from the set of committed manifests** — the manifest is
authoritative, so the counts are computable — and only then may decrements and deletions begin.
Turning on `-ref` against counts that were never maintained would start from zeros and delete live
objects immediately.

> ### ⚠ Everything below in §7.4 is a **Phase-3 contract, not round-one acceptance.**
> No part of it is implemented, tested, or owed by this slice. It is written now because the design
> decision is live now; do not read it as a checklist for the current round.

`DESIGN.md:205-213` already mandates it, and the landing point already exists:
`crates/meta/src/schema.rs:219-226` — `SST_FILES` carries `refcount` (col 4) and `state` (col 9).

> `+ref` commits **before** the manifest change that references the file; `-ref` only **after** the
> manifest change that drops it. Hence `refcount = 0` is at any moment a sufficient condition for
> safe deletion.

**The directionality is the entire value of the rule:** over-count leaks an object and wastes money;
under-count deletes live data. The rule can only err toward safety.

**And it lives in nothing the compiler checks.** It is an *ordering* between two code paths — not a
type, not an assertion. Someone later moving `+ref` after the commit to save a catalog round-trip
(it looks symmetric, it looks cheaper) reverses it, and the symptom is intermittent data loss rather
than an error. Both defences are required:

- **Put the order in the type.** The manifest-change constructor demands a `+ref` token obtainable
  only by having done `+ref`; the `-ref` constructor demands the applied receipt, which exists only
  after commit. **The wrong order becomes unwritable rather than writable-but-red.**
- **An ordering-mutation test:** move `+ref` after the commit ⇒ must go red. A type guard still has
  to prove that removing it admits the bad order, or it is one more unverified "structural
  guarantee".

*(Both defences land with the refcount itself, in Phase 3. They are specified here because the
design decision is live now — the ordering is the whole value of the rule, and it is far cheaper to
write down while the reasoning is fresh than to reconstruct when someone is implementing GC.)*

### 7.5 Deletion criterion — write the strong form now, use the weak form today

```
weak    (suffices today, breaks later)   no committed manifest of THIS region references it
strong  (the criterion to write down)    no committed manifest ANYWHERE references it —
                                         clones, branches and PITR snapshots included.
                                         The test is a global refcount, not a per-manifest check.
```

The weak form is sound today **because exactly one manifest can reference an object**. But
`DESIGN.md` plans backup/PITR/branch/clone as *reference* operations — re-referencing existing
objects is precisely what makes a clone cheap — so **a planned feature voids the premise.** Region
R's delete-intent would retire an object a clone's manifest still references, and the clone would
never learn why its data vanished.

Do not implement the cross-manifest refcount yet: clone does not exist, so it could not be
verified. **Do write the criterion in its correct form, with the degradation reason and the
condition that voids it in the same paragraph** — the reason it may degrade (a single referencer)
*is* the future failure condition (clone makes refcount > 1). Split across two paragraphs, the next
person inherits half a truth.

### 7.6 Delete goes through raft; physical execution follows

Leader-only GC is discipline, and discipline cannot stop an **in-flight** DELETE: a deposed leader's
already-issued DELETE still lands, because object storage does not consult raft.

GC therefore submits a **delete-intent** through the manifest path and may issue the physical DELETE
only after the receipt confirms it. Then who executes it, and how late, stops mattering — leader-only
GC degrades to an efficiency convention and correctness no longer depends on the executor still
being leader.

*Same shape as the LeaderRead lesson: "I am leader, so I may delete" (**receives**) becomes "raft
committed this delete, so it is safe" (**establishes**).*

---

## 8. Acceptance

### 8.1 The story — the only thing that counts as the slice being done

```
raw write → WAL/memtable → flush SST → upload to MinIO → propose ManifestChange
→ ordered apply installs authoritative manifest + watermark
→ reclaim closed segments covered by the watermark
→ kill -9
→ a NEW process recovers from manifest + WAL tail alone → raw read returns the original value
```

### 8.2 Five independent crash points

A single kill at a clean point does not stand on any of them.

| # | Crash point | Required behaviour |
|---|---|---|
| 1 | Clean point after commit | Recover from manifest + WAL tail |
| 2 | SST uploaded, manifest not committed | Orphan object only; **the WAL must not be reclaimed** |
| 3 | Manifest applied, segments not yet reclaimed | Cold start skips the absorbed prefix by watermark |
| 4 | Crash during segment reclamation | The live tail is undamaged |
| 5 | Manifest references a non-durable SST | Prevented by §5.1 — this one has no recovery |

### 8.3 Sensitivity — several of these pass and fail identically without controls

A test that reads the value back after a restart may be green for reasons that prove nothing:
an in-process memtable or handle was reused, the old process never died, or **recovery ignored the
manifest and replayed the whole WAL from byte 0** — which is exactly today's behaviour
(`wal.rs:234`). *(OS page cache is not one of these: bytes read through the page cache are still
bytes that reached the disk.)*

Crash point 3 is the sharpest case: "skips by watermark" and "ignores the watermark, replays
everything" **predict the identical observation**. Note that upgrading the record format is not
itself evidence that skipping happens — a wrong skip, or a quiet fall back to full replay, still
passes.

Required controls:

```
delete the SST the manifest references     → recovery MUST fail
                                             judges: the SST was never opened at all
corrupt a CLOSED segment at or below the   → cold start MUST succeed, from the SST
  watermark                                  judges: recovery never opened/replayed it
corrupt a segment ABOVE the watermark      → cold start MUST fail closed, typed
  (same corruption recipe)                   judges: the tail IS still read — the instrument fires
skip installing the manifest               → the recovery test MUST go red
```

**The two corruption cells must use the same corruption function, differing only in which side of
the watermark the target segment falls on.** The first cell is sound only if the corruption *would*
have been fatal had it been read, and the second cell is what establishes that. Corrupt different
things in each and the negative control no longer vouches for the positive one — the pair looks
complete and proves less than it appears to.

**"The same byte offset" is not sufficient, and assuming it is reintroduces the hole.** The two
segments need not be the same length, so one offset can land inside a record that would be replayed
in the tail segment while landing *past the last record* in the covered segment — where it would be
harmless even if the segment were read. The second cell fires, the first cell stays green, and the
trivial explanation the pair exists to exclude is still standing.

So the corruption target is **structurally identified, not positional** — a named record within the
segment (for instance the last complete one) — **and each side separately asserts that its chosen
point really does fall inside a record recovery would read.** That assertion is verified, not
reasoned: the control pair needs its own firing check on both sides.

The general form, worth carrying beyond this test: **a premise shared between two controls is not
automatically true on both sides, and something has to vouch for it.** Here the shared premise is
"this is a valid corruption point", and it is exactly what does not transfer for free.

A rejected earlier version of the first control modified a value *inside* the SST and expected the
modified value to come back. It cannot be used: an SST is content-addressed and checksummed, so the
correct response to a modified value is a corruption error. The probe could only produce its signal
by requiring the system to violate an invariant it is supposed to hold. **A probe's signal channel
must not be a contract the system under test is responsible for keeping.**

The dangling-reference regression (crash point 5) must run **after** the covered segments have been
reclaimed. Construct "manifest references a missing SST" while a complete WAL still exists and
recovery succeeds from the WAL — green, having demonstrated nothing. The unrecoverable state needs
all three conditions at once: manifest committed, SST not durable, covered segments already
reclaimed under that manifest's authority.

### 8.4 MinIO is a required job, and it may not be skipped

MinIO acceptance runs against a **real, independent MinIO process** over the real S3 API, covering
bucket/prefix handling, multipart or an explicit small-object threshold, checksum and ETag kept
distinct, timeout/retry idempotence, and restart recovery. Every one of those must have a negative
case that can be made red on its own.

**If MinIO is unreachable the job fails.** No `#[ignore]`, no "absent environment ⇒ return early",
no silent skip, and no fallback to `MemoryObjectStore`. It is a separately named required leg.

### 8.5 Pinning the backend's *shape*, not merely that it works

A test that simply calls the backend from inside `spawn_blocking` and succeeds does **not** pin
§3.2. All three implementations pass it — the dedicated worker, a runtime built per call, and a
`block_on` on the calling thread (a `spawn_blocking` thread has no ambient runtime, so `block_on`
there works fine). It demonstrates that the legitimate path runs; it says nothing about which shape
is underneath, which is the entire point of writing the rule.

The discriminator has to inspect structure rather than success:

```
issue N concurrent operations against ONE backend
  → every async operation lands on the SAME single worker thread
    runtime-per-call     → thread ids differ            → red
    caller block_on      → work happens on the CALLING thread → red
```

**Per backend instance, not per process.** "Exactly one runtime was constructed process-wide" is the
wrong invariant: constructing two backends (separate buckets, isolated fixtures) is legitimate and
would make a correct implementation red.

Stronger still, and preferred: the runtime and its worker are owned once at backend construction,
with no code path able to create a second. The test then *fires against* that structure rather than
being the only thing holding the rule up.

### 8.6 "Unavailable" is two different failure states

```
MinIO not started       connection refused → fails fast, the deadline is never reached
MinIO up but mute       accepts the connection, never answers → THIS is what exercises the deadline
```

Only the second demonstrates that a timeout fails loudly. Build it with a black-hole listener that
accepts and never responds.

**Bind it as `127.0.0.1:0` and hold the listener from assignment until the test ends**, letting the
kernel assign the port. Do not hand-pick one.

**Why this does not conflict with the "ports below 32768" convention** — the two rules cover
different constructions, and the distinction is worth stating so the apparent clash is not
re-litigated. `.github/workflows/ci.yml:179-181` gives the convention's own reason:

> All bases sit below 32768 to stay clear of the Linux ephemeral range (32768-60999), where an
> unrelated process can take a port **between the bind check and the bind itself**.

The hazard is the *check-then-bind window*: pre-compute a port number, verify it looks free, release
it, bind later. Every current use has that shape (`quickstart-smoke`, `root-trust`,
`dynamic-membership` with `23000+($$%1000)`). **A `:0` fixture has no check step at all** — the
kernel assigns and the listener never lets go, so no window exists for anything to race into. It
satisfies a strictly stronger property than the convention asks for, rather than being excused from
it.

**The residual condition, which is what actually makes this true:** ownership is held continuously
from assignment to end of test. Dropping the listener and rebinding, or using `SO_REUSEPORT`, brings
the window straight back and the convention applies again.

**The general form, which outruns this one case: the easiest way to make something "unavailable" is
usually not the way it actually becomes unavailable.** Refusing a connection, deleting a file, or
killing a process are cheap to construct and each tests a *different* failure than the hang, the
partial read, or the silently-wrong answer that occurs in practice. Whenever a test makes a
dependency unavailable, say which of the two it built and which one the requirement was about.

Today the repository has zero `#[ignore]` and zero env-gated tests, so this is prevention rather
than a defect report — and it is worth the sentence, because the failure it prevents is a green
suite containing a leg that never entered its own subject area.

---

## 9. Security posture for round one — deliberately narrow

EdHuang, 2026-09-05: internal phase, no TLS and no credential management for now.

```
allowed now   an explicitly named insecure-dev mode; static, clearly-labelled dev credentials
              injected by the test orchestration through environment/secret
never         credentials in CLI arguments, in the repository, in logs, in status output,
              or in error text
unchanged     HTTPS + credential management remain a hard gate before any external or
              untrusted-network use. Speaking the S3 API does not confer security
```

No production credential provider is built in round one, and the static test credentials must not
be described anywhere — code, documents, or acceptance — as a deployable configuration. The mode is
explicit and named so that it cannot later be mistaken for a default that was reviewed.

---

## 10. Open, and owned

- **Multi-raft-group lifecycle** (one group per region; creation/destruction; WAL reuse) is required
  under every ordering and depends on no storage decision. Rafa's lane, designable in parallel.
- **`split`/`merge` manifest attach** waits until this seam is stable. Its data half is a
  raft-committed "child references the parent's SSTs, bounded by range" change.
- **`fence` × 2PC interaction** is untestable today because Percolator is entirely
  `NotImplemented`; the fence's protection of in-flight prewrite/commit/resolve is a design argument
  and **not** test evidence. Recorded so that raw-fence evidence is never quietly substituted for it.
