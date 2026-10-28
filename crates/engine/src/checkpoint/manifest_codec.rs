//! Encode V1 or V2 without materializing an unbounded JSON Vec. Images within
//! the legacy limit retain the exact V1 bytes and identity.
use super::{codec_error, CheckpointManifest, LARGE_MAGIC, MAGIC, MAX_MANIFEST_BYTES};
use kv9_common::{Error, Result};
use serde::Serialize;
use std::io::{self, Write};

struct BoundedOutput {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}
impl BoundedOutput {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded: false,
        }
    }
}
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(end) = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|end| *end <= self.limit)
        else {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "checkpoint manifest too large",
            ));
        };
        if end > self.bytes.capacity() {
            // Ordinary Vec growth can exceed the logical limit. Request only
            // a capped capacity, with amortized growth for small JSON writes.
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(end)
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(io::Error::other)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn encode(manifest: &CheckpointManifest) -> Result<Vec<u8>> {
    let magic = if manifest.requires_large_format()? {
        LARGE_MAGIC
    } else {
        MAGIC
    };
    encode_frame(manifest, MAX_MANIFEST_BYTES, magic)
}

#[cfg(test)]
fn encode_with_limit(value: &impl Serialize, limit: usize) -> Result<Vec<u8>> {
    encode_frame(value, limit, MAGIC)
}
fn encode_frame(value: &impl Serialize, limit: usize, magic: &[u8]) -> Result<Vec<u8>> {
    let mut output = BoundedOutput::new(limit);
    let encoded = output
        .write_all(magic)
        .map_err(serde_json::Error::io)
        .and_then(|()| serde_json::to_writer(&mut output, value));
    if output.exceeded {
        return Err(Error::Engine("checkpoint manifest too large".into()));
    }
    encoded.map_err(codec_error)?;
    Ok(output.bytes)
}

#[cfg(test)]
mod tests;
