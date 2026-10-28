//! Streaming codec for the unchanged v1 prepared / v2 planned journal formats.
//! Lengths are bounded before allocation. No decoded plan escapes until its
//! complete checksum, object descriptors and exact EOF have all been checked.
use std::io::{Read, Write};

use sha2::{Digest, Sha256};

use super::*;
use crate::checkpoint::checked_sst_bytes;

const MAX_MANIFEST: usize = 1024 * 1024;

pub(super) struct PendingRef<'a> {
    pub generation: u64,
    pub manifest: &'a CheckpointManifest,
    pub objects: Option<&'a [Vec<u8>]>,
}
impl PendingRef<'_> {
    /// Complete all semantic validation before publishing even a temporary file.
    pub fn validate(&self) -> Result<Vec<u8>> {
        if self.generation == u64::MAX {
            return Err(invalid("pending flush generation exhausted"));
        }
        let manifest = self.manifest.encode()?;
        CheckpointManifest::decode(&manifest)?;
        if let Some(objects) = self.objects {
            if objects.len() != self.manifest.files.len() {
                return Err(invalid("planned object count mismatch"));
            }
            for (file, object) in self.manifest.files.iter().zip(objects) {
                checked_sst_bytes(file, object)?;
            }
        }
        Ok(manifest)
    }
    pub fn matches(&self, old: &PendingFlush) -> bool {
        self.generation == old.generation
            && *self.manifest == old.manifest
            && self.objects.is_some() == old.requires_upload()
            && (old.disk_objects.is_some() || self.objects == old.objects.as_deref())
    }
    pub fn write_to(&self, mut output: impl Write, manifest: &[u8]) -> Result<()> {
        let mut hash = Sha256::new();
        let mut part = |bytes: &[u8]| -> Result<()> {
            output.write_all(bytes).map_err(io)?;
            hash.update(bytes);
            Ok(())
        };
        part(if self.objects.is_some() {
            PLANNED_MAGIC
        } else {
            MAGIC
        })?;
        part(&self.generation.to_le_bytes())?;
        if self.objects.is_some() {
            part(&(manifest.len() as u32).to_le_bytes())?;
        }
        part(manifest)?;
        if let Some(objects) = self.objects {
            for object in objects {
                part(object)?;
            }
        }
        output.write_all(&hash.finalize()).map_err(io)
    }
}

