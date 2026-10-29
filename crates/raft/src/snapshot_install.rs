//! Offline, destination-bound preparation of an immutable engine/protocol pair.
//!
//! This module grants NO admission, voting, serving, or retention capability.
//! Source authority and durable object owners remain the caller's obligations.
//! Ordinary RaftPeer constructors refuse these bases; runtime adoption uses
//! the separate validated installed-base constructors. The supported destination
//! is a new group or a prior installation
//! made by this module; an existing flat active group is never adopted.
//!
//! A versioned group record fences older RegionManager readers. Each attempt
//! prepares an independent immutable generation. Only after the image, engine,
//! protocol and file hashes are durable do we rename and sync one selector.
//! Uncertain publication poisons the live owner. Recovery validates the selected
//! generation exactly and never falls back to an older one or recreates a file.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use kv9_common::data_range::{DataRange, RANGE_KEY};
use kv9_common::raw_ttl::ClockWatermark;
use kv9_common::store_lifecycle::{StoreGuard, StorePhase};
use kv9_common::{AppliedPosition, Error, Result, RootDigest, StoreIdentity, StoreIncarnation};
use kv9_engine::checkpoint::{CheckpointManifest, CheckpointRestore, RemoteUploader};
use kv9_engine::{
    ColumnFamily, DurableAppliedPosition, Engine, ReadView, ReplicatedEngine, WalEngine,
};
use raft::prelude::{HardState, Snapshot};
use raft::Storage;

use crate::state_machine::checkpoint_recovery::data_group::{
    inspect_image, DataCheckpointIdentity, InspectedDataImage,
};
use crate::storage::{snapshot, DiskRaftStorage, MAX_PROTOCOL_SNAPSHOT_BYTES};

const MAX_IMAGE: usize = MAX_PROTOCOL_SNAPSHOT_BYTES + 128;
const MAX_GENERATIONS: usize = 8;
const SELECTOR: &str = "installed-generation";
const ADOPTED: &str = "runtime-adopted";
const FILES: [&str; 3] = ["data.wal", "data.checkpoint", "raft/raft.log"];

fn invalid(message: &str) -> Error {
    Error::Raft(format!("joint snapshot installation: {message}"))
}
fn io(error: std::io::Error) -> Error {
    invalid(&error.to_string())
}
fn sync_dir(path: &Path) -> Result<()> {
    kv9_common::fs::sync_ancestors(&kv9_common::fs::OsFileSystem, path).map_err(io)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)
}
fn read(path: &Path, limit: usize) -> Result<Vec<u8>> {
    // All paths are derived locally while the parent and group locks are held.
    // A selected missing file is an error, never permission to initialize one.
    if !fs::symlink_metadata(path).map_err(io)?.is_file() {
        return Err(invalid("expected a regular generation file"));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > limit {
        return Err(invalid("oversized generation file"));
    }
    Ok(bytes)
}
fn checksum(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.extend_from_slice(RootDigest::sha256(&bytes).as_bytes());
    bytes
}
fn checked<'a>(bytes: &'a [u8], magic: &[u8; 8]) -> Result<&'a [u8]> {
    if bytes.len() < 40 || &bytes[..8] != magic {
        return Err(invalid("invalid record envelope"));
    }
    let end = bytes.len() - 32;
    if RootDigest::sha256(&bytes[..end]).as_bytes() != &bytes[end..] {
        return Err(invalid("record checksum mismatch"));
    }
    Ok(&bytes[8..end])
}

