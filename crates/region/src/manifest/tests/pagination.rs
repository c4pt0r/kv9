//! Seam cursor/settlement contracts. Actual engine page read bounds are tested
//! in kv9-raft; this fake isolates lost reads, cursor retention and proof priority.
use super::*;
use kv9_raft::{ManifestEffectPage, ManifestPair};
use std::collections::VecDeque;

struct Node {
    history: Option<ManifestPair>,
    pages: Mutex<VecDeque<Result<ManifestEffectPage, Error>>>,
    cursors: Mutex<Vec<Option<u64>>>,
}
impl ManifestNode for Node {
    fn propose_command(&self, _: &Command) -> Result<ProposeOutcome, Error> {
        panic!("spent window cannot be proposed")
    }
    fn wait_manifest(
        &self,
        _: kv9_raft::ProposedAt,
        _: Duration,
    ) -> Result<ApplyWaitOutcome, ApplyWaitError> {
        panic!("nothing proposed")
    }
    fn manifest_pair(&self, _: u64) -> Result<ManifestPair, Error> {
        Ok(ManifestPair {
            generation: u64::MAX,
            last_change_id: b"latest".to_vec(),
        })
    }
    fn manifest_transition(&self, _: u64, generation: u64) -> Result<Option<ManifestPair>, Error> {
        assert_eq!(generation, 1);
        Ok(self.history.clone())
    }
    fn manifest_effect_page(
        &self,
        _: u64,
        _: &[u8],
        next: Option<u64>,
    ) -> Result<ManifestEffectPage, Error> {
        self.cursors.lock().unwrap().push(next);
        self.pages
            .lock()
            .unwrap()
            .pop_front()
            .expect("exact proof must avoid all effect reads")
    }
}
fn page(found: bool, next: Option<u64>) -> Result<ManifestEffectPage, Error> {
    Ok(ManifestEffectPage {
        found,
        next,
        examined: 4,
    })
}
fn node(history: Option<ManifestPair>, pages: Vec<Result<ManifestEffectPage, Error>>) -> Arc<Node> {
    Arc::new(Node {
        history,
        pages: Mutex::new(pages.into()),
        cursors: Mutex::new(vec![]),
    })
}

#[test]
fn descriptor_preflight_uses_only_exact_history_and_never_enters_or_clears_a_slot() {
    use kv9_engine::checkpoint::{CheckpointManifest, FlushScope, SstReference};
    // A descriptor need not carry a remote-readback capability to ask about
    // committed identity; this method cannot turn it into PreparedFlush.
    let digest = "a".repeat(64);
    let manifest = CheckpointManifest {
        scope: FlushScope {
            cluster: "test".into(),
            region: 5,
            conf_ver: 1,
            version: 1,
        },
        term: 1,
        index: 1,
        files: vec![SstReference {
            key: format!("clusters/test/regions/5/sst/{digest}"),
            sha256: digest,
            cf: 0,
            smallest: vec![1],
            largest: vec![1],
            size: 1,
            count: 1,
        }],
    };
    let change = manifest.change_id(0).unwrap();
    for history in [
        None,
        Some(ManifestPair {
            generation: 1,
            last_change_id: change.clone(),
        }),
        Some(ManifestPair {
            generation: 1,
            last_change_id: b"other".to_vec(),
        }),
    ] {
        let source = node(history.clone(), vec![]);
        let seam = ManifestSeam::mint(SeamHandle::for_harness(source.clone()));
        let result = seam.inspect_checkpoint(&manifest, 0).unwrap();
        assert_eq!(
            result,
            history.map(|pair| if pair.last_change_id == change {
                SettledManifest::AlreadyApplied { generation: 1 }
            } else {
                SettledManifest::WindowRefused {
                    winner_change_id: pair.last_change_id,
                }
            })
        );
        assert!(!seam.in_flight(5));
        assert!(source.cursors.lock().unwrap().is_empty());
    }
    let source = node(None, vec![page(false, None)]);
    let seam = ManifestSeam::mint(SeamHandle::for_harness(source.clone()));
    assert_eq!(
        seam.resume_in_flight(attempt(5, b"unsettled", 0, 1), SHORT)
            .unwrap(),
        ManifestProposalState::Unknown
    );
    assert_eq!(seam.inspect_checkpoint(&manifest, 0).unwrap(), None);
    assert!(seam.in_flight(5));
    assert_eq!(*source.cursors.lock().unwrap(), vec![None]);
    let (driver, seam) = seam_over_driver();
    let before = driver.driver_applied();
    assert_eq!(seam.inspect_checkpoint(&manifest, 0).unwrap(), None);
    assert_eq!(driver.driver_applied(), before);
    assert!(!seam.in_flight(5));
    driver.stop();
}

