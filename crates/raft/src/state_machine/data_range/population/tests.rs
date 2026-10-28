use super::*;
use crate::ApplyOutcome;
use kv9_common::{KeyspaceId, TenantId};
use kv9_engine::{Engine, MemEngine};

fn range() -> DataRange {
    DataRange {
        root: RootDigest::from_bytes([1; 32]),
        creation: RootDigest::from_bytes([2; 32]),
        region: RegionId(100),
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: vec![],
        end: b"m".to_vec(),
        sealed: false,
    }
}
fn source() -> DataRange {
    let mut s = range();
    s.creation = RootDigest::from_bytes([3; 32]);
    s.region = RegionId(101);
    s.end.clear();
    s.sealed = true;
    s.version = 2;
    s
}
fn key() -> Vec<u8> {
    kv9_common::codec::encode_key(KeyMode::Raw, range().keyspace, b"a").unwrap()
}
fn population() -> Command {
    Command::PopulateRaw(
        RawPopulation::new(
            source(),
            range(),
            [1; 16],
            vec![(key(), b"parent".to_vec())],
        )
        .unwrap(),
    )
}
fn apply(sm: &mut MemStateMachine, command: &Command) -> Result<ApplyResult> {
    sm.apply_at(
        AppliedPosition {
            term: 1,
            index: sm.applied_index().0 + 1,
        },
        command,
    )
}

#[test]
fn delayed_population_cannot_overwrite_or_resurrect_after_binding_or_sealing() {
    let r = range();
    let engine = Arc::new(MemEngine::new());
    let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
    sm.set_data_group(r.root, r.region, r.creation).unwrap();
    assert!(!sm.can_group_raw(&population()));
    for _ in 0..2 {
        assert_eq!(
            apply(&mut sm, &population()).unwrap().outcome,
            ApplyOutcome::Plain
        );
    }
    apply(
        &mut sm,
        &Command::DataRange {
            expected: None,
            next: r.clone(),
        },
    )
    .unwrap();
    for op in [
        KvOp::Put {
            cf: 0,
            key: key(),
            value: b"child".to_vec(),
        },
        KvOp::Delete { cf: 0, key: key() },
    ] {
        apply(
            &mut sm,
            &Command::Fenced {
                fence: RegionFence {
                    region_id: r.region.0,
                    conf_ver: 1,
                    version: 1,
                },
                inner: FencedInner::Write {
                    ops: vec![op.clone()],
                },
            },
        )
        .unwrap();
        assert_eq!(
            apply(&mut sm, &population()).unwrap().outcome,
            ApplyOutcome::FenceRejected(r.region)
        );
        let expected = match op {
            KvOp::Put { value, .. } => Some(value),
            _ => None,
        };
        assert_eq!(
            Engine::get(engine.as_ref(), ColumnFamily::Default, &key()).unwrap(),
            expected
        );
    }
    let mut sealed = r.clone();
    sealed.sealed = true;
    sealed.version += 1;
    apply(
        &mut sm,
        &Command::DataRange {
            expected: Some(r.digest()),
            next: sealed.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        apply(&mut sm, &population()).unwrap().outcome,
        ApplyOutcome::FenceRejected(r.region)
    );
    assert_eq!(
        Engine::get(engine.as_ref(), ColumnFamily::Default, RANGE_KEY).unwrap(),
        Some(sealed.encode())
    );
    let previous = sm.applied_index();
    assert!(sm
        .apply_raw_group(&[(
            AppliedPosition {
                term: 1,
                index: previous.0 + 1
            },
            population()
        )])
        .is_err());
    assert_eq!(sm.applied_index(), previous);
}

#[test]
fn population_scope_corruption_and_legacy_replay_have_distinct_outcomes() {
    let r = range();
    let engine = Arc::new(MemEngine::new());
    let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
    sm.set_data_group(r.root, r.region, r.creation).unwrap();
    let mut foreign = r.clone();
    foreign.creation = RootDigest::from_bytes([4; 32]);
    let cmd = Command::PopulateRaw(
        RawPopulation::new(
            source(),
            foreign.clone(),
            [1; 16],
            vec![(key(), b"bad".to_vec())],
        )
        .unwrap(),
    );
    assert_eq!(
        apply(&mut sm, &cmd).unwrap().outcome,
        ApplyOutcome::FenceRejected(r.region)
    );
    let previous = sm.applied_index();
    for corrupt in [vec![0; 117], foreign.encode()] {
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), corrupt);
        Engine::write(engine.as_ref(), batch).unwrap();
        assert!(apply(&mut sm, &population()).is_err());
        assert_eq!(sm.applied_index(), previous);
    }
    // Old committed tags keep their original semantics. Reinterpreting legacy
    // Write would change the result of replaying pre-upgrade Raft history.
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), r.encode());
    Engine::write(engine.as_ref(), batch).unwrap();
    let legacy = Command::Write {
        ops: vec![KvOp::Put {
            cf: 0,
            key: key(),
            value: b"historical".to_vec(),
        }],
    };
    assert_eq!(
        apply(&mut sm, &legacy).unwrap().outcome,
        ApplyOutcome::Plain
    );
    assert_eq!(
        Engine::get(engine.as_ref(), ColumnFamily::Default, &key()).unwrap(),
        Some(b"historical".to_vec())
    );
}

#[test]
fn population_codec_rejects_truncation_trailing_scope_rows_and_oversize() {
    let command = population();
    let bytes = command.encode();
    assert_eq!(Command::decode(&bytes).unwrap(), command);
    assert!(command.to_write_batch().is_err());
    for cut in 0..bytes.len() {
        assert!(Command::decode(&bytes[..cut]).is_err());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(Command::decode(&extra).is_err());
    let mut no_rows = bytes.clone();
    let offset = 2 + 16 + 4 + source().encode().len() + 4 + range().encode().len();
    no_rows[offset..offset + 4].copy_from_slice(&257u32.to_be_bytes());
    assert!(Command::decode(&no_rows).is_err());
    let row = (key(), vec![]);
    for rows in [
        vec![],
        vec![row.clone(); 257],
        vec![row.clone(), row],
        vec![(RANGE_KEY.to_vec(), range().encode())],
        vec![(
            key(),
            vec![0; crate::command::population::MAX_COMMAND_BYTES],
        )],
    ] {
        assert!(RawPopulation::new(source(), range(), [1; 16], rows).is_err());
    }
    let mut unsealed = source();
    unsealed.sealed = false;
    assert!(RawPopulation::new(unsealed, range(), [1; 16], vec![(key(), vec![])]).is_err());
    let mut wrong = range();
    wrong.tenant = TenantId(2);
    assert!(RawPopulation::new(source(), wrong, [1; 16], vec![(key(), vec![])]).is_err());
    assert!(RawPopulation::new(source(), range(), [0; 16], vec![(key(), vec![])]).is_err());
}
