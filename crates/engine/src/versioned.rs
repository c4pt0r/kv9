//! `VersionedMemEngine`: the `MemEngine` contract on a B-tree that mutates in place.
//!
//! `MemEngine` gets its two load-bearing properties — an O(1) [`Engine::snapshot`] that
//! later writes never disturb, and a batch that becomes visible all-or-nothing across
//! the three column families — from a *persistent* red-black tree. That tree pays for
//! those properties on every read (one heap node and one `Arc` per level) and on every
//! write (path copying). This engine keeps the same contract on `bbtree`, whose nodes
//! hold dozens of inline keys per cache line and are updated in place, by moving the
//! versioning out of the tree and into the keys:
//!
//! * Every column family is one `BBTreeMap<(key, !version), Option<value>>`: all versions
//!   of a key are adjacent, newest first; `None` is a tombstone.
//! * A write stamps every mutation of a batch with one fresh version, inserts them, and
//!   only then publishes that version as the **visibility watermark**. A reader fixes
//!   its version when it starts and ignores anything newer, so a batch is invisible
//!   until the watermark moves and completely visible afterwards — across all three
//!   trees, with no lock between readers and writers.
//! * A [`ReadView`] is `(Arc<shared>, version)`: taking one is an atomic load plus a
//!   registration (so garbage collection knows the oldest version anyone may still
//!   read). Writes never touch versions a view can see; they only add newer ones.
//! * [`VersionedMemEngine::collect_garbage`] removes, for every key, the versions older
//!   than the newest one visible to the oldest live view, and a tombstone that has
//!   become that newest version. Older versions are removed *before* the tombstone, so
//!   a reader racing the collector can never see a value resurrected.
//!
//! What this engine does not offer, by design: `ReadView::get_resident` and
//! `resident_len` decline (a view cannot borrow into a tree that is being updated in
//! place; [`ReadView::get_resident_value`] serves the same no-wait, no-I/O purpose with
//! one copy), and memory held by superseded versions is returned by garbage collection
//! ([`VersionedMemEngine::gc_step`] incrementally, [`VersionedMemEngine::collect_garbage`]
//! in full), not by dropping a view. See `docs/BBTREE-EVALUATION.md` for the measurements.
//!
//! Lock order inside this module: `writer → published → registry shard`. `published` is
//! held while a positioned view registers itself, so that the version a view fixes and
//! the registration a collector sees are one event; nothing here is held across a call
//! back into user code.

use std::collections::{BTreeMap, VecDeque};
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bbtree::{BBTreeMap, InlineKey, InlineWord, WithInlineKey};
use kv9_common::{AppliedPosition, Error, Result, Value};

use crate::cf::ColumnFamily;
use crate::mem::changes_data_revision;
use crate::replicated::{DurableAppliedPosition, ReplicatedEngine};
use crate::write_batch::{Mutation, WriteBatch};
use crate::{Durability, Engine, PositionedReadView, ReadView, ScanEntry};

/// `(key, !version)`: keys sort by user key, then newest version first.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct VersionedKey {
    key: Vec<u8>,
    inverse_version: u64,
}

impl VersionedKey {
    fn new(key: Vec<u8>, version: u64) -> Self {
        Self {
            key,
            inverse_version: u64::MAX - version,
        }
    }

    /// The first entry of `key` a reader at `version` may use: the newest one not newer
    /// than `version`.
    fn newest_visible(key: &[u8], version: u64) -> Self {
        Self::new(key.to_vec(), version)
    }

    /// Sorts before every version of `key`.
    fn first_of(key: &[u8]) -> Self {
        Self {
            key: key.to_vec(),
            inverse_version: 0,
        }
    }

    /// Sorts after every version of `key` and before every other key.
    fn after_key(key: &[u8]) -> Self {
        let mut next = key.to_vec();
        next.push(0);
        Self::first_of(&next)
    }

    fn version(&self) -> u64 {
        u64::MAX - self.inverse_version
    }
}

impl InlineKey for VersionedKey {
    #[inline]
    fn inline_key_at(&self, offset: usize) -> InlineWord {
        self.key.as_slice().inline_key_at(offset)
    }
}

/// `None` is a tombstone: the key was deleted at this version.
type Slot = Option<Value>;
type Tree = BBTreeMap<VersionedKey, Slot, WithInlineKey>;

/// What a writer publishes once its batch is in the trees, under one lock so that a
/// positioned view sees version and position from the same instant.
#[derive(Clone, Copy, Debug, Default)]
struct Published {
    version: u64,
    applied: Option<AppliedPosition>,
    data_revision: u64,
}

const REGISTRY_SHARDS: usize = 32;

struct Shared {
    trees: [Tree; ColumnFamily::ALL.len()],
    /// The visibility watermark: the newest version a new reader may use.
    visible: AtomicU64,
    published: Mutex<Published>,
    /// Serialises writers. Holds the last version handed out.
    writer: Mutex<u64>,
    /// Live views by version, sharded by thread so that taking a view does not make
    /// every thread write one cache line. Garbage collection takes the minimum.
    registry: [Mutex<BTreeMap<u64, u32>>; REGISTRY_SHARDS],
}

impl Shared {
    fn tree(&self, cf: ColumnFamily) -> &Tree {
        &self.trees[cf as usize]
    }

    fn shard(&self) -> usize {
        thread_local! {
            static SHARD: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
        }
        SHARD.with(|slot| {
            if let Some(shard) = slot.get() {
                return shard;
            }
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let shard = NEXT.fetch_add(1, Ordering::Relaxed) as usize % REGISTRY_SHARDS;
            slot.set(Some(shard));
            shard
        })
    }

    fn register(&self, version: u64) -> usize {
        let shard = self.shard();
        *self.registry[shard]
            .lock()
            .expect("view registry poisoned")
            .entry(version)
            .or_insert(0) += 1;
        shard
    }