#[test]
fn exact_historical_identity_settles_without_scanning_even_an_enormous_later_history() {
    for winner in [b"old".to_vec(), b"other".to_vec()] {
        let source = node(
            Some(ManifestPair {
                generation: 1,
                last_change_id: winner.clone(),
            }),
            vec![],
        );
        let seam = ManifestSeam::mint(SeamHandle::for_harness(source.clone()));
        let result = seam
            .resume_in_flight(attempt(5, b"old", 0, 1), SHORT)
            .unwrap();
        if winner == b"old" {
            assert_eq!(
                result,
                ManifestProposalState::Settled(SettledManifest::AlreadyApplied { generation: 1 })
            );
        } else {
            assert_eq!(
                result,
                ManifestProposalState::Settled(SettledManifest::WindowRefused {
                    winner_change_id: winner
                })
            );
        }
        assert!(source.cursors.lock().unwrap().is_empty());
        assert!(!seam.in_flight(5));
    }
}

#[test]
fn interrupted_page_retries_same_cursor_and_exhaustion_keeps_attempt_until_positive_proof() {
    let source = node(
        None,
        vec![
            page(false, Some(8)),
            Err(Error::Engine("injected read error".into())),
            page(false, Some(4)),
            page(false, None),
            page(true, None),
        ],
    );
    let seam = ManifestSeam::mint(SeamHandle::for_harness(source.clone()));
    assert_eq!(
        seam.resume_in_flight(attempt(5, b"old", 0, 1), SHORT)
            .unwrap(),
        ManifestProposalState::Unknown
    );
    assert!(matches!(
        seam.propose_manifest_change(attempt(5, b"new", 0, 1), SHORT),
        Err(ManifestSeamError::Busy { .. })
    ));
    assert!(matches!(
        seam.converge(5, SHORT),
        Err(ManifestSeamError::Node(_))
    ));
    for _ in 0..2 {
        assert_eq!(
            seam.converge(5, SHORT).unwrap(),
            ManifestProposalState::Unknown
        );
        assert!(seam.in_flight(5));
    }
    assert_eq!(
        seam.converge(5, SHORT).unwrap(),
        ManifestProposalState::Settled(SettledManifest::EffectSettled)
    );
    assert_eq!(
        *source.cursors.lock().unwrap(),
        vec![None, Some(8), Some(8), Some(4), None]
    );
    assert!(!seam.in_flight(5));
}

#[test]
fn restarting_an_unresolved_attempt_restarts_its_scan_without_minting_a_verdict() {
    let source = node(
        None,
        vec![page(false, Some(8)), page(false, Some(8)), page(true, None)],
    );
    let first = ManifestSeam::mint(SeamHandle::for_harness(source.clone()));
    assert_eq!(
        first
            .resume_in_flight(attempt(5, b"old", 0, 1), SHORT)
            .unwrap(),
        ManifestProposalState::Unknown
    );
    drop(first);
    // Harness models a fresh process/driver; production mints its handle once
    // and supplies the same identity from the durable pending journal.
    let recovered = ManifestSeam::mint(SeamHandle::for_harness(source.clone()));
    assert_eq!(
        recovered
            .resume_in_flight(attempt(5, b"old", 0, 1), SHORT)
            .unwrap(),
        ManifestProposalState::Unknown
    );
    assert_eq!(
        recovered.converge(5, SHORT).unwrap(),
        ManifestProposalState::Settled(SettledManifest::EffectSettled)
    );
    assert_eq!(*source.cursors.lock().unwrap(), vec![None, None, Some(8)]);
}

