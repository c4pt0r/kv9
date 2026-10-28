use super::*;
use crate::retention::metadata_read_gate::tests::{fixture, publish, select, step, token};
use crate::retention::tests::{all, apply, owner, plan, seed_with_voters};
use kv9_engine::{Engine, MemEngine, WalEngine};
use std::sync::atomic::{AtomicUsize, Ordering};

fn source(
    e: &MemEngine,
    r: &RootDescriptor,
    i: &CheckpointIntent,
    kind: OwnerKind,
    n: u8,
) -> OwnerBinding {
    let mut s = i.binding(OwnerKind::Version).unwrap();
    if kind != OwnerKind::Version {
        s.descriptor.id = OwnerId::new([n; 16]).unwrap();
        s.descriptor.kind = kind;
        s.descriptor.operation = [n; 32];
        apply(e, r, LedgerRequest::Acquire(s.clone()));
        apply(e, r, LedgerRequest::Publish(s.token()));
    }
    s
}
fn reader(s: &OwnerBinding, n: u8) -> OwnerBinding {
    let l = LocalLifetime {
        node: 1,
        store: [1; 16],
        process: [n; 16],
    };
    if s.descriptor.kind == OwnerKind::Version {
        checkpoint_reader::binding(s, l)
    } else {
        image_reader::binding(s, l)
    }
    .unwrap()
}
fn enter(e: &MemEngine, r: &RootDescriptor, s: &OwnerBinding, b: &OwnerBinding) {
    apply(
        e,
        r,
        LedgerRequest::Share {
            from: s.token(),
            to: b.clone(),
        },
    );
    apply(e, r, LedgerRequest::Publish(b.token()));
}
fn finish(s: &OwnerBinding, b: &OwnerBinding) -> LedgerRequest {
    if s.descriptor.kind == OwnerKind::Version {
        LedgerRequest::FinishCheckpointRead {
            successor: s.token(),
            reader: b.clone(),
        }
    } else {
        LedgerRequest::FinishImageRead {
            successor: s.token(),
            reader: b.clone(),
        }
    }
}
fn close(e: &MemEngine, r: &RootDescriptor, old: &CheckpointIntent, new: &CheckpointIntent) {
    select(e, r, new, 1);
    step(e, r, old);
    apply(e, r, LedgerRequest::CloseMetadataImageReads(token(old)));
}
fn directory(e: &MemEngine, r: &RootDescriptor) {
    for _ in 0..300 {
        match image_reader_index::observe(e.snapshot().unwrap().as_ref(), r).unwrap() {
            image_reader_index::Work::Complete => return,
            image_reader_index::Work::Page { progress, next, .. } => {
                let request = progress.request(next);
                let bytes = encode_request(r.digest(), &request).unwrap();
                assert_eq!(decode_request(&bytes, r.digest()).unwrap(), request);
                apply(e, r, request);
            }
            image_reader_index::Work::Upgrade => {
                apply(e, r, LedgerRequest::Initialize);
            }
            _ => panic!(),
        }
    }
    panic!("image directory never completed");
}
fn observed(e: &impl Engine, r: &RootDescriptor, i: &CheckpointIntent) -> Work {
    observe(e.snapshot().unwrap().as_ref(), r, i.pending_id()).unwrap()
}
fn census(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) -> Cursor {
    for _ in 0..300 {
        match observed(e, r, i) {
            Work::Complete(c) => return c,
            Work::Advance { cursor, .. } => {
                apply(e, r, cursor.request(i.pending_id()));
            }
            _ => panic!(),
        }
    }
    panic!("image census never completed");
}
fn image(i: &CheckpointIntent) -> [u8; 32] {
    *RootDigest::sha256(i.manifest_bytes()).as_bytes()
}
fn generation_key(b: &OwnerBinding, g: u64) -> Vec<u8> {
    let mut suffix = b.descriptor.id.as_bytes().to_vec();
    suffix.extend_from_slice(&g.to_be_bytes());
    key(25, &suffix)
}
fn strip_new(e: &MemEngine) {
    let mut batch = WriteBatch::new();
    for (k, _) in all(e) {
        if k >= key(38, &[]) && k < key(42, &[]) {
            batch.delete(ColumnFamily::Default, k);
        }
    }
    e.write(batch).unwrap();
}