    fn unregister(&self, shard: usize, version: u64) {
        let mut registry = self.registry[shard].lock().expect("view registry poisoned");
        if let Some(count) = registry.get_mut(&version) {
            *count -= 1;
            if *count == 0 {
                registry.remove(&version);
            }
        }
    }

    /// The oldest version any live view may read, or `visible` when there is none.
    fn oldest_live_version(&self) -> u64 {
        let visible = self.visible.load(Ordering::Acquire);
        self.registry
            .iter()
            .filter_map(|shard| {
                shard
                    .lock()
                    .expect("view registry poisoned")
                    .keys()
                    .next()
                    .copied()
            })
            .min()
            .map_or(visible, |oldest| oldest.min(visible))
    }
}

/// In-memory engine with versioned keys on `bbtree`. Volatile, like [`crate::MemEngine`].
pub struct VersionedMemEngine {
    shared: Arc<Shared>,
}

impl Default for VersionedMemEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for VersionedMemEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let published = *self.shared.published.lock().expect("published poisoned");
        f.debug_struct("VersionedMemEngine")
            .field("version", &published.version)
            .field("applied", &published.applied)
            .finish()
    }
}

/// What one [`VersionedMemEngine::collect_garbage`] pass did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GarbageReport {
    /// The oldest version a live view could read when the pass started; nothing a reader
    /// at that version or newer needs was removed.
    pub oldest_live_version: u64,
    /// Superseded versions and settled tombstones removed, over all column families.
    pub removed: usize,
    /// Stored entries examined.
    pub scanned: usize,
    /// Whether the cursor wrapped around: every column family was walked to its end.
    pub cycle_complete: bool,
}

impl VersionedMemEngine {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                trees: std::array::from_fn(|_| BBTreeMap::with_strategy(WithInlineKey)),
                visible: AtomicU64::new(0),
                published: Mutex::new(Published::default()),
                writer: Mutex::new(0),
                registry: std::array::from_fn(|_| Mutex::new(BTreeMap::new())),
            }),
        }
    }

    /// One batch, one version, published atomically across the three trees.
    fn apply(&self, batch: &WriteBatch, at: Option<AppliedPosition>) -> Result<()> {
        let mut writer = self.shared.writer.lock().expect("writer lock poisoned");
        if let Some(at) = at {
            // Checked BEFORE anything is inserted, so a refused position leaves no
            // partial batch behind. Index only; see MemEngine::write_applied.
            let previous = self
                .shared
                .published
                .lock()
                .expect("published poisoned")
                .applied;
            if let Some(previous) = previous {
                if at.index <= previous.index {
                    return Err(Error::Engine(format!(
                        "engine: applied position must advance; last applied index {}, \
                         refused index {}",
                        previous.index, at.index
                    )));
                }
            }
        }
        let version = *writer + 1;
        let mut changes = false;
        for mutation in batch.mutations() {
            let (cf, key, slot) = match mutation {
                Mutation::Put { cf, key, value } => (*cf, key, Some(value.clone())),
                Mutation::Delete { cf, key } => (*cf, key, None),
            };
            changes |= changes_data_revision(cf, key);
            self.shared
                .tree(cf)
                .insert(VersionedKey::new(key.clone(), version), slot)
                .map_err(|_| Error::Engine("engine: out of memory for a versioned entry".into()))?;
        }
        {
            let mut published = self.shared.published.lock().expect("published poisoned");
            published.version = version;
            if at.is_some() {
                published.applied = at;
            }
            if changes {
                published.data_revision = published.data_revision.saturating_add(1);
            }
            // The batch becomes visible here, to every reader that starts from now on.
            self.shared.visible.store(version, Ordering::Release);
        }
        *writer = version;
        Ok(())
    }

    fn view(&self) -> View {
        View::current(self.shared.clone())
    }

    /// A view that never waits: readers never contend with writers here. Kept for API
    /// parity with [`crate::MemEngine::try_resident_snapshot`]; it always succeeds.
    pub fn try_resident_snapshot(&self) -> Option<Box<dyn ReadView>> {
        Some(Box::new(self.view()))
    }

    /// A view and the applied position published with its version, from one instant.
    pub fn try_positioned_resident_snapshot(&self) -> Option<PositionedReadView> {
        let (view, position, _) = self.freeze_parts();
        Some(PositionedReadView {
            view: Box::new(view),
            position,
        })
    }

    /// A view, the applied position and the data revision published with its version,
    /// from one instant. The view registers itself **while `published` is held**: a
    /// collector that runs after a later write has moved the watermark must already
    /// see this registration, or it would remove the versions this view reads.
    pub(crate) fn freeze_parts(&self) -> (View, Option<AppliedPosition>, u64) {
        let published = self.shared.published.lock().expect("published poisoned");
        let view = View::new(self.shared.clone(), published.version);
        (view, published.applied, published.data_revision)
    }

    /// The position last recorded by [`ReplicatedEngine::write_applied`]; meaningless
    /// after a restart and never a truncation bound (see `MemEngine`).
    pub fn volatile_applied_position(&self) -> Option<AppliedPosition> {
        self.shared
            .published
            .lock()
            .expect("published poisoned")
            .applied
    }

    pub fn data_revision(&self) -> u64 {
        self.shared
            .published
            .lock()
            .expect("published poisoned")
            .data_revision
    }

    /// The oldest version a live view may still read.
    pub fn oldest_live_version(&self) -> u64 {
        self.shared.oldest_live_version()
    }

    /// Entries currently stored, every version and tombstone included (diagnostics).
    pub fn stored_entries(&self, cf: ColumnFamily) -> usize {
        let mut count = 0usize;
        self.shared.tree(cf).scan(.., |_, _| {
            count += 1;
            ControlFlow::<()>::Continue(())
        });
        count
    }

    /// Remove what no live view can read any more: for every key, the versions older
    /// than the newest one visible at the oldest live version, and that newest one too
    /// when it is a tombstone. One full pass over every column family; runs
    /// concurrently with readers and writers. [`Self::gc_step`] is the incremental form.
    pub fn collect_garbage(&self) -> GarbageReport {
        let mut cursor = GcCursor::default();
        let mut total = GarbageReport::default();
        loop {
            let step = self.gc_step(&mut cursor, usize::MAX);
            total.oldest_live_version = step.oldest_live_version;
            total.removed += step.removed;
            total.scanned += step.scanned;
            if step.cycle_complete {
                total.cycle_complete = true;
                return total;
            }
        }
    }

    /// One bounded leg of garbage collection: scans about `scan_budget` stored entries
    /// from where `cursor` left off (always stopping at a key boundary), removes what
    /// is garbage among them, and advances the cursor; a cursor that reaches the end
    /// of the last column family wraps around and the report says so.
    ///
    /// Removal order is what keeps a racing reader safe: within a key the older
    /// versions go before the newer kept one or the tombstone, and a key is never split
    /// across two legs, so a reader at any live version meets the tombstone (or the kept
    /// version) before an older value could be missing.
    pub fn gc_step(&self, cursor: &mut GcCursor, scan_budget: usize) -> GarbageReport {
        const CHUNK: usize = 4096;
        let scan_budget = scan_budget.max(1);
        let oldest = self.shared.oldest_live_version();
        let mut report = GarbageReport {
            oldest_live_version: oldest,
            ..GarbageReport::default()
        };
        loop {
            let cf = ColumnFamily::ALL[cursor.cf];
            let tree = self.shared.tree(cf);
            let mut doomed: Vec<VersionedKey> = Vec::new();
            let mut resume: Option<VersionedKey> = None;
            let mut budget_spent = false;
            {
                let current = &mut cursor.current;
                let kept = &mut cursor.kept;
                let scanned = &mut report.scanned;
                let scan = |k: &VersionedKey, v: &Slot| {
                    if current.as_deref() != Some(k.key.as_slice()) {
                        // A key boundary: the only place a leg may end.
                        if doomed.len() >= CHUNK || *scanned >= scan_budget {
                            budget_spent = *scanned >= scan_budget;
                            resume = Some(VersionedKey::first_of(&k.key));
                            return ControlFlow::Break(());
                        }
                        *current = Some(k.key.clone());
                        *kept = false;
                    }
                    *scanned += 1;
                    if k.version() > oldest {
                        return ControlFlow::Continue(()); // a live view may still need it
                    }
                    if *kept {
                        doomed.push(k.clone()); // superseded by the version kept above
                    } else {
                        *kept = true;
                        if v.is_none() {
                            doomed.push(k.clone()); // a settled tombstone
                        }
                    }
                    ControlFlow::Continue(())
                };
                match &cursor.resume {
                    None => tree.scan(.., scan),
                    Some(from) => tree.scan(
                        (
                            std::ops::Bound::Included(from.clone()),
                            std::ops::Bound::Unbounded,
                        ),
                        scan,
                    ),
                };
            }
            // Oldest first within a key (the scan met them newest first).
            for key in doomed.iter().rev() {
                if tree.remove(key).is_some() {
                    report.removed += 1;
                }
            }
            match resume {
                Some(next) => {
                    cursor.resume = Some(next);
                    if budget_spent {
                        return report;
                    }
                }
                None => {
                    cursor.resume = None;
                    cursor.current = None;
                    cursor.kept = false;
                    cursor.cf += 1;
                    if cursor.cf == ColumnFamily::ALL.len() {
                        cursor.cf = 0;
                        report.cycle_complete = true;
                        return report;
                    }
                    if report.scanned >= scan_budget {
                        return report;
                    }
                }
            }
        }
    }
}

