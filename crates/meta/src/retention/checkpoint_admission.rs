//! Persistent admission fence and exact executor index. Closed prevents new
//! ledger entry/Publish, not already admitted work or legacy untracked I/O.
//! No gate, empty page or Released executor is a global drain/GC certificate.
use super::*;
use checkpoint_intent::CheckpointIntent;

pub mod drain;
pub mod enrollment;
pub mod retirement;
pub mod sweep;

const MAGIC: &[u8; 8] = b"KV9CGA01";
const INDEX_MAGIC: &[u8; 8] = b"KV9CEX01";
const PAGE_MAGIC: &[u8; 8] = b"KV9CEP01";
pub const MAX_EXECUTOR_PAGE_BYTES: usize = 16 * (MAX_OWNER_BYTES + 4) + 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admission {
    pub pending: OwnerId,
    pub opened_at: u64,
    pub closed_at: u64,
    digest: RootDigest,
}
fn gate_key(pending: OwnerId) -> Vec<u8> {
    key(9, pending.as_bytes())
}
fn index_key(pending: OwnerId, owner: OwnerId) -> Vec<u8> {
    let mut id = pending.as_bytes().to_vec();
    id.extend_from_slice(owner.as_bytes());
    key(10, &id)
}
impl Admission {
    pub fn is_closed(&self) -> bool {
        self.closed_at != 0
    }
    fn encode(&self, root: RootDigest) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(self.pending.as_bytes());
        b.extend_from_slice(self.digest.as_bytes());
        uint(&mut b, self.opened_at);
        uint(&mut b, self.closed_at);
        seal(b)
    }
}
pub(super) fn load(p: &Planner<'_>, intent: &CheckpointIntent) -> Result<Admission> {
    if !p
        .view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .is_some_and(|h| {
            (migration_format(&h) || h.starts_with(CLOSURE_HEADER_MAGIC))
                || h.starts_with(ATOMIC_HEADER_MAGIC)
                || h.starts_with(ENROLLMENT_HEADER_MAGIC)
                || h.starts_with(BUILD_INDEX_HEADER_MAGIC)
                || h.starts_with(GATED_HEADER_MAGIC)
        })
    {
        return Err(invalid("checkpoint gate requires version 3 ledger"));
    }
    let bytes = p
        .view
        .get(ColumnFamily::Default, &gate_key(intent.pending_id()))?
        .ok_or_else(|| invalid("checkpoint admission missing"))?;
    let mut r = Reader(checked(&bytes, MAGIC, 136)?);
    if r.array::<32>()? != *p.root.as_bytes() || r.array::<16>()? != *intent.pending_id().as_bytes()
    {
        return Err(invalid("checkpoint admission identity differs"));
    }
    let gate = Admission {
        pending: intent.pending_id(),
        digest: RootDigest::from_bytes(r.array()?),
        opened_at: r.uint()?,
        closed_at: r.uint()?,
    };
    if !r.0.is_empty()
        || gate.digest != RootDigest::sha256(&intent.encode())
        || gate.opened_at == 0
        || gate.opened_at > p.revision
        || (gate.is_closed() && (gate.closed_at < gate.opened_at || gate.closed_at > p.revision))
    {
        return Err(invalid("checkpoint admission digest or revision differs"));
    }
    Ok(gate)
}
pub(super) fn create(p: &mut Planner<'_>, intent: &CheckpointIntent) -> Result<()> {
    if p.view
        .get(ColumnFamily::Default, &gate_key(intent.pending_id()))?
        .is_some()
        || p.view
            .get(
                ColumnFamily::Default,
                &key(20, intent.pending_id().as_bytes()),
            )?
            .is_some()
    {
        return Err(invalid(
            "checkpoint admission or refusal exists without its gated intent",
        ));
    }
    let prefix = key(10, intent.pending_id().as_bytes());
    let mut end = prefix.clone();
    while end.last() == Some(&255) {
        end.pop();
    }
    *end.last_mut().unwrap() += 1;
    if p.view
        .iter(ColumnFamily::Default, &prefix, &end)?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("orphan checkpoint executor index"));
    }
    let gate = Admission {
        pending: intent.pending_id(),
        digest: RootDigest::sha256(&intent.encode()),
        opened_at: p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("ledger revision exhausted"))?,
        closed_at: 0,
    };
    p.writes.insert(gate_key(gate.pending), gate.encode(p.root));
    Ok(())
}
pub(super) fn close(p: &mut Planner<'_>, token: OwnerToken) -> Result<()> {
    let intent = checkpoint_intent::load(p, token.owner)?
        .ok_or_else(|| conflict("checkpoint intent absent"))?;
    if intent.binding(OwnerKind::Pending)?.token() != token {
        return Err(conflict("checkpoint close generation differs"));
    }
    let mut gate = load(p, &intent)?;
    if !gate.is_closed() {
        gate.closed_at = p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("ledger revision exhausted"))?;
        p.writes.insert(gate_key(gate.pending), gate.encode(p.root));
    }
    Ok(())
}

