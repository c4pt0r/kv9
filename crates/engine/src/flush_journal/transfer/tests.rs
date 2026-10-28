use super::*;
use crate::{MemoryObjectStore, ObjectStore};
use std::sync::Arc;

fn fixture() -> (tempfile::TempDir, RemoteUploader, SpooledFlush) {
    let (old, engine, plan) = super::super::tests::fixture();
    let dir = tempfile::tempdir().unwrap();
    let uploader = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()));
    let spool = uploader
        .plan_spooled_in(
            engine.freeze(plan.manifest.scope.clone()).unwrap(),
            dir.path(),
        )
        .unwrap();
    drop(engine);
    fs::remove_dir_all(old).unwrap();
    (dir, uploader, spool)
}

#[test]
fn streamed_replica_preserves_exact_file_and_holds_export_lock_and_inode_charge() {
    let (dir, uploader, mut spool) = fixture();
    let budget = Arc::new(PendingDiskBudget::new(1024 * 1024).unwrap());
    let uploader = uploader.with_pending_budget(budget.clone());
    let source = dir.path().join("capture.plan");
    let replica = dir.path().join("capture-replica.plan");
    let mut journal = uploader.transfer_journal(&source).unwrap();
    journal.stage_spooled(b"original cut", &mut spool).unwrap();
    let expected = fs::read(&source).unwrap();
    let reader = journal.into_reader().unwrap().unwrap();
    assert_eq!(reader.file_len(), expected.len() as u64);
    assert_eq!(
        TransferJournal::encoded_len(b"original cut", spool.manifest()).unwrap(),
        reader.file_len()
    );
    assert!(uploader.transfer_journal(&source).is_err());
    fs::remove_file(&source).unwrap();
    budget.forgotten(&source);
    fs::write(&source, b"unrelated new inode").unwrap();
    assert_eq!(budget.snapshot().retained_bytes, expected.len() as u64);
    struct Fragmented<R>(R);
    impl<R: Read> Read for Fragmented<R> {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            let n = bytes.len().min(7);
            self.0.read(&mut bytes[..n])
        }
    }
    let mut destination = uploader.transfer_journal(&replica).unwrap();
    destination
        .stage_received(b"original cut", spool.manifest(), Fragmented(reader))
        .unwrap();
    assert_eq!(fs::read(&replica).unwrap(), expected);
    assert_eq!(budget.snapshot().open_readers, 0);
    assert_eq!(budget.snapshot().retained_bytes, expected.len() as u64);
    destination
        .stage_received(b"original cut", spool.manifest(), std::io::empty())
        .unwrap();
    assert!(destination
        .stage_received(b"another cut", spool.manifest(), std::io::empty())
        .is_err());
    assert_eq!(
        destination
            .load()
            .unwrap()
            .unwrap()
            .upload(&uploader)
            .unwrap()
            .descriptor(),
        *spool.manifest()
    );
    let enclosing_scope = destination.hold_lock();
    drop(destination);
    assert!(uploader.transfer_journal(&replica).is_err());
    drop(enclosing_scope);
    assert!(uploader
        .transfer_journal(&replica)
        .unwrap()
        .load()
        .unwrap()
        .is_some());
}

