use super::*;
use crate::retention::bootstrap_reader::tests::{begin_request, setup};
use crate::retention::tests::{all, apply, owner, plan, seed};
use kv9_common::{NodeId, StoreIdentity, StoreIncarnation};
use kv9_engine::{Engine, MemEngine, WalEngine};
use std::sync::atomic::{AtomicUsize, Ordering};

// Explicit pre-admission bootstrap fixtures: preserve all forward pins/phases,
// without pretending an old arbitrary operation identifies its source/store.
fn fixture(
    n: u8,
) -> (
    MemEngine,
    RootDescriptor,
    OwnerBinding,
    OwnerBinding,
    Vec<LedgerOwner>,
) {
    let (e, r, v, b) = setup();
    let mut legacy = Vec::new();
    for n in 1..=n {
        let mut old = b.clone();
        let mut id = [254; 16];
        id[15] = n;
        old.descriptor.id = OwnerId::new(id).unwrap();
        old.descriptor.operation = [n; 32];
        old.descriptor.kind = OwnerKind::Legacy;
        apply(&e, &r, LedgerRequest::Acquire(old.clone()));
        if n % 3 != 0 {
            apply(&e, &r, LedgerRequest::Publish(old.token()));
        }
        if n % 3 == 2 {
            apply(
                &e,
                &r,
                LedgerRequest::QuiesceAfterTransfer {
                    from: old.token(),
                    to: v.token(),
                },
            );
            apply(&e, &r, LedgerRequest::Release(old.token()));
        }
        let mut current = owner(&e, &r, old.descriptor.id);
        current.binding.descriptor.kind = OwnerKind::BootstrapReader;
        let mut batch = WriteBatch::new();
        batch.put(
            ColumnFamily::Default,
            owner_key(old.descriptor.id),
            encode_owner(&current),
        );
        batch.put(
            ColumnFamily::Default,
            owner_registration_key(old.descriptor.id),
            owner_registration(&current.binding),
        );
        e.write(batch).unwrap();
        legacy.push(current);
    }
    (e, r, v, b, legacy)
}
fn as_v12(e: &MemEngine) {
    let mut batch = WriteBatch::new();
    for (k, _) in all(e) {
        if [29, 30, 33, 34]
            .iter()
            .any(|tag| k.starts_with(&key(*tag, &[])))
        {
            batch.delete(ColumnFamily::Default, k);
        }
    }
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(BOOTSTRAP_ADMISSION_HEADER_MAGIC);
    batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
    e.write(batch).unwrap();
}
fn request(e: &impl Engine, r: &RootDescriptor) -> LedgerRequest {
    let Work::Page { progress, next, .. } = observe(e.snapshot().unwrap().as_ref(), r).unwrap()
    else {
        panic!()
    };
    progress.request(next)
}
fn complete(e: &MemEngine, r: &RootDescriptor) {
    for _ in 0..1000 {
        match observe(e.snapshot().unwrap().as_ref(), r).unwrap() {
            Work::Complete => return,
            Work::Upgrade => {
                apply(e, r, LedgerRequest::Initialize);
            }
            Work::Page { progress, next, .. } => {
                apply(e, r, progress.request(next));
            }
            _ => panic!(),
        }
    }
    panic!("census did not finish")
}
fn collected(e: &impl Engine, r: &RootDescriptor, image: Option<[u8; 32]>) -> Vec<Grant> {
    let view = e.snapshot().unwrap();
    let mut after = None;
    let mut result = Vec::new();
    loop {
        let page = query(view.as_ref(), r, image, after).unwrap();
        result.extend(page.grants);
        match page.next {
            Some(next) => after = Some(next),
            None => return result,
        }
    }
}