/// Where an incremental garbage collection stands between two [`VersionedMemEngine::gc_step`]
/// calls: the column family, the key to resume at, and the per-key state of the key
/// that leg stopped in front of (none: a leg always ends at a key boundary).
#[derive(Debug, Default)]
pub struct GcCursor {
    cf: usize,
    resume: Option<VersionedKey>,
    current: Option<Vec<u8>>,
    kept: bool,
}

impl Engine for VersionedMemEngine {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>> {
        self.view().get(cf, key)
    }

    fn write(&self, batch: WriteBatch) -> Result<()> {
        self.apply(&batch, None)
    }

    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<ScanEntry>> {
        self.view().scan(cf, start, end, limit)
    }

    fn delete_range(&self, cf: ColumnFamily, start: &[u8], end: &[u8]) -> Result<()> {
        // Under the writer lock the visible set cannot change between the walk and the
        // tombstones, so the deletion is one atomic batch, as in MemEngine.
        let writer = self.shared.writer.lock().expect("writer lock poisoned");
        let view = View::new(self.shared.clone(), *writer);
        let mut batch = WriteBatch::new();
        for (key, _) in view.scan(cf, start, end, usize::MAX)? {
            batch.delete(cf, key);
        }
        drop(view);
        drop(writer);
        if batch.mutations().is_empty() {
            return Ok(());
        }
        self.apply(&batch, None)
    }

    fn checksum(&self, cf: ColumnFamily, start: &[u8], end: &[u8]) -> Result<u64> {
        let view = self.view();
        let mut h: u64 = 0xcbf29ce484222325;
        for entry in view.iter(cf, start, end)? {
            let (k, v) = entry?;
            for b in k.iter().chain(v.iter()) {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        Ok(h)
    }

    fn durability(&self) -> Durability {
        Durability::Volatile
    }

    fn snapshot(&self) -> Result<Box<dyn ReadView + '_>> {
        Ok(Box::new(self.view()))
    }
}

impl ReplicatedEngine for VersionedMemEngine {
    fn write_applied(&self, batch: WriteBatch, at: AppliedPosition) -> Result<()> {
        self.apply(&batch, Some(at))
    }

    fn applied_position(&self) -> Result<DurableAppliedPosition> {
        Ok(DurableAppliedPosition::Volatile)
    }
}

/// A point-in-time view: every read answers from the versions `<= version`.
pub(crate) struct View {
    shared: Arc<Shared>,
    version: u64,
    /// Where and as which version this view is registered (`<= version`).
    shard: usize,
    registered: u64,
}

impl View {
    fn new(shared: Arc<Shared>, version: u64) -> Self {
        let shard = shared.register(version);
        Self {
            shared,
            version,
            shard,
            registered: version,
        }
    }

