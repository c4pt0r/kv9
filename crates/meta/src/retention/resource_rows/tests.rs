use super::*;
use crate::retention::tests::{all, apply, binding, fixture, initialized, owner, plan, seed};
use kv9_common::retention::{PinCommand, MAX_PIN_OWNERS};
use kv9_common::{AppliedPosition, Value};
use kv9_engine::{
    DurableAppliedPosition, Engine, MemEngine, Mutation, ReplicatedEngine, ScanEntry, WalEngine,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "kv9-owner-rows-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn numbered(root: &RootDescriptor, n: u128) -> OwnerBinding {
    let mut a = binding(root, 1);
    a.descriptor.id = OwnerId::new(n.to_be_bytes()).unwrap();
    a.descriptor.operation[..16].copy_from_slice(&n.to_be_bytes());
    a
}
fn batch_bytes(batch: &WriteBatch) -> usize {
    batch
        .mutations()
        .iter()
        .map(|m| match m {
            Mutation::Put { key, value, .. } => key.len() + value.len() + 16,
            Mutation::Delete { .. } => panic!("no pin deletion allowed"),
        })
        .sum()
}
struct Count<'a> {
    inner: &'a dyn ReadView,
    calls: AtomicUsize,
    bytes: AtomicUsize,
}
impl Count<'_> {
    fn new(inner: &dyn ReadView) -> Count<'_> {
        Count {
            inner,
            calls: AtomicUsize::new(0),
            bytes: AtomicUsize::new(0),
        }
    }
    fn totals(&self) -> (usize, usize) {
        (
            self.calls.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed),
        )
    }
}
impl ReadView for Count<'_> {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>> {
        let result = self.inner.get(cf, key)?;
        if key.starts_with(PREFIX) {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.bytes
                .fetch_add(result.as_ref().map_or(0, Vec::len), Ordering::Relaxed);
        }
        Ok(result)
    }
    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<ScanEntry>> {
        assert!(
            !start.starts_with(PREFIX),
            "history scan in steady-state operation"
        );
        self.inner.scan(cf, start, end, limit)
    }
    fn seek_le(&self, cf: ColumnFamily, target: &[u8]) -> Result<Option<ScanEntry>> {
        assert!(!target.starts_with(PREFIX));
        self.inner.seek_le(cf, target)
    }
    fn iter<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<ScanEntry>> + 'a>> {
        assert!(
            !start.starts_with(PREFIX),
            "history scan in steady-state operation"
        );
        self.inner.iter(cf, start, end)
    }
    fn iter_rev<'a>(
        &'a self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Box<dyn Iterator<Item = Result<ScanEntry>> + 'a>> {
        assert!(!start.starts_with(PREFIX));
        self.inner.iter_rev(cf, start, end)
    }
}

#[test]
fn released_history_exceeds_4096_with_constant_point_work_and_survives_wal_reopen() {
    let temp = TempDir::new();
    let path = temp.path().join("owner-rows.wal");
    let (root, seed) = seed();
    let (engine, _) = WalEngine::open(&path).unwrap();
    engine
        .write_applied(seed, AppliedPosition { term: 1, index: 1 })
        .unwrap();
    let mut index = 1;
    let keeper = numbered(&root, u128::MAX);
    {
        let mut commit = |request: LedgerRequest| {
            let p = plan(&engine, &root, &request).unwrap();
            index += 1;
            engine
                .write_applied(p.into_batch(), AppliedPosition { term: 1, index })
                .unwrap();
        };
        commit(LedgerRequest::Initialize);
        commit(LedgerRequest::Register(keeper.resources.clone()));
        commit(LedgerRequest::Acquire(keeper.clone()));
        commit(LedgerRequest::Publish(keeper.token()));
    }
    let mut first = None;
    for n in 1..=MAX_PIN_OWNERS + 1 {
        let a = numbered(&root, n as u128);
        for request in [
            LedgerRequest::Share {
                from: keeper.token(),
                to: a.clone(),
            },
            LedgerRequest::QuiesceAfterTransfer {
                from: a.token(),
                to: keeper.token(),
            },
            LedgerRequest::Release(a.token()),
        ] {
            let snapshot = engine.snapshot().unwrap();
            let count = Count::new(snapshot.as_ref());
            let p = plan_retention(&count, &root, &request).unwrap();
            let batch = p.into_batch();
            if matches!(request, LedgerRequest::Share { .. }) {
                let work = (count.totals(), batch.len(), batch_bytes(&batch));
                if let Some(first) = first {
                    assert_eq!(work, first, "owner {n} grew history work");
                } else {
                    first = Some(work);
                }
            }
            index += 1;
            engine
                .write_applied(batch, AppliedPosition { term: 1, index })
                .unwrap();
        }
    }
    drop(engine);
    let (engine, _) = WalEngine::open(&path).unwrap();
    assert_eq!(
        engine.applied_position().unwrap(),
        DurableAppliedPosition::AppliedThrough(AppliedPosition { term: 1, index })
    );
    for n in 1..=MAX_PIN_OWNERS + 1 {
        let a = numbered(&root, n as u128);
        let observed = owner(&engine, &root, a.descriptor.id);
        assert_eq!(observed.binding, a);
        assert_eq!(observed.phase, PinPhase::Released);
    }
    let old = numbered(&root, 1);
    assert!(plan(&engine, &root, &LedgerRequest::Acquire(old.clone())).is_err());
    let mut next = old.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    let p = plan(
        &engine,
        &root,
        &LedgerRequest::Share {
            from: keeper.token(),
            to: next.clone(),
        },
    )
    .unwrap();
    engine
        .write_applied(
            p.into_batch(),
            AppliedPosition {
                term: 2,
                index: index + 1,
            },
        )
        .unwrap();
    assert!(plan(&engine, &root, &LedgerRequest::Release(old.token())).is_err());
    assert_eq!(owner(&engine, &root, old.descriptor.id).binding, next);
    assert_eq!(
        owner(&engine, &root, keeper.descriptor.id).phase,
        PinPhase::Published
    );
}

