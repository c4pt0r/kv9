use super::*;
use crate::{MemoryObjectStore, WalEngine};
use std::io::Write;
use std::sync::Arc;

struct Fixture {
    dir: PathBuf,
    engine: WalEngine,
    plan: PlannedFlush,
    encoded: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        let (dir, engine, plan) = super::tests::fixture();
        let mut journal = FlushJournal::new(dir.join("seed"));
        journal.stage_planned(&plan, 7).unwrap();
        let encoded = fs::read(dir.join("seed")).unwrap();
        journal.clear_settled().unwrap();
        Self {
            dir,
            engine,
            plan,
            encoded,
        }
    }
    fn bytes(&self) -> u64 {
        self.encoded.len() as u64
    }
    fn uploader(&self, budget: &Arc<PendingDiskBudget>) -> RemoteUploader {
        RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()))
            .with_pending_budget(budget.clone())
    }
    fn spool(&self, uploader: &RemoteUploader, journal: &mut FlushJournal) -> SpooledFlush {
        uploader
            .plan_spooled(
                self.engine
                    .freeze(self.plan.manifest().scope.clone())
                    .unwrap(),
                journal,
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).unwrap();
    }
}

#[test]
fn disk_refusal_before_publication_releases_only_the_unpublished_history_candidate() {
    let f = Fixture::new();
    let budget = Arc::new(PendingDiskBudget::new(1).unwrap());
    let uploader = f.uploader(&budget);
    let mut journal = uploader.journal(f.dir.join("data.pending"));
    let (frozen, mut candidate) = f
        .engine
        .freeze_checkpoint_candidate(|_| Ok(f.plan.manifest().scope.clone()))
        .unwrap();
    let mut spool = uploader.plan_spooled(frozen, &mut journal).unwrap();
    assert!(candidate
        .stage_spooled(&mut journal, &mut spool, 7)
        .is_err());
    assert!(!candidate.is_durable());
    assert!(journal.load().unwrap().is_none());
    assert_eq!(budget.snapshot().retained_bytes, 0);
    drop(candidate);
    assert!(f
        .engine
        .freeze_checkpoint_candidate(|_| Ok(f.plan.manifest().scope.clone()))
        .is_ok());
}

#[test]
fn named_pending_survives_handle_drop_and_unlinked_upload_keeps_its_quota() {
    let f = Fixture::new();
    let budget = Arc::new(PendingDiskBudget::new(f.bytes()).unwrap());
    let uploader = f.uploader(&budget);
    let other = f.uploader(&budget);
    let path = f.dir.join("first.pending");
    let mut first = uploader.journal(&path);
    let mut plan = f.spool(&uploader, &mut first);
    first.stage_spooled(&mut plan, 7).unwrap();
    drop((plan, first));
    assert_eq!(budget.snapshot().retained_bytes, f.bytes());
    let mut second = other.journal(f.dir.join("second.pending"));
    other.with_local_execution(|scoped| {
        let mut plan = f.spool(scoped, &mut second);
        assert!(second.stage_spooled(&mut plan, 7).is_err());
    });
    assert!(!f.dir.join("second.pending.tmp").exists());
    let mut first = uploader.journal(path);
    let pending = first.load().unwrap().unwrap();
    first.clear_settled().unwrap();
    assert_eq!(budget.snapshot().retained_bytes, f.bytes());
    assert_eq!(budget.snapshot().open_readers, 1);
    assert!(first.load().unwrap().is_none());
    // Recover from the actual unlinked FD, then close it; quota follows the FD.
    pending.recover(&uploader).unwrap();
    assert_eq!(budget.snapshot().retained_bytes, 0);
    assert_eq!(budget.snapshot().open_readers, 0);
    let mut plan = f.spool(&other, &mut second);
    second.stage_spooled(&mut plan, 7).unwrap();
    second.clear_settled().unwrap();
    assert_eq!(budget.snapshot().retained_bytes, 0);
}