#[test]
fn v12_census_preserves_managed_origins_and_all_unknown_legacy_phases_and_subjects() {
    let (e, r, v, b, mut legacy) = fixture(13);
    // An unknown old subject still belongs in the global inventory. It must not
    // silently disappear because no canonical Pending can be derived from it.
    legacy[0].binding.descriptor.subject = [37; 32];
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        owner_key(legacy[0].binding.descriptor.id),
        encode_owner(&legacy[0]),
    );
    batch.put(
        ColumnFamily::Default,
        owner_registration_key(legacy[0].binding.descriptor.id),
        owner_registration(&legacy[0].binding),
    );
    e.write(batch).unwrap();
    apply(&e, &r, begin_request(&r, &v));
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    let original =
        bootstrap_reader::admission::observe(e.snapshot().unwrap().as_ref(), &r, b.token())
            .unwrap();
    as_v12(&e);
    assert!(matches!(
        observe(e.snapshot().unwrap().as_ref(), &r).unwrap(),
        Work::Upgrade
    ));
    assert!(catalog(e.snapshot().unwrap().as_ref(), &r, None).is_err());
    assert_eq!(
        bootstrap_reader::admission::observe(e.snapshot().unwrap().as_ref(), &r, b.token())
            .unwrap(),
        original
    );
    complete(&e, &r);
    let actual = collected(&e, &r, None);
    assert_eq!(actual.len(), 14);
    for old in legacy {
        assert!(actual
            .iter()
            .any(|g| g.owner == old && g.admission.is_none()));
    }
    assert!(actual
        .iter()
        .any(|g| g.owner.binding == b && g.admission == original));
    assert_eq!(collected(&e, &r, Some(v.descriptor.subject)).len(), 13);
    assert_eq!(collected(&e, &r, Some([37; 32])).len(), 1);
    assert!(collected(&e, &r, Some([38; 32])).is_empty());
    assert_eq!(owner(&e, &r, v.descriptor.id).phase, PinPhase::Published);
}

#[test]
fn new_grants_behind_the_cursor_and_terminal_before_entry_are_atomically_indexed() {
    let (e, r, v, b, _) = fixture(9);
    // Reach a cursor beyond the still-absent canonical grant's ID.
    loop {
        let Work::Page { progress, .. } = observe(e.snapshot().unwrap().as_ref(), &r).unwrap()
        else {
            panic!()
        };
        if progress.stage == 0 && progress.after.is_some_and(|a| a > b.descriptor.id) {
            break;
        }
        assert_eq!(progress.stage, 0);
        apply(&e, &r, request(&e, &r));
    }
    apply(&e, &r, begin_request(&r, &v));
    complete(&e, &r);
    let before = e
        .get(
            ColumnFamily::Default,
            &index_key(b.descriptor.subject, b.descriptor.id),
        )
        .unwrap()
        .unwrap();
    apply(&e, &r, LedgerRequest::Publish(b.token()));
    let LedgerRequest::BeginBootstrapRead { pending, .. } = begin_request(&r, &v) else {
        panic!()
    };
    e.write(
        bootstrap_reader::plan_verified(
            e.snapshot().unwrap().as_ref(),
            &r,
            pending,
            StoreIdentity::for_voter(&r, NodeId(1)).unwrap(),
        )
        .unwrap()
        .into_batch(),
    )
    .unwrap();
    assert_eq!(
        e.get(
            ColumnFamily::Default,
            &index_key(b.descriptor.subject, b.descriptor.id)
        )
        .unwrap()
        .unwrap(),
        before
    );
    assert!(collected(&e, &r, Some(b.descriptor.subject))
        .iter()
        .any(|g| g.owner.binding == b
            && g.owner.phase == PinPhase::Released
            && g.admission.is_some()));
    let (e, r, v, b) = setup();
    complete(&e, &r);
    let LedgerRequest::BeginBootstrapRead { pending, .. } = begin_request(&r, &v) else {
        panic!()
    };
    e.write(
        bootstrap_reader::plan_verified(
            e.snapshot().unwrap().as_ref(),
            &r,
            pending,
            StoreIdentity::for_voter(&r, NodeId(1)).unwrap(),
        )
        .unwrap()
        .into_batch(),
    )
    .unwrap();
    let page = collected(&e, &r, Some(b.descriptor.subject));
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].owner.phase, PinPhase::Released);
    assert!(page[0].admission.is_none());
    assert!(page[0].unentered.is_some());
    assert!(plan(&e, &r, &begin_request(&r, &v)).is_err());
}