/// Construct the actual legacy codec and immutable owner registrations, not
/// modern rows with a renamed header. All generation-1 owners are Released.
fn legacy_fixture(count: usize) -> (MemEngine, RootDescriptor) {
    let (engine, root) = fixture();
    let rs = binding(&root, 1).resources;
    let mut records: Vec<_> = rs.iter().copied().map(RetentionRecord::new).collect();
    let mut batch = WriteBatch::new();
    for n in 1..=count {
        let a = numbered(&root, n as u128);
        for record in &mut records {
            for command in [
                PinCommand::Acquire(a.token()),
                PinCommand::Quiesce(a.token()),
                PinCommand::Release(a.token()),
            ] {
                record.apply(command).unwrap();
            }
        }
        batch.put(
            ColumnFamily::Default,
            owner_registration_key(a.descriptor.id),
            owner_registration(&a),
        );
        batch.put(
            ColumnFamily::Default,
            owner_key(a.descriptor.id),
            encode_owner(&LedgerOwner {
                binding: a,
                phase: PinPhase::Released,
            }),
        );
    }
    for record in records {
        batch.put(
            ColumnFamily::Default,
            resource_key(&record.identity()),
            record.encode(),
        );
        batch.put(
            ColumnFamily::Default,
            resource_registration_key(&record.identity()),
            resource_registration(&record.identity()),
        );
    }
    let mut old_header = header(root.digest(), (count * 3) as u64);
    old_header.truncate(old_header.len() - 32);
    old_header[..8].copy_from_slice(LEGACY_HEADER_MAGIC);
    batch.put(ColumnFamily::Default, key(1, &[]), seal(old_header));
    engine.write(batch).unwrap();
    (engine, root)
}

