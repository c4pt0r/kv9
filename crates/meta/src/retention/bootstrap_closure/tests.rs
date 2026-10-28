use super::*;
use crate::retention::metadata_read_gate::tests::{fixture, grant, publish, select, step, token};
use crate::retention::tests::{all, apply, owner, plan, seed_with_voters};
use kv9_common::{NodeId, StoreIdentity, StoreIncarnation};
use kv9_engine::{Engine, MemEngine, WalEngine};

fn catalog(e: &MemEngine, r: &RootDescriptor) {
    loop {
        match bootstrap_index::observe(e.snapshot().unwrap().as_ref(), r).unwrap() {
            bootstrap_index::Work::Complete => break,
            bootstrap_index::Work::Upgrade => {
                apply(e, r, LedgerRequest::Initialize);
            }
            bootstrap_index::Work::Page { progress, next, .. } => {
                apply(e, r, progress.request(next));
            }
            _ => panic!(),
        }
    }
}
fn close_gate(e: &MemEngine, r: &RootDescriptor, old: &CheckpointIntent, new: &CheckpointIntent) {
    for voter in &r.voters {
        select(e, r, new, voter.node_id.0);
    }
    loop {
        match metadata_read_gate::observe(e.snapshot().unwrap().as_ref(), r, old.pending_id())
            .unwrap()
        {
            metadata_read_gate::Work::Closed(_) => break,
            metadata_read_gate::Work::Advance(_) => {
                step(e, r, old);
            }
            _ => panic!(),
        }
    }
}
fn retire(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent, n: u64) {
    let id = StoreIdentity::for_voter(r, NodeId(n)).unwrap();
    e.write(
        bootstrap_reader::plan_verified(e.snapshot().unwrap().as_ref(), r, token(i), id)
            .unwrap()
            .into_batch(),
    )
    .unwrap();
}
fn observed(e: &impl Engine, r: &RootDescriptor, i: &CheckpointIntent) -> Work {
    observe(e.snapshot().unwrap().as_ref(), r, i.pending_id()).unwrap()
}
fn complete(e: &MemEngine, r: &RootDescriptor, i: &CheckpointIntent) -> Cursor {
    for _ in 0..1000 {
        match observed(e, r, i) {
            Work::Complete(c) => return c,
            Work::Advance { cursor, .. } => {
                apply(e, r, cursor.request(i.pending_id()));
            }
            w => panic!("unexpected {w:?}"),
        }
    }
    panic!("closure failed to complete")
}

#[test]
fn exact_typed_grants_remain_active_until_independent_retirement_and_late_change_restarts() {
    let (e, r, old, new) = fixture(5);
    for n in 1..=5 {
        grant(&e, &r, &old, n);
    }
    assert!(matches!(observed(&e, &r, &old), Work::Open));
    close_gate(&e, &r, &old, &new);
    assert!(matches!(observed(&e, &r, &old), Work::WaitingForCatalog));
    catalog(&e, &r);
    let first = complete(&e, &r, &old);
    assert_eq!((first.counts.grants, first.counts.active), (5, 5));
    assert!(!observed(&e, &r, &old).registered_grants_retired());
    let old_view = e.snapshot().unwrap();
    retire(&e, &r, &old, 1);
    assert!(matches!(
        observed(&e, &r, &old),
        Work::Advance { restart: true, .. }
    ));
    let changed = all(&e);
    // A lost-page acknowledgement never hides a later grant mutation.
    apply(
        &e,
        &r,
        LedgerRequest::AdvanceBootstrapClosure {
            pending: token(&old),
            expected: first.sequence - 1,
        },
    );
    assert_eq!(all(&e), changed);
    assert!(matches!(
        observed(&e, &r, &old),
        Work::Advance { restart: true, .. }
    ));
    assert!(matches!(
        observe(old_view.as_ref(), &r, old.pending_id()).unwrap(),
        Work::Complete(_)
    ));
    for n in 2..=5 {
        retire(&e, &r, &old, n);
    }
    let final_state = complete(&e, &r, &old);
    assert_eq!(
        (final_state.counts.retired, final_state.counts.active),
        (5, 0)
    );
    assert!(observed(&e, &r, &old).registered_grants_retired());
    assert_eq!(
        owner(
            &e,
            &r,
            old.binding(OwnerKind::Version).unwrap().descriptor.id
        )
        .phase,
        PinPhase::Published
    );
    let snapshot = all(&e);
    retire(&e, &r, &old, 1);
    assert_eq!(all(&e), snapshot);
}