#[test]
fn all_source_operations_and_aliases_share_one_census_and_late_actual_drain_invalidates_it() {
    let (e, r, old, new) = fixture(1);
    let alias = CheckpointIntent::new(&r, &old.manifest().unwrap(), 2).unwrap();
    publish(&e, &r, &alias);
    let sources = [
        source(&e, &r, &old, OwnerKind::Version, 1),
        source(&e, &r, &alias, OwnerKind::Version, 2),
        source(&e, &r, &old, OwnerKind::Snapshot, 81),
        source(&e, &r, &old, OwnerKind::Migration, 82),
    ];
    let readers: Vec<_> = sources
        .iter()
        .enumerate()
        .map(|(n, s)| reader(s, n as u8 + 1))
        .collect();
    for (s, b) in sources.iter().zip(&readers) {
        enter(&e, &r, s, b);
    }
    let newer = source(&e, &r, &new, OwnerKind::Snapshot, 83);
    enter(&e, &r, &newer, &reader(&newer, 5)); // Same SST resources, different complete image.
                                               // Even identical subject bytes cannot conflate another region or an
                                               // anchor identity with this metadata image's non-anchor read dependency.
    for (id, anchor) in [(84, false), (85, true)] {
        let mut other = sources[0].clone();
        other.descriptor.id = OwnerId::new([id; 16]).unwrap();
        other.descriptor.operation = [id; 32];
        other.descriptor.subject_is_anchor = anchor;
        if !anchor {
            other.descriptor.region = 99;
        }
        apply(&e, &r, LedgerRequest::Acquire(other.clone()));
        apply(&e, &r, LedgerRequest::Publish(other.token()));
        enter(&e, &r, &other, &reader(&other, id));
    }
    close(&e, &r, &old, &new);
    assert_eq!(observed(&e, &r, &old), Work::WaitingForCatalog);
    directory(&e, &r);
    let c = census(&e, &r, &old);
    assert_eq!(
        (c.counts.owners, c.counts.generations, c.counts.active),
        (4, 4, 4)
    );
    assert_eq!(observed(&e, &r, &alias), Work::Complete(c.clone()));
    let stable = e.snapshot().unwrap();
    for (s, b) in sources.iter().zip(&readers) {
        apply(&e, &r, finish(s, b));
    }
    assert!(matches!(
        observed(&e, &r, &alias),
        Work::Advance { restart: true, .. }
    ));
    assert_eq!(
        observe(stable.as_ref(), &r, old.pending_id()).unwrap(),
        Work::Complete(c)
    );
    let c = census(&e, &r, &alias);
    assert_eq!((c.counts.drained, c.counts.active), (4, 0));
    assert!(observed(&e, &r, &old).registered_generations_closed());
    for s in &sources {
        assert_eq!(owner(&e, &r, s.descriptor.id).phase, PinPhase::Published);
    }
    // An unentered finish after closure is new retained history, not new I/O.
    apply(&e, &r, finish(&sources[3], &reader(&sources[3], 9)));
    assert!(matches!(
        observed(&e, &r, &old),
        Work::Advance { restart: true, .. }
    ));
    let c = census(&e, &r, &old);
    assert_eq!((c.counts.drained, c.counts.unentered), (4, 1));
    apply(
        &e,
        &r,
        LedgerRequest::QuiesceAfterTransfer {
            from: sources[0].token(),
            to: sources[1].token(),
        },
    );
    apply(&e, &r, LedgerRequest::Release(sources[0].token()));
    assert_eq!(observed(&e, &r, &old), Work::Complete(c));
}

