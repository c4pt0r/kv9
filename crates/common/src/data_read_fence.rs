//! Durable local DATA image read floors. These records describe one store's
//! prospective read admission, never global replica coverage or GC authority.
use crate::{
    AppliedPosition, Error, NodeId, RegionId, Result, RootDigest, StoreIdentity, StoreIncarnation,
};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const DIRECTORY: &str = "data-read-fences";
const MAGIC: &[u8; 8] = b"KV9DRF01";
pub const MAX_BYTES: usize = 224;

/// Immutable image identity only; constructing it does not certify a read,
/// publication, selected checkpoint, or store's durable floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadImage {
    pub region: RegionId,
    pub digest: RootDigest,
    pub cut: AppliedPosition,
    pub conf_ver: u64,
    pub version: u64,
}
impl ReadImage {
    fn validate(&self) -> Result<()> {
        if self.region.0 < 100
            || self.digest.as_bytes() == &[0; 32]
            || self.cut.term == 0
            || self.cut.index == 0
            || self.conf_ver == 0
            || self.version == 0
        {
            return Err(invalid("invalid DATA image identity"));
        }
        Ok(())
    }
    fn covers(&self, previous: &Self) -> bool {
        self == previous
            || (self.region == previous.region
                && self.cut.index > previous.cut.index
                && self.cut.term >= previous.cut.term
                && self.conf_ver >= previous.conf_ver
                && self.version >= previous.version)
    }
}

