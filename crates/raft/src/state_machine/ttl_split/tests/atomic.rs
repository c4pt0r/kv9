use super::*;
use std::sync::atomic::{AtomicU8, Ordering};

struct Failing {
    engine: Arc<WalEngine>,
    fault: AtomicU8,
}
impl ApplyStore for Failing {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Engine::get(self.engine.as_ref(), cf, key)
    }
    fn applied_position(&self) -> Result<DurableAppliedPosition> {
        ReplicatedEngine::applied_position(self.engine.as_ref())
    }
    fn write_applied(&self, batch: WriteBatch, at: AppliedPosition) -> Result<()> {
        if self.fault.load(Ordering::SeqCst) == 1 {
            return Err(Error::Engine("before transfer WAL write".into()));
        }
        ReplicatedEngine::write_applied(self.engine.as_ref(), batch, at)?;
        if self.fault.load(Ordering::SeqCst) == 2 {
            return Err(Error::Engine("after transfer WAL write".into()));
        }
        Ok(())
    }
}
fn contents(engine: &WalEngine) -> Vec<Vec<kv9_engine::ScanEntry>> {
    let view = Engine::snapshot(engine).unwrap();
    ColumnFamily::ALL
        .into_iter()
        .map(|cf| view.scan(cf, b"", b"\xff", 100).unwrap())
        .collect()
}

#[test]
fn ttl_transfer_wal_failures_keep_clock_expiry_cursor_and_completion_atomic() {
    let directory = temp("atomic");
    let p = plan();
    let seal = SplitSeal {
        plan: p.clone(),
        cut: AppliedPosition { term: 2, index: 50 },
        watermark: ClockSample::with_uncertainty(1000, 0)
            .unwrap()
            .advance(ClockWatermark::default())
            .unwrap(),
    };
    let rows = vec![ImportRow {
        key: key(b"a"),
        value: b"opaque".to_vec(),
        expiry: Some(
            ExpiryRecord::new(
                p.parent.creation,
                AppliedPosition { term: 2, index: 49 },
                2000,
            )
            .unwrap(),
        ),
    }];
    let specification = spec(seal, false, &rows);
    for phase in 0..3 {
        for fault in [1, 2] {
            let path = directory.join(format!("{phase}-{fault}"));
            let engine = Arc::new(WalEngine::open(&path).unwrap().0);
            let failing = Arc::new(Failing {
                engine: engine.clone(),
                fault: AtomicU8::new(0),
            });
            let mut sm = MemStateMachine::with_engine(failing.clone()).unwrap();
            sm.set_data_group(p.low.root, p.low.region, p.low.creation)
                .unwrap();
            apply(&mut sm, &Command::Noop);
            let mut command = split(RawTtlSplitAction::Begin {
                spec: specification.clone(),
                empty_at: position(&engine),
            });
            if phase > 0 {
                apply(&mut sm, &command);
                command = split(RawTtlSplitAction::Append {
                    target: p.low.clone(),
                    expected: state(&engine).digest(),
                    rows: rows.clone(),
                });
            }
            if phase > 1 {
                apply(&mut sm, &command);
                command = split(RawTtlSplitAction::Finish {
                    target: p.low.clone(),
                    expected: state(&engine).digest(),
                });
            }
            let previous = position(&engine);
            let before = contents(&engine);
            let at = AppliedPosition {
                term: 7,
                index: previous.index + 1,
            };
            failing.fault.store(fault, Ordering::SeqCst);
            assert!(sm.apply_at(at, &command).is_err());
            assert_eq!(sm.applied_index().0, previous.index);
            drop(sm);
            drop(failing);
            drop(engine);
            let (engine, sm) = open(&path, &p.low);
            if fault == 1 {
                assert_eq!(position(&engine), previous);
                assert_eq!(contents(&engine), before);
            } else {
                assert_eq!(position(&engine), at);
                let state = state(&engine);
                assert_eq!(state.copied, u64::from(phase > 0));
                assert_eq!(state.complete, phase == 2);
                assert_eq!(
                    read(&engine, ColumnFamily::Default, CLOCK_KEY),
                    Some(specification.seal.watermark.encode().to_vec())
                );
                assert_eq!(
                    read(&engine, ColumnFamily::Default, RANGE_KEY).is_some(),
                    phase == 2
                );
                assert_eq!(
                    read(&engine, ColumnFamily::Default, &rows[0].key).is_some(),
                    phase > 0
                );
                assert_eq!(
                    read(&engine, ColumnFamily::Write, &rows[0].key),
                    (phase > 0).then(|| rows[0].expiry.unwrap().encode().to_vec())
                );
            }
            drop((engine, sm));
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}