#[test]
fn cursor_cas_retries_and_stable_views_cannot_skip_or_claim_unfinished_catalog() {
    let (e, r, _, _, _) = fixture(9);
    let first = request(&e, &r);
    let saved = e.snapshot().unwrap();
    assert_eq!(
        decode_request(&encode_request(r.digest(), &first).unwrap(), r.digest()).unwrap(),
        first
    );
    let mut bad = first.clone();
    let LedgerRequest::AdvanceBootstrapIndex { next, .. } = &mut bad else {
        panic!()
    };
    *next = None;
    let before = all(&e);
    assert!(plan(&e, &r, &bad).is_err());
    assert_eq!(all(&e), before);
    apply(&e, &r, first.clone());
    assert!(!plan(&e, &r, &first).unwrap().changed());
    apply(&e, &r, request(&e, &r));
    assert!(plan(&e, &r, &first).is_err());
    complete(&e, &r);
    assert!(catalog(saved.as_ref(), &r, None).is_err());
    let now = e.snapshot().unwrap();
    assert!(matches!(observe(now.as_ref(), &r).unwrap(), Work::Complete));
    assert!(query(
        now.as_ref(),
        &r,
        Some([5; 32]),
        Some(Cursor {
            image: [6; 32],
            owner: OwnerId::new([7; 16]).unwrap()
        })
    )
    .is_err());
}

#[test]
fn orphan_registration_and_orphan_admission_each_block_their_census_pass() {
    for admission in [false, true] {
        let (e, r, v, b) = setup();
        apply(&e, &r, begin_request(&r, &v));
        let mut batch = WriteBatch::new();
        batch.delete(ColumnFamily::Default, owner_key(b.descriptor.id));
        if admission {
            batch.delete(
                ColumnFamily::Default,
                owner_registration_key(b.descriptor.id),
            );
        }
        e.write(batch).unwrap();
        loop {
            let Work::Page { progress, next, .. } =
                observe(e.snapshot().unwrap().as_ref(), &r).unwrap()
            else {
                panic!("orphan was skipped")
            };
            let before = all(&e);
            match plan(&e, &r, &progress.request(next)) {
                Ok(p) => {
                    e.write(p.into_batch()).unwrap();
                }
                Err(_) => {
                    assert_eq!(progress.stage, if admission { 2 } else { 1 });
                    assert_eq!(all(&e), before);
                    break;
                }
            }
        }
    }
}