#[test]
fn received_truncation_trailing_corruption_and_foreign_binding_never_select_a_plan() {
    let (dir, uploader, mut spool) = fixture();
    let source = dir.path().join("capture.plan");
    uploader
        .transfer_journal(&source)
        .unwrap()
        .stage_spooled(b"selected cut", &mut spool)
        .unwrap();
    let bytes = fs::read(source).unwrap();
    let replica = dir.path().join("capture-replica.plan");
    let mut target = uploader.transfer_journal(&replica).unwrap();
    for n in 0..bytes.len() {
        assert!(target
            .stage_received(b"selected cut", spool.manifest(), &bytes[..n])
            .is_err());
        assert!(target.load().unwrap().is_none());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(target
        .stage_received(b"selected cut", spool.manifest(), extra.as_slice())
        .is_err());
    let mut bad_sst = bytes.clone();
    let n = bad_sst.len();
    bad_sst[n - HASH_LEN - 1] ^= 1;
    let hash = Sha256::digest(&bad_sst[..n - HASH_LEN]);
    bad_sst[n - HASH_LEN..].copy_from_slice(&hash);
    assert!(target
        .stage_received(b"selected cut", spool.manifest(), bad_sst.as_slice())
        .is_err());
    assert!(target
        .stage_received(b"other record", spool.manifest(), bytes.as_slice())
        .is_err());
    assert!(target.load().unwrap().is_none());
    target
        .stage_received(b"selected cut", spool.manifest(), bytes.as_slice())
        .unwrap();
    assert_eq!(fs::read(replica).unwrap(), bytes);
}

#[test]
fn receive_publication_faults_reopen_only_complete_files_and_preserve_disk_debt() {
    for cut in [
        PublicationStep::Written,
        PublicationStep::FileSynced,
        PublicationStep::Renamed,
        PublicationStep::DirectorySynced,
    ] {
        let (dir, uploader, mut spool) = fixture();
        let source = dir.path().join("capture.plan");
        uploader
            .transfer_journal(&source)
            .unwrap()
            .stage_spooled(b"cut", &mut spool)
            .unwrap();
        let bytes = fs::read(source).unwrap();
        let path = dir.path().join("capture-replica.plan");
        let mut target = uploader.transfer_journal(&path).unwrap();
        assert!(target
            .receive_observed(b"cut", spool.manifest(), bytes.as_slice(), &mut |at| {
                if at == cut {
                    Err(invalid("injected receive publication error"))
                } else {
                    Ok(())
                }
            })
            .is_err());
        drop(target);
        let budget = Arc::new(PendingDiskBudget::new(1).unwrap());
        budget.inventory_journal(&path).unwrap();
        assert_eq!(budget.snapshot().retained_bytes, bytes.len() as u64);
        let uploader = uploader.with_pending_budget(budget.clone());
        let mut target = uploader.transfer_journal(&path).unwrap();
        let loaded = target.load().unwrap();
        assert_eq!(
            loaded.is_some(),
            matches!(
                cut,
                PublicationStep::Renamed | PublicationStep::DirectorySynced
            )
        );
        if let Some(loaded) = loaded {
            assert_eq!(loaded.metadata(), b"cut");
            assert_eq!(loaded.manifest(), spool.manifest());
        }
        target.clear_settled().unwrap();
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert!(target
            .stage_received(b"cut", spool.manifest(), bytes.as_slice())
            .is_err());
        assert!(target.load().unwrap().is_none());
    }
}

#[test]
fn one_atomic_plan_survives_reopen_and_cannot_be_replaced_or_opened_twice() {
    let (dir, uploader, mut spool) = fixture();
    let path = dir.path().join("capture.plan");
    let mut journal = uploader.transfer_journal(&path).unwrap();
    journal.stage_spooled(b"fixed record", &mut spool).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(uploader.transfer_journal(&path).is_err());
    journal.stage_spooled(b"fixed record", &mut spool).unwrap();
    assert!(journal.stage_spooled(b"new record", &mut spool).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
    // A capture cannot be mistaken for a checkpoint reclamation candidate.
    assert!(uploader.journal(&path).load().is_err());
    drop(journal);
    let objects = Arc::new(MemoryObjectStore::new());
    let reboot = RemoteUploader::for_harness(objects.clone());
    let journal = reboot.transfer_journal(&path).unwrap();
    let pending = journal.load().unwrap().unwrap();
    assert_eq!(pending.metadata(), b"fixed record");
    assert_eq!(pending.manifest(), spool.manifest());
    assert!(objects.list("").unwrap().is_empty());
    assert_eq!(
        pending.upload(&reboot).unwrap().descriptor(),
        *spool.manifest()
    );
    assert_eq!(objects.list("").unwrap().len(), spool.manifest.files.len());
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn every_publication_fault_recovers_either_no_plan_or_the_exact_whole_plan() {
    for cut in [
        PublicationStep::Written,
        PublicationStep::FileSynced,
        PublicationStep::Renamed,
        PublicationStep::DirectorySynced,
    ] {
        let (dir, uploader, mut spool) = fixture();
        let path = dir.path().join("capture.plan");
        let mut journal = uploader.transfer_journal(&path).unwrap();
        assert!(journal
            .stage_observed(b"one cut", &mut spool, &mut |at| {
                if at == cut {
                    Err(invalid("injected publication failure"))
                } else {
                    Ok(())
                }
            })
            .is_err());
        drop(journal);
        let budget = Arc::new(PendingDiskBudget::new(1).unwrap());
        budget.inventory_journal(&path).unwrap();
        assert!(budget.snapshot().retained_bytes > 1);
        let reboot = RemoteUploader::for_harness(Arc::new(MemoryObjectStore::new()))
            .with_pending_budget(budget.clone());
        let mut journal = reboot.transfer_journal(&path).unwrap();
        let loaded = journal.load().unwrap();
        let published = matches!(
            cut,
            PublicationStep::Renamed | PublicationStep::DirectorySynced
        );
        assert_eq!(loaded.is_some(), published);
        if let Some(pending) = loaded {
            assert_eq!(pending.metadata(), b"one cut");
            assert_eq!(pending.manifest(), spool.manifest());
            pending.upload(&reboot).unwrap();
        } else {
            // Failed tmp debt can be removed, but the lowered budget still
            // refuses new publication without returning a plan.
            assert!(journal.stage_spooled(b"one cut", &mut spool).is_err());
            assert!(journal.load().unwrap().is_none());
        }
        journal.clear_settled().unwrap();
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }
}

#[test]
fn corruption_truncation_and_checksum_valid_bad_lengths_never_replan() {
    let (dir, uploader, mut spool) = fixture();
    let path = dir.path().join("capture.plan");
    let mut journal = uploader.transfer_journal(&path).unwrap();
    journal.stage_spooled(b"original", &mut spool).unwrap();
    let bytes = fs::read(&path).unwrap();
    for end in 0..bytes.len() {
        fs::write(&path, &bytes[..end]).unwrap();
        assert!(journal.load().is_err(), "truncation {end}");
    }
    for offset in [0, 8, 12, bytes.len() - 33, bytes.len() - 1] {
        let mut corrupt = bytes.clone();
        corrupt[offset] ^= 1;
        fs::write(&path, &corrupt).unwrap();
        assert!(journal.load().is_err());
        assert!(journal.stage_spooled(b"replacement", &mut spool).is_err());
        assert_eq!(fs::read(&path).unwrap(), corrupt);
    }
    let mut oversized = bytes.clone();
    oversized[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    let end = oversized.len() - HASH_LEN;
    let hash = Sha256::digest(&oversized[..end]);
    oversized[end..].copy_from_slice(&hash);
    fs::write(&path, oversized).unwrap();
    assert!(journal
        .load()
        .unwrap_err()
        .to_string()
        .contains("metadata length"));
}

#[test]
fn loaded_inode_stays_charged_and_cannot_be_redirected_by_path_replacement() {
    let (dir, uploader, mut spool) = fixture();
    let path = dir.path().join("capture.plan");
    let budget = Arc::new(PendingDiskBudget::new(1024 * 1024).unwrap());
    let uploader = uploader.with_pending_budget(budget.clone());
    let mut journal = uploader.transfer_journal(&path).unwrap();
    journal.stage_spooled(b"original", &mut spool).unwrap();
    let pending = journal.load().unwrap().unwrap();
    let bytes = fs::metadata(&path).unwrap().len();
    journal.clear_settled().unwrap();
    assert_eq!(budget.snapshot().retained_bytes, bytes);
    fs::write(&path, b"foreign inode").unwrap();
    pending.upload(&uploader).unwrap();
    assert_eq!(budget.snapshot().retained_bytes, 0);
    assert!(journal.load().is_err());
}
