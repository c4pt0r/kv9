//! Latest independently observed selection per (node, store incarnation, group).
//! Metadata selections additionally bind the exact persistent bootstrap grant.
//! This is a historical lower bound, not a current remote read, portable
//! recovery anchor, complete replica census or permission to retire Version.
use super::*;
use crate::endpoint::NodeEndpoint;
use kv9_common::{NodeId, RegionId, StoreIncarnation};

const MAGIC: &[u8; 8] = b"KV9CSR01";
const METADATA_MAGIC: &[u8; 8] = b"KV9CMS01";
pub(super) fn check_old_metadata_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(31, &[]), &key(32, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("metadata selection under an older ledger header"));
    }
    Ok(())
}
fn selection_key(source: NodeId, store: StoreIncarnation, region: RegionId) -> Vec<u8> {
    let mut suffix = source.0.to_be_bytes().to_vec();
    suffix.extend_from_slice(store.as_bytes());
    uint(&mut suffix, region.0);
    key(
        if region == kv9_common::META_REGION_0 {
            31
        } else {
            21
        },
        &suffix,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub source: NodeId,
    pub store: StoreIncarnation,
    pub endpoint_generation: u64,
    pub recorded_at: u64,
    /// Exact historical bootstrap grant for metadata; never a current read lease.
    pub bootstrap: Option<OwnerToken>,
    intent: CheckpointIntent,
}
impl Selection {
    pub fn intent(&self) -> &CheckpointIntent {
        &self.intent
    }
    /// A later observation can acknowledge an older retry without asserting
    /// that the older image was ever locally selected. Incomparable windows,
    /// epochs or image positions cannot advance or satisfy this lower bound.
    pub fn covers(&self, intent: &CheckpointIntent) -> Result<bool> {
        if self.intent == *intent {
            return Ok(true);
        }
        let current = self.intent.manifest()?;
        let prior = intent.manifest()?;
        Ok(self.intent.root() == intent.root()
            && self.intent.predecessor() > intent.predecessor()
            && current.scope.cluster == prior.scope.cluster
            && current.scope.region == prior.scope.region
            && current.scope.conf_ver >= prior.scope.conf_ver
            && current.scope.version >= prior.scope.version
            && current.index > prior.index
            && current.term >= prior.term)
    }
    fn encode(&self) -> Result<Vec<u8>> {
        let mut b = if self.bootstrap.is_some() {
            METADATA_MAGIC
        } else {
            MAGIC
        }
        .to_vec();
        b.extend_from_slice(self.intent.root().as_bytes());
        uint(&mut b, self.source.0);
        b.extend_from_slice(self.store.as_bytes());
        uint(&mut b, self.intent.manifest()?.scope.region);
        b.extend_from_slice(self.intent.pending_id().as_bytes());
        b.extend_from_slice(RootDigest::sha256(&self.intent.encode()).as_bytes());
        uint(&mut b, self.endpoint_generation);
        uint(&mut b, self.recorded_at);
        if let Some(grant) = self.bootstrap {
            b.extend_from_slice(grant.owner.as_bytes());
            uint(&mut b, grant.generation.get());
        }
        Ok(seal(b))
    }
}
fn planner<'a>(view: &'a dyn ReadView, root: &RootDescriptor) -> Result<Planner<'a>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("checkpoint selection header absent"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    if !(migration_format(&h) || h.starts_with(CLOSURE_HEADER_MAGIC)) {
        return Err(conflict("checkpoint selection requires version 7 ledger"));
    }
    Ok(Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    })
}
pub(super) fn protected_intent(p: &Planner<'_>, pending: OwnerToken) -> Result<CheckpointIntent> {
    let intent = checkpoint_intent::load(p, pending.owner)?
        .ok_or_else(|| invalid("checkpoint selection intent absent"))?;
    let version = intent.binding(OwnerKind::Version)?;
    let owner = p.exact_owner(version.token())?;
    if intent.binding(OwnerKind::Pending)?.token() != pending
        || owner.binding != version
        || owner.phase != PinPhase::Published
    {
        return Err(conflict(
            "checkpoint selection lacks exact Published Version",
        ));
    }
    Ok(intent)
}
fn bootstrap(
    p: &Planner<'_>,
    root: &RootDescriptor,
    intent: &CheckpointIntent,
    node: NodeId,
    store: StoreIncarnation,
) -> Result<LedgerOwner> {
    if root.voter(node).is_none() {
        return Err(conflict(
            "metadata selection store is not a provisioned voter",
        ));
    }
    let identity = kv9_common::StoreIdentity {
        cluster_id: root.cluster_id,
        root_digest: root.digest(),
        node_id: node,
        store_incarnation: store,
    };
    let binding =
        bootstrap_reader::binding(root, &intent.manifest()?, intent.predecessor(), identity)?;
    let grant = p.exact_owner(binding.token())?;
    if grant.binding != binding {
        return Err(conflict("metadata selection bootstrap differs"));
    }
    Ok(grant)
}
fn read(
    p: &Planner<'_>,
    root: &RootDescriptor,
    source: NodeId,
    store: StoreIncarnation,
    region: RegionId,
) -> Result<Option<Selection>> {
    let metadata = region == kv9_common::META_REGION_0;
    if source.0 == 0 || store.as_bytes() == &[0; 16] || (!metadata && region.0 < 100) {
        return Err(invalid("checkpoint selection source, store or region"));
    }
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &selection_key(source, store, region))?
    else {
        return Ok(None);
    };
    if metadata
        && !p
            .view
            .get(ColumnFamily::Default, &key(1, &[]))?
            .is_some_and(|h| metadata_selection_format(&h))
    {
        return Err(invalid("metadata selection requires version 14 ledger"));
    }
    let mut r = Reader(checked(
        &bytes,
        if metadata { METADATA_MAGIC } else { MAGIC },
        if metadata { 192 } else { 168 },
    )?);
    if r.array::<32>()? != *p.root.as_bytes()
        || r.uint()? != source.0
        || r.array::<16>()? != *store.as_bytes()
        || r.uint()? != region.0
    {
        return Err(invalid("checkpoint selection key binding differs"));
    }
    let pending = OwnerId::new(r.array()?).map_err(pin_error)?;
    let digest = RootDigest::from_bytes(r.array()?);
    let selection = Selection {
        source,
        store,
        endpoint_generation: r.uint()?,
        recorded_at: r.uint()?,
        bootstrap: if metadata {
            Some(OwnerToken {
                owner: OwnerId::new(r.array()?).map_err(pin_error)?,
                generation: NonZeroU64::new(r.uint()?)
                    .ok_or_else(|| invalid("metadata grant generation zero"))?,
            })
        } else {
            None
        },
        intent: protected_intent(
            p,
            OwnerToken {
                owner: pending,
                generation: NonZeroU64::new(1).unwrap(),
            },
        )?,
    };
    if metadata {
        let grant = bootstrap(p, root, &selection.intent, source, store)?;
        if selection.bootstrap != Some(grant.binding.token())
            || !matches!(grant.phase, PinPhase::Published | PinPhase::Released)
        {
            return Err(invalid("metadata selection bootstrap link differs"));
        }
    }
    if !r.0.is_empty()
        || selection.recorded_at == 0
        || selection.recorded_at > p.revision
        || selection.intent.manifest()?.scope.region != region.0
        || digest != RootDigest::sha256(&selection.intent.encode())
        || selection.encode()? != bytes
    {
        return Err(invalid("checkpoint selection intent or revision differs"));
    }
    Ok(Some(selection))
}
pub fn selection(
    view: &dyn ReadView,
    root: &RootDescriptor,
    source: NodeId,
    store: StoreIncarnation,
    region: RegionId,
) -> Result<Option<Selection>> {
    read(&planner(view, root)?, root, source, store, region)
}
pub fn plan_retry(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerToken,
    source: NodeId,
    store: StoreIncarnation,
) -> Result<LedgerPlan> {
    let p = planner(view, root)?;
    let intent = protected_intent(&p, pending)?;
    let current = read(
        &p,
        root,
        source,
        store,
        RegionId(intent.manifest()?.scope.region),
    )?
    .ok_or_else(|| conflict("checkpoint selection not yet observed"))?;
    if !current.covers(&intent)? {
        return Err(conflict("checkpoint selection does not cover retry"));
    }
    p.finish()
}
/// Trusted runtime entry only: independently query the exact source store's
/// live selection and committed history, recheck its endpoint under the catalog
/// lock, drain the metadata term and append in that same term. Public request
/// planning cannot manufacture this fact. No resource pins change here.
pub fn plan_verified(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerToken,
    source: NodeEndpoint,
) -> Result<LedgerPlan> {
    let mut p = planner(view, root)?;
    let intent = protected_intent(&p, pending)?;
    if !source.active
        || source.node.0 == 0
        || source.incarnation.as_bytes() == &[0; 16]
        || root
            .voter(source.node)
            .is_some_and(|v| v.store_incarnation != source.incarnation)
    {
        return Err(conflict("checkpoint selection source differs"));
    }
    let region = RegionId(intent.manifest()?.scope.region);
    let old = read(&p, root, source.node, source.incarnation, region)?;
    if let Some(old) = &old {
        if old.covers(&intent)? {
            return p.finish();
        }
    }
    let grant = if intent.manifest()?.scope.region == kv9_common::META_REGION_0.0 {
        let grant = bootstrap(&p, root, &intent, source.node, source.incarnation)?;
        if grant.phase != PinPhase::Published {
            return Err(conflict(
                "metadata selection requires Published bootstrap grant",
            ));
        }
        Some(grant.binding.token())
    } else {
        None
    };
    let next = Selection {
        source: source.node,
        bootstrap: grant,
        store: source.incarnation,
        endpoint_generation: source.generation,
        recorded_at: p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("ledger revision exhausted"))?,
        intent,
    };
    if let Some(old) = old {
        if !next.covers(&old.intent)? {
            return Err(conflict("checkpoint selection would regress or conflict"));
        }
    }
    p.writes.insert(
        selection_key(source.node, source.incarnation, region),
        next.encode()?,
    );
    p.finish()
}

#[cfg(test)]
mod tests;