#[test]
fn corrupt_index_cursor_and_header_refuse_without_repairing_or_reclassifying_owners() {
    for damage in 0..5 {
        let (e, r, v, b) = setup();
        apply(&e, &r, begin_request(&r, &v));
        complete(&e, &r);
        let mut batch = WriteBatch::new();
        match damage {
            0 => {
                batch.delete(
                    ColumnFamily::Default,
                    index_key(b.descriptor.subject, b.descriptor.id),
                );
            }
            1 => {
                batch.put(
                    ColumnFamily::Default,
                    index_key(b.descriptor.subject, b.descriptor.id),
                    vec![0],
                );
            }
            2 => {
                batch.put(ColumnFamily::Default, key(30, &[]), vec![0]);
            }
            3 => {
                let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
                h.truncate(h.len() - 32);
                h[..8].copy_from_slice(BOOTSTRAP_ADMISSION_HEADER_MAGIC);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(h));
            }
            _ => {
                let mut orphan = b.clone();
                orphan.descriptor.id = OwnerId::new([91; 16]).unwrap();
                batch.put(
                    ColumnFamily::Default,
                    index_key(orphan.descriptor.subject, orphan.descriptor.id),
                    index(&orphan).unwrap(),
                );
            }
        }
        e.write(batch).unwrap();
        let before = all(&e);
        if damage == 0 {
            assert!(retention_owner(e.snapshot().unwrap().as_ref(), &r, b.descriptor.id).is_err());
            assert!(plan(&e, &r, &LedgerRequest::Publish(b.token())).is_err());
        } else {
            assert!(catalog(e.snapshot().unwrap().as_ref(), &r, None).is_err());
        }
        if damage == 3 {
            assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
        }
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
fn catalog_and_image_pages_have_fixed_work_for_13_and_113_legacy_grants() {
    let mut costs = Vec::new();
    for n in [13, 113] {
        let (e, r, v, _, _) = fixture(n);
        let view = e.snapshot().unwrap();
        let count = Count {
            view: view.as_ref(),
            rows: AtomicUsize::new(0),
            gets: AtomicUsize::new(0),
        };
        let Work::Page {
            progress,
            next,
            examined,
        } = observe(&count, &r).unwrap()
        else {
            panic!()
        };
        assert_eq!(examined, PAGE);
        assert_eq!(count.rows.swap(0, Ordering::Relaxed), PAGE + 1);
        plan_retention(&count, &r, &progress.request(next)).unwrap();
        assert_eq!(count.rows.load(Ordering::Relaxed), PAGE + 1);
        drop(view);
        complete(&e, &r);
        let view = e.snapshot().unwrap();
        let count = Count {
            view: view.as_ref(),
            rows: AtomicUsize::new(0),
            gets: AtomicUsize::new(0),
        };
        let page = grants(&count, &r, v.descriptor.subject, None).unwrap();
        assert_eq!(page.grants.len(), PAGE);
        assert!(page.next.is_some());
        assert!(page.grants.iter().all(|g| g.admission.is_none()));
        assert_eq!(count.rows.swap(0, Ordering::Relaxed), PAGE + 1);
        let image_cost = count.gets.swap(0, Ordering::Relaxed);
        let page = catalog(&count, &r, None).unwrap();
        assert_eq!(page.grants.len(), PAGE);
        assert_eq!(count.rows.load(Ordering::Relaxed), PAGE + 1);
        costs.push((image_cost, count.gets.load(Ordering::Relaxed)));
    }
    assert_eq!(
        costs[0], costs[1],
        "bounded query grew with catalog history"
    );
}

#[test]
fn each_census_stage_and_its_indexes_survive_actual_wal_tail_cuts_atomically() {
    for stage in 0..4 {
        let (memory, r, v, b, _) = fixture(9);
        apply(&memory, &r, begin_request(&r, &v));
        let LedgerRequest::BeginBootstrapRead { pending, .. } = begin_request(&r, &v) else {
            panic!()
        };
        let source = checkpoint_intent::checkpoint_intent(
            memory.snapshot().unwrap().as_ref(),
            &r,
            pending.owner,
        )
        .unwrap()
        .unwrap();
        let alias = CheckpointIntent::new(&r, &source.manifest().unwrap(), 1).unwrap();
        metadata_read_gate::tests::publish(&memory, &r, &alias);
        memory
            .write(
                bootstrap_reader::plan_verified(
                    memory.snapshot().unwrap().as_ref(),
                    &r,
                    alias.binding(OwnerKind::Pending).unwrap().token(),
                    StoreIdentity::for_voter(&r, NodeId(1)).unwrap(),
                )
                .unwrap()
                .into_batch(),
            )
            .unwrap();
        loop {
            let Work::Page { progress, .. } =
                observe(memory.snapshot().unwrap().as_ref(), &r).unwrap()
            else {
                panic!()
            };
            if progress.stage == stage {
                break;
            }
            apply(&memory, &r, request(&memory, &r));
        }
        let request = request(&memory, &r);
        let dir = std::env::temp_dir().join(format!(
            "kv9-bootstrap-index-{}",
            StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("meta.wal");
        let (e, _) = WalEngine::open(&path).unwrap();
        let (_, mut batch) = seed();
        for (k, v) in all(&memory) {
            batch.put(ColumnFamily::Default, k, v);
        }
        e.write(batch).unwrap();
        let size = std::fs::read(&path).unwrap().len();
        let next = plan_retention(e.snapshot().unwrap().as_ref(), &r, &request).unwrap();
        e.write(next.into_batch()).unwrap();
        drop(e);
        let bytes = std::fs::read(&path).unwrap();
        for cut in [
            size,
            size + 1,
            size + (bytes.len() - size) / 2,
            bytes.len() - 1,
            bytes.len(),
        ] {
            let p = dir.join(format!("cut-{cut}"));
            std::fs::write(&p, &bytes[..cut]).unwrap();
            let (e, _) = WalEngine::open(p).unwrap();
            let next = plan_retention(e.snapshot().unwrap().as_ref(), &r, &request).unwrap();
            assert_eq!(next.changed(), cut != bytes.len());
            e.write(next.into_batch()).unwrap();
            assert!(
                !plan_retention(e.snapshot().unwrap().as_ref(), &r, &request)
                    .unwrap()
                    .changed()
            );
            loop {
                match observe(e.snapshot().unwrap().as_ref(), &r).unwrap() {
                    Work::Complete => break,
                    Work::Page { progress, next, .. } => {
                        let plan = plan_retention(
                            e.snapshot().unwrap().as_ref(),
                            &r,
                            &progress.request(next),
                        )
                        .unwrap();
                        e.write(plan.into_batch()).unwrap();
                    }
                    _ => panic!(),
                }
            }
            let grants = collected(&e, &r, None);
            assert_eq!(grants.len(), 11);
            assert_eq!(grants.iter().filter(|g| g.unentered.is_some()).count(), 1);
            assert!(grants
                .iter()
                .any(|g| g.owner.binding == b && g.admission.is_some()));
            assert_eq!(owner(&e, &r, v.descriptor.id).phase, PinPhase::Published);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn fourth_census_pass_has_fixed_work_for_13_and_113_unentered_grants() {
    let mut costs = Vec::new();
    for n in [13, 113] {
        let (e, r, v, _) = setup();
        let LedgerRequest::BeginBootstrapRead { pending, .. } = begin_request(&r, &v) else {
            panic!()
        };
        let source =
            checkpoint_intent::checkpoint_intent(e.snapshot().unwrap().as_ref(), &r, pending.owner)
                .unwrap()
                .unwrap();
        for predecessor in 1..=n {
            let i = CheckpointIntent::new(&r, &source.manifest().unwrap(), predecessor).unwrap();
            metadata_read_gate::tests::publish(&e, &r, &i);
            e.write(
                bootstrap_reader::plan_verified(
                    e.snapshot().unwrap().as_ref(),
                    &r,
                    i.binding(OwnerKind::Pending).unwrap().token(),
                    StoreIdentity::for_voter(&r, NodeId(1)).unwrap(),
                )
                .unwrap()
                .into_batch(),
            )
            .unwrap();
        }
        loop {
            let Work::Page { progress, next, .. } =
                observe(e.snapshot().unwrap().as_ref(), &r).unwrap()
            else {
                panic!()
            };
            if progress.stage == 3 {
                break;
            }
            apply(&e, &r, progress.request(next));
        }
        let view = e.snapshot().unwrap();
        let count = Count {
            view: view.as_ref(),
            rows: AtomicUsize::new(0),
            gets: AtomicUsize::new(0),
        };
        let Work::Page {
            progress,
            next,
            examined,
        } = observe(&count, &r).unwrap()
        else {
            panic!()
        };
        assert_eq!((progress.stage, examined), (3, PAGE));
        assert_eq!(count.rows.swap(0, Ordering::Relaxed), PAGE + 1);
        let plan = plan_retention(&count, &r, &progress.request(next)).unwrap();
        assert_eq!(count.rows.load(Ordering::Relaxed), PAGE + 1);
        costs.push(count.gets.load(Ordering::Relaxed));
        e.write(plan.into_batch()).unwrap();
    }
    assert_eq!(costs[0], costs[1]);
}