#[test]
fn legacy_and_missing_dense_generations_never_become_known_drained() {
    let (e, r, old, new) = fixture(1);
    let s = source(&e, &r, &old, OwnerKind::Migration, 81);
    let mut b = reader(&s, 1);
    for g in 1..=6 {
        b.generation = NonZeroU64::new(g).unwrap();
        enter(&e, &r, &s, &b);
        apply(&e, &r, finish(&s, &b));
    }
    let legacy = reader(&s, 2);
    enter(&e, &r, &s, &legacy);
    let mut batch = WriteBatch::new();
    batch.delete(ColumnFamily::Default, generation_key(&b, 2));
    batch.delete(ColumnFamily::Default, generation_key(&legacy, 1));
    batch.put(
        ColumnFamily::Default,
        owner_key(legacy.descriptor.id),
        encode_owner(&LedgerOwner {
            binding: legacy.clone(),
            phase: PinPhase::Published,
        }),
    );
    batch.put(
        ColumnFamily::Default,
        owner_registration_key(legacy.descriptor.id),
        owner_registration(&legacy),
    );
    e.write(batch).unwrap();
    apply(&e, &r, finish(&s, &legacy));
    close(&e, &r, &old, &new);
    directory(&e, &r);
    let c = census(&e, &r, &old);
    assert_eq!(
        (
            c.counts.generations,
            c.counts.drained,
            c.counts.legacy,
            c.counts.missing
        ),
        (7, 5, 1, 1)
    );
    assert!(!observed(&e, &r, &old).registered_generations_closed());
}

#[test]
fn v18_backfill_preserves_old_history_and_registration_pass_detects_lost_owner() {
    let (e, r, old, new) = fixture(1);
    let s = source(&e, &r, &old, OwnerKind::Snapshot, 81);
    let b = reader(&s, 1);
    enter(&e, &r, &s, &b);
    close(&e, &r, &old, &new);
    strip_new(&e);
    let before: Vec<_> = all(&e)
        .into_iter()
        .filter(|(k, _)| *k != key(1, &[]))
        .collect();
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(METADATA_IMAGE_GATE_HEADER_MAGIC);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
    e.write(batch).unwrap();
    assert!(matches!(
        image_reader_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap(),
        image_reader_index::Work::Upgrade
    ));
    apply(&e, &r, LedgerRequest::Initialize);
    for (k, v) in before {
        assert_eq!(e.get(ColumnFamily::Default, &k).unwrap(), Some(v));
    }
    let saved = e
        .get(ColumnFamily::Default, &owner_key(b.descriptor.id))
        .unwrap()
        .unwrap();
    let mut batch = WriteBatch::new();
    batch.delete(ColumnFamily::Default, owner_key(b.descriptor.id));
    e.write(batch).unwrap();
    let mut failed = false;
    for _ in 0..50 {
        let image_reader_index::Work::Page { progress, next, .. } =
            image_reader_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap()
        else {
            panic!()
        };
        let request = progress.request(next);
        if plan(&e, &r, &request).is_err() {
            assert_eq!(progress.stage, 1);
            failed = true;
            break;
        }
        apply(&e, &r, request);
    }
    assert!(failed);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, owner_key(b.descriptor.id), saved);
    e.write(batch).unwrap();
    directory(&e, &r);
    assert_eq!(census(&e, &r, &old).counts.active, 1);
    // A complete directory does not repair a lost reverse link on a retry.
    let mut k = s.descriptor.region.to_be_bytes().to_vec();
    k.extend_from_slice(&image(&old));
    k.extend_from_slice(b.descriptor.id.as_bytes());
    let mut batch = WriteBatch::new();
    batch.delete(ColumnFamily::Default, key(38, &k));
    e.write(batch).unwrap();
    assert!(plan(&e, &r, &finish(&s, &b)).is_err());
    assert!(image_reader_index::readers(
        e.snapshot().unwrap().as_ref(),
        &r,
        kv9_common::META_REGION_0.0,
        image(&old),
        None
    )
    .unwrap()
    .owners
    .is_empty());
    // The existing current aggregate has not turned the active generation into a drain proof.
    assert!(!observed(&e, &r, &old).registered_generations_closed());
}

