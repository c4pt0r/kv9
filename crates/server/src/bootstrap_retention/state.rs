//! One atomic, bounded local admission/fence outbox. The store has one writer.
use super::*;
use kv9_common::retention::OwnerId;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

const MAGIC: &[u8; 8] = b"KV9MBR02";
const LEGACY: &[u8; 8] = b"KV9MBR01";
pub(super) const MAX_BYTES: usize = 212; // 64 + two counts + two 57-byte slots + checksum
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Grant {
    pub predecessor: u64,
    pub image: [u8; 32],
    pub pending: Option<OwnerId>,
    pub authorized: bool,
}
#[derive(Default, Debug)]
pub(super) struct State {
    pub active: Vec<Grant>,
    pub retiring: Vec<Grant>,
}
pub(super) fn path(dir: &Path) -> PathBuf {
    dir.join("catalog.bootstrap-reads")
}
pub(super) fn sync(dir: &Path) -> Result<()> {
    File::open(path(dir))
        .and_then(|f| f.sync_all())
        .map_err(error)?;
    File::open(dir).and_then(|f| f.sync_all()).map_err(error)
}
fn prefix(identity: StoreIdentity, magic: &[u8; 8]) -> Vec<u8> {
    let mut bytes = magic.to_vec();
    bytes.extend_from_slice(identity.root_digest.as_bytes());
    bytes.extend_from_slice(&identity.node_id.0.to_be_bytes());
    bytes.extend_from_slice(identity.store_incarnation.as_bytes());
    bytes
}
pub(super) fn load(dir: &Path, identity: StoreIdentity) -> Result<State> {
    let path = path(dir);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
        Err(e) => return Err(error(e)),
    };
    if !metadata.is_file() || metadata.len() > MAX_BYTES as u64 {
        return Err(error("receipt type or size differs"));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(error)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    let legacy = bytes.starts_with(LEGACY);
    let head = prefix(identity, if legacy { LEGACY } else { MAGIC });
    if bytes.len() > MAX_BYTES || bytes.len() < head.len() + 1 + 32 || !bytes.starts_with(&head) {
        return Err(error("receipt identity or length differs"));
    }
    let end = bytes.len() - 32;
    if RootDigest::sha256(&bytes[..end]).as_bytes() != &bytes[end..] {
        return Err(error("receipt checksum differs"));
    }
    let active = bytes[head.len()] as usize;
    let retiring = if legacy {
        0
    } else {
        *bytes
            .get(head.len() + 1)
            .ok_or_else(|| error("retiring count absent"))? as usize
    };
    let start = head.len() + if legacy { 1 } else { 2 };
    let width = if legacy { 40 } else { 57 };
    if active + retiring > 2
        || (legacy && active == 0)
        || end != start + width * (active + retiring)
    {
        return Err(error("receipt slots differ"));
    }
    let mut grants = Vec::new();
    for (i, slot) in bytes[start..end].chunks_exact(width).enumerate() {
        let grant = Grant {
            predecessor: u64::from_be_bytes(slot[..8].try_into().unwrap()),
            image: slot[8..40].try_into().unwrap(),
            pending: if legacy {
                None
            } else {
                Some(OwnerId::new(slot[40..56].try_into().unwrap()).map_err(error)?)
            },
            authorized: if legacy {
                true
            } else {
                match slot[56] {
                    0 => false,
                    1 => true,
                    _ => return Err(error("invalid admission flag")),
                }
            },
        };
        if grant.predecessor == u64::MAX
            || grant.image == [0; 32]
            || (i >= active && grant.authorized)
            || grants.iter().any(|g: &Grant| {
                g.image == grant.image
                    || g.predecessor == grant.predecessor
                    || (g.pending.is_some() && g.pending == grant.pending)
            })
        {
            return Err(error("receipt grant differs"));
        }
        grants.push(grant);
    }
    let retiring = grants.split_off(active);
    Ok(State {
        active: grants,
        retiring,
    })
}
pub(super) fn save(dir: &Path, identity: StoreIdentity, state: &State) -> Result<()> {
    if state.active.len() + state.retiring.len() > 2 {
        return Err(error("receipt slot bound"));
    }
    let mut bytes = prefix(identity, MAGIC);
    bytes.push(state.active.len() as u8);
    bytes.push(state.retiring.len() as u8);
    for g in state.active.iter().chain(&state.retiring) {
        bytes.extend_from_slice(&g.predecessor.to_be_bytes());
        bytes.extend_from_slice(&g.image);
        bytes.extend_from_slice(
            g.pending
                .ok_or_else(|| error("legacy receipt needs exact history"))?
                .as_bytes(),
        );
        bytes.push(u8::from(g.authorized));
    }
    bytes.extend_from_slice(RootDigest::sha256(&bytes).as_bytes());
    let temporary = dir.join("catalog.bootstrap-reads.tmp");
    match fs::remove_file(&temporary) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(error(e)),
    }
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(error)?;
    f.write_all(&bytes).map_err(error)?;
    f.sync_all().map_err(error)?;
    fs::rename(temporary, path(dir)).map_err(error)?;
    File::open(dir).and_then(|f| f.sync_all()).map_err(error)
}
