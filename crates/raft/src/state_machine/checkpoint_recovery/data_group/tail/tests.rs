use super::*;
use kv9_common::raw_ttl::split::{ImportSpec, SplitPlan};
use kv9_common::{KeyspaceId, TenantId};

fn fixture() -> (DataCheckpointIdentity, ImportState, ImportRow) {
    let p = DataRange {
        root: RootDigest::from_bytes([1; 32]),
        creation: RootDigest::from_bytes([2; 32]),
        region: RegionId(100),
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: vec![],
        end: vec![],
        sealed: false,
    };
    let mut low = p.clone();
    low.region = RegionId(101);
    low.creation = RootDigest::from_bytes([3; 32]);
    low.end = b"m".to_vec();
    let mut high = low.clone();
    high.region = RegionId(102);
    high.creation = RootDigest::from_bytes([4; 32]);
    high.start = b"m".to_vec();
    high.end.clear();
    let identity = DataCheckpointIdentity {
        cluster: ClusterId::from_bytes([5; 16]),
        root: low.root,
        region: low.region,
        creation: low.creation,
    };
    let plan = SplitPlan {
        operation: [6; 16],
        parent: p,
        low,
        high,
    };
    let seal = SplitSeal {
        plan,
        cut: AppliedPosition { term: 2, index: 9 },
        watermark: ClockWatermark::default(),
    };
    let row = ImportRow {
        key: kv9_common::codec::encode_key(KeyMode::Raw, KeyspaceId(100), b"a").unwrap(),
        value: vec![0, 255],
        expiry: Some(
            ExpiryRecord::new(
                RootDigest::from_bytes([8; 32]),
                AppliedPosition {
                    term: 8,
                    index: 8000,
                },
                1000,
            )
            .unwrap(),
        ),
    };
    let mut spec = ImportSpec {
        seal,
        high: false,
        rows: 1,
        digest: RootDigest::from_bytes([1; 32]),
    };
    spec.digest = row.fold(spec.seed());
    (identity, ImportState::begin(spec).unwrap(), row)
}
fn at(i: u64) -> Option<AppliedPosition> {
    Some(AppliedPosition { term: 4, index: i })
}
fn begin(state: &ImportState) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, IMPORT_KEY.to_vec(), state.encode());
    b.put(
        ColumnFamily::Default,
        CLOCK_KEY.to_vec(),
        state.spec.seal.watermark.encode().to_vec(),
    );
    b
}
fn page(state: &ImportState, row: &ImportRow) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.put(ColumnFamily::Default, row.key.clone(), row.value.clone());
    b.put(
        ColumnFamily::Write,
        row.key.clone(),
        row.expiry.unwrap().encode().to_vec(),
    );
    b.put(
        ColumnFamily::Default,
        IMPORT_KEY.to_vec(),
        state.append(std::slice::from_ref(row)).unwrap().encode(),
    );
    b
}