#[test]
fn damaged_image_progress_or_stamp_and_old_headers_cannot_be_repaired_by_retry() {
    let (e, r, old, new) = fixture(1);
    let s = source(&e, &r, &old, OwnerKind::Migration, 81);
    let b = reader(&s, 1);
    enter(&e, &r, &s, &b);
    close(&e, &r, &old, &new);
    directory(&e, &r);
    let c = census(&e, &r, &old);
    let request = c.request(old.pending_id());
    let bytes = encode_request(r.digest(), &request).unwrap();
    assert_eq!(decode_request(&bytes, r.digest()).unwrap(), request);
    for n in 0..bytes.len() {
        assert!(decode_request(&bytes[..n], r.digest()).is_err());
    }
    let saved = all(&e);
    for damage in 0..5 {
        let mut batch = WriteBatch::new();
        for (k, v) in &saved {
            batch.put(ColumnFamily::Default, k.clone(), v.clone());
        }
        match damage {
            0 => {
                batch.delete(ColumnFamily::Default, stamp_key(image(&old)));
            }
            1 | 2 => {
                let k = progress_key(image(&old));
                let mut b = e.get(ColumnFamily::Default, &k).unwrap().unwrap();
                b.truncate(b.len() - 32);
                b[if damage == 1 { 40 } else { 80 }] ^= 1;
                batch.put(ColumnFamily::Default, k, seal(b));
            }
            3 => {
                let mut b = e
                    .get(ColumnFamily::Default, &stamp_key(image(&old)))
                    .unwrap()
                    .unwrap();
                b.truncate(b.len() - 32);
                b[72..80].copy_from_slice(&0u64.to_be_bytes());
                batch.put(ColumnFamily::Default, stamp_key(image(&old)), seal(b));
            }
            _ => {
                let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
                h.truncate(h.len() - 32);
                h[..8].copy_from_slice(METADATA_IMAGE_GATE_HEADER_MAGIC);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
            }
        }
        e.write(batch).unwrap();
        let before = all(&e);
        assert!(observe(e.snapshot().unwrap().as_ref(), &r, old.pending_id()).is_err());
        assert!(plan(&e, &r, &request).is_err());
        assert_eq!(all(&e), before);
    }
}

struct Count<'a> {
    view: &'a dyn ReadView,
    rows: AtomicUsize,
    gets: AtomicUsize,
}
impl ReadView for Count<'_> {
    fn get(&self, cf: ColumnFamily, k: &[u8]) -> Result<Option<Vec<u8>>> {
        if k.starts_with(PREFIX) {
            self.gets.fetch_add(1, Ordering::Relaxed);
        }
        self.view.get(cf, k)
    }
    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<kv9_engine::ScanEntry>> {
        assert!(
            !start.starts_with(PREFIX),
            "census must consume a bounded iterator"
        );
        self.view.scan(cf, start, end, limit)
    }
    fn seek_le(&self, cf: ColumnFamily, k: &[u8]) -> Result<Option<kv9_engine::ScanEntry>> {
        assert!(!k.starts_with(PREFIX));
        self.view.seek_le(cf, k)
    }
    fn iter<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
        let retention = start.starts_with(PREFIX);
        Ok(Box::new(self.view.iter(cf, start, end)?.inspect(
            move |_| {
                if retention {
                    self.rows.fetch_add(1, Ordering::Relaxed);
                }
            },
        )))
    }
    fn iter_rev<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
        assert!(!start.starts_with(PREFIX));
        self.view.iter_rev(cf, start, end)
    }
}