/// `remaining` excludes the trailing checksum. Even untrusted manifest fields
/// cannot request a buffer larger than the remaining, capped file payload.
pub(super) struct Body<R> {
    pub(super) input: R,
    pub(super) remaining: usize,
    pub(super) hash: Sha256,
}
impl<R: Read> Body<R> {
    fn read(&mut self, out: &mut [u8]) -> Result<()> {
        if out.len() > self.remaining {
            return Err(invalid("pending flush body is truncated"));
        }
        self.input.read_exact(out).map_err(io)?;
        self.remaining -= out.len();
        self.hash.update(out);
        Ok(())
    }
    pub(super) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0; N];
        self.read(&mut out)?;
        Ok(out)
    }
    pub(super) fn bytes(&mut self, n: usize) -> Result<Vec<u8>> {
        if n > self.remaining {
            return Err(invalid("pending flush body is truncated"));
        }
        let mut out = vec![0; n];
        self.read(&mut out)?;
        Ok(out)
    }
    pub(super) fn finish(mut self) -> Result<R> {
        if self.remaining != 0 {
            return Err(invalid("pending journal has trailing bytes"));
        }
        let mut stored = [0; HASH_LEN];
        self.input.read_exact(&mut stored).map_err(io)?;
        if self.hash.finalize().as_slice() != stored {
            return Err(invalid("pending flush checksum mismatch"));
        }
        // Also reject growth after the file length was captured. Short reads
        // and in-place changes must pass every checksum/descriptor gate too.
        let mut extra = [0];
        loop {
            match self.input.read(&mut extra) {
                Ok(0) => return Ok(self.input),
                Ok(_) => return Err(invalid("pending journal has trailing bytes")),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(io(e)),
            }
        }
    }
}
impl PendingFlush {
    #[cfg(test)]
    pub(super) fn read_from(input: impl Read, file_len: u64) -> Result<Self> {
        Self::read_with_budget(input, file_len, None)
    }
    pub(super) fn read_with_budget(
        input: impl Read,
        file_len: u64,
        budget: Option<&std::sync::Arc<crate::checkpoint_memory::ImageBudget>>,
    ) -> Result<Self> {
        Self::read_inner(input, file_len, budget, true).map(|(pending, _, _)| pending)
    }
    pub(super) fn read_disk(
        file: TrackedFile,
        budget: Option<&std::sync::Arc<crate::checkpoint_memory::ImageBudget>>,
    ) -> Result<Self> {
        let len = file.metadata().map_err(io)?.len();
        let (mut pending, reader, start) =
            Self::read_inner(BufReader::new(file), len, budget, false)?;
        let file = reader.into_inner();
        file.sync_all().map_err(io)?;
        if let Some(start) = start {
            pending.disk_objects = Some(DiskObjects {
                file,
                start,
                budget: budget.cloned(),
            });
        }
        Ok(pending)
    }
    fn read_inner<R: Read>(
        input: R,
        file_len: u64,
        budget: Option<&std::sync::Arc<crate::checkpoint_memory::ImageBudget>>,
        retain: bool,
    ) -> Result<(Self, R, Option<u64>)> {
        if file_len <= (MAGIC.len() + 8 + HASH_LEN) as u64
            || file_len > MAX_FILE_BYTES
            || (retain && file_len > MAX_RESIDENT_FILE_BYTES)
        {
            return Err(invalid("invalid pending flush version or length"));
        }
        let mut body = Body {
            input,
            remaining: usize::try_from(file_len)
                .map_err(|_| invalid("pending file length overflow"))?
                - HASH_LEN,
            hash: Sha256::new(),
        };
        let magic = body.array::<{ MAGIC.len() }>()?;
        let planned = magic == PLANNED_MAGIC;
        if !planned && magic != MAGIC {
            return Err(invalid("invalid pending flush version or length"));
        }
        let generation = u64::from_le_bytes(body.array()?);
        if generation == u64::MAX {
            return Err(invalid("pending flush generation exhausted"));
        }
        let size = if planned {
            u32::from_le_bytes(body.array()?) as usize
        } else {
            body.remaining
        };
        if size > MAX_MANIFEST {
            return Err(invalid("planned manifest length invalid"));
        }
        let manifest = CheckpointManifest::decode(&body.bytes(size)?)?;
        if planned && retain && manifest.requires_large_format()? {
            return Err(invalid("large planned journal requires streaming load"));
        }
        let start = planned.then_some(file_len - HASH_LEN as u64 - body.remaining as u64);
        let reservation = if planned && retain {
            budget
                .map(|b| b.reserve(manifest.files.iter().map(|f| f.size).sum()))
                .transpose()?
        } else {
            None
        };
        let objects = if planned {
            let mut objects = if retain {
                Vec::with_capacity(manifest.files.len())
            } else {
                Vec::new()
            };
            for file in &manifest.files {
                let _reservation = if retain {
                    None
                } else {
                    budget.map(|b| b.reserve(file.size)).transpose()?
                };
                let size = usize::try_from(file.size)
                    .map_err(|_| invalid("planned object size invalid"))?;
                let object = body.bytes(size)?;
                checked_sst_bytes(file, &object)?;
                if retain {
                    objects.push(object);
                }
            }
            retain.then_some(objects)
        } else {
            None
        };
        let input = body.finish()?;
        Ok((
            Self {
                generation,
                manifest,
                objects,
                disk_objects: None,
                reservation,
            },
            input,
            start,
        ))
    }
    #[cfg(test)]
    pub(super) fn decode(bytes: &[u8]) -> Result<Self> {
        Self::read_from(bytes, bytes.len() as u64)
    }
}
fn invalid(message: &str) -> Error {
    Error::Engine(message.into())
}