#[test]
fn full_legacy_base_upgrades_without_evicting_tombstones_or_resetting_generation() {
    let (engine, root) = legacy_fixture(MAX_PIN_OWNERS);
    let a = numbered(&root, 1);
    let original: Vec<_> = a
        .resources
        .iter()
        .map(|r| {
            engine
                .get(ColumnFamily::Default, &resource_key(r))
                .unwrap()
                .unwrap()
        })
        .collect();
    let before = owner(&engine, &root, a.descriptor.id);
    // Acquisition alone can upgrade a saturated legacy resource atomically.
    let newcomer = numbered(&root, (MAX_PIN_OWNERS + 1) as u128);
    apply(&engine, &root, LedgerRequest::Acquire(newcomer.clone()));
    assert_eq!(owner(&engine, &root, a.descriptor.id), before);
    assert_eq!(
        owner(&engine, &root, newcomer.descriptor.id).phase,
        PinPhase::Held
    );
    let bytes = engine
        .get(ColumnFamily::Default, &key(1, &[]))
        .unwrap()
        .unwrap();
    assert!(
        checked(&bytes, LEGACY_HEADER_MAGIC, 81).is_err(),
        "old ledger reader must refuse"
    );
    assert!(read_header(&bytes, root.digest()).unwrap().1);
    let mut next = a.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    apply(&engine, &root, LedgerRequest::Acquire(next.clone()));
    assert!(plan(&engine, &root, &LedgerRequest::Release(a.token())).is_err());
    for (resource, original) in a.resources.iter().zip(original) {
        assert_eq!(
            engine
                .get(ColumnFamily::Default, &base_key(resource))
                .unwrap()
                .unwrap(),
            original
        );
    }
    assert_eq!(owner(&engine, &root, a.descriptor.id).binding, next);
    let snapshot = engine.snapshot().unwrap();
    let bytes = engine
        .get(ColumnFamily::Default, &resource_key(&a.resources[0]))
        .unwrap()
        .unwrap();
    let record = ResourceRecord::load(snapshot.as_ref(), a.resources[0], &bytes, true).unwrap();
    for n in 2..=MAX_PIN_OWNERS {
        // The immutable base still owns every unchanged generation fence.
        let id = numbered(&root, n as u128).descriptor.id;
        assert_eq!(
            record.owner(snapshot.as_ref(), id).unwrap().unwrap().phase,
            PinPhase::Released
        );
    }
}

#[test]
fn maximum_legacy_closure_normalization_fits_existing_atomic_batch_bound() {
    let (engine, root) = fixture();
    apply(&engine, &root, LedgerRequest::Initialize);
    let mut rs = Vec::new();
    let mut batch = WriteBatch::new();
    for n in 1..=MAX_OWNER_RESOURCES {
        let r = ResourceIdentity::new(root.digest(), ResourceKind::Sst, [n as u8; 16], [9; 32])
            .unwrap();
        let mut record = RetentionRecord::new(r);
        for owner in 1..=MAX_PIN_OWNERS {
            record
                .apply(PinCommand::Acquire(numbered(&root, owner as u128).token()))
                .unwrap();
        }
        batch.put(ColumnFamily::Default, resource_key(&r), record.encode());
        batch.put(
            ColumnFamily::Default,
            resource_registration_key(&r),
            resource_registration(&r),
        );
        rs.push(r);
    }
    engine.write(batch).unwrap();
    // This isolates the migration byte bound; no claim of a full owner backfill
    // is made from these synthetic forward pins without owner descriptors.
    let p = plan(&engine, &root, &LedgerRequest::Register(rs.clone())).unwrap();
    let batch = p.into_batch();
    assert_eq!(batch.len(), MAX_OWNER_RESOURCES * 2 + 1);
    assert!(batch_bytes(&batch) < MAX_LEDGER_BATCH_BYTES);
    assert!(batch_bytes(&batch) > 6 * 1024 * 1024);
    engine.write(batch).unwrap();
    assert!(!plan(&engine, &root, &LedgerRequest::Register(rs))
        .unwrap()
        .changed());
}