#[test]
fn a_lost_image_link_cannot_turn_a_registered_active_reader_into_an_empty_closed_set() {
    for count_lost in [false, true] {
        let (e, r, old, new) = fixture(1);
        let s = source(&e, &r, &old, OwnerKind::Snapshot, 81);
        let b = reader(&s, 1);
        enter(&e, &r, &s, &b);
        close(&e, &r, &old, &new);
        directory(&e, &r);
        let Work::Advance { cursor, .. } = observed(&e, &r, &old) else {
            panic!()
        };
        let mut suffix = s.descriptor.region.to_be_bytes().to_vec();
        suffix.extend_from_slice(&image(&old));
        suffix.extend_from_slice(b.descriptor.id.as_bytes());
        let mut batch = WriteBatch::new();
        batch.delete(
            ColumnFamily::Default,
            if count_lost {
                stamp_key(image(&old))
            } else {
                key(38, &suffix)
            },
        );
        e.write(batch).unwrap();
        let before = all(&e);
        assert!(
            plan(&e, &r, &cursor.request(old.pending_id())).is_err(),
            "missing index row or count must not mint a complete empty census"
        );
        assert!(
            plan(&e, &r, &finish(&s, &b)).is_err(),
            "a lifecycle mutation cannot repair lost directory history"
        );
        assert_eq!(all(&e), before);
    }
}

#[test]
fn directory_and_dense_generation_pages_have_fixed_work_for_13_and_113_histories() {
    for many_generations in [false, true] {
        let mut costs = Vec::new();
        for size in [13_u64, 113] {
            let (e, r, old, new) = fixture(1);
            let s = source(&e, &r, &old, OwnerKind::Migration, 81);
            for n in 1..=if many_generations { 1 } else { size } {
                let mut b = reader(&s, n as u8);
                for generation in 1..=if many_generations { size } else { 1 } {
                    b.generation = NonZeroU64::new(generation).unwrap();
                    enter(&e, &r, &s, &b);
                    apply(&e, &r, finish(&s, &b));
                }
            }
            close(&e, &r, &old, &new);
            let view = e.snapshot().unwrap();
            let measured = Count {
                view: view.as_ref(),
                rows: AtomicUsize::new(0),
                gets: AtomicUsize::new(0),
            };
            let image_reader_index::Work::Page {
                progress,
                next,
                examined,
            } = image_reader_index::observe(&measured, &r).unwrap()
            else {
                panic!()
            };
            assert!(examined <= 4);
            assert!(measured.rows.swap(0, Ordering::Relaxed) <= 5);
            plan_retention(&measured, &r, &progress.request(next)).unwrap();
            assert!(measured.rows.load(Ordering::Relaxed) <= 5);
            drop(view);
            directory(&e, &r);
            let Work::Advance { cursor, .. } = observed(&e, &r, &old) else {
                panic!()
            };
            let view = e.snapshot().unwrap();
            let measured = Count {
                view: view.as_ref(),
                rows: AtomicUsize::new(0),
                gets: AtomicUsize::new(0),
            };
            let page = plan_retention(&measured, &r, &cursor.request(old.pending_id())).unwrap();
            assert!(measured.rows.load(Ordering::Relaxed) <= 5);
            costs.push((
                measured.rows.load(Ordering::Relaxed),
                measured.gets.load(Ordering::Relaxed),
            ));
            e.write(page.into_batch()).unwrap();
            let Work::Advance { cursor, .. } = observed(&e, &r, &old) else {
                panic!()
            };
            assert!(cursor.counts.generations <= 4);
            let done = census(&e, &r, &old);
            assert_eq!(done.counts.generations, size);
        }
        assert_eq!(
            costs[0], costs[1],
            "per-page work must not grow with history"
        );
    }
}