pub fn executor_binding(
    intent: &CheckpointIntent,
    lifetime: LocalLifetime,
) -> Result<OwnerBinding> {
    let mut binding = intent.binding(OwnerKind::Pending)?;
    let mut bytes = b"kv9-checkpoint-executor-v1".to_vec();
    bytes.extend_from_slice(intent.root().as_bytes());
    bytes.extend_from_slice(&binding.descriptor.operation);
    uint(&mut bytes, lifetime.node);
    bytes.extend_from_slice(&lifetime.store);
    bytes.extend_from_slice(&lifetime.process);
    binding.descriptor.id = OwnerId::new(
        RootDigest::sha256(&bytes).as_bytes()[..16]
            .try_into()
            .unwrap(),
    )
    .map_err(pin_error)?;
    binding.descriptor.kind = OwnerKind::Build;
    binding.descriptor.local = Some(lifetime);
    binding.validate(intent.root())?;
    Ok(binding)
}
pub(super) fn managed(p: &Planner<'_>, binding: &OwnerBinding) -> Result<Option<Admission>> {
    let Some((intent, gate)) = managed_intent(p, binding)? else {
        return Ok(None);
    };
    checkpoint_intent::check_owners(p, &intent)?;
    Ok(Some(gate))
}
/// Recognize an existing executor's immutable binding and gate without
/// depending on the original Pending/Version still being Published.
pub(super) fn retained(p: &Planner<'_>, binding: &OwnerBinding) -> Result<Option<Admission>> {
    Ok(managed_intent(p, binding)?.map(|(_, gate)| gate))
}
fn managed_intent(
    p: &Planner<'_>,
    binding: &OwnerBinding,
) -> Result<Option<(CheckpointIntent, Admission)>> {
    if binding.descriptor.kind != OwnerKind::Build {
        return Ok(None);
    }
    let pending = checkpoint_intent::owner_id(
        p.root,
        &binding.descriptor.operation,
        b"kv9-checkpoint-pending-v1",
    )?;
    let Some(intent) = checkpoint_intent::retained(p, pending)? else {
        if p.view
            .get(ColumnFamily::Default, &gate_key(pending))?
            .is_some()
        {
            return Err(invalid("gated checkpoint intent missing"));
        }
        return Ok(None);
    };
    let mut expected = executor_binding(
        &intent,
        binding
            .descriptor
            .local
            .ok_or_else(|| invalid("checkpoint executor lifetime missing"))?,
    )?;
    expected.generation = binding.generation;
    if expected != *binding {
        return Err(invalid("checkpoint executor binding differs"));
    }
    let gate = load(p, &intent)?;
    Ok(Some((intent, gate)))
}
pub(super) fn entry(p: &Planner<'_>, binding: &OwnerBinding) -> Result<Option<OwnerId>> {
    if binding.descriptor.kind == OwnerKind::Build {
        metadata_image_write_gate::check_entry(p, binding)?;
    }
    let gate = managed(p, binding)?;
    if gate.as_ref().is_some_and(Admission::is_closed) {
        return Err(conflict("checkpoint executor admission is closed"));
    }
    Ok(gate.map(|g| g.pending))
}
fn index(root: RootDigest, pending: OwnerId, owner: &LedgerOwner) -> Vec<u8> {
    let mut bytes = INDEX_MAGIC.to_vec();
    bytes.extend_from_slice(root.as_bytes());
    bytes.extend_from_slice(pending.as_bytes());
    bytes.extend_from_slice(owner.binding.descriptor.id.as_bytes());
    uint(&mut bytes, owner.binding.generation.get());
    bytes.push(owner.phase as u8);
    bytes.extend_from_slice(RootDigest::sha256(&owner_registration(&owner.binding)).as_bytes());
    seal(bytes)
}
pub(super) fn encode_executor(owner: &LedgerOwner) -> Vec<u8> {
    let mut bytes = encode_owner(owner);
    bytes.truncate(bytes.len() - 32);
    bytes[..8].copy_from_slice(EXECUTOR_OWNER_MAGIC);
    seal(bytes)
}
pub(super) fn executor_registration(binding: &OwnerBinding) -> Vec<u8> {
    let mut binding = binding.clone();
    binding.generation = NonZeroU64::new(1).unwrap();
    encode_executor(&LedgerOwner {
        binding,
        phase: PinPhase::Held,
    })
}
pub(super) fn write_index(
    p: &mut Planner<'_>,
    pending: OwnerId,
    owner: &LedgerOwner,
) -> Result<()> {
    build_index::record(p, &owner.binding)?;
    for (key, value) in [
        (
            index_key(pending, owner.binding.descriptor.id),
            index(p.root, pending, owner),
        ),
        (
            owner_key(owner.binding.descriptor.id),
            encode_executor(owner),
        ),
        (
            owner_registration_key(owner.binding.descriptor.id),
            executor_registration(&owner.binding),
        ),
    ] {
        if p.view.get(ColumnFamily::Default, &key)?.as_ref() != Some(&value) {
            p.writes.insert(key, value);
        }
    }
    Ok(())
}
pub(super) fn check_index(p: &Planner<'_>, owner: &LedgerOwner) -> Result<()> {
    let gate = retained(p, &owner.binding)?
        .ok_or_else(|| invalid("executor owner lost its checkpoint gate"))?;
    let bytes = p.view.get(
        ColumnFamily::Default,
        &index_key(gate.pending, owner.binding.descriptor.id),
    )?;
    if bytes != Some(index(p.root, gate.pending, owner)) {
        return Err(invalid("checkpoint executor index missing or disagrees"));
    }
    Ok(())
}

