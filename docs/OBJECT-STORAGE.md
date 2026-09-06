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

Because the ack does not wait for object storage, a **double failure** — node down *and* its local
disk lost — can only recover from the drain watermark. Already-acked data inside that window
depends on at least one local disk in the quorum surviving.

That is raft's ordinary durability model, but state it plainly: **object storage does not cover
single-node disk loss.**

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
mapping is **local, never replicated, and never part of a `ManifestChange`.** Authority is
unanimous; the reclaim predicate stays locally decidable.

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
idempotent, a torn segment must be detectable. This one is not, **and raft makes the loss
unanimous: every replica agrees on a manifest pointing at an object that does not exist.**

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

### 7.1 Identity is content-derived, and the epoch is load-bearing twice

Identity is a hash of canonicalized content — `(region_id, region_epoch, adds, removes,
new_watermark)` — never an allocated id. Content-derived identity is what lets a proposer
*recompute* the identity after a crash rather than having to have durably remembered one.

**Stable identity enables reconciliation. It does not authorize a retry.**

`region_epoch` is a mandatory member of the hashed content, and it is both:

- the **fence** — a change carrying the wrong epoch is rejected;
- the **nonce** — what makes a legitimate repeat hash differently from a retry.

The repeat is real: a split may drop a file from a region while its refcount stays above zero
because a sibling still references it, and a later merge may add that same file back. So "add F to
R" can legitimately occur twice in one region's history. Under a bare content hash the second
occurrence is judged a retry and dropped — losing a `+ref`, which under-counts, which is the
dangerous direction. With the epoch inside the hash the argument closes: within one epoch a file-id
enters a region at most once, and any legitimate re-add necessarily crosses an epoch bump.

**Removing the epoch field — or excluding it from the hashed content as "redundant with the fence
check" — dismantles both protections at once, and nothing goes red, because the loss is silent
under-counting.**

### 7.2 Propose outcomes are three states, divided by result semantics

Not by error source:

```
NotLeader { leader }   KNOWN not applied. Only if refused before proposing, or proven by
                       authoritative reconciliation. Observing a leadership change AFTER the
                       proposal went out is NOT enough — it may still commit under the new
                       term. That case is Unconfirmed.
Unconfirmed            UNKNOWN. Deadline, lost response, driver dropped mid-wait. May yet
                       commit. Must not drive any reclamation decision, and must not be
                       blindly re-proposed — reconcile against the authoritative applied
                       manifest by change-id first.
Failed(Error)          KNOWN failed, and not a leadership change (queue full, driver closed).
```

Collapsing `Unconfirmed` into `Failed` guarantees callers treat *unknown* as *known-failed*.
**`Failed(Error::NotLeader)` and `Failed(timeout)` are both forbidden.**

**Ordering consequence: if `ManifestChange` has no identity usable for authoritative reconciliation,
the drain worker cannot start.** So the first thing built is stable change-id plus a query seam —
*not* the proposer. Otherwise `Unconfirmed`'s contract is unexecutable and the variant is
decoration.

### 7.3 Why change-id exists — name the direction, or it gets deleted as redundant

```
Live -> Retired   absolute assignment  => state-idempotent (Retired->Retired is a no-op)
+ref              replay => over-count => leaked object      => SAFE direction
-ref              replay => UNDER-count => premature delete   => DATA LOSS   <-- the danger
```

Reconciliation is **universal** (any `Unconfirmed` needs it); exactly-once is **`-ref`'s additional**
requirement. `DESIGN.md`'s "crashes only leak over-counts" protects against *ordering* skew; a
replay performs one decrement twice, which ordering cannot police.

**Write it as "the change-id exists for `-ref`", or a later reader who sees that the state
transition is idempotent will remove it as redundant.**

### 7.4 The refcount ordering rule, and where it lives

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