/// Canonical per-data-group payload. This binds an image, not its authority.
/// The enclosing Snapshot metadata must describe this manifest's exact cut.
pub fn data_image(range: &DataRange, manifest: &CheckpointManifest) -> Result<Vec<u8>> {
    range.validate()?;
    if manifest.scope.region != range.region.0
        || manifest.scope.conf_ver != range.conf_ver
        || manifest.scope.version != range.version
    {
        return Err(invalid("manifest and range scope disagree"));
    }
    let manifest_bytes = manifest.encode()?;
    if CheckpointManifest::decode(&manifest_bytes)? != *manifest {
        return Err(invalid("invalid manifest"));
    }
    let range_bytes = range.encode();
    let mut bytes = b"KV9DIMG1".to_vec();
    bytes.extend_from_slice(&(range_bytes.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&range_bytes);
    bytes.extend_from_slice(&manifest_bytes);
    Ok(bytes)
}
fn parse_image(image: &Snapshot) -> Result<(DataRange, CheckpointManifest)> {
    let bytes = image.data.as_ref();
    if bytes.len() < 12 || bytes.len() > MAX_PROTOCOL_SNAPSHOT_BYTES || &bytes[..8] != b"KV9DIMG1" {
        return Err(invalid("invalid data image"));
    }
    let n = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let range = DataRange::decode(
        bytes
            .get(12..12 + n)
            .ok_or_else(|| invalid("truncated range"))?,
    )?;
    let manifest = CheckpointManifest::decode(
        bytes
            .get(12 + n..)
            .ok_or_else(|| invalid("truncated manifest"))?,
    )?;
    if data_image(&range, &manifest)? != bytes
        || image.get_metadata().index != manifest.index
        || image.get_metadata().term != manifest.term
    {
        return Err(invalid(
            "noncanonical image or different protocol/image cut",
        ));
    }
    Ok((range, manifest))
}

/// Diagnostics only. No storage handle or Raft-start capability is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledImage {
    pub generation: StoreIncarnation,
    pub image_digest: RootDigest,
    pub position: AppliedPosition,
    pub target: StoreIdentity,
    /// Raw values only; expiry and other system records are excluded.
    pub records: u64,
    /// Observation from the verified image, not a clock or serving authority.
    pub ttl_watermark: Option<ClockWatermark>,
    pub object_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub enum InstallStep {
    AfterEngine,
    AfterProtocol,
    BeforeSelector,
    AfterSelectorRename,
    AfterSelectorSync,
}

/// The borrow keeps the exclusive parent store owner alive. The child lock is
/// the same lock used by RegionManager, not a parallel installation-only lock.
pub struct JointInstaller<'a> {
    _store: &'a StoreGuard,
    _lock: File,
    directory: PathBuf,
    identity: StoreIdentity,
    range: DataRange,
    binding: RootDigest,
    failed: bool,
}

impl<'a> JointInstaller<'a> {
    pub fn open(store: &'a StoreGuard, identity: StoreIdentity, range: DataRange) -> Result<Self> {
        Self::open_inner(store, identity, range, false)
    }

