use super::*;
use crate::retention::metadata_read_gate::tests::{fixture, select, step, token};
use crate::retention::tests::{all, apply, owner, plan, seed_with_voters};
use kv9_engine::{Engine, MemEngine, WalEngine};
use std::sync::atomic::{AtomicUsize, Ordering};

fn alias(e: &MemEngine, r: &RootDescriptor, old: &CheckpointIntent, n: u64) -> CheckpointIntent {
    let i = CheckpointIntent::new(r, &old.manifest().unwrap(), n).unwrap();
    apply(e, r, LedgerRequest::BeginCheckpoint(i.clone()));
    i
}
fn build(i: &CheckpointIntent, n: u8) -> OwnerBinding {
    checkpoint_admission::executor_binding(
        i,
        LocalLifetime {
            node: 1,
            store: [1; 16],
            process: [n; 16],
        },
    )
    .unwrap()
}
fn share(i: &CheckpointIntent, b: &OwnerBinding) -> LedgerRequest {
    LedgerRequest::Share {
        from: token(i),
        to: b.clone(),
    }
}
fn enter(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent, b: &OwnerBinding) {
    apply(e, r, share(i, b));
    apply(e, r, LedgerRequest::Publish(b.token()));
}
fn finish(i: &CheckpointIntent, b: &OwnerBinding, issued: u64, resolved: u64) -> LedgerRequest {
    LedgerRequest::FinishCheckpointExecution {
        pending: token(i),
        executor: b.clone(),
        issued,
        resolved,
    }
}
fn close_reads(e: &MemEngine, r: &RootDescriptor, old: &CheckpointIntent, new: &CheckpointIntent) {
    select(e, r, new, 1);
    step(e, r, old);
    apply(e, r, LedgerRequest::CloseMetadataImageReads(token(old)));
}
fn close(e: &MemEngine, r: &RootDescriptor, old: &CheckpointIntent, new: &CheckpointIntent) {
    close_reads(e, r, old, new);
    apply(e, r, LedgerRequest::CloseMetadataImageWrites(token(old)));
}
fn directory(e: &MemEngine, r: &RootDescriptor) {
    for _ in 0..1000 {
        match image_publisher_index::observe(e.snapshot().unwrap().as_ref(), r).unwrap() {
            image_publisher_index::Work::Complete => return,
            image_publisher_index::Work::Page { progress, next, .. } => {
                let req = progress.request(next);
                assert_eq!(
                    decode_request(&encode_request(r.digest(), &req).unwrap(), r.digest()).unwrap(),
                    req
                );
                apply(e, r, req);
            }
            image_publisher_index::Work::Upgrade => {
                apply(e, r, LedgerRequest::Initialize);
            }
            _ => panic!("unexpected directory state"),
        }
    }
    panic!("directory failed to finish")
}
fn observed(e: &impl Engine, r: &RootDescriptor, i: &CheckpointIntent) -> Work {
    observe(e.snapshot().unwrap().as_ref(), r, i.pending_id()).unwrap()
}
fn census(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) -> Cursor {
    for _ in 0..1000 {
        match observed(e, r, i) {
            Work::Complete(c) => return c,
            Work::Advance { cursor, .. } => {
                let req = cursor.request(i.pending_id());
                assert_eq!(
                    decode_request(&encode_request(r.digest(), &req).unwrap(), r.digest()).unwrap(),
                    req
                );
                apply(e, r, req);
            }
            _ => panic!("unexpected census state"),
        }
    }
    panic!("census failed to finish")
}
fn image(i: &CheckpointIntent) -> [u8; 32] {
    *RootDigest::sha256(i.manifest_bytes()).as_bytes()
}