    /// A view of the current watermark. The version is registered *before* it is
    /// fixed: a collector that computes its oldest live version after this
    /// registration sees `v0 <= version`, and one that computed it before did so
    /// against a watermark `<= version`; either way nothing this view needs is removed.
    fn current(shared: Arc<Shared>) -> Self {
        let v0 = shared.visible.load(Ordering::Acquire);
        let shard = shared.register(v0);
        let version = shared.visible.load(Ordering::Acquire).max(v0);
        Self {
            shared,
            version,
            shard,
            registered: v0,
        }
    }

    /// The slot of `key`'s newest version this view may use, if the key has one.
    fn newest_slot(&self, tree: &Tree, key: &[u8]) -> Option<Slot> {
        let from = VersionedKey::newest_visible(key, self.version);
        tree.scan(from.., |k, v| {
            ControlFlow::Break(if k.key.as_slice() == key {
                Some(v.clone())
            } else {
                None
            })
        })
        .flatten()
    }

    /// How many entries of one key a page walks sequentially before it seeks past
    /// them instead. A key with a handful of versions stays on the leaf walk; a hot key
    /// overwritten thousands of times under a held view costs one descent per page
    /// step, never one step per version.
    const SKIP_LIMIT: usize = 8;

    /// Forward page of `[start, end)` at this version: up to `limit` live entries.
    fn page(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: Option<&[u8]>,
        limit: usize,
    ) -> Vec<ScanEntry> {
        use std::ops::Bound::{Excluded, Included, Unbounded};
        let mut out = Vec::new();
        if limit == 0 || end.is_some_and(|end| start >= end) {
            return out;
        }
        let tree = self.shared.tree(cf);
        let version = self.version;
        let upper = end.map_or(Unbounded, |end| Excluded(VersionedKey::first_of(end)));
        let mut cursor = VersionedKey::first_of(start);
        loop {
            // One leaf walk from `cursor`. Per key: skip the versions newer than this
            // view, take the first one that is not, skip the rest. Too many skips of one
            // key end the walk with a seek target instead.
            let mut current: Option<Vec<u8>> = None;
            let mut decided = false;
            let mut skipped = 0usize;
            let mut seek: Option<VersionedKey> = None;
            tree.scan(
                (Included(cursor.clone()), upper.clone()),
                |k, v| {
                    if current.as_deref() != Some(k.key.as_slice()) {
                        current = Some(k.key.clone());
                        decided = false;
                        skipped = 0;
                    }
                    if decided {
                        skipped += 1;
                        if skipped >= Self::SKIP_LIMIT {
                            seek = Some(VersionedKey::after_key(&k.key));
                            return ControlFlow::Break(());
                        }
                        return ControlFlow::Continue(()); // an older version of a decided key
                    }
                    if k.version() > version {
                        skipped += 1;
                        if skipped >= Self::SKIP_LIMIT {
                            seek = Some(VersionedKey::newest_visible(&k.key, version));
                            return ControlFlow::Break(());
                        }
                        return ControlFlow::Continue(()); // newer than this view
                    }
                    decided = true;
                    skipped = 0;
                    if let Some(value) = v {
                        out.push((k.key.clone(), value.clone()));
                        if out.len() >= limit {
                            return ControlFlow::Break(());
                        }
                    }
                    ControlFlow::Continue(())
                },
            );
            match seek {
                Some(next) if out.len() < limit => cursor = next,
                _ => return out,
            }
        }
    }

    /// Backward page of `[start, end)` at this version, greatest key first. Each step
    /// finds the greatest stored key below the cursor and looks up its newest visible
    /// version: two descents per key, independent of how many versions the key has.
    fn page_rev(&self, cf: ColumnFamily, start: &[u8], end: &[u8], limit: usize) -> Vec<ScanEntry> {
        use std::ops::Bound::{Excluded, Included};
        let mut out = Vec::new();
        if limit == 0 || start >= end {
            return out;
        }
        let tree = self.shared.tree(cf);
        let lower = VersionedKey::first_of(start);
        let mut upper = Excluded(VersionedKey::first_of(end));
        while out.len() < limit {
            let mut next: Option<Vec<u8>> = None;
            tree.scan_rev((Included(lower.clone()), upper.clone()), |k, _| {
                next = Some(k.key.clone());
                ControlFlow::Break(())
            });
            let Some(key) = next else { break };
            if let Some(Some(value)) = self.newest_slot(tree, &key) {
                out.push((key.clone(), value));
            }
            upper = Excluded(VersionedKey::first_of(&key));
        }
        out
    }
}

impl View {
    /// Every live entry of `cf` at this version, ascending, fetched one page at a time
    /// and handed out owned: a checkpoint writer streams it while apply continues.
    pub(crate) fn iter_all(&self, cf: ColumnFamily) -> impl Iterator<Item = ScanEntry> + '_ {
        Pages {
            view: self,
            cf,
            start: Vec::new(),
            end: None,
            forward: true,
            buffer: VecDeque::new(),
            done: false,
        }
        .map(|entry| entry.expect("an in-memory page never fails"))
    }
}

impl Drop for View {
    fn drop(&mut self) {
        self.shared.unregister(self.shard, self.registered);
    }
}

/// Builds the image of a verified checkpoint: every row at version 1, published once
/// together with the exact applied position by [`CheckpointBuilder::finish`]. Neither
/// the engine nor its position escapes before every row is in (`crate::mem` has the
/// same shape for `MemEngine`).
#[derive(Default)]
pub(crate) struct CheckpointBuilder {
    engine: VersionedMemEngine,
    changes: bool,
}