#[test]
fn actual_wal_cuts_keep_directory_counts_census_and_late_invalidation_atomic() {
    for mutation in 0..3 {
        let (memory, r, old, new) = fixture(1);
        let s = source(&memory, &r, &old, OwnerKind::Snapshot, 81);
        let b = reader(&s, 1);
        enter(&memory, &r, &s, &b);
        close(&memory, &r, &old, &new);
        let request = if mutation == 0 {
            strip_new(&memory);
            let image_reader_index::Work::Page { progress, next, .. } =
                image_reader_index::observe(memory.snapshot().unwrap().as_ref(), &r).unwrap()
            else {
                panic!()
            };
            progress.request(next)
        } else {
            directory(&memory, &r);
            if mutation == 1 {
                let Work::Advance { cursor, .. } = observed(&memory, &r, &old) else {
                    panic!()
                };
                cursor.request(old.pending_id())
            } else {
                assert_eq!(census(&memory, &r, &old).counts.active, 1);
                finish(&s, &b)
            }
        };
        let before_rows = all(&memory);
        let after_plan = plan(&memory, &r, &request).unwrap();
        memory.write(after_plan.into_batch()).unwrap();
        let after_rows = all(&memory);
        let dir = std::env::temp_dir().join(format!(
            "kv9-image-reader-census-{}",
            kv9_common::StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("base");
        let (e, _) = WalEngine::open(&path).unwrap();
        let (_, mut batch) = seed_with_voters(r.voters.clone());
        for (k, v) in &before_rows {
            batch.put(ColumnFamily::Default, k.clone(), v.clone());
        }
        e.write(batch).unwrap();
        let before = std::fs::read(&path).unwrap();
        e.write(plan(&e, &r, &request).unwrap().into_batch())
            .unwrap();
        drop(e);
        let after = std::fs::read(&path).unwrap();
        for cut in [
            before.len(),
            before.len() + 1,
            before.len() + (after.len() - before.len()) / 2,
            after.len() - 1,
            after.len(),
        ] {
            let path = dir.join(format!("cut-{cut}"));
            std::fs::write(&path, &after[..cut]).unwrap();
            let (e, _) = WalEngine::open(&path).unwrap();
            assert_eq!(
                all(&e),
                if cut == after.len() {
                    after_rows.clone()
                } else {
                    before_rows.clone()
                }
            );
            e.write(plan(&e, &r, &request).unwrap().into_batch())
                .unwrap();
            assert_eq!(all(&e), after_rows);
            assert!(!plan(&e, &r, &request).unwrap().changed());
            assert_eq!(owner(&e, &r, s.descriptor.id).phase, PinPhase::Published);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn unentered_owner_inserted_behind_backfill_cursor_is_counted_once_and_pages_cannot_skip() {
    let (e, r, old, new) = fixture(1);
    let s = source(&e, &r, &old, OwnerKind::Snapshot, 81);
    let b = reader(&s, 1);
    enter(&e, &r, &s, &b);
    close(&e, &r, &old, &new);
    strip_new(&e);
    let image_reader_index::Work::Page { progress, next, .. } =
        image_reader_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap()
    else {
        panic!()
    };
    let after = next.expect("fixture has more than one owner page");
    let skipped = progress.request(None);
    let before = all(&e);
    assert!(plan(&e, &r, &skipped).is_err());
    assert_eq!(all(&e), before);
    let request = progress.request(next);
    apply(&e, &r, request.clone());
    assert!(!plan(&e, &r, &request).unwrap().changed());
    let late = (2..=255)
        .map(|n| reader(&s, n))
        .find(|b| b.descriptor.id < after)
        .unwrap();
    apply(&e, &r, finish(&s, &late));
    directory(&e, &r);
    let done = census(&e, &r, &old);
    assert_eq!(
        (
            done.counts.owners,
            done.counts.active,
            done.counts.unentered
        ),
        (2, 1, 1)
    );
    apply(&e, &r, finish(&s, &late));
    assert_eq!(observed(&e, &r, &old), Work::Complete(done));
}