#[test]
fn ttl_split_replay_rejects_torn_reordered_or_rebased_import_batches() {
    let (identity, initial, row) = fixture();
    let mut tail = TailState::default();
    tail.batch(&begin(&initial), at(1), identity).unwrap();
    let next = initial.append(std::slice::from_ref(&row)).unwrap();
    for fault in 0..8 {
        let mut b = WriteBatch::new();
        match fault {
            0 => {
                b.put(ColumnFamily::Default, row.key.clone(), row.value.clone());
            }
            1 => {
                b.put(ColumnFamily::Default, IMPORT_KEY.to_vec(), next.encode());
            }
            2 => {
                b.put(ColumnFamily::Default, row.key.clone(), row.value.clone());
                b.put(ColumnFamily::Default, IMPORT_KEY.to_vec(), next.encode());
            }
            3..=5 => {
                b.put(
                    ColumnFamily::Default,
                    row.key.clone(),
                    if fault == 3 {
                        vec![7]
                    } else {
                        row.value.clone()
                    },
                );
                if fault == 4 {
                    b.delete(ColumnFamily::Write, row.key.clone());
                } else {
                    let expiry = if fault == 5 {
                        ExpiryRecord::new(
                            identity.creation,
                            AppliedPosition { term: 4, index: 2 },
                            1000,
                        )
                        .unwrap()
                    } else {
                        row.expiry.unwrap()
                    };
                    b.put(
                        ColumnFamily::Write,
                        row.key.clone(),
                        expiry.encode().to_vec(),
                    );
                }
                b.put(ColumnFamily::Default, IMPORT_KEY.to_vec(), next.encode());
            }
            6 => {
                b.put(
                    ColumnFamily::Default,
                    RANGE_KEY.to_vec(),
                    initial.spec.target().encode(),
                );
            }
            7 => {
                b.delete(ColumnFamily::Default, IMPORT_KEY.to_vec());
            }
            _ => unreachable!(),
        }
        assert!(
            tail.clone().batch(&b, at(2), identity).is_err(),
            "fault {fault}"
        );
    }
    tail.batch(&page(&initial, &row), at(2), identity).unwrap();
    let mut complete = WriteBatch::new();
    complete.put(
        ColumnFamily::Default,
        IMPORT_KEY.to_vec(),
        next.finish().unwrap().encode(),
    );
    assert!(
        tail.clone().batch(&complete, at(3), identity).is_err(),
        "completion cannot lose its ownership row"
    );
    complete.put(
        ColumnFamily::Default,
        RANGE_KEY.to_vec(),
        initial.spec.target().encode(),
    );
    tail.batch(&complete, at(3), identity).unwrap();
    assert!(
        tail.batch(&page(&initial, &row), at(4), identity).is_err(),
        "completed imports cannot replay copy data"
    );
}

#[test]
fn ttl_split_replay_requires_atomic_clock_inheritance_and_exact_source_seal_cut() {
    let (identity, initial, _) = fixture();
    let mut only_clock = WriteBatch::new();
    only_clock.put(
        ColumnFamily::Default,
        CLOCK_KEY.to_vec(),
        ClockWatermark::default().encode().to_vec(),
    );
    assert!(TailState::default()
        .batch(&only_clock, at(1), identity)
        .is_err());
    let mut only_cursor = WriteBatch::new();
    only_cursor.put(ColumnFamily::Default, IMPORT_KEY.to_vec(), initial.encode());
    assert!(TailState::default()
        .batch(&only_cursor, at(1), identity)
        .is_err());
    let seal = initial.spec.seal;
    let source = DataCheckpointIdentity {
        region: seal.plan.parent.region,
        creation: seal.plan.parent.creation,
        ..identity
    };
    let mut tail = TailState::default();
    let mut binding = WriteBatch::new();
    binding.put(
        ColumnFamily::Default,
        RANGE_KEY.to_vec(),
        seal.plan.parent.encode(),
    );
    tail.batch(&binding, at(1), source).unwrap();
    tail.batch(&only_clock, at(2), source).unwrap();
    let mut freeze = WriteBatch::new();
    freeze.put(ColumnFamily::Default, SEAL_KEY.to_vec(), seal.encode());
    assert!(tail.clone().batch(&freeze, Some(seal.cut), source).is_err());
    freeze.put(
        ColumnFamily::Default,
        RANGE_KEY.to_vec(),
        seal.plan.sealed_range().unwrap().encode(),
    );
    assert!(
        tail.clone().batch(&freeze, at(10), source).is_err(),
        "seal record must name this actual WAL position"
    );
    tail.batch(&freeze, Some(seal.cut), source).unwrap();
    let mut bypass = WriteBatch::new();
    bypass.put(
        ColumnFamily::Default,
        kv9_common::codec::encode_key(KeyMode::Raw, KeyspaceId(100), b"b").unwrap(),
        vec![1],
    );
    assert!(tail.batch(&bypass, at(11), source).is_err());
}
