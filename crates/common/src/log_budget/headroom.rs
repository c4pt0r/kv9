//! Fresh local filesystem observations. Space is not preallocated: this gate
//! cannot promise ENOSPC immunity against control/replication or external I/O.
use super::*;
use std::time::{Duration, Instant};

pub const DEFAULT_MIN_FREE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PROBE_AGE: Duration = Duration::from_secs(1);

/// Implementations observe the filesystem owned by the node, never a remote
/// replica's capacity. Injection is also used for deterministic I/O races.
pub trait DiskSpaceProbe: std::fmt::Debug + Send + Sync {
    fn available_bytes(&self) -> std::io::Result<u64>;
    fn check_path(&self, path: &Path) -> std::io::Result<()>;
}

#[derive(Debug)]
pub(super) struct DiskHeadroom {
    pub minimum_free_bytes: u64,
    pub probe: Arc<dyn DiskSpaceProbe>,
}
impl DiskHeadroom {
    pub fn new(minimum_free_bytes: u64, probe: Arc<dyn DiskSpaceProbe>) -> Result<Self> {
        if minimum_free_bytes == 0 {
            return Err(Error::Config("disk headroom must be positive".into()));
        }
        Ok(Self {
            minimum_free_bytes,
            probe,
        })
    }
    pub fn sample(&self) -> Sample {
        let began = Instant::now();
        let available = self.probe.available_bytes();
        Sample { began, available }
    }
}
pub(super) struct Sample {
    began: Instant,
    available: std::io::Result<u64>,
}
#[derive(Debug, Default)]
pub(super) struct Observation {
    available: Option<u64>,
    overlap: u64,
    probes: u64,
    errors: u64,
    refusals: u64,
    last_error: Option<String>,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct HeadroomSnapshot {
    pub minimum_free_bytes: u64,
    pub last_available_bytes: Option<u64>,
    pub last_probe_overlap_bytes: u64,
    pub in_flight_write_bytes: u64,
    pub probes: u64,
    pub probe_errors: u64,
    pub refusals: u64,
    pub last_probe_error: Option<String>,
}
impl Observation {
    pub fn snapshot(&self, minimum_free_bytes: u64, writing: u64) -> HeadroomSnapshot {
        HeadroomSnapshot {
            minimum_free_bytes,
            last_available_bytes: self.available,
            last_probe_overlap_bytes: self.overlap,
            in_flight_write_bytes: writing,
            probes: self.probes,
            probe_errors: self.errors,
            refusals: self.refusals,
            last_probe_error: self.last_error.clone(),
        }
    }
    pub fn admit(
        &mut self,
        minimum: u64,
        sample: Sample,
        overlap: u128,
        reserved: u128,
        bytes: u64,
    ) -> bool {
        let available = self.observe(sample, overlap);
        let needed = u128::from(minimum) + reserved + u128::from(bytes);
        let accepted = available.is_some_and(|available| {
            overlap
                .checked_add(needed)
                .is_some_and(|n| n <= u128::from(available))
        });
        if !accepted {
            self.refusals = self.refusals.saturating_add(1);
        }
        accepted
    }
    /// Observing pressure does not manufacture a proposal or a refusal count.
    pub fn observe(&mut self, sample: Sample, overlap: u128) -> Option<u64> {
        self.probes = self.probes.saturating_add(1);
        self.overlap = overlap.min(u128::from(u64::MAX)) as u64;
        let available = if sample.began.elapsed() > MAX_PROBE_AGE {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "disk space sample exceeded one second",
            ))
        } else {
            sample.available
        };
        match available {
            Ok(available) => {
                self.available = Some(available);
                self.last_error = None;
                Some(available)
            }
            Err(error) => {
                self.available = None;
                self.last_error = Some(error.to_string());
                self.errors = self.errors.saturating_add(1);
                None
            }
        }
    }
}

/// Retains an open directory descriptor, so renaming a path cannot silently
/// switch the observed filesystem. Source attachment rejects other devices.
#[derive(Debug)]
pub struct OsDiskSpaceProbe {
    #[cfg(target_os = "linux")]
    directory: std::fs::File,
    #[cfg(target_os = "linux")]
    device: u64,
}
impl OsDiskSpaceProbe {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let directory = std::fs::File::open(path)?;
            let metadata = directory.metadata()?;
            if !metadata.is_dir() {
                return Err(std::io::Error::other("disk probe requires a directory"));
            }
            Ok(Self {
                directory,
                device: metadata.dev(),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = path;
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "disk headroom requires Linux fstatvfs",
            ))
        }
    }
}
impl DiskSpaceProbe for OsDiskSpaceProbe {
    fn available_bytes(&self) -> std::io::Result<u64> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            let mut statistics = std::mem::MaybeUninit::<libc::statvfs>::uninit();
            // SAFETY: the owned descriptor remains open for this call; libc
            // writes the complete struct on success. Inspect it only then.
            if unsafe { libc::fstatvfs(self.directory.as_raw_fd(), statistics.as_mut_ptr()) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // SAFETY: successful fstatvfs initialized statistics above.
            let statistics = unsafe { statistics.assume_init() };
            if statistics.f_flag & libc::ST_RDONLY != 0 || statistics.f_frsize == 0 {
                return Err(std::io::Error::other(
                    "filesystem is read-only or has invalid allocation units",
                ));
            }
            // f_bavail excludes blocks reserved for privileged users. Never
            // count f_bfree as application-writable space (statvfs(3)).
            let available = u128::from(statistics.f_bavail) * u128::from(statistics.f_frsize);
            u64::try_from(available)
                .map_err(|_| std::io::Error::other("filesystem available-byte overflow"))
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "disk space probe unavailable",
            ))
        }
    }
    fn check_path(&self, path: &Path) -> std::io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let absolute = std::path::absolute(path)?;
            let mut cursor = absolute.as_path();
            loop {
                match std::fs::metadata(cursor) {
                    Ok(metadata) if metadata.dev() == self.device => return Ok(()),
                    Ok(_) => {
                        return Err(std::io::Error::other("log source is on another filesystem"))
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        cursor = cursor.parent().ok_or(e)?;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = path;
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "disk space probe unavailable",
            ))
        }
    }
}

#[cfg(test)]
mod tests;