#[test]
fn upload_fence_blocks_new_and_held_builds_across_pending_aliases() {
    let (e, r, old, new) = fixture(1);
    let a = alias(&e, &r, &old, 2);
    let held = build(&a, 1);
    apply(&e, &r, share(&a, &held));
    let active = build(&a, 2);
    enter(&e, &r, &a, &active);
    close_reads(&e, &r, &old, &new);
    // V19 read fencing alone must not be reinterpreted as a write fence.
    assert!(plan(&e, &r, &share(&a, &build(&a, 3))).is_ok());
    assert_eq!(observed(&e, &r, &old), Work::Open);
    let req = LedgerRequest::CloseMetadataImageWrites(token(&old));
    assert_eq!(
        decode_request(&encode_request(r.digest(), &req).unwrap(), r.digest()).unwrap(),
        req
    );
    apply(&e, &r, req.clone());
    let unchanged = all(&e);
    assert!(
        plan(&e, &r, &share(&a, &build(&a, 3))).is_err(),
        "another Pending alias still admits new uploads"
    );
    assert!(plan(&e, &r, &LedgerRequest::Publish(held.token())).is_err());
    assert!(!plan(&e, &r, &req).unwrap().changed());
    assert_eq!(all(&e), unchanged);
    let late = alias(&e, &r, &old, 3);
    assert!(plan(&e, &r, &share(&late, &build(&late, 4))).is_err());
    // Admitted scopes can still submit their actual drain facts.
    apply(&e, &r, finish(&a, &active, 1, 0));
    apply(&e, &r, finish(&a, &held, 0, 0));
    apply(&e, &r, finish(&late, &build(&late, 5), 0, 0));
    directory(&e, &r);
    let c = census(&e, &r, &old);
    assert_eq!(
        (
            c.counts.owners,
            c.counts.zero_put,
            c.counts.nonzero_put,
            c.counts.unresolved_puts
        ),
        (3, 2, 1, 1)
    );
    assert_eq!(observed(&e, &r, &late), Work::Complete(c));
    assert!(!observed(&e, &r, &old).registered_zero_put_closed());
    assert_eq!(owner(&e, &r, token(&a).owner).phase, PinPhase::Published);
}

#[test]
fn dense_history_preserves_unknown_puts_legacy_and_missing_facts_after_zero_generations() {
    let (e, r, old, new) = fixture(1);
    let a = alias(&e, &r, &old, 2);
    let mut b = build(&a, 1);
    for g in 1..=6 {
        b.generation = NonZeroU64::new(g).unwrap();
        enter(&e, &r, &a, &b);
        apply(
            &e,
            &r,
            finish(&a, &b, u64::from(g == 2) * 3, u64::from(g == 2)),
        );
    }
    let mut legacy = a.binding(OwnerKind::Pending).unwrap();
    legacy.descriptor.kind = OwnerKind::Build;
    legacy.descriptor.operation = [81; 32];
    legacy.descriptor.id = OwnerId::new([81; 16]).unwrap();
    apply(&e, &r, LedgerRequest::Acquire(legacy));
    // Earlier missing origins/reports cannot be inferred from the current zero generation.
    let mut batch = WriteBatch::new();
    for (tag, g) in [(16, 1u64), (13, 3)] {
        let mut id = a.pending_id().as_bytes().to_vec();
        id.extend_from_slice(b.descriptor.id.as_bytes());
        id.extend_from_slice(&g.to_be_bytes());
        batch.delete(ColumnFamily::Default, key(tag, &id));
    }
    e.write(batch).unwrap();
    close(&e, &r, &old, &new);
    directory(&e, &r);
    let c = census(&e, &r, &old);
    assert_eq!(
        (c.counts.owners, c.counts.generations, c.counts.zero_put),
        (2, 7, 3)
    );
    assert_eq!(
        (
            c.counts.missing_origin,
            c.counts.missing_terminal,
            c.counts.nonzero_put,
            c.counts.unresolved_puts
        ),
        (2, 1, 1, 2)
    );
    assert!(!observed(&e, &r, &old).registered_zero_put_closed());
}

#[test]
fn late_report_restarts_shared_image_census_but_stable_view_and_exact_retry_do_not_change() {
    let (e, r, old, new) = fixture(1);
    let a = alias(&e, &r, &old, 2);
    let b = build(&a, 1);
    enter(&e, &r, &a, &b);
    close(&e, &r, &old, &new);
    directory(&e, &r);
    let c = census(&e, &r, &old);
    assert_eq!(c.counts.active, 1);
    let snapshot = e.snapshot().unwrap();
    apply(&e, &r, finish(&a, &b, 4, 4));
    assert!(matches!(
        observed(&e, &r, &a),
        Work::Advance { restart: true, .. }
    ));
    assert_eq!(
        observe(snapshot.as_ref(), &r, old.pending_id()).unwrap(),
        Work::Complete(c)
    );
    let done = census(&e, &r, &a);
    assert_eq!(
        (
            done.counts.active,
            done.counts.nonzero_put,
            done.counts.unresolved_puts
        ),
        (0, 1, 0)
    );
    assert!(
        !observed(&e, &r, &a).registered_zero_put_closed(),
        "even resolved PUTs are not a zero-PUT certificate"
    );
    let before = all(&e);
    assert!(!plan(&e, &r, &finish(&a, &b, 4, 4)).unwrap().changed());
    assert_eq!(all(&e), before);
    // Retained Build history remains readable after same-image source transfer.
    let v = a.binding(OwnerKind::Version).unwrap();
    apply(
        &e,
        &r,
        LedgerRequest::Share {
            from: token(&a),
            to: v.clone(),
        },
    );
    apply(&e, &r, LedgerRequest::Publish(v.token()));
    apply(
        &e,
        &r,
        LedgerRequest::QuiesceAfterTransfer {
            from: token(&a),
            to: v.token(),
        },
    );
    apply(&e, &r, LedgerRequest::Release(token(&a)));
    assert_eq!(observed(&e, &r, &a), Work::Complete(done));
}

