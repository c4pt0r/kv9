//! Immutable exact-window negative settlement. Only a runtime that independently
//! verified committed history may use the trusted planner. This record closes
//! admission; it grants no owner release, object deletion or PUT qualification.
use super::*;
use crate::endpoint::NodeEndpoint;
use kv9_common::{NodeId, StoreIncarnation};

const MAGIC: &[u8; 8] = b"KV9CRF01";
fn refusal_key(pending: OwnerId) -> Vec<u8> {
    key(20, pending.as_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub pending: OwnerId,
    pub winner: [u8; 32],
    pub source: NodeId,
    pub source_store: StoreIncarnation,
    pub endpoint_generation: u64,
    pub closed_at: u64,
    pub recorded_at: u64,
    intent: RootDigest,
}
impl Refusal {
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(self.pending.as_bytes());
        b.extend_from_slice(self.intent.as_bytes());
        b.extend_from_slice(&self.winner);
        uint(&mut b, self.source.0);
        b.extend_from_slice(self.source_store.as_bytes());
        uint(&mut b, self.endpoint_generation);
        uint(&mut b, self.closed_at);
        uint(&mut b, self.recorded_at);
        seal(b)
    }
}
fn planner<'a>(view: &'a dyn ReadView, root: &RootDescriptor) -> Result<Planner<'a>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("checkpoint refusal header absent"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    if !(migration_format(&h) || h.starts_with(CLOSURE_HEADER_MAGIC)) {
        return Err(conflict("checkpoint refusal requires version 7 ledger"));
    }
    Ok(Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    })
}
fn read(p: &Planner<'_>, intent: &CheckpointIntent, bytes: &[u8]) -> Result<Refusal> {
    let mut r = Reader(checked(bytes, MAGIC, 200)?);
    if r.array::<32>()? != *p.root.as_bytes() {
        return Err(invalid("checkpoint refusal root differs"));
    }
    let fact = Refusal {
        pending: OwnerId::new(r.array()?).map_err(pin_error)?,
        intent: RootDigest::from_bytes(r.array()?),
        winner: r.array()?,
        source: NodeId(r.uint()?),
        source_store: StoreIncarnation::from_bytes(r.array()?),
        endpoint_generation: r.uint()?,
        closed_at: r.uint()?,
        recorded_at: r.uint()?,
    };
    let gate = checkpoint_admission::load(p, intent)?;
    if !r.0.is_empty()
        || fact.pending != intent.pending_id()
        || fact.intent != RootDigest::sha256(&intent.encode())
        || fact.winner == [0; 32]
        || fact.winner.as_slice() == intent.manifest()?.change_id(intent.predecessor())?
        || fact.source.0 == 0
        || fact.source_store.as_bytes() == &[0; 16]
        || !gate.is_closed()
        || fact.closed_at != gate.closed_at
        || fact.recorded_at < fact.closed_at
        || fact.recorded_at > p.revision
        || fact.encode(p.root) != bytes
    {
        return Err(invalid("checkpoint refusal binding or revision differs"));
    }
    Ok(fact)
}

/// Refusal closes new Reader entry and Publish, including retries. A merely
/// Closed Build gate also occurs on positive handoff and is not a read fence.
pub(super) fn check_reader_entry(p: &Planner<'_>, source: &OwnerBinding) -> Result<()> {
    metadata_image_gate::check_reader_entry(p, source)?;
    let operation = source.descriptor.operation;
    metadata_read_gate::check_reader_entry(p, operation)?;
    let pending = checkpoint_intent::owner_id(p.root, &operation, b"kv9-checkpoint-pending-v1")?;
    let Some(bytes) = p.view.get(ColumnFamily::Default, &refusal_key(pending))? else {
        return Ok(());
    };
    let intent = checkpoint_intent::load(p, pending)?
        .ok_or_else(|| invalid("Reader refusal intent absent"))?;
    read(p, &intent, &bytes)?;
    Err(conflict("checkpoint Reader admission is refused"))
}

/// Read a historical fact from one established view. Its source may since have
/// retired. Missing evidence remains None, even for a Closed gate or empty census.
pub fn refusal(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
) -> Result<Option<Refusal>> {
    let Some(bytes) = view.get(ColumnFamily::Default, &refusal_key(pending))? else {
        return Ok(None);
    };
    let p = planner(view, root)?;
    let intent = checkpoint_intent::load(&p, pending)?
        .ok_or_else(|| invalid("checkpoint refusal intent absent"))?;
    read(&p, &intent, &bytes).map(Some)
}

/// Confirm an exact already-durable fact without depending on the old source
/// still being available. This cannot create or modify any evidence.
pub fn plan_retry(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerToken,
    winner: [u8; 32],
    source: NodeId,
) -> Result<LedgerPlan> {
    let p = planner(view, root)?;
    let intent = checkpoint_intent::load(&p, pending.owner)?
        .ok_or_else(|| conflict("checkpoint refusal intent absent"))?;
    let bytes = view
        .get(ColumnFamily::Default, &refusal_key(pending.owner))?
        .ok_or_else(|| conflict("checkpoint refusal not yet recorded"))?;
    let fact = read(&p, &intent, &bytes)?;
    if intent.binding(OwnerKind::Pending)?.token() != pending
        || fact.winner != winner
        || fact.source != source
    {
        return Err(conflict("immutable checkpoint refusal retry differs"));
    }
    p.finish()
}

/// Trusted runtime entry point, NOT an untrusted ledger request planner. The
/// caller must independently verify the exact committed CAS winner from this
/// source, then recheck the complete endpoint under the catalog lock, drain the
/// current term and commit the whole plan in that term. No network assertion or
/// local absence is proof. The ordinary planner rejects RecordCheckpointRefusal.
pub fn plan_verified(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerToken,
    winner: [u8; 32],
    source: NodeEndpoint,
) -> Result<LedgerPlan> {
    let mut p = planner(view, root)?;
    let intent = checkpoint_intent::load(&p, pending.owner)?
        .ok_or_else(|| conflict("checkpoint refusal intent absent"))?;
    if intent.binding(OwnerKind::Pending)?.token() != pending
        || winner == [0; 32]
        || winner.as_slice() == intent.manifest()?.change_id(intent.predecessor())?
        || source.node.0 == 0
        || !source.active
        || source.incarnation.as_bytes() == &[0; 16]
        || root
            .voter(source.node)
            .is_some_and(|v| v.store_incarnation != source.incarnation)
    {
        return Err(conflict(
            "checkpoint refusal winner, generation or source differs",
        ));
    }
    if let Some(bytes) = view.get(ColumnFamily::Default, &refusal_key(pending.owner))? {
        let fact = read(&p, &intent, &bytes)?;
        if fact.winner != winner || fact.source != source.node {
            return Err(conflict("immutable checkpoint refusal differs"));
        }
        return p.finish();
    }
    let gate = checkpoint_admission::load(&p, &intent)?;
    let recorded_at = p
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("ledger revision exhausted"))?;
    let fact = Refusal {
        pending: pending.owner,
        intent: RootDigest::sha256(&intent.encode()),
        winner,
        source: source.node,
        source_store: source.incarnation,
        endpoint_generation: source.generation,
        closed_at: if gate.is_closed() {
            gate.closed_at
        } else {
            recorded_at
        },
        recorded_at,
    };
    checkpoint_admission::close(&mut p, pending)?;
    p.writes
        .insert(refusal_key(pending.owner), fact.encode(p.root));
    p.finish()
}

#[cfg(test)]
mod tests;