#[test]
fn same_image_alias_can_enter_after_target_gate_and_invalidates_every_existing_image_census() {
    let (e, r, old, new) = fixture(1);
    grant(&e, &r, &old, 1);
    close_gate(&e, &r, &old, &new);
    retire(&e, &r, &old, 1);
    catalog(&e, &r);
    complete(&e, &r, &old);
    assert!(observed(&e, &r, &old).registered_grants_retired());
    let alias = CheckpointIntent::new(&r, &old.manifest().unwrap(), 2).unwrap();
    publish(&e, &r, &alias);
    let other = grant(&e, &r, &alias, 1);
    assert!(matches!(
        observed(&e, &r, &old),
        Work::Advance { restart: true, .. }
    ));
    let blocked = complete(&e, &r, &old);
    assert_eq!(
        (
            blocked.counts.grants,
            blocked.counts.retired,
            blocked.counts.active,
            blocked.counts.other_sources
        ),
        (2, 1, 1, 1)
    );
    retire(&e, &r, &alias, 1);
    assert_eq!(owner(&e, &r, other.descriptor.id).phase, PinPhase::Released);
    // A retired alias does not regain a dependency on its still-Published
    // source; its retained canonical intent and admission remain exact.
    apply(
        &e,
        &r,
        LedgerRequest::QuiesceAfterTransfer {
            from: alias.binding(OwnerKind::Version).unwrap().token(),
            to: old.binding(OwnerKind::Version).unwrap().token(),
        },
    );
    apply(
        &e,
        &r,
        LedgerRequest::Release(alias.binding(OwnerKind::Version).unwrap().token()),
    );
    let done = complete(&e, &r, &old);
    assert_eq!((done.counts.retired, done.counts.other_sources), (2, 1));
    assert!(observed(&e, &r, &old).registered_grants_retired());
}

#[test]
fn legacy_released_and_unentered_without_origin_are_explicit_unknowns_not_retirement_proofs() {
    let (e, r, old, new) = fixture(2);
    // Explicit pre-v17 terminal: no unentered provenance existed in that format.
    retire(&e, &r, &old, 1);
    let id = bootstrap_reader::binding(
        &r,
        &old.manifest().unwrap(),
        old.predecessor(),
        StoreIdentity::for_voter(&r, NodeId(1)).unwrap(),
    )
    .unwrap();
    let terminal = owner(&e, &r, id.descriptor.id);
    let mut historical = WriteBatch::new();
    historical.delete(ColumnFamily::Default, key(35, id.descriptor.id.as_bytes()));
    historical.put(
        ColumnFamily::Default,
        owner_key(id.descriptor.id),
        encode_owner(&terminal),
    );
    historical.put(
        ColumnFamily::Default,
        owner_registration_key(id.descriptor.id),
        owner_registration(&id),
    );
    e.write(historical).unwrap();
    let mut legacy = bootstrap_reader::binding(
        &r,
        &old.manifest().unwrap(),
        old.predecessor(),
        StoreIdentity::for_voter(&r, NodeId(2)).unwrap(),
    )
    .unwrap();
    legacy.descriptor.kind = OwnerKind::Legacy;
    legacy.descriptor.id = OwnerId::new([77; 16]).unwrap();
    legacy.descriptor.operation = [88; 32];
    apply(&e, &r, LedgerRequest::Acquire(legacy.clone()));
    apply(&e, &r, LedgerRequest::Publish(legacy.token()));
    apply(
        &e,
        &r,
        LedgerRequest::QuiesceAfterTransfer {
            from: legacy.token(),
            to: old.binding(OwnerKind::Version).unwrap().token(),
        },
    );
    apply(&e, &r, LedgerRequest::Release(legacy.token()));
    let mut historical = owner(&e, &r, legacy.descriptor.id);
    historical.binding.descriptor.kind = OwnerKind::BootstrapReader;
    let mut b = WriteBatch::new();
    b.put(
        ColumnFamily::Default,
        owner_key(legacy.descriptor.id),
        encode_owner(&historical),
    );
    b.put(
        ColumnFamily::Default,
        owner_registration_key(legacy.descriptor.id),
        owner_registration(&historical.binding),
    );
    e.write(b).unwrap();
    close_gate(&e, &r, &old, &new);
    catalog(&e, &r);
    let result = complete(&e, &r, &old);
    assert_eq!(
        (
            result.counts.grants,
            result.counts.legacy,
            result.counts.retired
        ),
        (2, 2, 0)
    );
    assert!(!observed(&e, &r, &old).registered_grants_retired());
}