fn strip_directory(e: &MemEngine) {
    let mut batch = WriteBatch::new();
    for (k, _) in all(e) {
        if k >= key(44, &[]) && k < key(48, &[]) {
            batch.delete(ColumnFamily::Default, k);
        }
    }
    e.write(batch).unwrap();
}
fn old_header(e: &MemEngine) {
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(IMAGE_READER_HEADER_MAGIC);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
    e.write(batch).unwrap();
}

#[test]
fn v19_backfill_preserves_rows_and_registration_pass_detects_lost_owner() {
    let (e, r, old, new) = fixture(1);
    let a = alias(&e, &r, &old, 2);
    let b = build(&a, 1);
    enter(&e, &r, &a, &b);
    close_reads(&e, &r, &old, &new);
    strip_directory(&e);
    old_header(&e);
    let before = all(&e);
    assert!(matches!(
        image_publisher_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap(),
        image_publisher_index::Work::Upgrade
    ));
    apply(&e, &r, LedgerRequest::Initialize);
    for (k, v) in before {
        if k != key(1, &[]) {
            assert_eq!(e.get(ColumnFamily::Default, &k).unwrap(), Some(v));
        }
    }
    let saved = e
        .get(ColumnFamily::Default, &owner_key(b.descriptor.id))
        .unwrap()
        .unwrap();
    let mut batch = WriteBatch::new();
    batch.delete(ColumnFamily::Default, owner_key(b.descriptor.id));
    e.write(batch).unwrap();
    let mut failed = false;
    for _ in 0..100 {
        let image_publisher_index::Work::Page { progress, next, .. } =
            image_publisher_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap()
        else {
            panic!()
        };
        if plan(&e, &r, &progress.request(next)).is_err() {
            assert_eq!(progress.stage, 1);
            failed = true;
            break;
        }
        apply(&e, &r, progress.request(next));
    }
    assert!(failed);
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, owner_key(b.descriptor.id), saved);
    e.write(batch).unwrap();
    directory(&e, &r);
    apply(&e, &r, LedgerRequest::CloseMetadataImageWrites(token(&old)));
    assert_eq!(census(&e, &r, &old).counts.active, 1);
}

#[test]
fn missing_directory_count_or_fence_evidence_cannot_manufacture_closed_empty_history() {
    for lost in [42, 43, 44, 46] {
        let (e, r, old, new) = fixture(1);
        let a = alias(&e, &r, &old, 2);
        let b = build(&a, 1);
        enter(&e, &r, &a, &b);
        close(&e, &r, &old, &new);
        directory(&e, &r);
        let Work::Advance { cursor, .. } = observed(&e, &r, &old) else {
            panic!()
        };
        let suffix = if lost == 44 {
            let mut suffix = kv9_common::META_REGION_0.0.to_be_bytes().to_vec();
            suffix.extend_from_slice(&image(&old));
            suffix.extend_from_slice(b.descriptor.id.as_bytes());
            suffix
        } else {
            image(&old).to_vec()
        };
        let mut batch = WriteBatch::new();
        batch.delete(ColumnFamily::Default, key(lost, &suffix));
        e.write(batch).unwrap();
        let before = all(&e);
        assert!(
            plan(&e, &r, &cursor.request(old.pending_id())).is_err(),
            "lost tag {lost}"
        );
        if matches!(lost, 44 | 46) {
            assert!(plan(&e, &r, &finish(&a, &b, 0, 0)).is_err());
        }
        assert_eq!(all(&e), before);
    }
    let (e, r, old, new) = fixture(1);
    close(&e, &r, &old, &new);
    directory(&e, &r);
    census(&e, &r, &old);
    let mut batch = WriteBatch::new();
    for tag in [42, 43] {
        batch.delete(ColumnFamily::Default, key(tag, &image(&old)));
    }
    e.write(batch).unwrap();
    assert!(plan(
        &e,
        &r,
        &LedgerRequest::CloseMetadataImageWrites(token(&old))
    )
    .is_err());
}