pub(super) fn cancel(p: &mut Planner<'_>, binding: &OwnerBinding) -> Result<()> {
    binding.validate(p.root)?;
    let gate = managed(p, binding)?
        .ok_or_else(|| invalid("unentered cancellation requires checkpoint gate"))?;
    let previous = match p.owner(binding.descriptor.id)? {
        None => {
            if binding.generation.get() != 1 {
                return Err(conflict("initial cancelled generation must be one"));
            }
            for resource in &binding.resources {
                if p.resource(*resource)?
                    .owner(p.view, binding.descriptor.id)?
                    .is_some()
                {
                    return Err(invalid("orphan resource pin during cancellation"));
                }
            }
            p.writes.insert(
                owner_registration_key(binding.descriptor.id),
                owner_registration(binding),
            );
            None
        }
        Some(old) => {
            if old.binding.descriptor != binding.descriptor
                || old.binding.resources != binding.resources
            {
                return Err(conflict("cancelled executor binding differs"));
            }
            if old.phase != PinPhase::Released {
                return Err(conflict("entered executor must drain and transfer"));
            }
            if old.binding.generation == binding.generation {
                // A Released generation may have uploaded before generic
                // handoff. A cancellation retry cannot manufacture provenance
                // or upgrade its original owner format.
                return Ok(());
            }
            if old.binding.generation.get().checked_add(1) != Some(binding.generation.get()) {
                return Err(conflict("cancelled executor generation differs"));
            }
            Some(OwnerSlot {
                generation: old.binding.generation,
                phase: old.phase,
            })
        }
    };
    let owner = LedgerOwner {
        binding: binding.clone(),
        phase: PinPhase::Released,
    };
    enrollment::record_new(p, binding, enrollment::Origin::Cancelled)?;
    p.update_resources(&owner, previous, PinPhase::Released)?;
    write_index(p, gate.pending, &owner)
}