    fn open_inner(
        store: &'a StoreGuard,
        identity: StoreIdentity,
        range: DataRange,
        allow_adopted: bool,
    ) -> Result<Self> {
        range.validate()?;
        store.require_current_writer_format()?;
        let record = store.verify(&identity)?;
        if !matches!(record.phase, StorePhase::Bound(root) | StorePhase::Active(root) if root == range.root)
            || identity.root_digest != range.root
        {
            return Err(invalid("destination is not bound to the image root"));
        }
        let directory = store
            .directory()
            .join("data-groups")
            .join(range.region.0.to_string());
        fs::create_dir_all(&directory).map_err(io)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("group-lock"))
            .map_err(io)?;
        lock.try_lock()
            .map_err(|e| invalid(&format!("group owner lock: {e}")))?;
        // Adoption is one-way: once a generation is runtime storage, this
        // module never installs, replaces or re-verifies files for the group.
        if !allow_adopted && directory.join(ADOPTED).try_exists().map_err(io)? {
            return Err(invalid(
                "group generation is runtime-adopted; installation is closed",
            ));
        }
        let mut record = b"KV9INS01".to_vec();
        record.extend_from_slice(identity.cluster_id.as_bytes());
        record.extend_from_slice(&identity.node_id.0.to_be_bytes());
        record.extend_from_slice(identity.store_incarnation.as_bytes());
        record.extend_from_slice(&range.encode());
        let record = checksum(record);
        let path = directory.join("group-record");
        if path.try_exists().map_err(io)? {
            if read(&path, 256 * 1024)? != record {
                return Err(invalid(
                    "destination group is active, foreign or uses a different format",
                ));
            }
            File::open(path).map_err(io)?.sync_all().map_err(io)?;
        } else {
            for entry in fs::read_dir(&directory).map_err(io)? {
                let name = entry.map_err(io)?.file_name();
                if name != "group-lock" && name != ".install-record.tmp" {
                    return Err(invalid(
                        "new installation cannot adopt existing group files",
                    ));
                }
            }
            // Before this rename there is no generation data. A torn marker
            // temporary may safely be replaced on recovery under both locks.
            let temporary = directory.join(".install-record.tmp");
            let mut file = File::create(&temporary).map_err(io)?;
            file.write_all(&record).map_err(io)?;
            file.sync_all().map_err(io)?;
            fs::rename(temporary, path).map_err(io)?;
        }
        sync_dir(&directory)?;
        Ok(Self {
            _store: store,
            _lock: lock,
            directory,
            identity,
            range,
            binding: RootDigest::sha256(&record),
            failed: false,
        })
    }

    fn healthy(&self) -> Result<()> {
        if self.failed {
            Err(invalid(
                "uncertain installation requires reopening the owner",
            ))
        } else {
            Ok(())
        }
    }

    fn validate(&self, image: &Snapshot, hs: &HardState) -> Result<(Vec<u8>, CheckpointManifest)> {
        let bytes = snapshot::encode(image, hs)?;
        let (range, manifest) = parse_image(image)?;
        let cs = image.get_metadata().get_conf_state();
        let target = self.identity.node_id.0;
        if range != self.range
            || manifest.scope.cluster != self.identity.cluster_id.to_string()
            || !(cs.voters.contains(&target)
                || cs.voters_outgoing.contains(&target)
                || cs.learners.contains(&target))
        {
            return Err(invalid(
                "image does not match the destination scope or membership",
            ));
        }
        Ok((bytes, manifest))
    }

    fn selected(&self) -> Result<Option<(StoreIncarnation, RootDigest)>> {
        let path = self.directory.join(SELECTOR);
        if !path.try_exists().map_err(io)? {
            return Ok(None);
        }
        let bytes = read(&path, 120)?;
        let fields = checked(&bytes, b"KV9SEL01")?;
        if fields.len() != 80 || &fields[..32] != self.binding.as_bytes() {
            return Err(invalid("selector belongs to a different destination"));
        }
        File::open(path).map_err(io)?.sync_all().map_err(io)?;
        sync_dir(&self.directory)?;
        Ok(Some((
            StoreIncarnation::from_bytes(fields[32..48].try_into().unwrap()),
            RootDigest::from_bytes(fields[48..80].try_into().unwrap()),
        )))
    }

    fn check_read_floor(&self, manifest: &CheckpointManifest) -> Result<()> {
        crate::state_machine::checkpoint_recovery::data_group::check_local_read_floor(
            self._store.data_read_fences(&self.identity)?.as_ref(),
            DataCheckpointIdentity {
                cluster: self.identity.cluster_id,
                root: self.range.root,
                region: self.range.region,
                creation: self.range.creation,
            },
            Some(manifest),
        )
    }

    fn verify_generation(
        &self,
        generation: StoreIncarnation,
        digest: RootDigest,
        uploader: &dyn CheckpointRestore,
    ) -> Result<(InstalledImage, Snapshot, HardState)> {
        let directory = self.directory.join(format!("install-{generation}"));
        if !fs::symlink_metadata(&directory).map_err(io)?.is_dir() {
            return Err(invalid("selected generation is not a directory"));
        }
        let bytes = read(&directory.join("image"), MAX_IMAGE)?;
        if RootDigest::sha256(&bytes) != digest {
            return Err(invalid("selected image digest mismatch"));
        }
        let (image, hs) = snapshot::decode(&bytes)?;
        let (_, manifest) = self.validate(&image, &hs)?;
        self.check_read_floor(&manifest)?;
        let ready = read(&directory.join("ready"), 168)?;
        let fields = checked(&ready, b"KV9PAIR1")?;
        if fields.len() != 128 || &fields[..32] != self.binding.as_bytes() {
            return Err(invalid("generation is not sealed for this destination"));
        }
        for (n, name) in FILES.iter().enumerate() {
            let bytes = read(&directory.join(name), MAX_IMAGE)?;
            if RootDigest::sha256(&bytes).as_bytes() != &fields[32 + n * 32..64 + n * 32] {
                return Err(invalid("selected generation file changed or is incomplete"));
            }
        }
        if read(&directory.join("data.checkpoint"), MAX_IMAGE)? != manifest.encode()? {
            return Err(invalid("engine checkpoint differs from protocol image"));
        }
        let storage = DiskRaftStorage::recover(&directory.join("raft"))?;
        let state = storage
            .initial_state()
            .map_err(|e| invalid(&e.to_string()))?;
        if state.hard_state != hs
            || state.conf_state != *image.get_metadata().get_conf_state()
            || storage
                .snapshot(0, self.identity.node_id.0)
                .map_err(|e| invalid(&e.to_string()))?
                != image
            || storage.first_index().map_err(|e| invalid(&e.to_string()))? != manifest.index + 1
            || storage.last_index().map_err(|e| invalid(&e.to_string()))? != manifest.index
        {
            return Err(invalid("selected protocol base differs from engine image"));
        }
        let mut inspected = None;
        let (engine, replay) = WalEngine::open_with_checkpoint_reader(
            directory.join("data.wal"),
            Some(uploader),
            |base| {
                if base == Some(&manifest) {
                    Ok(())
                } else {
                    Err(invalid("missing engine base"))
                }
            },
            |_, view| {
                inspected = Some(validate_view(view, &self.range, &manifest, self.identity)?);
                Ok(())
            },
            |_, _| Err(invalid("immutable installation contains a WAL tail")),
        )?;
        if engine.applied_position()? != DurableAppliedPosition::AppliedThrough(manifest.position())
            || replay.replayed_records != 0
            || replay.covered_records != 0
            || replay.discarded_tail_bytes != 0
        {
            return Err(invalid("engine position or immutable WAL differs"));
        }
        let inspected = inspected.ok_or_else(|| invalid("missing restored image inspection"))?;
        Ok((
            InstalledImage {
                generation,
                image_digest: digest,
                position: manifest.position(),
                target: self.identity,
                records: inspected.raw_values,
                ttl_watermark: inspected.watermark,
                object_bytes: manifest.files.iter().map(|f| f.size).sum(),
            },
            image,
            hs,
        ))
    }

    /// Revalidate all selected files and remote objects. No selector means that
    /// preparation did not publish; unselected files remain retained for audit.
    pub fn recover(&mut self, uploader: &RemoteUploader) -> Result<Option<InstalledImage>> {
        self.healthy()?;
        let result = (|| {
            self.selected()?
                .map(|(g, d)| self.verify_generation(g, d, uploader).map(|v| v.0))
                .transpose()
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    pub fn install(
        &mut self,
        image: &Snapshot,
        hs: &HardState,
        uploader: &RemoteUploader,
    ) -> Result<InstalledImage> {
        self.install_inner(image, hs, uploader, &mut |_, _| Ok(()))
    }

    /// Install through a caller's GET-only, retained image reader. This same
    /// dependency covers the old selected image, preparation and final verify.
    pub fn install_with_reader(
        &mut self,
        image: &Snapshot,
        hs: &HardState,
        reader: &dyn CheckpointRestore,
    ) -> Result<InstalledImage> {
        self.install_inner(image, hs, reader, &mut |_, _| Ok(()))
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn install_observed(
        &mut self,
        image: &Snapshot,
        hs: &HardState,
        uploader: &RemoteUploader,
        observe: &mut impl FnMut(InstallStep, StoreIncarnation) -> Result<()>,
    ) -> Result<InstalledImage> {
        self.install_inner(image, hs, uploader, observe)
    }

    fn install_inner(
        &mut self,
        image: &Snapshot,
        hs: &HardState,
        uploader: &dyn CheckpointRestore,
        observe: &mut impl FnMut(InstallStep, StoreIncarnation) -> Result<()>,
    ) -> Result<InstalledImage> {
        self.healthy()?;
        let (bytes, manifest) = self.validate(image, hs)?;
        self.check_read_floor(&manifest)?;
        let digest = RootDigest::sha256(&bytes);
        let result = (|| {
            let mut previous_watermark = None;
            if let Some((g, d)) = self.selected()? {
                let (old, old_image, old_hs) = self.verify_generation(g, d, uploader)?;
                if d == digest {
                    return Ok(old);
                }
                if hs.term < old_hs.term
                    || (hs.term == old_hs.term && old_hs.vote != 0 && hs.vote != old_hs.vote)
                    || manifest.index <= old_hs.commit
                    || manifest.term < old_image.get_metadata().term
                {
                    return Err(invalid(
                        "replacement rolls back committed cut, term or vote",
                    ));
                }
                previous_watermark = old.ttl_watermark;
            }
            let count =
                fs::read_dir(&self.directory)
                    .map_err(io)?
                    .try_fold(0usize, |n, entry| {
                        Ok::<_, Error>(
                            n + usize::from(
                                entry
                                    .map_err(io)?
                                    .file_name()
                                    .to_string_lossy()
                                    .starts_with("install-"),
                            ),
                        )
                    })?;
            if count >= MAX_GENERATIONS {
                return Err(invalid("retained generation budget exhausted"));
            }
            let generation = StoreIncarnation::mint()?;
            let directory = self.directory.join(format!("install-{generation}"));
            fs::create_dir(&directory).map_err(io)?;
            write_new(&directory.join("image"), &bytes)?;
            // Open a NEW, private, unselected WAL, then persist its full base.
            // The final validation below checks exact bytes and both watermarks.
            drop(WalEngine::open(directory.join("data.wal"))?);
            write_new(&directory.join("data.checkpoint"), &manifest.encode()?)?;
            let (engine, _) = WalEngine::open_with_checkpoint_reader(
                directory.join("data.wal"),
                Some(uploader),
                |_| Ok(()),
                |_, view| {
                    let inspected = validate_view(view, &self.range, &manifest, self.identity)?;
                    if previous_watermark
                        .is_some_and(|old| inspected.watermark.is_none_or(|next| next < old))
                    {
                        return Err(invalid(
                            "replacement removes or regresses TTL activation/watermark",
                        ));
                    }
                    Ok(())
                },
                |_, _| Err(invalid("new generation unexpectedly contains WAL records")),
            )?;
            if engine.applied_position()?
                != DurableAppliedPosition::AppliedThrough(manifest.position())
            {
                return Err(invalid("prepared engine has a different position"));
            }
            drop(engine);
            sync_dir(&directory)?;
            observe(InstallStep::AfterEngine, generation)?;
            let (storage, pristine) = DiskRaftStorage::open(&directory.join("raft"), &[])?;
            if !pristine {
                return Err(invalid("new generation already has protocol history"));
            }
            storage.install_protocol_snapshot(image, hs)?;
            drop(storage);
            sync_dir(&directory)?;
            observe(InstallStep::AfterProtocol, generation)?;
            let mut ready = b"KV9PAIR1".to_vec();
            ready.extend_from_slice(self.binding.as_bytes());
            for name in FILES {
                ready.extend_from_slice(
                    RootDigest::sha256(&read(&directory.join(name), MAX_IMAGE)?).as_bytes(),
                );
            }
            write_new(&directory.join("ready"), &checksum(ready))?;
            sync_dir(&directory)?;
            let (observation, _, _) = self.verify_generation(generation, digest, uploader)?;
            let mut selector = b"KV9SEL01".to_vec();
            selector.extend_from_slice(self.binding.as_bytes());
            selector.extend_from_slice(generation.as_bytes());
            selector.extend_from_slice(digest.as_bytes());
            // A fresh name per attempt avoids overwriting another uncertain
            // publication's retained temporary. Generation count bounds these.
            let temporary = self.directory.join(format!(".selector-{generation}"));
            write_new(&temporary, &checksum(selector))?;
            observe(InstallStep::BeforeSelector, generation)?;
            fs::rename(temporary, self.directory.join(SELECTOR)).map_err(io)?;
            observe(InstallStep::AfterSelectorRename, generation)?;
            sync_dir(&self.directory)?;
            observe(InstallStep::AfterSelectorSync, generation)?;
            Ok(observation)
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
}

fn validate_view(
    view: &dyn ReadView,
    range: &DataRange,
    manifest: &CheckpointManifest,
    identity: StoreIdentity,
) -> Result<InspectedDataImage> {
    if view.get(ColumnFamily::Default, RANGE_KEY)?.as_deref() != Some(range.encode().as_slice()) {
        return Err(invalid(
            "restored image has a different ownership descriptor",
        ));
    }
    // Restore has verified the complete SST closure. Reuse the same TTL,
    // namespace, CF, history and count rules as ordinary group recovery;
    // source expiry identities and deadlines must survive unchanged.
    inspect_image(
        view,
        manifest,
        DataCheckpointIdentity {
            cluster: identity.cluster_id,
            root: range.root,
            region: range.region,
            creation: range.creation,
        },
    )
}

/// The one-way receipt turning a SELECTED installed generation into the
/// group's runtime storage. The returned paths are the live storage; the
/// held lock is the same per-group lock RegionManager uses and must stay
/// alive for the group's lifetime. No peer, serving or voting is started
/// here; the validated constructors (`RaftPeer::with_installed_storage`,
/// `NodeDriver::with_installed_base`) own that separately.
pub struct AdoptedGeneration {
    pub generation: StoreIncarnation,
    pub range: DataRange,
    pub cut: AppliedPosition,
    pub configuration: raft::prelude::ConfState,
    pub image_digest: RootDigest,
    /// sha256 of the canonical manifest bytes — the retention owner subject
    /// this image was pinned under, for destination-install evidence.
    pub subject: [u8; 32],
    pub raft_directory: PathBuf,
    pub engine_wal: PathBuf,
    pub group_lock: File,
    recovery: recovery::Anchor,
    data_read_fences: std::sync::Arc<kv9_common::data_read_fence::DataReadFences>,
}

/// Adopt the selected generation for runtime use, or re-open a previously
/// adopted one after restart.
///
/// First adoption verifies the complete pair and every remote object exactly
/// like recovery, then durably publishes a V2 `runtime-adopted` marker including
/// the original image's verified TTL state; from
/// that point installation for this group is closed and the generation's
/// files may legitimately diverge from their sealed hashes (the log appends,
/// the WAL grows). Re-adoption therefore validates the marker against the
/// selector and leaves file validation to the ordinary storage and engine
/// recovery machinery. V1 markers are upgraded only after restoring their
/// original image; V2 permits later published checkpoints to recover without
/// the superseded image's remote closure. The group record and range come from the destination's
/// own durable state, never from a caller-supplied description.
pub fn adopt_for_runtime(
    store: &StoreGuard,
    identity: StoreIdentity,
    region: kv9_common::RegionId,
    uploader: &RemoteUploader,
) -> Result<AdoptedGeneration> {
    adopt_for_runtime_observed(store, identity, region, uploader, |_| Ok(()))
}

/// Fault-observation seam for the same durable adoption operation. Any error
/// returns no receipt; retry must reopen and validate the selected marker.
#[doc(hidden)]
pub fn adopt_for_runtime_observed(
    store: &StoreGuard,
    identity: StoreIdentity,
    region: kv9_common::RegionId,
    uploader: &RemoteUploader,
    observe: impl FnMut(AdoptStep) -> Result<()>,
) -> Result<AdoptedGeneration> {
    adopt_with_reader_observed(store, identity, region, uploader, observe)
}

/// Runtime adoption with an externally retained GET-only dependency. Original
/// validation and V1 upgrades use it; a valid V2 marker needs no original GET.
pub fn adopt_for_runtime_with_reader(
    store: &StoreGuard,
    identity: StoreIdentity,
    region: kv9_common::RegionId,
    reader: &dyn CheckpointRestore,
) -> Result<AdoptedGeneration> {
    adopt_with_reader_observed(store, identity, region, reader, |_| Ok(()))
}

fn adopt_with_reader_observed(
    store: &StoreGuard,
    identity: StoreIdentity,
    region: kv9_common::RegionId,
    uploader: &dyn CheckpointRestore,
    mut observe: impl FnMut(AdoptStep) -> Result<()>,
) -> Result<AdoptedGeneration> {
    let directory = store
        .directory()
        .join("data-groups")
        .join(region.0.to_string());
    let record = read(&directory.join("group-record"), 256 * 1024)?;
    let fields = checked(&record, b"KV9INS01")?;
    if fields.len() < 40 {
        return Err(invalid("group record is truncated"));
    }
    let range = DataRange::decode(&fields[40..])?;
    if range.region != region {
        return Err(invalid("group record names a different region"));
    }
    // Decide "nothing to adopt" WITHOUT the owner lock: a runtime that polls
    // adoption while an installer is still filling the generation must not
    // hold the lock (and its fsyncs) against that installer's retries. The
    // locked re-check below remains authoritative.
    if !directory.join(SELECTOR).try_exists().map_err(io)? {
        return Err(invalid("no generation is selected for adoption"));
    }
    let installer = JointInstaller::open_inner(store, identity, range.clone(), true)?;
    let marker_path = directory.join(ADOPTED);
    let adopted_before = marker_path.try_exists().map_err(io)?;
    let (generation, digest) = installer
        .selected()?
        .ok_or_else(|| invalid("no generation is selected for adoption"))?;
    let binding = adoption::Binding::new(installer.binding, generation, digest);
    let generation_dir = directory.join(format!("install-{generation}"));
    let (cut, configuration, subject, manifest, watermark, verified_original) = if adopted_before {
        let marker = binding.decode(&read(&marker_path, adoption::MAX_MARKER_BYTES)?)?;
        // Files may legitimately have grown; read the image record only.
        let bytes = read(&generation_dir.join("image"), MAX_IMAGE)?;
        if RootDigest::sha256(&bytes) != digest {
            return Err(invalid("adopted image digest mismatch"));
        }
        let (image, _) = snapshot::decode(&bytes)?;
        let (_, manifest) = parse_image(&image)?;
        let (watermark, verified_original) = match marker {
            adoption::Marker::Verified { watermark } => {
                // Resolve a previous rename whose directory sync was uncertain
                // before this same-process retry exposes a runtime owner.
                sync_dir(&directory)?;
                (watermark, false)
            }
            adoption::Marker::Legacy => {
                // V1 carried no TTL facts. Never guess from the current engine
                // or a later image: verify the original closure before upgrade.
                installer.check_read_floor(&manifest)?;
                let restored = uploader.restore(&manifest)?;
                let inspected =
                    validate_view(restored.snapshot()?.as_ref(), &range, &manifest, identity)?;
                binding.publish(&directory, inspected.watermark, &mut observe)?;
                (inspected.watermark, true)
            }
        };
        let metadata = image.get_metadata();
        (
            AppliedPosition {
                term: metadata.term,
                index: metadata.index,
            },
            metadata.get_conf_state().clone(),
            *RootDigest::sha256(&manifest.encode()?).as_bytes(),
            manifest,
            watermark,
            verified_original,
        )
    } else {
        let (observation, image, _) = installer.verify_generation(generation, digest, uploader)?;
        let (_, manifest) = parse_image(&image)?;
        binding.publish(&directory, observation.ttl_watermark, &mut observe)?;
        (
            observation.position,
            image.get_metadata().get_conf_state().clone(),
            *RootDigest::sha256(&manifest.encode()?).as_bytes(),
            manifest,
            observation.ttl_watermark,
            true,
        )
    };
    let recovery = recovery::Anchor::new(
        identity,
        range.clone(),
        manifest,
        configuration.clone(),
        generation_dir.join("data.wal"),
        watermark,
        (generation, digest, verified_original),
    )?;
    Ok(AdoptedGeneration {
        generation,
        range,
        cut,
        configuration,
        image_digest: digest,
        subject,
        raft_directory: generation_dir.join("raft"),
        engine_wal: generation_dir.join("data.wal"),
        data_read_fences: store.data_read_fences(&identity)?,
        group_lock: installer._lock,
        recovery,
    })
}

mod adoption;
pub mod capture;
pub use adoption::AdoptStep;
mod recovery;

/// Offline convenience for a destination operator: decode one canonical
/// captured record, derive its exact range from the image itself, and run
/// the unchanged installer at the destination store. This grants nothing
/// beyond `JointInstaller`'s own validation — including the membership gate
/// requiring the destination inside the image configuration.
pub fn install_captured_record(
    guard: &StoreGuard,
    identity: StoreIdentity,
    record: &[u8],
    uploader: &RemoteUploader,
) -> Result<InstalledImage> {
    install_captured_record_with_reader(guard, identity, record, uploader)
}

/// Exact installer with an externally retained, GET-only restore dependency.
/// The checkpoint manifest a captured image record names, for callers that
/// want to confirm the image's objects are present before opening a
/// generation (`RemoteUploader::verify_checkpoint_objects`).
pub fn captured_record_manifest(record: &[u8]) -> Result<CheckpointManifest> {
    let (image, _) = snapshot::decode(record)?;
    Ok(parse_image(&image)?.1)
}

pub fn install_captured_record_with_reader(
    guard: &StoreGuard,
    identity: StoreIdentity,
    record: &[u8],
    reader: &dyn CheckpointRestore,
) -> Result<InstalledImage> {
    let (image, hs) = snapshot::decode(record)?;
    let (range, _) = parse_image(&image)?;
    let mut installer = JointInstaller::open(guard, identity, range)?;
    installer.install_with_reader(&image, &hs, reader)
}

#[cfg(test)]
mod tests;