#[test]
fn missing_or_wrong_rows_bases_and_format_markers_refuse_without_repair() {
    for damage in 0..8 {
        let (engine, root) = legacy_fixture(2);
        let mut a = numbered(&root, 1);
        a.generation = NonZeroU64::new(2).unwrap();
        apply(&engine, &root, LedgerRequest::Acquire(a.clone()));
        let r = a.resources[0];
        let mut batch = WriteBatch::new();
        match damage {
            0 => {
                batch.delete(ColumnFamily::Default, pin_key(&r, a.descriptor.id));
            }
            1 => {
                batch.delete(ColumnFamily::Default, base_key(&r));
            }
            2 => {
                let wrong = RetentionRecord::new(r).encode();
                batch.put(ColumnFamily::Default, base_key(&r), wrong);
            }
            3 => {
                let mut bytes = engine
                    .get(ColumnFamily::Default, &pin_key(&r, a.descriptor.id))
                    .unwrap()
                    .unwrap();
                bytes[90] ^= 1;
                batch.put(ColumnFamily::Default, pin_key(&r, a.descriptor.id), bytes);
            }
            4 => {
                let bytes = engine
                    .get(
                        ColumnFamily::Default,
                        &pin_key(&a.resources[1], a.descriptor.id),
                    )
                    .unwrap()
                    .unwrap();
                batch.put(ColumnFamily::Default, pin_key(&r, a.descriptor.id), bytes);
            }
            5 => {
                let old = engine
                    .get(ColumnFamily::Default, &base_key(&r))
                    .unwrap()
                    .unwrap();
                batch.put(ColumnFamily::Default, resource_key(&r), old);
            }
            6 => {
                let mut bytes = header(root.digest(), 9);
                bytes.truncate(bytes.len() - 32);
                bytes[..8].copy_from_slice(LEGACY_HEADER_MAGIC);
                batch.put(ColumnFamily::Default, key(1, &[]), seal(bytes));
            }
            _ => {
                let mut bytes = engine
                    .get(ColumnFamily::Default, &pin_key(&r, a.descriptor.id))
                    .unwrap()
                    .unwrap();
                bytes.truncate(bytes.len() - 32);
                let end = bytes.len();
                bytes[end - 8..].copy_from_slice(&u64::MAX.to_be_bytes());
                batch.put(
                    ColumnFamily::Default,
                    pin_key(&r, a.descriptor.id),
                    seal(bytes),
                );
            }
        }
        engine.write(batch).unwrap();
        let before = all(&engine);
        assert!(
            retention_owner(engine.snapshot().unwrap().as_ref(), &root, a.descriptor.id).is_err(),
            "damage {damage}"
        );
        assert!(plan(&engine, &root, &LedgerRequest::Publish(a.token())).is_err());
        assert_eq!(all(&engine), before);
    }
    let (engine, root) = fixture();
    initialized(&engine, &root);
    let a = numbered(&root, 1);
    apply(&engine, &root, LedgerRequest::Acquire(a.clone()));
    let mut damage = WriteBatch::new();
    damage.delete(ColumnFamily::Default, resource_key(&a.resources[0]));
    damage.delete(
        ColumnFamily::Default,
        resource_registration_key(&a.resources[0]),
    );
    engine.write(damage).unwrap();
    assert!(
        plan(&engine, &root, &LedgerRequest::Register(a.resources)).is_err(),
        "orphan pins cannot be registered away"
    );
}

#[test]
fn wal_tail_cuts_keep_legacy_upgrade_and_acquisition_in_one_transaction() {
    let (legacy, root) = legacy_fixture(2);
    let temp = TempDir::new();
    let path = temp.path().join("source.wal");
    let (engine, _) = WalEngine::open(&path).unwrap();
    let mut seed = WriteBatch::new();
    for entry in legacy
        .snapshot()
        .unwrap()
        .iter(ColumnFamily::Default, b"", b"\xff")
        .unwrap()
    {
        let (key, value) = entry.unwrap();
        seed.put(ColumnFamily::Default, key, value);
    }
    engine
        .write_applied(seed, AppliedPosition { term: 1, index: 1 })
        .unwrap();
    let prefix = std::fs::read(&path).unwrap();
    let a = numbered(&root, 3);
    let p = plan(&engine, &root, &LedgerRequest::Acquire(a.clone())).unwrap();
    engine
        .write_applied(p.into_batch(), AppliedPosition { term: 1, index: 2 })
        .unwrap();
    drop(engine);
    let complete = std::fs::read(&path).unwrap();
    for end in [
        prefix.len(),
        prefix.len() + 1,
        prefix.len() + 32,
        (prefix.len() + complete.len()) / 2,
        complete.len() - 1,
        complete.len(),
    ] {
        let cut = temp.path().join(format!("cut-{end}.wal"));
        std::fs::write(&cut, &complete[..end]).unwrap();
        let (engine, _) = WalEngine::open(&cut).unwrap();
        let header = engine
            .get(ColumnFamily::Default, &key(1, &[]))
            .unwrap()
            .unwrap();
        let is_complete = end == complete.len();
        assert_eq!(read_header(&header, root.digest()).unwrap().1, is_complete);
        assert_eq!(
            retention_owner(engine.snapshot().unwrap().as_ref(), &root, a.descriptor.id)
                .unwrap()
                .is_some(),
            is_complete
        );
        assert_eq!(
            owner(&engine, &root, numbered(&root, 1).descriptor.id).phase,
            PinPhase::Released
        );
        let bytes = engine
            .get(ColumnFamily::Default, &resource_key(&a.resources[0]))
            .unwrap()
            .unwrap();
        assert_eq!(bytes.starts_with(RESOURCE_MAGIC), is_complete);
        assert_eq!(
            engine
                .get(ColumnFamily::Default, &base_key(&a.resources[0]))
                .unwrap()
                .is_some(),
            is_complete
        );
    }
}