impl CheckpointBuilder {
    pub(crate) fn put(&mut self, cf: ColumnFamily, key: &[u8], value: &[u8]) -> Result<()> {
        self.changes |= changes_data_revision(cf, key);
        self.engine
            .shared
            .tree(cf)
            .insert(VersionedKey::new(key.to_vec(), 1), Some(value.to_vec()))
            .map(drop)
            .map_err(|_| Error::Engine("engine: out of memory for a checkpoint row".into()))
    }

    pub(crate) fn finish(self, at: AppliedPosition) -> VersionedMemEngine {
        let shared = &self.engine.shared;
        {
            let mut writer = shared.writer.lock().expect("writer lock poisoned");
            let mut published = shared.published.lock().expect("published poisoned");
            *published = Published {
                version: 1,
                applied: Some(at),
                data_revision: u64::from(self.changes),
            };
            shared.visible.store(1, Ordering::Release);
            *writer = 1;
        }
        self.engine
    }
}

impl ReadView for View {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>> {
        Ok(self.newest_slot(self.shared.tree(cf), key).flatten())
    }

    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<ScanEntry>> {
        Ok(self.page(cf, start, Some(end), limit))
    }

    fn get_resident_value(&self, cf: ColumnFamily, key: &[u8]) -> Option<Option<Value>> {
        Some(self.newest_slot(self.shared.tree(cf), key).flatten())
    }

    fn seek_le(&self, cf: ColumnFamily, target: &[u8]) -> Result<Option<ScanEntry>> {
        // `[.., target]` is `[.., target + 0x00)`: every version of `target` included.
        let mut end = target.to_vec();
        end.push(0);
        Ok(self.page_rev(cf, b"", &end, 1).pop())
    }

    fn iter<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<ScanEntry>> + 'a>> {
        Ok(Box::new(Pages {
            view: self,
            cf,
            start: start.to_vec(),
            end: Some(end.to_vec()),
            forward: true,
            buffer: VecDeque::new(),
            done: false,
        }))
    }

    fn iter_rev<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<ScanEntry>> + 'a>> {
        Ok(Box::new(Pages {
            view: self,
            cf,
            start: start.to_vec(),
            end: Some(end.to_vec()),
            forward: false,
            buffer: VecDeque::new(),
            done: false,
        }))
    }
}

/// A pull iterator over a view, fetched one bounded page at a time so that a caller
/// honouring a `limit` never materialises the whole range.
struct Pages<'a> {
    view: &'a View,
    cf: ColumnFamily,
    start: Vec<u8>,
    /// Exclusive upper bound; `None` is unbounded (forward only).
    end: Option<Vec<u8>>,
    forward: bool,
    buffer: VecDeque<ScanEntry>,
    done: bool,
}

impl Pages<'_> {
    const PAGE: usize = 256;

    fn refill(&mut self) {
        let page = if self.forward {
            self.view
                .page(self.cf, &self.start, self.end.as_deref(), Self::PAGE)
        } else {
            let end = self.end.as_deref().unwrap_or_default();
            self.view.page_rev(self.cf, &self.start, end, Self::PAGE)
        };
        if page.len() < Self::PAGE {
            self.done = true;
        }
        if let Some((last, _)) = page.last() {
            if self.forward {
                let mut next = last.clone();
                next.push(0);
                self.start = next;
            } else {
                self.end = Some(last.clone()); // exclusive: everything below the last key handed out
            }
        }
        self.buffer.extend(page);
    }
}