#[test]
fn replacement_charges_old_open_inode_and_new_prepared_journal_until_they_close() {
    let f = Fixture::new();
    let budget = Arc::new(PendingDiskBudget::new(f.bytes() * 3).unwrap());
    let uploader = f.uploader(&budget);
    let mut journal = uploader.journal(f.dir.join("data.pending"));
    journal.stage_planned(&f.plan, 7).unwrap();
    let held = journal.load().unwrap().unwrap();
    let (prepared, generation) = journal.load().unwrap().unwrap().recover(&uploader).unwrap();
    journal.stage(&prepared, generation).unwrap();
    let prepared_bytes = fs::metadata(f.dir.join("data.pending")).unwrap().len();
    assert_eq!(budget.snapshot().retained_bytes, f.bytes() + prepared_bytes);
    assert_eq!(budget.snapshot().open_readers, 1);
    journal.clear_settled().unwrap();
    assert_eq!(budget.snapshot().retained_bytes, f.bytes());
    drop(held);
    assert_eq!(budget.snapshot().retained_bytes, 0);
    assert_eq!(budget.snapshot().named_paths, 0);
}

#[test]
fn full_quota_keeps_only_the_exact_durable_plan_so_upload_can_settle() {
    let f = Fixture::new();
    let budget = Arc::new(PendingDiskBudget::new(f.bytes()).unwrap());
    let uploader = f.uploader(&budget);
    let path = f.dir.join("data.pending");
    let mut journal = uploader.journal(&path);
    journal.stage_planned(&f.plan, 7).unwrap();
    let original = fs::read(&path).unwrap();
    let (prepared, generation) = journal.load().unwrap().unwrap().recover(&uploader).unwrap();
    assert!(journal.stage(&prepared, generation).is_err());
    assert_eq!(
        journal.record_uploaded(&prepared, generation).unwrap(),
        UploadedJournal::PlannedRetained
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(journal.load().unwrap().unwrap().requires_upload());
    assert!(journal.record_uploaded(&prepared, generation + 1).is_err());
    assert_eq!(budget.snapshot().prepared_updates_deferred, 1);
    assert_eq!(budget.snapshot().retained_bytes, f.bytes());
    fs::create_dir(path.with_extension("pending.tmp")).unwrap();
    assert!(
        journal.record_uploaded(&prepared, generation).is_err(),
        "I/O failures cannot use the quota-only fallback"
    );
    fs::remove_dir(path.with_extension("pending.tmp")).unwrap();
    // Existing debt is inventoried without refusing startup, even under a lower limit.
    let reopened_budget = Arc::new(PendingDiskBudget::new(1).unwrap());
    reopened_budget.inventory_journal(&path).unwrap();
    let reopened_uploader = f.uploader(&reopened_budget);
    let mut reopened = reopened_uploader.journal(path);
    let (prepared, generation) = reopened
        .load()
        .unwrap()
        .unwrap()
        .recover(&reopened_uploader)
        .unwrap();
    assert_eq!(
        reopened.record_uploaded(&prepared, generation).unwrap(),
        UploadedJournal::PlannedRetained
    );
    reopened.clear_settled().unwrap();
    assert_eq!(reopened_budget.snapshot().retained_bytes, 0);
    assert!(!f.dir.join("data.pending.tmp").exists());
}

#[test]
fn each_publication_failure_keeps_named_debt_and_recovery_reconstructs_actual_bytes() {
    let f = Fixture::new();
    for (i, cut) in [
        PublicationStep::Written,
        PublicationStep::FileSynced,
        PublicationStep::Renamed,
        PublicationStep::DirectorySynced,
    ]
    .into_iter()
    .enumerate()
    {
        let path = f.dir.join(format!("cut-{i}.pending"));
        let budget = Arc::new(PendingDiskBudget::new(f.bytes()).unwrap());
        let mut journal = f.uploader(&budget).journal(&path);
        assert!(journal
            .publish_observed(
                f.bytes(),
                |output| output.write_all(&f.encoded).map_err(io),
                &mut |step| {
                    if step == cut {
                        Err(Error::Engine("injected publication failure".into()))
                    } else {
                        Ok(())
                    }
                }
            )
            .is_err());
        assert_eq!(budget.snapshot().retained_bytes, f.bytes());
        assert_eq!(budget.snapshot().open_writers, 0);
        let recovered = Arc::new(PendingDiskBudget::new(1).unwrap());
        recovered.inventory_journal(&path).unwrap();
        assert_eq!(recovered.snapshot().retained_bytes, f.bytes());
        assert_eq!(
            journal.load().unwrap().is_some(),
            matches!(
                cut,
                PublicationStep::Renamed | PublicationStep::DirectorySynced
            )
        );
        journal.clear_settled().unwrap();
        assert_eq!(budget.snapshot().retained_bytes, 0);
        recovered.observe_removed_journal(&path).unwrap();
        assert_eq!(recovered.snapshot().retained_bytes, 0);
    }
}

#[test]
fn partial_temporary_and_unsynced_clear_keep_debt_until_cleanup_is_confirmed() {
    let f = Fixture::new();
    let path = f.dir.join("data.pending");
    let budget = Arc::new(PendingDiskBudget::new(f.bytes()).unwrap());
    let mut journal = f.uploader(&budget).journal(&path);
    assert!(journal
        .publish_with(f.bytes(), |output| {
            output.write_all(b"partial").map_err(io)?;
            Err(Error::Engine("injected partial write".into()))
        })
        .is_err());
    assert_eq!(budget.snapshot().retained_bytes, f.bytes());
    assert_eq!(
        fs::metadata(path.with_extension("pending.tmp"))
            .unwrap()
            .len(),
        7
    );
    let recovered = PendingDiskBudget::new(1).unwrap();
    recovered.inventory_journal(&path).unwrap();
    assert_eq!(recovered.snapshot().retained_bytes, 7);
    // The next attempt removes and syncs its fixed scratch slot before admission.
    journal.stage_planned(&f.plan, 7).unwrap();
    assert!(journal
        .clear_observed(&mut || Err(Error::Engine("injected directory-sync failure".into())))
        .is_err());
    assert!(!path.exists());
    assert_eq!(budget.snapshot().retained_bytes, f.bytes());
    journal.clear_settled().unwrap();
    assert_eq!(budget.snapshot().retained_bytes, 0);
}

#[test]
fn concurrent_publication_admits_only_one_group_before_any_second_file_write() {
    let f = Fixture::new();
    let budget = Arc::new(PendingDiskBudget::new(f.bytes()).unwrap());
    let first = f.uploader(&budget).journal(f.dir.join("first.pending"));
    let mut second = f.uploader(&budget).journal(f.dir.join("second.pending"));
    let bytes = f.encoded.clone();
    let (entered, observed) = std::sync::mpsc::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let owner = std::thread::spawn(move || {
        first
            .publish_with(bytes.len() as u64, |output| {
                output.write_all(&bytes).map_err(io)?;
                entered.send(()).unwrap();
                blocked.recv().unwrap();
                Ok(())
            })
            .unwrap()
    });
    observed
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(budget.snapshot().open_writers, 1);
    assert!(second.stage_planned(&f.plan, 7).is_err());
    assert!(!f.dir.join("second.pending.tmp").exists());
    release.send(()).unwrap();
    assert!(owner.join().unwrap());
    assert_eq!(budget.snapshot().open_writers, 0);
    assert_eq!(budget.snapshot().peak_bytes, f.bytes());
    f.uploader(&budget)
        .journal(f.dir.join("first.pending"))
        .clear_settled()
        .unwrap();
    assert_eq!(budget.snapshot().retained_bytes, 0);
}