/// Canonical descriptor. A decoded record alone grants no remote authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadFloor {
    pub identity: StoreIdentity,
    pub creation: RootDigest,
    pub image: ReadImage,
    pub generation: u64,
}
impl ReadFloor {
    /// Compare descriptors only. This does not establish remote observation,
    /// publication, membership coverage, or permission to release an owner.
    pub fn covers(&self, previous: &Self) -> bool {
        self.identity == previous.identity
            && self.creation == previous.creation
            && self.image.covers(&previous.image)
            && self.generation >= previous.generation
            && (self.image == previous.image || self.generation > previous.generation)
    }
    pub fn validate(&self) -> Result<()> {
        self.image.validate()?;
        if self.identity.cluster_id.as_bytes() == &[0; 16]
            || self.identity.root_digest.as_bytes() == &[0; 32]
            || self.identity.node_id.0 == 0
            || self.identity.store_incarnation.as_bytes() == &[0; 16]
            || self.creation.as_bytes() == &[0; 32]
            || self.generation == 0
        {
            return Err(invalid("invalid store, creation or generation"));
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(self.identity.cluster_id.as_bytes());
        b.extend_from_slice(self.identity.root_digest.as_bytes());
        b.extend_from_slice(&self.identity.node_id.0.to_be_bytes());
        b.extend_from_slice(self.identity.store_incarnation.as_bytes());
        b.extend_from_slice(&self.image.region.0.to_be_bytes());
        b.extend_from_slice(self.creation.as_bytes());
        b.extend_from_slice(self.image.digest.as_bytes());
        for n in [
            self.image.cut.term,
            self.image.cut.index,
            self.image.conf_ver,
            self.image.version,
            self.generation,
        ] {
            b.extend_from_slice(&n.to_be_bytes());
        }
        b.extend_from_slice(RootDigest::sha256(&b).as_bytes());
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != MAX_BYTES
            || !b.starts_with(MAGIC)
            || RootDigest::sha256(&b[..MAX_BYTES - 32]).as_bytes() != &b[MAX_BYTES - 32..]
        {
            return Err(invalid("floor checksum or length"));
        }
        let u = |at| u64::from_be_bytes(b[at..at + 8].try_into().unwrap());
        let f = Self {
            identity: StoreIdentity {
                cluster_id: crate::ClusterId::from_bytes(b[8..24].try_into().unwrap()),
                root_digest: RootDigest::from_bytes(b[24..56].try_into().unwrap()),
                node_id: NodeId(u(56)),
                store_incarnation: StoreIncarnation::from_bytes(b[64..80].try_into().unwrap()),
            },
            creation: RootDigest::from_bytes(b[88..120].try_into().unwrap()),
            image: ReadImage {
                region: RegionId(u(80)),
                digest: RootDigest::from_bytes(b[120..152].try_into().unwrap()),
                cut: AppliedPosition {
                    term: u(152),
                    index: u(160),
                },
                conf_ver: u(168),
                version: u(176),
            },
            generation: u(184),
        };
        f.validate()?;
        if f.encode()? != b {
            return Err(invalid("noncanonical floor"));
        }
        Ok(f)
    }
}

/// Shared by all users of one StoreGuard; there is no per-group cached state.
/// The shared lock descriptor keeps exclusive store ownership for every user.
#[derive(Debug)]
pub struct DataReadFences {
    _store_lock: Arc<File>,
    directory: PathBuf,
    identity: StoreIdentity,
    failed: Mutex<bool>,
}
impl DataReadFences {
    pub(crate) fn open(
        directory: &Path,
        identity: StoreIdentity,
        store_lock: Arc<File>,
    ) -> Result<Self> {
        check_directory(directory)?;
        Ok(Self {
            _store_lock: store_lock,
            directory: directory.join(DIRECTORY),
            identity,
            failed: Mutex::new(false),
        })
    }
    pub fn identity(&self) -> StoreIdentity {
        self.identity
    }
    fn path(&self, region: RegionId) -> Result<PathBuf> {
        if region.0 < 100 {
            return Err(invalid("non-DATA group"));
        }
        Ok(self.directory.join(region.0.to_string()))
    }
    fn load(&self, region: RegionId) -> Result<Option<ReadFloor>> {
        check_directory(
            self.directory
                .parent()
                .ok_or_else(|| invalid("floor directory parent"))?,
        )?;
        let path = self.path(region)?;
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(m) if m.is_file() && m.len() == MAX_BYTES as u64 => {}
            Ok(_) => return Err(invalid("floor file type or size")),
            Err(e) => return Err(invalid(&e.to_string())),
        }
        let mut file = File::open(path).map_err(io)?;
        let mut b = Vec::with_capacity(MAX_BYTES);
        (&mut file)
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut b)
            .map_err(io)?;
        let f = ReadFloor::decode(&b)?;
        if f.identity != self.identity || f.image.region != region {
            return Err(invalid("floor belongs to another store or group"));
        }
        // Stabilize a visible rename from an earlier uncertain process before
        // observing it. An incomplete temporary never becomes a floor.
        file.sync_all().map_err(io)?;
        File::open(&self.directory)
            .and_then(|d| d.sync_all())
            .map_err(io)?;
        Ok(Some(f))
    }
    pub fn floor(&self, region: RegionId) -> Result<Option<ReadFloor>> {
        let failed = self
            .failed
            .lock()
            .map_err(|_| invalid("floor lock poisoned"))?;
        if *failed {
            return Err(invalid("floor publication failed; reopen store"));
        }
        self.load(region)
    }
    pub fn check_read(&self, image: &ReadImage) -> Result<()> {
        image.validate()?;
        if self
            .floor(image.region)?
            .is_some_and(|f| !image.covers(&f.image))
        {
            return Err(invalid("DATA image is below the durable local read floor"));
        }
        Ok(())
    }
    pub fn check_recovery(
        &self,
        region: RegionId,
        creation: RootDigest,
        image: Option<&ReadImage>,
    ) -> Result<()> {
        let Some(f) = self.floor(region)? else {
            return Ok(());
        };
        if f.creation != creation || image.is_none_or(|i| !i.covers(&f.image)) {
            return Err(invalid(
                "recovery does not cover the durable local read floor",
            ));
        }
        Ok(())
    }
    /// Caller must establish actual successful selection and exact creation.
    /// This method serializes publication; its data argument is not authority.
    pub fn advance(&self, floor: ReadFloor) -> Result<bool> {
        self.advance_observed(floor, &mut |_| Ok(()))
    }
    fn advance_observed(
        &self,
        floor: ReadFloor,
        observe: &mut impl FnMut(Step) -> Result<()>,
    ) -> Result<bool> {
        floor.validate()?;
        if floor.identity != self.identity {
            return Err(invalid("new floor has another identity"));
        }
        let mut failed = self
            .failed
            .lock()
            .map_err(|_| invalid("floor lock poisoned"))?;
        if *failed {
            return Err(invalid("floor publication failed; reopen store"));
        }
        if let Some(old) = self.load(floor.image.region)? {
            if old == floor {
                return Ok(false);
            }
            if !floor.covers(&old) {
                return Err(invalid("read floor regressed or changed creation"));
            }
        }
        let path = self.path(floor.image.region)?;
        let temp = self
            .directory
            .join(format!(".{}.tmp", floor.image.region.0));
        let result = (|| {
            match fs::remove_file(&temp) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io(e)),
            }
            observe(Step::Create)?;
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(io)?;
            f.write_all(&floor.encode()?).map_err(io)?;
            observe(Step::Write)?;
            f.sync_all().map_err(io)?;
            observe(Step::SyncFile)?;
            fs::rename(&temp, &path).map_err(io)?;
            observe(Step::Rename)?;
            File::open(&self.directory)
                .and_then(|d| d.sync_all())
                .map_err(io)?;
            observe(Step::SyncDirectory)?;
            Ok(())
        })();
        if result.is_err() {
            *failed = true;
        }
        result.map(|()| true)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Create,
    Write,
    SyncFile,
    Rename,
    SyncDirectory,
}
pub(crate) fn initialize_directory(directory: &Path) -> Result<()> {
    let path = directory.join(DIRECTORY);
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(io(e)),
    }
    check_directory(directory)?;
    crate::fs::sync_ancestors(&crate::fs::OsFileSystem, &path).map_err(io)
}
pub(crate) fn check_directory(directory: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(directory.join(DIRECTORY)).map_err(io)?;
    if !metadata.is_dir() {
        return Err(invalid(
            "DATA read-fence directory absent or not a directory",
        ));
    }
    Ok(())
}
fn invalid(s: &str) -> Error {
    Error::Config(format!("DATA read fence: {s}"))
}
fn io(e: std::io::Error) -> Error {
    invalid(&e.to_string())
}

#[cfg(test)]
mod tests;