#[test]
fn request_upgrade_and_damaged_stamp_or_cursor_cannot_skip_census_or_repair_history() {
    let (e, r, old, new) = fixture(1);
    grant(&e, &r, &old, 1);
    close_gate(&e, &r, &old, &new);
    catalog(&e, &r);
    let request = Cursor::default().request(old.pending_id());
    let encoded = encode_request(r.digest(), &request).unwrap();
    assert_eq!(decode_request(&encoded, r.digest()).unwrap(), request);
    for cut in 0..encoded.len() {
        assert!(decode_request(&encoded[..cut], r.digest()).is_err());
    }
    assert!(encode_request(
        r.digest(),
        &LedgerRequest::AdvanceBootstrapClosure {
            pending: OwnerToken {
                owner: old.pending_id(),
                generation: NonZeroU64::new(2).unwrap()
            },
            expected: 0,
        }
    )
    .is_err());
    // Merely relabelling a new store as v15 must not permit mixed records.
    let header = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    let mut older = header[..header.len() - 32].to_vec();
    older[..8].copy_from_slice(METADATA_READ_GATE_HEADER_MAGIC);
    let mut downgrade = WriteBatch::new();
    downgrade.put(ColumnFamily::Default, key(1, &[]), seal(older));
    e.write(downgrade).unwrap();
    let before = all(&e);
    assert!(plan(&e, &r, &LedgerRequest::Initialize).is_err());
    assert_eq!(all(&e), before);
    let mut restore = WriteBatch::new();
    restore.put(ColumnFamily::Default, key(1, &[]), header);
    e.write(restore).unwrap();
    assert!(plan(
        &e,
        &r,
        &LedgerRequest::AdvanceBootstrapClosure {
            pending: token(&old),
            expected: 1
        }
    )
    .is_err());
    // Explicit V15 fixture: no bootstrap census/stamp existed in that format.
    let mut b = WriteBatch::new();
    b.delete(ColumnFamily::Default, stamp_key(image(&old)));
    b.delete(ColumnFamily::Default, stamp_key(image(&new)));
    // The V15 catalog had completed its original three-pass index.
    let mut index = e
        .get(ColumnFamily::Default, &key(30, &[]))
        .unwrap()
        .unwrap();
    index.truncate(index.len() - 32);
    index[..8].copy_from_slice(b"KV9BRP01");
    index[48] = 3;
    index[49] = 2;
    b.put(ColumnFamily::Default, key(30, &[]), seal(index));
    let mut h = e.get(ColumnFamily::Default, &key(1, &[])).unwrap().unwrap();
    h.truncate(h.len() - 32);
    h[..8].copy_from_slice(METADATA_READ_GATE_HEADER_MAGIC);
    b.put(ColumnFamily::Default, key(1, &[]), seal(h));
    e.write(b).unwrap();
    assert!(matches!(observed(&e, &r, &old), Work::WaitingForCatalog));
    let gate_before = e
        .get(ColumnFamily::Default, &key(32, old.pending_id().as_bytes()))
        .unwrap();
    apply(&e, &r, LedgerRequest::Initialize);
    assert_eq!(
        e.get(ColumnFamily::Default, &key(32, old.pending_id().as_bytes()))
            .unwrap(),
        gate_before
    );
    catalog(&e, &r);
    complete(&e, &r, &old);
    let saved_stamp = e
        .get(ColumnFamily::Default, &stamp_key(image(&old)))
        .unwrap()
        .unwrap();
    assert_eq!(saved_stamp.len(), 112);
    let saved_cursor = e
        .get(
            ColumnFamily::Default,
            &progress_key(image(&old), old.pending_id()),
        )
        .unwrap()
        .unwrap();
    assert!(saved_cursor.len() <= MAX_BYTES);
    for damage in 0..4 {
        let mut b = WriteBatch::new();
        b.put(
            ColumnFamily::Default,
            stamp_key(image(&old)),
            saved_stamp.clone(),
        );
        b.put(
            ColumnFamily::Default,
            progress_key(image(&old), old.pending_id()),
            saved_cursor.clone(),
        );
        if damage == 0 {
            b.delete(ColumnFamily::Default, stamp_key(image(&old)));
        } else if damage == 1 {
            let mut bytes = saved_stamp[..saved_stamp.len() - 32].to_vec();
            bytes[72..80].copy_from_slice(&1u64.to_be_bytes());
            b.put(ColumnFamily::Default, stamp_key(image(&old)), seal(bytes));
        } else {
            let mut bytes = saved_cursor[..saved_cursor.len() - 32].to_vec();
            if damage == 2 {
                bytes[104..112].copy_from_slice(&u64::MAX.to_be_bytes());
            } else {
                bytes[72] ^= 1;
            }
            b.put(
                ColumnFamily::Default,
                progress_key(image(&old), old.pending_id()),
                seal(bytes),
            );
        }
        e.write(b).unwrap();
        let before = all(&e);
        assert!(observe(e.snapshot().unwrap().as_ref(), &r, old.pending_id()).is_err());
        assert!(plan(&e, &r, &request).is_err());
        assert_eq!(all(&e), before);
        if damage == 0 {
            let id = StoreIdentity::for_voter(&r, NodeId(1)).unwrap();
            assert!(bootstrap_reader::plan_verified(
                e.snapshot().unwrap().as_ref(),
                &r,
                token(&old),
                id
            )
            .is_err());
        }
    }
}