/// Read-only observation of this gate and a bounded page of tracked executors.
/// Legacy untracked publishers are not certified absent when a page ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorPage {
    pub admission: Admission,
    pub owners: Vec<LedgerOwner>,
    pub next: Option<OwnerId>,
}
impl ExecutorPage {
    pub fn encode(&self, root: RootDigest) -> Result<Vec<u8>> {
        if self.owners.len() > 16 {
            return Err(invalid("executor page count"));
        }
        let mut b = PAGE_MAGIC.to_vec();
        b.extend_from_slice(&self.admission.encode(root));
        b.push(u8::from(self.next.is_some()));
        if let Some(next) = self.next {
            b.extend_from_slice(next.as_bytes());
        }
        b.extend_from_slice(&(self.owners.len() as u32).to_be_bytes());
        for owner in &self.owners {
            let encoded = encode_owner_observation(owner)?;
            b.extend_from_slice(&(encoded.len() as u32).to_be_bytes());
            b.extend_from_slice(&encoded);
        }
        if b.len() + 32 > MAX_EXECUTOR_PAGE_BYTES {
            return Err(invalid("executor page byte bound"));
        }
        Ok(seal(b))
    }
    pub fn decode(
        bytes: &[u8],
        root: RootDigest,
        pending: OwnerId,
        after: Option<OwnerId>,
        limit: usize,
    ) -> Result<Self> {
        if !(1..=16).contains(&limit) || root.as_bytes() == &[0; 32] {
            return Err(invalid("executor page query bounds"));
        }
        let mut r = Reader(checked(bytes, PAGE_MAGIC, MAX_EXECUTOR_PAGE_BYTES)?);
        let mut g = Reader(checked(r.take(136)?, MAGIC, 136)?);
        if g.array::<32>()? != *root.as_bytes() || g.array::<16>()? != *pending.as_bytes() {
            return Err(invalid("executor page identity differs"));
        }
        let admission = Admission {
            pending,
            digest: RootDigest::from_bytes(g.array()?),
            opened_at: g.uint()?,
            closed_at: g.uint()?,
        };
        if !g.0.is_empty()
            || admission.opened_at == 0
            || (admission.is_closed() && admission.closed_at < admission.opened_at)
        {
            return Err(invalid("executor page admission revision"));
        }
        let next = if r.boolean()? {
            Some(OwnerId::new(r.array()?).map_err(pin_error)?)
        } else {
            None
        };
        let count = u32::from_be_bytes(r.array()?) as usize;
        if count > limit {
            return Err(invalid("executor page count exceeds query"));
        }
        let mut owners = Vec::with_capacity(count);
        let mut previous = after;
        for _ in 0..count {
            let len = u32::from_be_bytes(r.array()?) as usize;
            if !(56..=MAX_OWNER_BYTES).contains(&len) {
                return Err(invalid("executor page owner length"));
            }
            let bytes = r.take(len)?;
            let id = OwnerId::new(bytes[40..56].try_into().unwrap()).map_err(pin_error)?;
            let owner = decode_owner_observation(bytes, root, id)?;
            if previous.is_some_and(|p| p >= id)
                || owner.binding.descriptor.kind != OwnerKind::Build
                || owner.binding.descriptor.local.is_none()
                || checkpoint_intent::owner_id(
                    root,
                    &owner.binding.descriptor.operation,
                    b"kv9-checkpoint-pending-v1",
                )? != pending
            {
                return Err(invalid("executor page binding or order differs"));
            }
            previous = Some(id);
            owners.push(owner);
        }
        if !r.0.is_empty() || (next.is_some() && (count != limit || next != previous)) {
            return Err(invalid("executor page cursor differs"));
        }
        let page = Self {
            admission,
            owners,
            next,
        };
        if page.encode(root)? != bytes {
            return Err(invalid("executor page encoding is not canonical"));
        }
        Ok(page)
    }
}
pub fn executors(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
    after: Option<OwnerId>,
    limit: usize,
) -> Result<ExecutorPage> {
    if !(1..=16).contains(&limit) {
        return Err(invalid("executor page limit"));
    }
    let intent = checkpoint_intent::checkpoint_intent(view, root, pending)?
        .ok_or_else(|| invalid("checkpoint intent missing"))?;
    let header = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("ledger header missing"))?;
    let (revision, rows_enabled) = read_header(&header, root.digest())?;
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    let admission = load(&p, &intent)?;
    let prefix = key(10, pending.as_bytes());
    let mut end = prefix.clone();
    while end.last() == Some(&255) {
        end.pop();
    }
    *end.last_mut().unwrap() += 1;
    let mut start = after.map_or_else(|| prefix.clone(), |id| index_key(pending, id));
    if after.is_some() {
        start.push(0);
    }
    let mut owners = Vec::with_capacity(limit);
    let mut next = None;
    for entry in view
        .iter(ColumnFamily::Default, &start, &end)?
        .take(limit + 1)
    {
        let (key, bytes) = entry?;
        if key.len() != prefix.len() + 16 {
            return Err(invalid("executor index key length"));
        }
        let id = OwnerId::new(key[prefix.len()..].try_into().unwrap()).map_err(pin_error)?;
        let owner = p
            .owner(id)?
            .ok_or_else(|| invalid("executor index owner missing"))?;
        if bytes != index(p.root, pending, &owner) {
            return Err(invalid("executor page link differs"));
        }
        if owners.len() == limit {
            next = owners.last().map(|o: &LedgerOwner| o.binding.descriptor.id);
            break;
        }
        owners.push(owner);
    }
    Ok(ExecutorPage {
        admission,
        owners,
        next,
    })
}

#[cfg(test)]
mod tests;
