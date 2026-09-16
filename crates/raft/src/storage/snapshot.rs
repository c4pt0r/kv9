//! Durable protocol half of snapshot installation. This does not install an
//! engine, certify remote authority, pin objects or enable snapshot reception.
//! The existing peer constructors refuse this base until the engine journal is
//! integrated. Keep that gate while developing the higher-level protocol.
use super::*;
use raft::prelude::Snapshot;
use raft::Storage;
use std::collections::BTreeSet;

pub const MAX_PROTOCOL_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"KV9RSN01";

fn invalid(reason: &str) -> Error {
    Error::Raft(format!("protocol snapshot: {reason}"))
}

fn validate(image: &Snapshot, hs: &HardState) -> Result<()> {
    let meta = image.get_metadata();
    let cs = meta.get_conf_state();
    if image.compute_size() as usize > MAX_PROTOCOL_SNAPSHOT_BYTES
        || image.data.is_empty()
        || image.get_unknown_fields().iter().next().is_some()
        || meta.get_unknown_fields().iter().next().is_some()
        || cs.get_unknown_fields().iter().next().is_some()
        || hs.get_unknown_fields().iter().next().is_some()
    {
        return Err(invalid("oversized, empty or unsupported snapshot"));
    }
    if meta.index == 0
        || meta.index == u64::MAX
        || meta.term == 0
        || hs.commit != meta.index
        || hs.term < meta.term
    {
        return Err(invalid("snapshot cut and HardState disagree"));
    }
    let sets = [
        cs.get_voters(),
        cs.get_voters_outgoing(),
        cs.get_learners(),
        cs.get_learners_next(),
    ];
    for nodes in sets {
        if nodes.len() > kv9_common::anchor::MAX_ANCHOR_MEMBERS
            || nodes.contains(&0)
            || nodes.iter().copied().collect::<BTreeSet<_>>().len() != nodes.len()
        {
            return Err(invalid("invalid configuration members"));
        }
    }
    if cs.voters.is_empty()
        || cs
            .learners
            .iter()
            .any(|n| cs.voters.contains(n) || cs.voters_outgoing.contains(n))
        || cs.learners_next.iter().any(|n| {
            !cs.voters_outgoing.contains(n) || cs.voters.contains(n) || cs.learners.contains(n)
        })
        || (cs.voters_outgoing.is_empty() && (cs.auto_leave || !cs.learners_next.is_empty()))
    {
        return Err(invalid("invalid joint configuration"));
    }
    Ok(())
}

pub(super) fn validate_transition(
    memory: &MemStorage,
    prior: Option<&Snapshot>,
    image: &Snapshot,
    hs: &HardState,
) -> Result<()> {
    validate(image, hs)?;
    let old = memory
        .initial_state()
        .map_err(|e| invalid(&e.to_string()))?
        .hard_state;
    if hs.term < old.term
        || (hs.term == old.term && old.vote != 0 && hs.vote != old.vote)
        || image.get_metadata().index <= old.commit
        || prior.is_some_and(|old| image.get_metadata().index <= old.get_metadata().index)
    {
        return Err(invalid(
            "installation lowers term/vote or replaces committed history",
        ));
    }
    // A snapshot whose cut predates a later durable log term is not a valid
    // extension of that log. Its authority must be checked above this layer.
    if old.commit != 0
        && memory
            .term(old.commit)
            .map_err(|e| invalid(&e.to_string()))?
            > image.get_metadata().term
    {
        return Err(invalid("snapshot term predates committed history"));
    }
    Ok(())
}

fn encode(image: &Snapshot, hs: &HardState) -> Result<Vec<u8>> {
    validate(image, hs)?;
    let pb = image
        .write_to_bytes()
        .map_err(|e| invalid(&e.to_string()))?;
    let state = hs.write_to_bytes().map_err(|e| invalid(&e.to_string()))?;
    let mut out = Vec::with_capacity(16 + pb.len() + state.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(pb.len() as u32).to_be_bytes());
    out.extend_from_slice(&pb);
    out.extend_from_slice(&(state.len() as u32).to_be_bytes());
    out.extend_from_slice(&state);
    // The existing outer record checksum covers the complete pair and lengths.
    Ok(out)
}

pub(super) fn decode(bytes: &[u8]) -> Result<(Snapshot, HardState)> {
    if bytes.len() < 16 || bytes.len() > MAX_PROTOCOL_SNAPSHOT_BYTES + 128 || &bytes[..8] != MAGIC {
        return Err(invalid("invalid snapshot record envelope"));
    }
    let n = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if n > MAX_PROTOCOL_SNAPSHOT_BYTES || n > bytes.len() - 16 {
        return Err(invalid("invalid snapshot length"));
    }
    let end = 12 + n;
    let count = u32::from_be_bytes(bytes[end..end + 4].try_into().unwrap()) as usize;
    if count > 64 || bytes.len() - end - 4 != count {
        return Err(invalid("invalid HardState length"));
    }
    let image = Snapshot::parse_from_bytes(&bytes[12..end]).map_err(|e| invalid(&e.to_string()))?;
    let hs = HardState::parse_from_bytes(&bytes[end + 4..]).map_err(|e| invalid(&e.to_string()))?;
    validate(&image, &hs)?;
    // Require canonical protobuf bytes so ignored/duplicate field encodings
    // cannot create a second identity for the selected record.
    if encode(&image, &hs)? != bytes {
        return Err(invalid("noncanonical snapshot record"));
    }
    Ok((image, hs))
}

impl<F: FileSystem> DiskRaftStorage<F> {
    /// Persist one forward protocol snapshot with its exact HardState. This is
    /// a low-level storage operation, NOT remote admission or engine installation.
    /// A caller must own the store and validate image/retention authority first.
    /// Current peer constructors deliberately refuse snapshot-backed stores.
    ///
    /// This replaces the logical prefix, not the physical append-only file.
    /// There is no disk-space reclamation or automatic network install yet.
    pub fn install_protocol_snapshot(&self, image: &Snapshot, hs: &HardState) -> Result<()> {
        let bytes = encode(image, hs)?;
        let mut writer = self.file.lock().expect("raft log file poisoned");
        let file = writer
            .as_mut()
            .ok_or_else(|| invalid("failed writer requires recovery"))?;
        if self
            .lease_epoch
            .lock()
            .expect("lease epoch poisoned")
            .is_some()
        {
            return Err(invalid("lease configuration cannot be replaced"));
        }
        let mut selected = self.snapshot.lock().expect("snapshot poisoned");
        // Exact retry has no IO. A later commit/term/vote makes a stale retry a
        // refusal, never a rollback to the old installation HardState.
        if selected.as_ref() == Some(image)
            && self
                .mem
                .initial_state()
                .map_err(|e| invalid(&e.to_string()))?
                .hard_state
                == *hs
        {
            return Ok(());
        }
        validate_transition(&self.mem, selected.as_ref(), image, hs)?;
        let result = (|| {
            Self::write_record(&self.io_metrics, file, REC_SNAPSHOT, &bytes)?;
            let mut memory = self.mem.wl();
            memory
                .apply_snapshot(image.clone())
                .map_err(|e| invalid(&e.to_string()))?;
            memory.set_hardstate(hs.clone());
            *self.conf_index.lock().expect("conf index poisoned") = image.get_metadata().index;
            *self.conf_history.lock().expect("conf history poisoned") =
                ConfigurationHistory::default();
            *selected = Some(image.clone());
            Ok(())
        })();
        if result.is_err() {
            *writer = None;
        }
        result
    }
}

#[cfg(test)]
mod tests;