struct Count<'a> {
    view: &'a dyn ReadView,
    rows: std::sync::atomic::AtomicUsize,
    gets: std::sync::atomic::AtomicUsize,
}
impl ReadView for Count<'_> {
    fn get(&self, cf: ColumnFamily, k: &[u8]) -> Result<Option<Vec<u8>>> {
        if k.starts_with(PREFIX) {
            self.gets.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        self.view.get(cf, k)
    }
    fn scan(
        &self,
        cf: ColumnFamily,
        a: &[u8],
        b: &[u8],
        n: usize,
    ) -> Result<Vec<kv9_engine::ScanEntry>> {
        assert!(!a.starts_with(PREFIX));
        self.view.scan(cf, a, b, n)
    }
    fn seek_le(&self, cf: ColumnFamily, k: &[u8]) -> Result<Option<kv9_engine::ScanEntry>> {
        assert!(!k.starts_with(PREFIX));
        self.view.seek_le(cf, k)
    }
    fn iter<'a>(
        &'a self,
        cf: ColumnFamily,
        a: &[u8],
        b: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
        let retention = a.starts_with(PREFIX);
        if retention {
            assert!(a.starts_with(&key(29, &[])) || a.starts_with(&key(34, &[])));
        }
        Ok(Box::new(self.view.iter(cf, a, b)?.inspect(move |_| {
            if retention {
                self.rows.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })))
    }
    fn iter_rev<'a>(
        &'a self,
        cf: ColumnFamily,
        a: &[u8],
        b: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<kv9_engine::ScanEntry>> + 'a>> {
        assert!(!a.starts_with(PREFIX));
        self.view.iter_rev(cf, a, b)
    }
}
#[test]
fn exact_image_page_work_is_constant_for_13_and_113_grants_across_distinct_source_attempts() {
    let mut costs = Vec::new();
    for n in [13, 113] {
        let (e, r, old, new) = fixture(1);
        close_gate(&e, &r, &old, &new);
        for predecessor in 2..n + 2 {
            let alias = CheckpointIntent::new(&r, &old.manifest().unwrap(), predecessor).unwrap();
            publish(&e, &r, &alias);
            grant(&e, &r, &alias, 1);
        }
        catalog(&e, &r);
        let view = e.snapshot().unwrap();
        let count = Count {
            view: view.as_ref(),
            rows: Default::default(),
            gets: Default::default(),
        };
        let next =
            plan_retention(&count, &r, &Cursor::default().request(old.pending_id())).unwrap();
        assert_eq!(
            count.rows.load(std::sync::atomic::Ordering::Relaxed),
            bootstrap_index::PAGE + 1
        );
        costs.push(count.gets.load(std::sync::atomic::Ordering::Relaxed));
        e.write(next.into_batch()).unwrap();
        let Work::Advance { cursor, .. } = observed(&e, &r, &old) else {
            panic!()
        };
        assert_eq!(cursor.counts.grants, 4);
        assert_eq!(cursor.counts.other_sources, 4);
    }
    assert_eq!(costs[0], costs[1]);
}