impl Iterator for Pages<'_> {
    type Item = Result<ScanEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buffer.is_empty() && !self.done {
            self.refill();
        }
        self.buffer.pop_front().map(Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn put(batch: &mut WriteBatch, cf: ColumnFamily, k: &[u8], v: &[u8]) {
        batch.put(cf, k.to_vec(), v.to_vec());
    }

    fn write(engine: &VersionedMemEngine, cf: ColumnFamily, pairs: &[(&[u8], &[u8])]) {
        let mut batch = WriteBatch::new();
        for (k, v) in pairs {
            put(&mut batch, cf, k, v);
        }
        engine.write(batch).unwrap();
    }

    fn keys(entries: &[ScanEntry]) -> Vec<&[u8]> {
        entries.iter().map(|(k, _)| k.as_slice()).collect()
    }

    #[test]
    fn a_view_is_fixed_at_its_version_and_a_batch_is_visible_whole_across_cfs() {
        let engine = VersionedMemEngine::new();
        write(&engine, ColumnFamily::Default, &[(b"k", b"v1")]);
        let old = engine.snapshot().unwrap();
        let mut batch = WriteBatch::new();
        put(&mut batch, ColumnFamily::Default, b"k", b"v2");
        put(&mut batch, ColumnFamily::Lock, b"k", b"lock");
        put(&mut batch, ColumnFamily::Write, b"k", b"write");
        batch.delete(ColumnFamily::Default, b"k".to_vec()); // later mutation of the same key wins
        put(&mut batch, ColumnFamily::Default, b"k2", b"v");
        engine.write(batch).unwrap();
        let new = engine.snapshot().unwrap();

        assert_eq!(
            old.get(ColumnFamily::Default, b"k").unwrap().as_deref(),
            Some(&b"v1"[..])
        );
        assert_eq!(old.get(ColumnFamily::Lock, b"k").unwrap(), None);
        assert_eq!(old.get(ColumnFamily::Default, b"k2").unwrap(), None);
        assert_eq!(new.get(ColumnFamily::Default, b"k").unwrap(), None);
        assert_eq!(
            new.get(ColumnFamily::Lock, b"k").unwrap().as_deref(),
            Some(&b"lock"[..])
        );
        assert_eq!(
            new.get(ColumnFamily::Write, b"k").unwrap().as_deref(),
            Some(&b"write"[..])
        );
        assert_eq!(
            new.get(ColumnFamily::Default, b"k2").unwrap().as_deref(),
            Some(&b"v"[..])
        );
        assert_eq!(engine.get(ColumnFamily::Default, b"k").unwrap(), None);
    }

    #[test]
    fn scans_seeks_and_iterators_collapse_versions_and_skip_tombstones() {
        let engine = VersionedMemEngine::new();
        let cf = ColumnFamily::Default;
        write(
            &engine,
            cf,
            &[(b"a", b"1"), (b"b", b"1"), (b"c", b"1"), (b"d", b"1")],
        );
        write(&engine, cf, &[(b"b", b"2")]);
        let mut batch = WriteBatch::new();
        batch.delete(cf, b"c".to_vec());
        engine.write(batch).unwrap();
        write(&engine, cf, &[(b"e", b"1")]);
        let view = engine.snapshot().unwrap();

        assert_eq!(
            view.scan(cf, b"a", b"z", 10).unwrap(),
            vec![
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"2".to_vec()),
                (b"d".to_vec(), b"1".to_vec()),
                (b"e".to_vec(), b"1".to_vec()),
            ]
        );
        assert_eq!(
            keys(&view.scan(cf, b"b", b"e", 10).unwrap()),
            vec![&b"b"[..], b"d"]
        );
        assert_eq!(
            keys(&view.scan(cf, b"a", b"z", 2).unwrap()),
            vec![&b"a"[..], b"b"]
        );
        assert!(view.scan(cf, b"b", b"b", 10).unwrap().is_empty());
        assert!(view.scan(cf, b"b", b"", 10).unwrap().is_empty());
        assert!(view.scan(cf, b"a", b"z", 0).unwrap().is_empty());

        // seek_le steps over the tombstoned c and finds the newest version of b.
        assert_eq!(
            view.seek_le(cf, b"c").unwrap(),
            Some((b"b".to_vec(), b"2".to_vec()))
        );
        assert_eq!(
            view.seek_le(cf, b"cc").unwrap(),
            Some((b"b".to_vec(), b"2".to_vec()))
        );
        assert_eq!(
            view.seek_le(cf, b"d").unwrap(),
            Some((b"d".to_vec(), b"1".to_vec()))
        );
        assert_eq!(view.seek_le(cf, b"0").unwrap(), None);

        let forward: Vec<_> = view
            .iter(cf, b"a", b"z")
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(keys(&forward), vec![&b"a"[..], b"b", b"d", b"e"]);
        let backward: Vec<_> = view
            .iter_rev(cf, b"a", b"e")
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(keys(&backward), vec![&b"d"[..], b"b", b"a"]);
        assert_eq!(backward[1].1, b"2");

        // An iterator pulls pages lazily: more entries than one page, in order, each once.
        for i in 0..1000u32 {
            write(
                &engine,
                ColumnFamily::Lock,
                &[(format!("k{i:04}").as_bytes(), b"v")],
            );
        }
        let big = engine.snapshot().unwrap();
        let walked: Vec<_> = big
            .iter(ColumnFamily::Lock, b"k", b"l")
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(walked.len(), 1000);
        assert!(walked.windows(2).all(|w| w[0].0 < w[1].0));
        let walked_rev: Vec<_> = big
            .iter_rev(ColumnFamily::Lock, b"k", b"l")
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(walked_rev.len(), 1000);
        assert!(walked_rev.windows(2).all(|w| w[0].0 > w[1].0));
    }

    #[test]
    fn hot_keys_with_many_versions_stay_correct_and_cheap_for_old_and_new_views() {
        let engine = VersionedMemEngine::new();
        let cf = ColumnFamily::Default;
        write(
            &engine,
            cf,
            &[(b"a", b"a0"), (b"b", b"b0"), (b"c", b"c0"), (b"d", b"d0")],
        );
        let old = engine.snapshot().unwrap();
        // Thousands of newer versions above the old view, then a tombstone and a revival.
        for i in 0..5_000u32 {
            write(
                &engine,
                cf,
                &[(b"a", &i.to_be_bytes()), (b"c", &i.to_be_bytes())],
            );
        }
        let mut batch = WriteBatch::new();
        batch.delete(cf, b"c".to_vec());
        engine.write(batch).unwrap();
        let middle = engine.snapshot().unwrap();
        write(&engine, cf, &[(b"c", b"c-back")]);
        let new = engine.snapshot().unwrap();

        let started = std::time::Instant::now();
        for _ in 0..200 {
            assert_eq!(
                old.scan(cf, b"a", b"z", 10).unwrap(),
                vec![
                    (b"a".to_vec(), b"a0".to_vec()),
                    (b"b".to_vec(), b"b0".to_vec()),
                    (b"c".to_vec(), b"c0".to_vec()),
                    (b"d".to_vec(), b"d0".to_vec()),
                ]
            );
            assert_eq!(
                keys(&middle.scan(cf, b"a", b"z", 10).unwrap()),
                vec![&b"a"[..], b"b", b"d"]
            );
            assert_eq!(new.get(cf, b"c").unwrap().as_deref(), Some(&b"c-back"[..]));
            assert_eq!(
                old.seek_le(cf, b"c").unwrap(),
                Some((b"c".to_vec(), b"c0".to_vec()))
            );
            assert_eq!(
                middle.seek_le(cf, b"c").unwrap(),
                Some((b"b".to_vec(), b"b0".to_vec()))
            );
            let rev: Vec<_> = old
                .iter_rev(cf, b"a", b"z")
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert_eq!(keys(&rev), vec![&b"d"[..], b"c", b"b", b"a"]);
            let rev: Vec<_> = new
                .iter_rev(cf, b"a", b"z")
                .unwrap()
                .map(Result::unwrap)
                .collect();
            assert_eq!(rev[1], (b"c".to_vec(), b"c-back".to_vec()));
        }
        // 200 rounds over 10k superseded versions: seeks, not steps. A linear walk would
        // take seconds here; the bound is generous so that a slow CI box cannot fail it.
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn delete_range_is_one_batch_and_only_touches_the_range() {
        let engine = VersionedMemEngine::new();
        let cf = ColumnFamily::Default;
        write(
            &engine,
            cf,
            &[(b"a", b"1"), (b"b", b"1"), (b"c", b"1"), (b"d", b"1")],
        );
        let before = engine.snapshot().unwrap();
        engine.delete_range(cf, b"b", b"d").unwrap();
        assert_eq!(
            keys(&engine.scan(cf, b"a", b"z", 10).unwrap()),
            vec![&b"a"[..], b"d"]
        );
        assert_eq!(before.scan(cf, b"a", b"z", 10).unwrap().len(), 4);
        engine.delete_range(cf, b"x", b"z").unwrap(); // nothing there: no version spent
        assert_eq!(
            engine.checksum(cf, b"a", b"z").unwrap(),
            engine.checksum(cf, b"a", b"z").unwrap()
        );
    }

    #[test]
    fn write_applied_refuses_a_non_advancing_index_before_touching_anything() {
        let engine = VersionedMemEngine::new();
        let at = |index| AppliedPosition { term: 1, index };
        let mut batch = WriteBatch::new();
        put(&mut batch, ColumnFamily::Default, b"k", b"1");
        engine.write_applied(batch, at(5)).unwrap();
        assert_eq!(engine.volatile_applied_position(), Some(at(5)));
        for refused in [5, 4] {
            let mut batch = WriteBatch::new();
            put(&mut batch, ColumnFamily::Default, b"k", b"2");
            put(&mut batch, ColumnFamily::Lock, b"k", b"2");
            assert!(engine.write_applied(batch, at(refused)).is_err());
        }
        assert_eq!(
            engine.get(ColumnFamily::Default, b"k").unwrap().as_deref(),
            Some(&b"1"[..])
        );
        assert_eq!(engine.get(ColumnFamily::Lock, b"k").unwrap(), None);
        assert_eq!(engine.volatile_applied_position(), Some(at(5)));
        let mut batch = WriteBatch::new();
        put(&mut batch, ColumnFamily::Default, b"k", b"3");
        engine.write_applied(batch, at(9)).unwrap(); // a gap is legal
        let positioned = engine.try_positioned_resident_snapshot().unwrap();
        assert_eq!(positioned.position(), Some(at(9)));
        assert_eq!(
            positioned
                .into_view()
                .get(ColumnFamily::Default, b"k")
                .unwrap()
                .as_deref(),
            Some(&b"3"[..])
        );
        assert_eq!(
            engine.applied_position().unwrap(),
            DurableAppliedPosition::Volatile
        );
        assert_eq!(engine.durability(), Durability::Volatile);
    }

    #[test]
    fn data_revision_ignores_internal_maintenance_records() {
        let engine = VersionedMemEngine::new();
        assert_eq!(engine.data_revision(), 0);
        write(
            &engine,
            ColumnFamily::Default,
            &[(b"\x00kv9\x00manifest_x", b"m")],
        );
        assert_eq!(engine.data_revision(), 0);
        write(&engine, ColumnFamily::Default, &[(b"user", b"v")]);
        assert_eq!(engine.data_revision(), 1);
        write(
            &engine,
            ColumnFamily::Write,
            &[(b"\x00kv9\x00manifest_x", b"m")],
        );
        assert_eq!(engine.data_revision(), 2);
    }

    #[test]
    fn garbage_collection_keeps_what_live_views_read_and_removes_the_rest_in_order() {
        let engine = VersionedMemEngine::new();
        let cf = ColumnFamily::Default;
        write(&engine, cf, &[(b"k", b"1")]);
        let v1 = engine.snapshot().unwrap();
        write(&engine, cf, &[(b"k", b"2")]);
        let v2 = engine.snapshot().unwrap();
        let mut batch = WriteBatch::new();
        batch.delete(cf, b"k".to_vec());
        engine.write(batch).unwrap();
        assert_eq!(engine.stored_entries(cf), 3);

        // v1 is live: nothing it needs goes.
        let report = engine.collect_garbage();
        assert_eq!(report.oldest_live_version, 1);
        assert_eq!(report.removed, 0);
        drop(v1);
        // v2 is live: version 1 is superseded for it; the tombstone is newer than v2.
        let report = engine.collect_garbage();
        assert_eq!((report.oldest_live_version, report.removed), (2, 1));
        assert_eq!(v2.get(cf, b"k").unwrap().as_deref(), Some(&b"2"[..]));
        assert_eq!(engine.stored_entries(cf), 2);
        drop(v2);
        // Nothing live: the tombstone settles and takes the key with it.
        let report = engine.collect_garbage();
        assert_eq!(report.removed, 2);
        assert_eq!(engine.stored_entries(cf), 0);
        assert_eq!(engine.get(cf, b"k").unwrap(), None);

        // A key with many versions across chunk boundaries collapses to one.
        for i in 0..10_000u32 {
            write(&engine, cf, &[(b"hot", i.to_be_bytes().as_slice())]);
            write(&engine, cf, &[(format!("cold{i:05}").as_bytes(), b"c")]);
        }
        assert_eq!(engine.stored_entries(cf), 20_000);
        let report = engine.collect_garbage();
        assert_eq!(report.removed, 9_999);
        assert_eq!(engine.stored_entries(cf), 10_001);
        assert_eq!(
            engine.get(cf, b"hot").unwrap().as_deref(),
            Some(9_999u32.to_be_bytes().as_slice())
        );
        assert_eq!(
            engine.scan(cf, b"cold", b"cole", usize::MAX).unwrap().len(),
            10_000
        );
    }

    #[test]
    fn incremental_legs_collect_exactly_what_one_pass_collects() {
        let fill = |engine: &VersionedMemEngine| {
            for cf in ColumnFamily::ALL {
                for i in 0..1_500u32 {
                    let key = format!("k{i:05}");
                    write(engine, cf, &[(key.as_bytes(), b"a")]);
                    if i % 3 == 0 {
                        write(engine, cf, &[(key.as_bytes(), b"b")]); // one superseded version
                    }
                    if i % 5 == 0 {
                        let mut batch = WriteBatch::new();
                        batch.delete(cf, key.into_bytes()); // a tombstone that settles
                        engine.write(batch).unwrap();
                    }
                }
            }
        };
        let whole = VersionedMemEngine::new();
        fill(&whole);
        let expected = whole.collect_garbage();
        assert!(expected.cycle_complete && expected.removed > 0);

        let legs = VersionedMemEngine::new();
        fill(&legs);
        let mut cursor = GcCursor::default();
        let (mut removed, mut steps) = (0usize, 0usize);
        loop {
            let report = legs.gc_step(&mut cursor, 400);
            removed += report.removed;
            steps += 1;
            // A leg stops at the first key boundary past its budget: at most one key's
            // versions beyond it (three here).
            assert!(report.scanned <= 400 + 3, "leg scanned {}", report.scanned);
            if report.cycle_complete {
                break;
            }
            assert!(steps < 1_000, "the cursor must make progress");
        }
        assert_eq!(removed, expected.removed);
        assert!(steps > 3);
        for cf in ColumnFamily::ALL {
            assert_eq!(legs.stored_entries(cf), whole.stored_entries(cf));
            assert_eq!(
                legs.scan(cf, b"", b"z", usize::MAX).unwrap(),
                whole.scan(cf, b"", b"z", usize::MAX).unwrap()
            );
        }
        // A second cycle finds nothing more; the cursor is reusable.
        let again = legs.collect_garbage();
        assert_eq!(again.removed, 0);
    }

    #[test]
    fn a_frozen_view_streams_every_entry_at_its_version_while_writes_continue() {
        let engine = VersionedMemEngine::new();
        let cf = ColumnFamily::Default;
        let mut batch = WriteBatch::new();
        for i in 0..600u32 {
            put(&mut batch, cf, format!("k{i:04}").as_bytes(), b"old");
            put(&mut batch, ColumnFamily::Lock, format!("l{i:04}").as_bytes(), b"x");
        }
        engine
            .write_applied(batch, AppliedPosition { term: 3, index: 9 })
            .unwrap();
        let (view, position, revision) = engine.freeze_parts();
        assert_eq!(position, Some(AppliedPosition { term: 3, index: 9 }));
        assert_eq!(revision, 1);

        // Overwrite, delete and add after the freeze; collect what can be collected.
        for i in (0..600u32).step_by(2) {
            write(&engine, cf, &[(format!("k{i:04}").as_bytes(), b"new")]);
        }
        let mut batch = WriteBatch::new();
        batch.delete(cf, b"k0001".to_vec());
        put(&mut batch, cf, b"k9999", b"added");
        engine
            .write_applied(batch, AppliedPosition { term: 3, index: 10 })
            .unwrap();
        assert_eq!(engine.collect_garbage().removed, 0, "the frozen view pins its versions");

        let frozen: Vec<ScanEntry> = view.iter_all(cf).collect();
        assert_eq!(frozen.len(), 600);
        assert!(frozen.iter().all(|(_, v)| v == b"old"));
        assert!(frozen.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(view.iter_all(ColumnFamily::Lock).count(), 600);
        assert_eq!(view.iter_all(ColumnFamily::Write).count(), 0);

        let now = engine.snapshot().unwrap();
        assert_eq!(now.get(cf, b"k0000").unwrap().as_deref(), Some(&b"new"[..]));
        assert_eq!(now.get(cf, b"k0001").unwrap(), None);
        assert_eq!(now.scan(cf, b"", b"z", usize::MAX).unwrap().len(), 600);
        drop(view);
        assert!(engine.collect_garbage().removed >= 301);
    }

    #[test]
    fn readers_never_see_half_a_batch_or_a_resurrected_value_under_concurrent_gc() {
        let engine = Arc::new(VersionedMemEngine::new());
        let stop = Arc::new(AtomicBool::new(false));
        let cf = ColumnFamily::Default;
        // The seed is version 0 of the counter: a view may legitimately start there.
        write(
            &engine,
            cf,
            &[(b"a", &0u64.to_be_bytes()), (b"b", &0u64.to_be_bytes())],
        );
        let writer = {
            let (engine, stop) = (engine.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut n = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    n += 1;
                    let v = n.to_be_bytes();
                    let mut batch = WriteBatch::new();
                    put(&mut batch, cf, b"a", &v);
                    put(&mut batch, cf, b"b", &v);
                    if n.is_multiple_of(3) {
                        batch.delete(cf, b"t".to_vec());
                    } else {
                        put(&mut batch, cf, b"t", &v);
                    }
                    engine.write(batch).unwrap();
                }
            })
        };
        let collector = {
            let (engine, stop) = (engine.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    engine.collect_garbage();
                }
            })
        };
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let (engine, stop) = (engine.clone(), stop.clone());
                std::thread::spawn(move || {
                    let mut checks = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        let view = engine.snapshot().unwrap();
                        let a = view.get(cf, b"a").unwrap().unwrap();
                        let b = view.get(cf, b"b").unwrap().unwrap();
                        assert_eq!(a, b, "a view saw half of a batch");
                        let n = u64::from_be_bytes(a.as_slice().try_into().unwrap());
                        let t = view.get(cf, b"t").unwrap();
                        // t follows a and b in the same batch: present iff n % 3 != 0.
                        assert_eq!(
                            t.is_some(),
                            n % 3 != 0 && n > 0,
                            "t resurrected or lost at {n}"
                        );
                        let scanned = view.scan(cf, b"a", b"z", 10).unwrap();
                        assert!(
                            scanned.iter().all(|(k, v)| k == b"t" || *v == a),
                            "scan mixed versions"
                        );
                        checks += 1;
                    }
                    checks
                })
            })
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(400));
        stop.store(true, Ordering::Relaxed);
        writer.join().unwrap();
        collector.join().unwrap();
        let checks: u64 = readers.into_iter().map(|r| r.join().unwrap()).sum();
        assert!(checks > 1000, "readers barely ran: {checks}");
        // With no view left, a final pass leaves exactly one version per live key.
        engine.collect_garbage();
        let live = engine.scan(cf, b"a", b"z", 10).unwrap().len();
        assert_eq!(engine.stored_entries(cf), live);
    }
}