#[test]
fn lower_header_and_post_fence_origin_are_refused_without_repair() {
    let (e, r, old, new) = fixture(1);
    let a = alias(&e, &r, &old, 2);
    let b = build(&a, 1);
    enter(&e, &r, &a, &b);
    close(&e, &r, &old, &new);
    directory(&e, &r);
    let saved = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    old_header(&e);
    assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, key(1, &[]), saved);
    e.write(batch).unwrap();
    let metadata_image_write_gate::Work::Closed(gate) =
        metadata_image_write_gate::observe(e.snapshot().unwrap().as_ref(), &r, old.pending_id())
            .unwrap()
    else {
        panic!()
    };
    let mut suffix = a.pending_id().as_bytes().to_vec();
    suffix.extend_from_slice(b.descriptor.id.as_bytes());
    suffix.extend_from_slice(&1u64.to_be_bytes());
    let k = key(16, &suffix);
    let mut bytes = e.get(ColumnFamily::Default, &k).unwrap().unwrap();
    bytes.truncate(bytes.len() - 32);
    // Enrollment is magic/root/Pending/executor/generation/binding/opened/recorded/origin.
    bytes[120..128].copy_from_slice(&gate.closed_at.to_be_bytes());
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, k, seal(bytes));
    e.write(batch).unwrap();
    let Work::Advance { cursor, .. } = observed(&e, &r, &old) else {
        panic!()
    };
    assert!(plan(&e, &r, &cursor.request(old.pending_id())).is_err());
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
fn directory_and_dense_generation_pages_have_fixed_work_for_13_and_113_histories() {
    for many_generations in [false, true] {
        let mut costs = Vec::new();
        for size in [13_u64, 113] {
            let (e, r, old, new) = fixture(1);
            let s = alias(&e, &r, &old, 2);
            for n in 1..=if many_generations { 1 } else { size } {
                let mut b = build(&s, n as u8);
                for generation in 1..=if many_generations { size } else { 1 } {
                    b.generation = NonZeroU64::new(generation).unwrap();
                    enter(&e, &r, &s, &b);
                    apply(&e, &r, finish(&s, &b, 0, 0));
                }
            }
            close(&e, &r, &old, &new);
            let view = e.snapshot().unwrap();
            let measured = Count {
                view: view.as_ref(),
                rows: AtomicUsize::new(0),
                gets: AtomicUsize::new(0),
            };
            let image_publisher_index::Work::Page {
                progress,
                next,
                examined,
            } = image_publisher_index::observe(&measured, &r).unwrap()
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
    for mutation in 0..4 {
        let (memory, r, old, new) = fixture(1);
        let s = alias(&memory, &r, &old, 2);
        let b = build(&s, 1);
        enter(&memory, &r, &s, &b);
        close_reads(&memory, &r, &old, &new);
        if mutation != 3 {
            apply(
                &memory,
                &r,
                LedgerRequest::CloseMetadataImageWrites(token(&old)),
            );
        }
        let request = if mutation == 3 {
            LedgerRequest::CloseMetadataImageWrites(token(&old))
        } else if mutation == 0 {
            strip_directory(&memory);
            let image_publisher_index::Work::Page { progress, next, .. } =
                image_publisher_index::observe(memory.snapshot().unwrap().as_ref(), &r).unwrap()
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
                finish(&s, &b, 0, 0)
            }
        };
        let before_rows = all(&memory);
        let after_plan = plan(&memory, &r, &request).unwrap();
        memory.write(after_plan.into_batch()).unwrap();
        let after_rows = all(&memory);
        let dir = std::env::temp_dir().join(format!(
            "kv9-image-publisher-census-{}",
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
            assert_eq!(owner(&e, &r, token(&s).owner).phase, PinPhase::Published);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn unentered_owner_inserted_behind_backfill_cursor_is_counted_once_and_pages_cannot_skip() {
    let (e, r, old, new) = fixture(1);
    let s = alias(&e, &r, &old, 2);
    let b = build(&s, 1);
    enter(&e, &r, &s, &b);
    close(&e, &r, &old, &new);
    strip_directory(&e);
    let image_publisher_index::Work::Page { progress, next, .. } =
        image_publisher_index::observe(e.snapshot().unwrap().as_ref(), &r).unwrap()
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
        .map(|n| build(&s, n))
        .find(|b| b.descriptor.id < after)
        .unwrap();
    apply(&e, &r, finish(&s, &late, 0, 0));
    directory(&e, &r);
    let done = census(&e, &r, &old);
    assert_eq!(
        (done.counts.owners, done.counts.active, done.counts.zero_put),
        (2, 1, 1)
    );
    apply(&e, &r, finish(&s, &late, 0, 0));
    assert_eq!(observed(&e, &r, &old), Work::Complete(done));
}
