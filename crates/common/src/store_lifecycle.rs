//! Local store preparation and one-way Raft activation.
//!
//! A root descriptor records a prepared disk identity; it cannot mint that
//! identity on another empty disk. Activation is persisted before Raft runs.
//! Cooperating processes hold one exclusive directory lock for the full owner
//! lifetime. File deletion/replacement by unrelated processes is outside that
//! lock contract; malformed or missing records are not repaired by guessing.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{Error, NodeId, Result, RootDescriptor, RootDigest, StoreIdentity, StoreIncarnation};

pub const STORE_LIFECYCLE_FILE: &str = "kv9-store-lifecycle";
const LOCK_FILE: &str = "kv9-store-lock";
const LEGACY_MAGIC: &[u8; 8] = b"KV9LIFE1";
const MAGIC: &[u8; 8] = b"KV9LIFE2";
const PAYLOAD_LEN: usize = 8 + 8 + 16 + 1 + 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorePhase {
    Prepared,
    Bound(RootDigest),
    Active(RootDigest),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StoreRecord {
    pub node_id: NodeId,
    pub incarnation: StoreIncarnation,
    pub phase: StorePhase,
}

impl StoreRecord {
    fn encode(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(PAYLOAD_LEN + 32);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.node_id.0.to_be_bytes());
        bytes.extend_from_slice(self.incarnation.as_bytes());
        let (phase, root) = match self.phase {
            StorePhase::Prepared => (0, RootDigest::from_bytes([0; 32])),
            StorePhase::Bound(root) => (1, root),
            StorePhase::Active(root) => (2, root),
        };
        bytes.push(phase);
        bytes.extend_from_slice(root.as_bytes());
        bytes.extend_from_slice(RootDigest::sha256(&bytes).as_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != PAYLOAD_LEN + 32 || (&bytes[..8] != MAGIC && &bytes[..8] != LEGACY_MAGIC)
        {
            return Err(Error::Config(
                "invalid store lifecycle record format".into(),
            ));
        }
        if RootDigest::sha256(&bytes[..PAYLOAD_LEN]).as_bytes() != &bytes[PAYLOAD_LEN..] {
            return Err(Error::Config("store lifecycle checksum mismatch".into()));
        }
        let node_id = NodeId(u64::from_be_bytes(bytes[8..16].try_into().unwrap()));
        let incarnation = StoreIncarnation::from_bytes(bytes[16..32].try_into().unwrap());
        let root = RootDigest::from_bytes(bytes[33..65].try_into().unwrap());
        let phase = match bytes[32] {
            0 if root.as_bytes() == &[0; 32] => StorePhase::Prepared,
            1 => StorePhase::Bound(root),
            2 => StorePhase::Active(root),
            _ => return Err(Error::Config("invalid store lifecycle phase".into())),
        };
        if node_id.0 == 0 || incarnation.as_bytes() == &[0; 16] {
            return Err(Error::Config(
                "store lifecycle identity must be non-zero".into(),
            ));
        }
        Ok(Self {
            node_id,
            incarnation,
            phase,
        })
    }
}

pub struct StoreGuard {
    // Keep this descriptor open until every user of the store has stopped.
    _lock: File,
    directory: PathBuf,
    record: Option<StoreRecord>,
    legacy_format: bool,
    failed: bool,
}

impl StoreGuard {
    pub fn lock(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory)
            .map_err(|e| Error::Config(format!("create store directory: {e}")))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(LOCK_FILE))
            .map_err(|e| Error::Config(format!("open store ownership lock: {e}")))?;
        lock.try_lock().map_err(|e| {
            Error::Config(format!(
                "store directory already owned or lock unavailable: {e}"
            ))
        })?;
        let path = directory.join(STORE_LIFECYCLE_FILE);
        let (record, legacy_format) = match fs::read(&path) {
            Ok(bytes) => {
                let record = StoreRecord::decode(&bytes)?;
                // Recovered visible state must be durable before it authorizes
                // this process, including a previous failed directory sync.
                File::open(&path)
                    .and_then(|file| file.sync_all())
                    .map_err(|e| Error::Config(format!("sync recovered store lifecycle: {e}")))?;
                crate::fs::sync_ancestors(&crate::fs::OsFileSystem, directory).map_err(|e| {
                    Error::Config(format!("publish recovered store lifecycle: {e}"))
                })?;
                (Some(record), &bytes[..8] == LEGACY_MAGIC)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (None, false),
            Err(e) => return Err(Error::Config(format!("read store lifecycle: {e}"))),
        };
        Ok(Self {
            _lock: lock,
            directory: directory.to_path_buf(),
            record,
            legacy_format,
            failed: false,
        })
    }

    pub fn record(&self) -> Option<StoreRecord> {
        self.record
    }

    /// Persist the V2 format before this process opens its Raft owner. V1
    /// readers reject this header; the V2 wire service rejects V1 processes
    /// that still own other stores. Preserve node, incarnation, phase and root.
    /// A failed publication poisons the guard, including a failed directory
    /// sync after rename, so no owner may start on a merely visible upgrade.
    pub fn fence_legacy_writers(&mut self) -> Result<()> {
        self.fence_legacy_writers_observed(&mut |_, _| Ok(()))
    }

    fn fence_legacy_writers_observed(
        &mut self,
        observe: &mut impl FnMut(PublicationStep, bool) -> Result<()>,
    ) -> Result<()> {
        self.healthy()?;
        if self.legacy_format {
            self.publish_observed(
                self.record
                    .expect("legacy format requires a decoded record"),
                observe,
            )?;
        }
        Ok(())
    }

    pub fn prepare(&mut self, node_id: NodeId) -> Result<StoreRecord> {
        self.healthy()?;
        if node_id.0 == 0 {
            return Err(Error::Config("store node id must be non-zero".into()));
        }
        if let Some(record) = self.record {
            if record.node_id != node_id {
                return Err(Error::Config(
                    "store is prepared for another node id".into(),
                ));
            }
            return Ok(record);
        }
        for entry in fs::read_dir(&self.directory)
            .map_err(|e| Error::Config(format!("inspect store directory: {e}")))?
        {
            let entry = entry.map_err(|e| Error::Config(format!("inspect store entry: {e}")))?;
            if entry.file_name() == LOCK_FILE {
                continue;
            }
            // A crash before the first rename can leave a complete Prepared
            // temporary record. It was never returned as an authority. Preserve
            // it for diagnosis and mint a fresh identity, never import its id.
            let name = entry.file_name();
            let temporary = name.to_str().is_some_and(|name| {
                name.starts_with(&format!(".{STORE_LIFECYCLE_FILE}.")) && name.ends_with(".tmp")
            });
            if temporary && entry.file_type().is_ok_and(|kind| kind.is_file()) {
                let bytes = fs::read(entry.path()).map_err(|e| {
                    Error::Config(format!("read preparation temporary record: {e}"))
                })?;
                if StoreRecord::decode(&bytes).is_ok_and(|record| {
                    record.node_id == node_id && record.phase == StorePhase::Prepared
                }) {
                    continue;
                }
            }
            {
                return Err(Error::Config("store preparation requires an empty data directory; existing state has no lifecycle record".into()));
            }
        }
        let incarnation = loop {
            let incarnation = StoreIncarnation::mint()?;
            if incarnation.as_bytes() != &[0; 16] {
                break incarnation;
            }
        };
        let record = StoreRecord {
            node_id,
            incarnation,
            phase: StorePhase::Prepared,
        };
        self.publish(record)?;
        Ok(record)
    }

    pub fn verify(&self, identity: &StoreIdentity) -> Result<StoreRecord> {
        self.healthy()?;
        let record = self
            .record
            .ok_or_else(|| Error::Config("store has not been independently prepared".into()))?;
        if record.node_id != identity.node_id || record.incarnation != identity.store_incarnation {
            return Err(Error::Config(
                "prepared store identity does not match root/store identity".into(),
            ));
        }
        if matches!(record.phase, StorePhase::Bound(root) | StorePhase::Active(root) if root != identity.root_digest)
        {
            return Err(Error::Config(
                "store is already bound to another root".into(),
            ));
        }
        Ok(record)
    }

    pub fn bind(&mut self, root: &RootDescriptor, identity: &StoreIdentity) -> Result<()> {
        identity.verify(root, identity.node_id)?;
        let record = self.verify(identity)?;
        if record.phase == StorePhase::Prepared {
            self.publish(StoreRecord {
                phase: StorePhase::Bound(root.digest()),
                ..record
            })?;
        }
        Ok(())
    }

    /// Call only after the initial Raft file and its namespace are durable.
    /// An Active store must use recovery-only storage open on every restart.
    pub fn activate(&mut self, identity: &StoreIdentity) -> Result<()> {
        let record = self.verify(identity)?;
        match record.phase {
            StorePhase::Prepared => Err(Error::Config("cannot activate an unbound store".into())),
            StorePhase::Bound(root) => self.publish(StoreRecord {
                phase: StorePhase::Active(root),
                ..record
            }),
            StorePhase::Active(_) => Ok(()),
        }
    }

    /// Migration requires the caller to establish the matching root certificate
    /// from recovered committed Raft/catalog state. A copied identity bundle is
    /// insufficient. This operation cannot overwrite a modern lifecycle record.
    pub fn adopt_certified_recovery(
        &mut self,
        root: &RootDescriptor,
        identity: &StoreIdentity,
    ) -> Result<()> {
        self.healthy()?;
        identity.verify(root, identity.node_id)?;
        if self.record.is_some() {
            return Err(Error::Config("store lifecycle already exists".into()));
        }
        self.publish(StoreRecord {
            node_id: identity.node_id,
            incarnation: identity.store_incarnation,
            phase: StorePhase::Active(root.digest()),
        })
    }

    fn healthy(&self) -> Result<()> {
        if self.failed {
            return Err(Error::Config(
                "store lifecycle publication failed; reopen before retry".into(),
            ));
        }
        Ok(())
    }

    fn publish(&mut self, record: StoreRecord) -> Result<()> {
        self.publish_observed(record, &mut |_, _| Ok(()))
    }

    fn publish_observed(
        &mut self,
        record: StoreRecord,
        observe: &mut impl FnMut(PublicationStep, bool) -> Result<()>,
    ) -> Result<()> {
        self.healthy()?;
        let result = publish_record(&self.directory, &record.encode(), observe);
        match result {
            Ok(()) => {
                self.record = Some(record);
                self.legacy_format = false;
                Ok(())
            }
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }
}

// The observer is an internal deterministic test seam, monomorphized to a
// no-op in production. It cannot be selected through a runtime configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublicationStep {
    Create,
    Write,
    SyncFile,
    Rename,
    SyncDirectory,
}

