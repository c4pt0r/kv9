//! Latest independently confirmed prospective DATA read floor for one exact
//! store and group. No membership census, read drain or object release is implied.
use super::*;
use crate::endpoint::NodeEndpoint;
use kv9_common::data_read_fence::{ReadFloor, ReadImage};
use kv9_common::{NodeId, RegionId, StoreIdentity, StoreIncarnation};

const MAGIC: &[u8; 8] = b"KV9DCF01";
const BYTES: usize = 8 + kv9_common::data_read_fence::MAX_BYTES + 16 + 32;
fn record_key(source: NodeId, store: StoreIncarnation, region: RegionId) -> Vec<u8> {
    let mut suffix = source.0.to_be_bytes().to_vec();
    suffix.extend_from_slice(store.as_bytes());
    uint(&mut suffix, region.0);
    key(48, &suffix)
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(48, &[]), &key(49, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("DATA read-fence fact under an older ledger header"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Observation {
    pub floor: ReadFloor,
    pub endpoint_generation: u64,
    pub recorded_at: u64,
}
impl Observation {
    fn encode(&self) -> Result<Vec<u8>> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(&self.floor.encode()?);
        uint(&mut b, self.endpoint_generation);
        uint(&mut b, self.recorded_at);
        Ok(seal(b))
    }
}
fn planner<'a>(view: &'a dyn ReadView, root: &RootDescriptor) -> Result<Planner<'a>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("DATA read-fence header absent"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    if !image_publisher_format(&h) {
        return Err(conflict(
            "DATA read-fence observation requires ledger 20 or later",
        ));
    }
    if !data_fence_format(&h) {
        check_old_format(view)?;
    }
    Ok(Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    })
}
fn read(
    p: &Planner<'_>,
    root: &RootDescriptor,
    source: NodeId,
    store: StoreIncarnation,
    region: RegionId,
) -> Result<Option<Observation>> {
    if source.0 == 0 || store.as_bytes() == &[0; 16] || region.0 < 100 {
        return Err(invalid("DATA read-fence lookup identity"));
    }
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &record_key(source, store, region))?
    else {
        return Ok(None);
    };
    let payload = checked(&bytes, MAGIC, BYTES)?;
    let (floor, tail) = payload.split_at(kv9_common::data_read_fence::MAX_BYTES);
    let mut r = Reader(tail);
    let observation = Observation {
        floor: ReadFloor::decode(floor)?,
        endpoint_generation: r.uint()?,
        recorded_at: r.uint()?,
    };
    if observation.floor.identity
        != (StoreIdentity {
            cluster_id: root.cluster_id,
            root_digest: root.digest(),
            node_id: source,
            store_incarnation: store,
        })
        || observation.floor.image.region != region
        || observation.recorded_at == 0
        || observation.recorded_at > p.revision
        || !r.0.is_empty()
        || observation.encode()? != bytes
    {
        return Err(invalid("DATA read-fence row identity or revision differs"));
    }
    Ok(Some(observation))
}
pub fn observation(
    view: &dyn ReadView,
    root: &RootDescriptor,
    source: NodeId,
    store: StoreIncarnation,
    region: RegionId,
) -> Result<Option<Observation>> {
    read(&planner(view, root)?, root, source, store, region)
}
/// Expected descriptor only: this is not proof of a store's durable floor.
pub fn expected(
    root: &RootDescriptor,
    intent: &CheckpointIntent,
    source: NodeId,
    store: StoreIncarnation,
    creation: RootDigest,
) -> Result<ReadFloor> {
    let manifest = intent.manifest()?;
    if intent.root() != root.digest() || manifest.scope.cluster != root.cluster_id.to_string() {
        return Err(invalid("DATA read-fence intent root differs"));
    }
    let floor = ReadFloor {
        identity: StoreIdentity {
            cluster_id: root.cluster_id,
            root_digest: root.digest(),
            node_id: source,
            store_incarnation: store,
        },
        creation,
        image: ReadImage {
            region: RegionId(manifest.scope.region),
            digest: RootDigest::sha256(&manifest.encode()?),
            cut: manifest.position(),
            conf_ver: manifest.scope.conf_ver,
            version: manifest.scope.version,
        },
        generation: intent
            .predecessor()
            .checked_add(1)
            .ok_or_else(|| invalid("DATA floor generation exhausted"))?,
    };
    floor.validate()?;
    Ok(floor)
}
pub fn plan_retry(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerToken,
    source: NodeId,
    store: StoreIncarnation,
    creation: RootDigest,
) -> Result<LedgerPlan> {
    let p = planner(view, root)?;
    let intent = checkpoint_selection::protected_intent(&p, pending)?;
    let expected = expected(root, &intent, source, store, creation)?;
    let old = read(&p, root, source, store, expected.image.region)?
        .ok_or_else(|| conflict("DATA floor has not been independently observed"))?;
    if !old.floor.covers(&expected) {
        return Err(conflict("DATA floor does not cover retry"));
    }
    p.finish()
}
/// Trusted runtime only: obtain the exact durable descriptor from the distinct
/// inspection path and recheck the endpoint under the catalog lock before this
/// plan is committed in the drained metadata term. No owner transitions occur.
pub fn plan_verified(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerToken,
    source: NodeEndpoint,
    floor: ReadFloor,
) -> Result<LedgerPlan> {
    let mut p = planner(view, root)?;
    let intent = checkpoint_selection::protected_intent(&p, pending)?;
    if !source.active
        || source.address.port() == 0
        || source.address.ip().is_unspecified()
        || root
            .voter(source.node)
            .is_some_and(|v| v.store_incarnation != source.incarnation)
        || expected(
            root,
            &intent,
            source.node,
            source.incarnation,
            floor.creation,
        )? != floor
    {
        return Err(conflict("DATA floor source or exact checkpoint differs"));
    }
    if let Some(old) = read(
        &p,
        root,
        source.node,
        source.incarnation,
        floor.image.region,
    )? {
        if old.floor.covers(&floor) {
            return p.finish();
        }
        if !floor.covers(&old.floor) {
            return Err(conflict("DATA floor observation would regress or conflict"));
        }
    }
    let observation = Observation {
        floor,
        endpoint_generation: source.generation,
        recorded_at: p
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("ledger revision exhausted"))?,
    };
    p.writes.insert(
        record_key(source.node, source.incarnation, floor.image.region),
        observation.encode()?,
    );
    p.finish()
}

#[cfg(test)]
mod tests;
