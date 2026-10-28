//! A local, destination-bound receipt of the original image's verified TTL state.
//! V2 is deliberately rejected by the old exact V1 reader. It is not a portable
//! recovery capability or permission to release any object's retention owner.
use super::*;

pub(super) const MAX_MARKER_BYTES: usize = 137;
const BINDING_BYTES: usize = 80;
const TTL_BYTES: usize = 17;

pub(super) struct Binding([u8; BINDING_BYTES]);
pub(super) enum Marker {
    Legacy,
    Verified { watermark: Option<ClockWatermark> },
}

impl Binding {
    pub(super) fn new(
        binding: RootDigest,
        generation: StoreIncarnation,
        image: RootDigest,
    ) -> Self {
        let mut bytes = [0; BINDING_BYTES];
        bytes[..32].copy_from_slice(binding.as_bytes());
        bytes[32..48].copy_from_slice(generation.as_bytes());
        bytes[48..].copy_from_slice(image.as_bytes());
        Self(bytes)
    }

    pub(super) fn decode(&self, bytes: &[u8]) -> Result<Marker> {
        let bad =
            || invalid("adoption marker differs from selected generation or has invalid encoding");
        let Some(magic) = bytes.get(..8) else {
            return Err(bad());
        };
        match magic {
            b"KV9ADP01" => {
                let fields = checked(bytes, b"KV9ADP01").map_err(|_| bad())?;
                if fields != self.0 {
                    return Err(bad());
                }
                Ok(Marker::Legacy)
            }
            b"KV9ADP02" => {
                let fields = checked(bytes, b"KV9ADP02").map_err(|_| bad())?;
                if fields.len() != BINDING_BYTES + TTL_BYTES || fields[..BINDING_BYTES] != self.0 {
                    return Err(bad());
                }
                let ttl = &fields[BINDING_BYTES..];
                let watermark = match ttl[0] {
                    0 if ttl[1..] == [0; 16] => None,
                    1 => Some(ClockWatermark::decode(&ttl[1..]).map_err(|_| bad())?),
                    _ => return Err(bad()),
                };
                Ok(Marker::Verified { watermark })
            }
            _ => Err(bad()),
        }
    }

    fn encode(&self, watermark: Option<ClockWatermark>) -> Vec<u8> {
        let mut bytes = b"KV9ADP02".to_vec();
        bytes.extend_from_slice(&self.0);
        bytes.push(u8::from(watermark.is_some()));
        bytes.extend_from_slice(&watermark.map_or([0; 16], ClockWatermark::encode));
        checksum(bytes)
    }

    /// The caller obtained these facts from this exact restored image while
    /// holding the group/store locks. Publish them before exposing adoption.
    pub(super) fn publish(
        &self,
        directory: &Path,
        watermark: Option<ClockWatermark>,
        observe: &mut impl FnMut(AdoptStep) -> Result<()>,
    ) -> Result<()> {
        let temporary = directory.join(".runtime-adopted.tmp");
        let mut file = File::create(&temporary).map_err(io)?;
        file.write_all(&self.encode(watermark)).map_err(io)?;
        file.sync_all().map_err(io)?;
        observe(AdoptStep::AfterMarkerFileSync)?;
        fs::rename(temporary, directory.join(ADOPTED)).map_err(io)?;
        observe(AdoptStep::AfterMarkerRename)?;
        sync_dir(directory)?;
        observe(AdoptStep::AfterMarkerDirectorySync)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub enum AdoptStep {
    AfterMarkerFileSync,
    AfterMarkerRename,
    AfterMarkerDirectorySync,
}
