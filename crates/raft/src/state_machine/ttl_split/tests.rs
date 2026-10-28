use super::*;
use crate::command::{RawTtlAction, RawTtlCommand, RawTtlMutation};
use kv9_common::codec::{encode_key, KeyMode};
use kv9_common::raw_ttl::split::{ImportRow, ImportSpec, SplitPlan};
use kv9_common::raw_ttl::{ClockSample, ExpiryPlan, ExpiryRecord};
use kv9_common::{KeyspaceId, StoreIncarnation, TenantId};
use kv9_engine::{Engine, WalEngine};
use std::path::{Path, PathBuf};

mod atomic;
mod replicated;

fn plan() -> SplitPlan {
    let parent = DataRange {
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
    let mut low = parent.clone();
    low.region = RegionId(101);
    low.creation = RootDigest::from_bytes([3; 32]);
    low.end = b"m".to_vec();
    let mut high = parent.clone();
    high.region = RegionId(102);
    high.creation = RootDigest::from_bytes([4; 32]);
    high.start = b"m".to_vec();
    SplitPlan {
        operation: [7; 16],
        parent,
        low,
        high,
    }
}
fn temp(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "kv9-ttl-transfer-{label}-{}",
        StoreIncarnation::mint().unwrap()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn split(action: RawTtlSplitAction) -> Command {
    Command::RawTtlSplit(RawTtlSplit::new(action).unwrap())
}
fn key(k: &[u8]) -> Vec<u8> {
    encode_key(KeyMode::Raw, plan().parent.keyspace, k).unwrap()
}
fn read(engine: &WalEngine, cf: ColumnFamily, k: &[u8]) -> Option<Vec<u8>> {
    Engine::get(engine, cf, k).unwrap()
}
fn state(engine: &WalEngine) -> ImportState {
    ImportState::decode(&read(engine, ColumnFamily::Default, IMPORT_KEY).unwrap()).unwrap()
}
fn open(path: &Path, range: &DataRange) -> (Arc<WalEngine>, MemStateMachine<WalEngine>) {
    let engine = Arc::new(WalEngine::open(path).unwrap().0);
    let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
    sm.set_data_group(range.root, range.region, range.creation)
        .unwrap();
    (engine, sm)
}
fn apply<E: ApplyStore>(sm: &mut MemStateMachine<E>, command: &Command) -> ApplyResult {
    sm.apply_at(
        AppliedPosition {
            term: 7,
            index: sm.applied_index().0 + 1,
        },
        command,
    )
    .unwrap()
}
fn write(range: &DataRange, k: &[u8], v: &[u8], ttl: Option<u64>, now: u64) -> Command {
    Command::RawTtl(
        RawTtlCommand::new(
            range,
            RawTtlAction::Write {
                plan: ExpiryPlan::resolve(
                    ttl,
                    ClockSample::with_uncertainty(now, 0).unwrap(),
                    ClockWatermark::default(),
                )
                .unwrap(),
                mutations: vec![RawTtlMutation::Put {
                    key: k.to_vec(),
                    value: v.to_vec(),
                }],
            },
        )
        .unwrap(),
    )
}
fn spec(seal: SplitSeal, high: bool, rows: &[ImportRow]) -> ImportSpec {
    let mut spec = ImportSpec {
        seal,
        high,
        rows: rows.len() as u64,
        digest: RootDigest::from_bytes([1; 32]),
    };
    spec.digest = rows.iter().fold(spec.seed(), |hash, row| row.fold(hash));
    spec.validate().unwrap();
    spec
}
fn captured(engine: &WalEngine, k: &[u8]) -> ImportRow {
    ImportRow {
        key: key(k),
        value: read(engine, ColumnFamily::Default, &key(k)).unwrap(),
        expiry: read(engine, ColumnFamily::Write, &key(k))
            .map(|b| ExpiryRecord::decode(&b).unwrap()),
    }
}
fn position(engine: &WalEngine) -> AppliedPosition {
    let DurableAppliedPosition::AppliedThrough(at) =
        ReplicatedEngine::applied_position(engine).unwrap()
    else {
        panic!("durable position required")
    };
    at
}

#[test]
fn ttl_transfer_resumes_cursor_preserves_original_expiry_and_empty_child_watermark() {
    let directory = temp("ordered");
    let p = plan();
    let (parent, mut source) = open(&directory.join("parent"), &p.parent);
    apply(
        &mut source,
        &Command::DataRange {
            expected: None,
            next: p.parent.clone(),
        },
    );
    apply(
        &mut source,
        &Command::RawTtl(
            RawTtlCommand::new(
                &p.parent,
                RawTtlAction::Enable {
                    observed: ClockWatermark::default(),
                },
            )
            .unwrap(),
        ),
    );
    for _ in 0..30 {
        apply(&mut source, &Command::Noop);
    }
    apply(
        &mut source,
        &write(&p.parent, b"a", b"KV9TTL01 opaque value", Some(1), 1000),
    );
    apply(
        &mut source,
        &write(&p.parent, b"b", b"permanent", None, 1000),
    );
    apply(&mut source, &split(RawTtlSplitAction::Seal(p.clone())));
    let seal = SplitSeal::decode(&read(&parent, ColumnFamily::Default, SEAL_KEY).unwrap()).unwrap();
    let rows = [captured(&parent, b"a"), captured(&parent, b"b")];
    assert_eq!(
        apply(
            &mut source,
            &write(&p.parent, b"a", b"too late", None, 5000)
        )
        .outcome,
        ApplyOutcome::FenceRejected(p.parent.region)
    );
    apply(&mut source, &split(RawTtlSplitAction::Seal(p.clone())));
    assert_eq!(
        SplitSeal::decode(&read(&parent, ColumnFamily::Default, SEAL_KEY).unwrap()).unwrap(),
        seal
    );
    let expected = spec(seal.clone(), false, &rows);
    let child_path = directory.join("low");
    let (child, mut sm) = open(&child_path, &p.low);
    apply(&mut sm, &Command::Noop);
    let old = position(&child);
    apply(&mut sm, &Command::Noop);
    assert_eq!(
        apply(
            &mut sm,
            &split(RawTtlSplitAction::Begin {
                spec: expected.clone(),
                empty_at: old
            })
        )
        .outcome,
        ApplyOutcome::FenceRejected(p.low.region)
    );
    let begin = split(RawTtlSplitAction::Begin {
        spec: expected.clone(),
        empty_at: position(&child),
    });
    apply(&mut sm, &begin);
    assert!(read(&child, ColumnFamily::Default, RANGE_KEY).is_none());
    assert_eq!(
        read(&child, ColumnFamily::Default, CLOCK_KEY),
        Some(seal.watermark.encode().to_vec())
    );
    assert_eq!(
        apply(
            &mut sm,
            &Command::DataRange {
                expected: None,
                next: p.low.clone()
            }
        )
        .outcome,
        ApplyOutcome::FenceRejected(p.low.region)
    );
    assert_eq!(
        apply(&mut sm, &write(&p.low, b"a", b"early", None, 2000)).outcome,
        ApplyOutcome::FenceRejected(p.low.region)
    );
    let initial = state(&child);
    assert_eq!(
        apply(
            &mut sm,
            &split(RawTtlSplitAction::Finish {
                target: p.low.clone(),
                expected: initial.digest()
            })
        )
        .outcome,
        ApplyOutcome::FenceRejected(p.low.region)
    );
    let page = split(RawTtlSplitAction::Append {
        target: p.low.clone(),
        expected: initial.digest(),
        rows: vec![rows[0].clone()],
    });
    apply(&mut sm, &page);
    assert_eq!(
        apply(&mut sm, &page).outcome,
        ApplyOutcome::FenceRejected(p.low.region)
    );
    let partial = state(&child);
    assert_eq!(partial.copied, 1);
    drop(sm);
    drop(child);
    let (child, mut sm) = open(&child_path, &p.low);
    assert_eq!(state(&child), partial);
    apply(&mut sm, &begin);
    assert_eq!(state(&child), partial);
    apply(
        &mut sm,
        &split(RawTtlSplitAction::Append {
            target: p.low.clone(),
            expected: partial.digest(),
            rows: vec![rows[1].clone()],
        }),
    );
    let finish = split(RawTtlSplitAction::Finish {
        target: p.low.clone(),
        expected: state(&child).digest(),
    });
    apply(&mut sm, &finish);
    let complete = state(&child);
    assert!(complete.complete);
    assert_eq!(
        read(&child, ColumnFamily::Write, &key(b"a")),
        Some(rows[0].expiry.unwrap().encode().to_vec())
    );
    assert_eq!(read(&child, ColumnFamily::Write, &key(b"b")), None);
    // The parent identity survives transfer; old cleanup cannot delete renewal.
    apply(&mut sm, &write(&p.low, b"a", b"renewed", Some(60), 3000));
    let current = read(&child, ColumnFamily::Write, &key(b"a")).unwrap();
    assert_ne!(current, rows[0].expiry.unwrap().encode());
    apply(
        &mut sm,
        &Command::RawTtl(
            RawTtlCommand::new(
                &p.low,
                RawTtlAction::Cleanup {
                    candidates: vec![(b"a".to_vec(), rows[0].expiry.unwrap())],
                },
            )
            .unwrap(),
        ),
    );
    apply(&mut sm, &begin);
    apply(&mut sm, &finish);
    assert_eq!(state(&child), complete);
    assert_eq!(
        read(&child, ColumnFamily::Default, &key(b"a")),
        Some(b"renewed".to_vec())
    );
    assert_eq!(read(&child, ColumnFamily::Write, &key(b"a")), Some(current));
    assert_eq!(
        apply(&mut sm, &page).outcome,
        ApplyOutcome::FenceRejected(p.low.region)
    );
    assert_eq!(
        ClockWatermark::decode(&read(&child, ColumnFamily::Default, CLOCK_KEY).unwrap())
            .unwrap()
            .unix_ms(),
        3000
    );
    // Even an empty half must persist the source clock before becoming bound.
    let (high, mut high_sm) = open(&directory.join("high"), &p.high);
    apply(&mut high_sm, &Command::Noop);
    apply(
        &mut high_sm,
        &split(RawTtlSplitAction::Begin {
            spec: spec(seal.clone(), true, &[]),
            empty_at: position(&high),
        }),
    );
    apply(
        &mut high_sm,
        &split(RawTtlSplitAction::Finish {
            target: p.high.clone(),
            expected: state(&high).digest(),
        }),
    );
    assert_eq!(
        read(&high, ColumnFamily::Default, CLOCK_KEY),
        Some(seal.watermark.encode().to_vec())
    );
    assert_eq!(
        read(&high, ColumnFamily::Default, RANGE_KEY),
        Some(p.high.encode())
    );
    drop((source, parent, sm, child, high_sm, high));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn ttl_transfer_codecs_scope_hashes_and_ordinary_proposal_bypass_are_refused() {
    let p = plan();
    let seal = SplitSeal {
        plan: p.clone(),
        cut: AppliedPosition { term: 4, index: 99 },
        watermark: ClockWatermark::default(),
    };
    // A previous group's log index may exceed this source's own cut.
    let rows = vec![ImportRow {
        key: key(b"a"),
        value: vec![0, 255, 0],
        expiry: Some(
            ExpiryRecord::new(
                RootDigest::from_bytes([9; 32]),
                AppliedPosition {
                    term: 80,
                    index: 9000,
                },
                4000,
            )
            .unwrap(),
        ),
    }];
    let spec = spec(seal.clone(), false, &rows);
    let state = ImportState::begin(spec.clone()).unwrap();
    let next = state.append(&rows).unwrap();
    assert!(next.finish().is_ok());
    assert!(state.finish().is_err());
    let mut bad_row = rows[0].clone();
    bad_row.expiry = None;
    assert!(state.append(&[bad_row]).unwrap().finish().is_err());
    for command in [
        split(RawTtlSplitAction::Seal(p)),
        split(RawTtlSplitAction::Begin {
            spec: spec.clone(),
            empty_at: AppliedPosition { term: 4, index: 1 },
        }),
        split(RawTtlSplitAction::Append {
            target: spec.target().clone(),
            expected: state.digest(),
            rows,
        }),
        split(RawTtlSplitAction::Finish {
            target: spec.target().clone(),
            expected: next.digest(),
        }),
    ] {
        assert!(command.require_ordinary_proposal().is_err());
        assert!(command.to_write_batch().is_err());
        let bytes = command.encode();
        assert_eq!(Command::decode(&bytes).unwrap(), command);
        for end in 0..bytes.len() {
            assert!(Command::decode(&bytes[..end]).is_err());
        }
        let mut extra = bytes;
        extra.push(0);
        assert!(Command::decode(&extra).is_err());
    }
    for (bytes, decoder) in [(
        seal.encode(),
        SplitSeal::decode as fn(&[u8]) -> Result<SplitSeal>,
    )] {
        assert_eq!(decoder(&bytes).unwrap(), seal);
        for end in 0..bytes.len() {
            assert!(decoder(&bytes[..end]).is_err());
        }
    }
    let bytes = next.encode();
    assert_eq!(ImportState::decode(&bytes).unwrap(), next);
    for end in 0..bytes.len() {
        assert!(ImportState::decode(&bytes[..end]).is_err());
    }
    let mut damaged = bytes;
    damaged[20] ^= 1;
    assert!(ImportState::decode(&damaged).is_err());
}