#[test]
fn a_blocked_convergence_refuses_duplicate_drivers_but_allows_another_region() {
    use std::sync::mpsc;
    struct Gated {
        inner: Arc<Node>,
        entered: mpsc::Sender<()>,
        release: Mutex<Option<mpsc::Receiver<()>>>,
        reads: Mutex<Vec<u64>>,
    }
    impl ManifestNode for Gated {
        fn propose_command(&self, cmd: &Command) -> Result<ProposeOutcome, Error> {
            self.inner.propose_command(cmd)
        }
        fn wait_manifest(
            &self,
            at: kv9_raft::ProposedAt,
            deadline: Duration,
        ) -> Result<ApplyWaitOutcome, ApplyWaitError> {
            self.inner.wait_manifest(at, deadline)
        }
        fn manifest_pair(&self, region: u64) -> Result<ManifestPair, Error> {
            self.reads.lock().unwrap().push(region);
            let gate = if region == 5 {
                self.release.lock().unwrap().take()
            } else {
                None
            };
            if let Some(gate) = gate {
                self.entered.send(()).unwrap();
                // A broken global-lock implementation must fail, not hang CI.
                gate.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            self.inner.manifest_pair(region)
        }
        fn manifest_transition(
            &self,
            region: u64,
            generation: u64,
        ) -> Result<Option<ManifestPair>, Error> {
            self.inner.manifest_transition(region, generation)
        }
        fn manifest_effect_page(
            &self,
            region: u64,
            intended: &[u8],
            next: Option<u64>,
        ) -> Result<ManifestEffectPage, Error> {
            self.inner.manifest_effect_page(region, intended, next)
        }
    }
    let (entered, observe) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let source = Arc::new(Gated {
        inner: node(
            None,
            vec![page(false, Some(8)), page(false, Some(8)), page(true, None)],
        ),
        entered,
        release: Mutex::new(Some(gate)),
        reads: Mutex::new(vec![]),
    });
    let seam = Arc::new(ManifestSeam::mint(SeamHandle::for_harness(source.clone())));
    let worker = {
        let seam = seam.clone();
        std::thread::spawn(move || seam.resume_in_flight(attempt(5, b"old", 0, 1), SHORT))
    };
    observe.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(
        seam.converge(5, SHORT),
        Err(ManifestSeamError::Busy { region: 5, .. })
    ));
    assert!(matches!(
        seam.propose_manifest_change(attempt(5, b"new", 0, 1), SHORT),
        Err(ManifestSeamError::Busy { region: 5, .. })
    ));
    assert_eq!(
        seam.resume_in_flight(attempt(6, b"other-region", 0, 1), SHORT)
            .unwrap(),
        ManifestProposalState::Unknown
    );
    assert_eq!(*source.reads.lock().unwrap(), vec![5, 6]);
    release.send(()).unwrap();
    assert_eq!(
        worker.join().unwrap().unwrap(),
        ManifestProposalState::Unknown
    );
    assert_eq!(
        seam.converge(5, SHORT).unwrap(),
        ManifestProposalState::Settled(SettledManifest::EffectSettled)
    );
    assert_eq!(*source.reads.lock().unwrap(), vec![5, 6, 5]);
    assert_eq!(
        *source.inner.cursors.lock().unwrap(),
        vec![None, None, Some(8)]
    );
    assert!(!seam.in_flight(5));
    assert!(seam.in_flight(6));
}

#[test]
fn a_retired_call_cannot_clear_or_unlock_its_replacement_slot() {
    let seam = ManifestSeam::mint(SeamHandle::for_harness(node(None, vec![])));
    let retired = seam.start(attempt(5, b"old", 0, 1)).unwrap();
    seam.clear(&retired);
    // Model preemption just after settlement removes the old slot, while its
    // call's Running guard still exists and a new proposer takes the region.
    let replacement = seam.start(attempt(5, b"new", 1, 2)).unwrap();
    seam.clear(&retired);
    assert!(seam.in_flight(5));
    drop(retired);
    assert!(matches!(
        seam.enter(5),
        Err(ManifestSeamError::Busy {
            holder_expected_generation: 1,
            ..
        })
    ));
    drop(replacement);
    let resumed = seam.enter(5).unwrap();
    assert_eq!(resumed.0.attempt.change_id(), b"new");
    seam.clear(&resumed);
    assert!(!seam.in_flight(5));
}
