//! Local persistence operations shared by production and deterministic tests.
//!
//! The production implementation delegates directly to `std::fs`. Generic
//! callers execute the same ordering and record encoding with either backend.
//! The testing backend is excluded from default builds.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub trait DurableFile: Read + Write + Seek + Send + Sync {
    fn sync_data(&self) -> io::Result<()>;
    fn set_len(&self, len: u64) -> io::Result<()>;
}

impl DurableFile for File {
    fn sync_data(&self) -> io::Result<()> {
        File::sync_data(self)
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        File::set_len(self, len)
    }
}

/// The initial persistence seam covers append logs and directory publication.
/// Filesystem mutation by unrelated processes is outside this ownership contract.
pub trait FileSystem: Clone + Send + Sync + 'static {
    type File: DurableFile;

    fn create_dir(&self, path: &Path) -> io::Result<()>;
    fn is_dir(&self, path: &Path) -> io::Result<bool>;
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    fn open_append(&self, path: &Path) -> io::Result<Self::File>;
    fn open_existing_append(&self, path: &Path) -> io::Result<Self::File>;
    fn sync_dir(&self, path: &Path) -> io::Result<()>;
}

#[derive(Clone, Copy, Default)]
pub struct OsFileSystem;

impl FileSystem for OsFileSystem {
    type File = File;

    fn create_dir(&self, path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }

    fn is_dir(&self, path: &Path) -> io::Result<bool> {
        fs::metadata(path).map(|m| m.is_dir())
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }

    fn open_append(&self, path: &Path) -> io::Result<Self::File> {
        OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)
    }

    fn open_existing_append(&self, path: &Path) -> io::Result<Self::File> {
        OpenOptions::new().read(true).append(true).open(path)
    }

    fn sync_dir(&self, path: &Path) -> io::Result<()> {
        File::open(path)?.sync_all()
    }
}

/// Create each component through the same seam. Existence is not evidence of
/// durability: a previous process may have died before publishing that entry.
pub fn create_dirs<F: FileSystem>(fs: &F, path: &Path) -> io::Result<()> {
    let absolute = std::path::absolute(path)?;
    let mut current = PathBuf::new();
    for component in absolute.components() {
        current.push(component);
        match fs.create_dir(&current) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && fs.is_dir(&current)? => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Publish the log's directory and its ancestors, including components that
/// already existed at open. Resolve symlinks before traversing the ancestry.
/// Call after synchronizing the initial log contents and any tail repair.
pub fn sync_ancestors<F: FileSystem>(fs: &F, directory: &Path) -> io::Result<()> {
    let canonical = fs.canonicalize(directory)?;
    for ancestor in canonical.ancestors() {
        fs.sync_dir(ancestor)?;
    }
    Ok(())
}
