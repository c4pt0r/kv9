//! Request routing (DESIGN §11, §6.1).
//!
//! Resolve keyspace → region, epoch-check, and validate the request's API type against
//! the keyspace declaration, before dispatching to the txn or raw executor.

use kv9_common::codec::encode_key;
use kv9_common::{ApiType, Error, Keyspace, KeyspaceId, Result, UserKey};
use kv9_meta::Catalog;
use kv9_region::{RegionEpoch, RegionRouter};

/// The outcome of routing a request (DESIGN §11).
#[derive(Debug, Clone)]
pub struct Routed {
    pub keyspace: KeyspaceId,
    pub region: kv9_common::RegionId,
    pub epoch: RegionEpoch,
    pub api_type: ApiType,
    /// The physical (prefix-encoded) key within the region's range.
    pub physical_key: UserKey,
}

/// Route a single-key data request (DESIGN §11):
/// 1. look up the keyspace in the catalog,
/// 2. validate the request's API type matches the keyspace declaration,
/// 3. encode the physical key (prefix), resolve the region, and epoch-check.
pub fn route_request(
    catalog: &Catalog,
    router: &RegionRouter,
    keyspace_id: KeyspaceId,
    req_api: ApiType,
    req_epoch: &RegionEpoch,
    user_key: &[u8],
) -> Result<Routed> {
    let ks: &Keyspace = catalog.keyspace(keyspace_id)?;
    if ks.api_type != req_api {
        return Err(Error::ApiTypeMismatch {
            keyspace: keyspace_id,
        });
    }
    let physical_key = encode_key(ks.key_mode(), keyspace_id, user_key)?;
    let region = router.route(&physical_key)?;
    router.check_epoch(region.id, req_epoch)?;
    Ok(Routed {
        keyspace: keyspace_id,
        region: region.id,
        epoch: region.epoch,
        api_type: ks.api_type,
        physical_key,
    })
}
