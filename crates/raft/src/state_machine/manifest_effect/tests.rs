use super::*;
use kv9_engine::checkpoint::{FlushScope, SstReference};
use kv9_engine::Engine;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
struct Counting {
    engine: MemEngine,
    reads: AtomicU64,
}
impl ApplyStore for Counting {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Engine::get(&self.engine, cf, key)
    }
    fn write_applied(&self, batch: WriteBatch, at: AppliedPosition) -> Result<()> {
        ReplicatedEngine::write_applied(&self.engine, batch, at)
    }
    fn applied_position(&self) -> Result<DurableAppliedPosition> {
        ReplicatedEngine::applied_position(&self.engine)
    }
}
struct Fresh;
impl FenceAdjudicator for Fresh {
    fn is_fresh(&self, _: &crate::RegionFence) -> Result<bool> {
        Ok(true)
    }
}
fn manifest(generation: u64) -> CheckpointManifest {
    let hash = format!("{generation:064x}");
    CheckpointManifest {
        scope: FlushScope {
            cluster: "bounded-history".into(),
            region: 100,
            conf_ver: 1,
            version: 1,
        },
        term: 1,
        index: generation * 2,
        files: vec![SstReference {
            key: format!("clusters/bounded-history/regions/100/sst/{hash}"),
            sha256: hash,
            cf: 0,
            smallest: b"a".to_vec(),
            largest: b"z".to_vec(),
            size: 100,
            count: 2,
        }],
    }
}
fn append(sm: &mut MemStateMachine<Counting>, generation: u64) {
    let m = manifest(generation);
    let command = Command::ManifestChange(ManifestChangePayload::for_harness(
        100,
        m.change_id(generation - 1).unwrap(),
        generation - 1,
        m.encode().unwrap(),
        m.term,
        m.index,
    ));
    let result = sm
        .apply_at(
            AppliedPosition {
                term: 1,
                index: m.index + 1,
            },
            &command,
        )
        .unwrap();
    assert!(matches!(
        result.outcome,
        ApplyOutcome::Manifest(ManifestVerdict::Applied { .. })
    ));
}
fn fixture(count: u64) -> (Arc<Counting>, MemStateMachine<Counting>) {
    let engine = Arc::new(Counting::default());
    let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
    sm.set_fence_adjudicator(Arc::new(Fresh));
    for generation in 1..=count {
        append(&mut sm, generation);
    }
    (engine, sm)
}

#[test]
fn large_checkpoint_has_distinct_tag_and_legacy_apply_cannot_reinterpret_it() {
    let mut image = manifest(1);
    let file = image.files[0].clone();
    image.files = (0..3)
        .map(|i| SstReference {
            smallest: vec![b'a' + i],
            largest: vec![b'a' + i],
            size: 20 * 1024 * 1024,
            ..file.clone()
        })
        .collect();
    let payload = ManifestChangePayload::for_harness(
        100,
        image.change_id(0).unwrap(),
        0,
        image.encode().unwrap(),
        image.term,
        image.index,
    );
    let at = AppliedPosition { term: 1, index: 3 };
    let command = Command::ManifestChange(payload.clone());
    let (engine, mut sm) = fixture(0);
    assert!(command
        .decode_checkpoint_manifest(payload.changeset())
        .is_err());
    let before = ReplicatedEngine::applied_position(&engine.engine).unwrap();
    assert!(sm.apply_at(at, &command).is_err());
    assert_eq!(
        ReplicatedEngine::applied_position(&engine.engine).unwrap(),
        before
    );
    assert_eq!(sm.manifest_pair(100).unwrap().generation, 0);
    let command = Command::LargeManifestChange(payload);
    assert_eq!(command.encode()[1], 14);
    assert_eq!(
        Command::decode_for_harness(&command.encode()).unwrap(),
        command
    );
    assert!(command.to_write_batch().is_err());
    assert!(command
        .decode_checkpoint_manifest(&manifest(1).encode().unwrap())
        .is_err());
    let (_, mut sm) = fixture(0);
    assert!(matches!(
        sm.apply_at(at, &command).unwrap().outcome,
        ApplyOutcome::Manifest(ManifestVerdict::Applied { .. })
    ));
    assert_eq!(sm.manifest_pair(100).unwrap().generation, 1);
    assert!(matches!(
        sm.apply_at(AppliedPosition { term: 1, index: 4 }, &command)
            .unwrap()
            .outcome,
        ApplyOutcome::Manifest(ManifestVerdict::AlreadyApplied { .. })
    ));
    assert_eq!(sm.manifest_pair(100).unwrap().generation, 1);
}