fn publication_operation<T>(
    observe: &mut impl FnMut(PublicationStep, bool) -> Result<()>,
    step: PublicationStep,
    action: impl FnOnce() -> std::io::Result<T>,
) -> Result<T> {
    observe(step, false)?;
    let value = action().map_err(|e| Error::Config(format!("store lifecycle {step:?}: {e}")))?;
    observe(step, true)?;
    Ok(value)
}

fn publish_record(
    directory: &Path,
    bytes: &[u8],
    observe: &mut impl FnMut(PublicationStep, bool) -> Result<()>,
) -> Result<()> {
    let temporary = directory.join(format!(
        ".{STORE_LIFECYCLE_FILE}.{}.{}.tmp",
        std::process::id(),
        StoreIncarnation::mint()?
    ));
    let result = (|| {
        let mut file = publication_operation(observe, PublicationStep::Create, || {
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
        })?;
        publication_operation(observe, PublicationStep::Write, || file.write_all(bytes))?;
        publication_operation(observe, PublicationStep::SyncFile, || file.sync_all())?;
        publication_operation(observe, PublicationStep::Rename, || {
            fs::rename(&temporary, directory.join(STORE_LIFECYCLE_FILE))
        })?;
        let canonical = directory
            .canonicalize()
            .map_err(|e| Error::Config(format!("canonicalize store lifecycle directory: {e}")))?;
        for ancestor in canonical.ancestors() {
            publication_operation(observe, PublicationStep::SyncDirectory, || {
                File::open(ancestor)?.sync_all()
            })?;
        }
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BootstrapGeneration, ClusterId, RootVoter};

    fn directory() -> PathBuf {
        std::env::temp_dir().join(format!(
            "kv9-store-lifecycle-{}",
            StoreIncarnation::mint().unwrap()
        ))
    }

    fn root(record: StoreRecord) -> RootDescriptor {
        RootDescriptor::new(
            ClusterId::mint().unwrap(),
            BootstrapGeneration::mint().unwrap(),
            vec![RootVoter {
                node_id: record.node_id,
                addr: "127.0.0.1:20160".parse().unwrap(),
                store_incarnation: record.incarnation,
            }],
            b"test-bootstrap",
        )
        .unwrap()
    }

    fn legacy_record(path: &Path, record: StoreRecord) {
        fs::create_dir_all(path).unwrap();
        let mut bytes = record.encode();
        bytes[..8].copy_from_slice(LEGACY_MAGIC);
        let digest = RootDigest::sha256(&bytes[..PAYLOAD_LEN]);
        bytes[PAYLOAD_LEN..].copy_from_slice(digest.as_bytes());
        fs::write(path.join(STORE_LIFECYCLE_FILE), bytes).unwrap();
    }

    #[test]
    fn writer_format_upgrade_preserves_identity_and_is_idempotent() {
        for phase in [
            StorePhase::Prepared,
            StorePhase::Bound(RootDigest::from_bytes([7; 32])),
            StorePhase::Active(RootDigest::from_bytes([7; 32])),
        ] {
            let path = directory();
            let record = StoreRecord {
                node_id: NodeId(4),
                incarnation: StoreIncarnation::mint().unwrap(),
                phase,
            };
            legacy_record(&path, record);
            let mut guard = StoreGuard::lock(&path).unwrap();
            assert!(guard.legacy_format);
            guard.fence_legacy_writers().unwrap();
            assert_eq!(guard.record(), Some(record));
            let bytes = fs::read(path.join(STORE_LIFECYCLE_FILE)).unwrap();
            assert_eq!(
                &bytes[..8],
                MAGIC,
                "writer startup did not fence the legacy lifecycle reader"
            );
            assert_ne!(&bytes[..8], LEGACY_MAGIC);
            assert_eq!(StoreRecord::decode(&bytes).unwrap(), record);
            guard
                .fence_legacy_writers_observed(&mut |_, _| {
                    panic!("idempotent upgrade published again")
                })
                .unwrap();
            drop(guard);
            let recovered = StoreGuard::lock(&path).unwrap();
            assert!(!recovered.legacy_format);
            assert_eq!(recovered.record(), Some(record));
            drop(recovered);
            fs::remove_dir_all(path).unwrap();
        }
    }

    #[test]
    fn every_writer_format_publication_cut_refuses_owner_authority_until_reopen() {
        let record = StoreRecord {
            node_id: NodeId(4),
            incarnation: StoreIncarnation::mint().unwrap(),
            phase: StorePhase::Active(RootDigest::from_bytes([7; 32])),
        };
        let probe = directory();
        legacy_record(&probe, record);
        let mut guard = StoreGuard::lock(&probe).unwrap();
        let mut events = Vec::new();
        guard
            .fence_legacy_writers_observed(&mut |step, after| {
                events.push((step, after));
                Ok(())
            })
            .unwrap();
        assert!(events.len() >= 12);
        drop(guard);
        fs::remove_dir_all(probe).unwrap();
        for cut in 0..events.len() {
            let path = directory();
            legacy_record(&path, record);
            let mut guard = StoreGuard::lock(&path).unwrap();
            let mut reached = 0;
            assert!(guard
                .fence_legacy_writers_observed(&mut |step, after| {
                    assert_eq!((step, after), events[reached]);
                    reached += 1;
                    if reached == cut + 1 {
                        Err(Error::Config("injected format publication error".into()))
                    } else {
                        Ok(())
                    }
                })
                .is_err());
            assert_eq!(reached, cut + 1);
            assert!(
                guard.fence_legacy_writers().is_err(),
                "failed format publication permitted startup without reopen"
            );
            assert!(guard.prepare(record.node_id).is_err());
            drop(guard);
            let mut recovered = StoreGuard::lock(&path).unwrap();
            recovered.fence_legacy_writers().unwrap();
            assert_eq!(recovered.record(), Some(record));
            assert_eq!(
                &fs::read(path.join(STORE_LIFECYCLE_FILE)).unwrap()[..8],
                MAGIC
            );
            drop(recovered);
            fs::remove_dir_all(path).unwrap();
        }
    }

    #[test]
    fn preparation_is_independent_and_activation_is_monotonic_across_restarts() {
        let path = directory();
        let other = directory();
        let mut guard = StoreGuard::lock(&path).unwrap();
        let record = guard.prepare(NodeId(1)).unwrap();
        let root = root(record);
        let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
        assert!(guard.activate(&identity).is_err());
        let mut replacement = StoreGuard::lock(&other).unwrap();
        assert_ne!(
            replacement.prepare(NodeId(1)).unwrap().incarnation,
            record.incarnation
        );
        assert!(
            replacement.bind(&root, &identity).is_err(),
            "another disk copied the root's incarnation"
        );
        assert!(
            StoreGuard::lock(&path).is_err(),
            "a second owner acquired the same store"
        );
        guard.bind(&root, &identity).unwrap();
        guard.activate(&identity).unwrap();
        drop(guard);
        let mut recovered = StoreGuard::lock(&path).unwrap();
        assert_eq!(
            recovered.prepare(NodeId(1)).unwrap().phase,
            StorePhase::Active(root.digest())
        );
        assert!(recovered.prepare(NodeId(2)).is_err());
        let different = self::root(record);
        let changed = StoreIdentity::for_voter(&different, NodeId(1)).unwrap();
        assert!(recovered.bind(&different, &changed).is_err());
        drop(recovered);
        drop(replacement);
        fs::remove_dir_all(path).unwrap();
        fs::remove_dir_all(other).unwrap();
    }

    #[test]
    fn missing_or_corrupt_lifecycle_does_not_prepare_over_existing_state() {
        let path = directory();
        let mut guard = StoreGuard::lock(&path).unwrap();
        fs::write(path.join("raft.log"), b"retained log").unwrap();
        assert!(guard.prepare(NodeId(1)).is_err());
        drop(guard);
        fs::write(path.join(STORE_LIFECYCLE_FILE), b"torn").unwrap();
        assert!(StoreGuard::lock(&path).is_err());
        let record = StoreRecord {
            node_id: NodeId(1),
            incarnation: StoreIncarnation::mint().unwrap(),
            phase: StorePhase::Prepared,
        };
        let bytes = record.encode();
        for offset in 0..bytes.len() {
            let mut corrupt = bytes.clone();
            corrupt[offset] ^= 1;
            assert!(StoreRecord::decode(&corrupt).is_err());
        }
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn publication_errors_poison_authority_and_reopen_stabilizes_visible_state() {
        let probe = directory();
        let mut guard = StoreGuard::lock(&probe).unwrap();
        let prepared = guard.prepare(NodeId(1)).unwrap();
        let root = root(prepared);
        let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
        guard.bind(&root, &identity).unwrap();
        let active = StoreRecord {
            phase: StorePhase::Active(root.digest()),
            ..prepared
        };
        let mut events = Vec::new();
        guard
            .publish_observed(active, &mut |step, after| {
                events.push((step, after));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            &events[..8],
            &[
                (PublicationStep::Create, false),
                (PublicationStep::Create, true),
                (PublicationStep::Write, false),
                (PublicationStep::Write, true),
                (PublicationStep::SyncFile, false),
                (PublicationStep::SyncFile, true),
                (PublicationStep::Rename, false),
                (PublicationStep::Rename, true),
            ]
        );
        assert!(events[8..]
            .iter()
            .all(|(step, _)| *step == PublicationStep::SyncDirectory));
        assert!(
            events.len() >= 12,
            "store directory and parent were not synchronized"
        );
        drop(guard);
        fs::remove_dir_all(probe).unwrap();
        for cut in 0..events.len() {
            for errno in [5, 28] {
                let path = directory();
                let mut guard = StoreGuard::lock(&path).unwrap();
                let prepared = guard.prepare(NodeId(1)).unwrap();
                let root = self::root(prepared);
                let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
                guard.bind(&root, &identity).unwrap();
                let bound = guard.record().unwrap();
                let active = StoreRecord {
                    phase: StorePhase::Active(root.digest()),
                    ..prepared
                };
                let mut reached = 0;
                let error = guard
                    .publish_observed(active, &mut |step, after| {
                        assert_eq!((step, after), events[reached]);
                        reached += 1;
                        if reached == cut + 1 {
                            return Err(Error::Config(format!(
                                "injected lifecycle error: {}",
                                std::io::Error::from_raw_os_error(errno)
                            )));
                        }
                        Ok(())
                    })
                    .unwrap_err();
                assert!(error.to_string().contains(&format!("os error {errno}")));
                assert_eq!(reached, cut + 1);
                assert_eq!(
                    guard.record(),
                    Some(bound),
                    "failed publication exposed new authority"
                );
                assert!(
                    guard.verify(&identity).is_err(),
                    "failed guard still authorized a store"
                );
                assert!(
                    guard.activate(&identity).is_err(),
                    "failed activation was retried in place"
                );
                drop(guard);
                let recovered = StoreGuard::lock(&path).unwrap();
                let expected = if cut >= 7 { active } else { bound };
                assert_eq!(recovered.verify(&identity).unwrap(), expected);
                drop(recovered);
                fs::remove_dir_all(path).unwrap();
            }
        }
    }

    #[test]
    fn interrupted_first_publication_never_imports_an_orphan_identity() {
        let path = directory();
        let mut guard = StoreGuard::lock(&path).unwrap();
        let orphan = StoreRecord {
            node_id: NodeId(1),
            incarnation: StoreIncarnation::mint().unwrap(),
            phase: StorePhase::Prepared,
        };
        let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            guard
                .publish_observed(orphan, &mut |step, after| {
                    assert!(
                        !(step == PublicationStep::SyncFile && after),
                        "simulated crash after temporary file sync"
                    );
                    Ok(())
                })
                .unwrap();
        }));
        assert!(crash.is_err());
        drop(guard);
        assert!(!path.join(STORE_LIFECYCLE_FILE).exists());
        let mut recovered = StoreGuard::lock(&path).unwrap();
        assert!(recovered.record().is_none());
        let prepared = recovered.prepare(NodeId(1)).unwrap();
        assert_ne!(
            prepared.incarnation, orphan.incarnation,
            "orphan became a returned authority"
        );
        let old_root = root(orphan);
        let identity = StoreIdentity::for_voter(&old_root, NodeId(1)).unwrap();
        assert!(recovered.bind(&old_root, &identity).is_err());
        drop(recovered);
        fs::remove_dir_all(path).unwrap();
    }
}