#[test]
fn actual_wal_tail_cuts_keep_census_and_late_retirement_invalidation_atomic() {
    let (memory, r, old, new) = fixture(5);
    for n in 1..=5 {
        grant(&memory, &r, &old, n);
    }
    close_gate(&memory, &r, &old, &new);
    catalog(&memory, &r);
    for mutation in 0..2 {
        if mutation == 1 {
            complete(&memory, &r, &old);
        }
        let dir = std::env::temp_dir().join(format!(
            "kv9-bootstrap-closure-{}",
            StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("base");
        let (e, _) = WalEngine::open(&path).unwrap();
        let (_, mut batch) = seed_with_voters(r.voters.clone());
        for (k, v) in all(&memory) {
            batch.put(ColumnFamily::Default, k, v);
        }
        e.write(batch).unwrap();
        let size = std::fs::read(&path).unwrap().len();
        let apply_next = |e: &WalEngine| {
            if mutation == 0 {
                plan(e, &r, &Cursor::default().request(old.pending_id()))
            } else {
                bootstrap_reader::plan_verified(
                    e.snapshot().unwrap().as_ref(),
                    &r,
                    token(&old),
                    StoreIdentity::for_voter(&r, NodeId(1)).unwrap(),
                )
            }
        };
        e.write(apply_next(&e).unwrap().into_batch()).unwrap();
        drop(e);
        let bytes = std::fs::read(&path).unwrap();
        for cut in [
            size,
            size + 1,
            size + (bytes.len() - size) / 2,
            bytes.len() - 1,
            bytes.len(),
        ] {
            let path = dir.join(format!("cut-{cut}"));
            std::fs::write(&path, &bytes[..cut]).unwrap();
            let (e, _) = WalEngine::open(path).unwrap();
            if mutation == 1 {
                assert_eq!(
                    matches!(observed(&e, &r, &old), Work::Advance { restart: true, .. }),
                    cut == bytes.len()
                );
            }
            let next = apply_next(&e).unwrap();
            assert_eq!(next.changed(), cut != bytes.len());
            e.write(next.into_batch()).unwrap();
            assert!(!apply_next(&e).unwrap().changed());
            assert_eq!(
                owner(
                    &e,
                    &r,
                    old.binding(OwnerKind::Version).unwrap().descriptor.id
                )
                .phase,
                PinPhase::Published
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