#[test]
fn actual_history_reads_are_bounded_and_old_effect_progresses_while_history_grows() {
    let (engine, mut sm) = fixture(41);
    let intended = manifest(1).encode().unwrap();
    let mut next = None;
    let mut latest = 41;
    let mut calls = 0;
    loop {
        engine.reads.store(0, Ordering::SeqCst);
        let page = sm.manifest_effect_page(100, &intended, next).unwrap();
        calls += 1;
        assert!(page.examined <= MAX_MANIFEST_EFFECT_ROWS);
        assert_eq!(engine.reads.load(Ordering::SeqCst), page.examined + 1);
        if page.found {
            break;
        }
        let older = page.next.expect("the old match must remain reachable");
        assert!(next.is_none_or(|before| older < before));
        next = Some(older);
        for _ in 0..8 {
            latest += 1;
            append(&mut sm, latest);
        }
        assert!(
            calls < 12,
            "new appends cannot restart each page at the head"
        );
    }
    assert_eq!(calls, 11);
    assert!(latest > 100);
}

#[test]
fn exhausted_or_missing_history_is_only_a_page_result_including_max_generation() {
    let (engine, sm) = fixture(0);
    let intended = manifest(1).encode().unwrap();
    assert_eq!(
        sm.manifest_effect_page(100, &intended, None).unwrap(),
        ManifestEffectPage::default()
    );
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        manifest_pair_key(100),
        ManifestPair {
            generation: u64::MAX,
            last_change_id: vec![1; 32],
        }
        .encode(),
    );
    engine.engine.write(batch).unwrap(); // Deliberately incomplete historical fixture.
    engine.reads.store(0, Ordering::SeqCst);
    let page = sm.manifest_effect_page(100, &intended, None).unwrap();
    assert!(!page.found);
    assert_eq!(page.next, Some(u64::MAX - MAX_MANIFEST_EFFECT_ROWS));
    assert_eq!(
        engine.reads.load(Ordering::SeqCst),
        MAX_MANIFEST_EFFECT_ROWS + 1
    );
    let final_page = sm.manifest_effect_page(100, &intended, Some(2)).unwrap();
    assert!(!final_page.found && final_page.next.is_none());
    assert_eq!(final_page.examined, 2);
    assert_eq!(
        sm.manifest_effect_page(100, &intended, Some(0)).unwrap(),
        ManifestEffectPage::default()
    );
    assert!(sm.manifest_effect_page(101, &intended, None).is_err());
}

#[test]
fn page_refuses_corrupt_or_misbound_history_instead_of_returning_positive_effect() {
    for corrupt in [false, true] {
        let (engine, sm) = fixture(1);
        let m = manifest(1);
        let bytes = if corrupt {
            b"KV9CHECKPOINT broken".to_vec()
        } else {
            m.encode().unwrap()
        };
        let mut value = m.term.to_be_bytes().to_vec();
        value.extend_from_slice(&(m.index + 1).to_be_bytes());
        value.extend_from_slice(&bytes);
        let mut batch = WriteBatch::new();
        batch.put(ColumnFamily::Default, manifest_gen_key(100, 1), value);
        engine.engine.write(batch).unwrap();
        assert!(sm
            .manifest_effect_page(100, &m.encode().unwrap(), None)
            .is_err());
    }
}
