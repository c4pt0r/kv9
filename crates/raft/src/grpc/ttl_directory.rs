//! A read-only, authenticated metadata snapshot. This is directory data, never
//! an activation capability; the data peer must bind its own full ConfState.
use super::*;
use kv9_common::{data_range::DataRange, StoreIdentity};
use prost::Message as _;
use std::time::Instant;

pub const MAX_DIRECTORY_BYTES: usize = 256 * 1024;
const MAX_REQUEST_BYTES: usize = kv9_common::data_range::MAX_ENCODED_BYTES
    + 10 * crate::rawnode::ttl_activation::MAX_ACTIVATION_REPLICAS
    + 64;
const MAX_BUDGET_MICROS: u64 = 30_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTtlStore {
    pub node: NodeId,
    pub incarnation: StoreIncarnation,
    pub address: SocketAddr,
    pub generation: u64,
    pub previous_address: Option<SocketAddr>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawTtlDirectory {
    pub cluster: ClusterId,
    pub root: RootDigest,
    pub binding_digest: RootDigest,
    pub stores: Vec<RawTtlStore>,
}
fn invalid() -> Error {
    Error::MetaNotReady("invalid Raw TTL directory observation".into())
}
fn socket(text: &str) -> kv9_common::Result<SocketAddr> {
    let address: SocketAddr = text.parse().map_err(|_| invalid())?;
    if address.port() == 0 || address.ip().is_unspecified() || address.to_string() != text {
        return Err(invalid());
    }
    Ok(address)
}
pub fn validate_query(binding: &DataRange, replicas: &[NodeId]) -> kv9_common::Result<()> {
    binding.validate()?;
    if binding.sealed
        || replicas.is_empty()
        || replicas.len() > crate::rawnode::ttl_activation::MAX_ACTIVATION_REPLICAS
        || replicas[0].0 == 0
        || replicas.windows(2).any(|w| w[0] >= w[1])
    {
        return Err(invalid());
    }
    Ok(())
}
impl RawTtlDirectory {
    pub fn validate(&self, binding: &DataRange, replicas: &[NodeId]) -> kv9_common::Result<()> {
        validate_query(binding, replicas)?;
        if self.cluster.as_bytes() == &[0; 16]
            || self.root != binding.root
            || self.binding_digest != binding.digest()
            || self.stores.len() != replicas.len()
        {
            return Err(invalid());
        }
        for (store, node) in self.stores.iter().zip(replicas) {
            if store.node != *node
                || store.incarnation.as_bytes() == &[0; 16]
                || store.address.port() == 0
                || store.address.ip().is_unspecified()
                || (store.generation == 0) != store.previous_address.is_none()
                || store
                    .previous_address
                    .is_some_and(|a| a.port() == 0 || a.ip().is_unspecified())
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
    pub fn identities(&self) -> Vec<StoreIdentity> {
        self.stores
            .iter()
            .map(|s| StoreIdentity {
                cluster_id: self.cluster,
                root_digest: self.root,
                node_id: s.node,
                store_incarnation: s.incarnation,
            })
            .collect()
    }
    fn encode(self) -> pb::RawTtlStoresResponse {
        pb::RawTtlStoresResponse {
            cluster_id: self.cluster.as_bytes().to_vec(),
            root_digest: self.root.as_bytes().to_vec(),
            binding_digest: self.binding_digest.as_bytes().to_vec(),
            stores: self
                .stores
                .into_iter()
                .map(|s| pb::RawTtlStore {
                    node_id: s.node.0,
                    store_incarnation: s.incarnation.as_bytes().to_vec(),
                    address: s.address.to_string(),
                    generation: s.generation,
                    previous_address: s.previous_address.map(|a| a.to_string()),
                })
                .collect(),
        }
    }
    fn decode(
        reply: pb::RawTtlStoresResponse,
        binding: &DataRange,
        replicas: &[NodeId],
    ) -> kv9_common::Result<Self> {
        let value = Self {
            cluster: ClusterId::from_bytes(reply.cluster_id.try_into().map_err(|_| invalid())?),
            root: RootDigest::from_bytes(reply.root_digest.try_into().map_err(|_| invalid())?),
            binding_digest: RootDigest::from_bytes(
                reply.binding_digest.try_into().map_err(|_| invalid())?,
            ),
            stores: reply
                .stores
                .into_iter()
                .map(|s| {
                    Ok(RawTtlStore {
                        node: NodeId(s.node_id),
                        incarnation: StoreIncarnation::from_bytes(
                            s.store_incarnation.try_into().map_err(|_| invalid())?,
                        ),
                        address: socket(&s.address)?,
                        generation: s.generation,
                        previous_address: s.previous_address.map(|a| socket(&a)).transpose()?,
                    })
                })
                .collect::<kv9_common::Result<Vec<_>>>()?,
        };
        value.validate(binding, replicas)?;
        Ok(value)
    }
}
impl RaftGrpcService {
    pub(super) async fn resolve_raw_ttl_stores_request(
        &self,
        request: Request<pb::RawTtlStoresRequest>,
    ) -> std::result::Result<Response<pb::RawTtlStoresResponse>, Status> {
        let started = Instant::now();
        request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        if !self.discovery.raft_receive_allowed() {
            return Err(Status::failed_precondition("replica authority unavailable"));
        }
        let request = request.into_inner();
        if request.encoded_len() > MAX_REQUEST_BYTES
            || !(1..=MAX_BUDGET_MICROS).contains(&request.budget_micros)
        {
            return Err(Status::invalid_argument("invalid TTL directory bounds"));
        }
        let binding = DataRange::decode(&request.binding)
            .map_err(|_| Status::invalid_argument("invalid TTL directory binding"))?;
        let replicas: Vec<_> = request.replicas.into_iter().map(NodeId).collect();
        validate_query(&binding, &replicas)
            .map_err(|_| Status::invalid_argument("invalid TTL directory members"))?;
        if binding.root != self.discovery.root_identity().root_digest {
            return Err(Status::failed_precondition("TTL directory root differs"));
        }
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("TTL directory backend unavailable"))?;
        let permit = self
            .ttl_directory_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("TTL directory capacity exhausted"))?;
        let deadline = started + Duration::from_micros(request.budget_micros);
        let directory = tokio::task::spawn_blocking(move || {
            let _permit = permit; // Even cancellation retains capacity until this job exits.
            let directory = backend.raw_ttl_directory(&binding, &replicas, deadline)?;
            directory.validate(&binding, &replicas)?;
            if Instant::now() >= deadline {
                return Err(invalid());
            }
            Ok::<_, Error>(directory)
        })
        .await
        .map_err(|_| Status::internal("TTL directory worker failed"))?
        .map_err(|_| Status::unavailable("TTL directory remains unconfirmed"))?;
        let response = directory.encode();
        if response.encoded_len() > MAX_DIRECTORY_BYTES {
            return Err(Status::resource_exhausted(
                "TTL directory exceeds byte bound",
            ));
        }
        Ok(Response::new(response))
    }
}

/// The same strict request/receipt path can use a caller-owned bounded channel.
/// Connections do not themselves establish metadata leadership or store identity.
pub async fn resolve_on_channel(
    channel: tonic::transport::Channel,
    binding: &DataRange,
    replicas: &[NodeId],
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<RawTtlDirectory> {
    validate_query(binding, replicas)?;
    if caller.0 == 0 || budget.is_zero() || budget > Duration::from_secs(30) {
        return Err(invalid());
    }
    let mut client = Kv9RaftClient::new(channel)
        .max_encoding_message_size(MAX_REQUEST_BYTES)
        .max_decoding_message_size(MAX_DIRECTORY_BYTES);
    let mut request = Request::new(pb::RawTtlStoresRequest {
        binding: binding.encode(),
        replicas: replicas.iter().map(|n| n.0).collect(),
        budget_micros: budget.as_micros().min(MAX_BUDGET_MICROS as u128) as u64,
    });
    attach_auth(&mut request, &token, caller);
    let reply = tokio::time::timeout(budget, client.resolve_raw_ttl_stores(request))
        .await
        .map_err(|_| invalid())?
        .map_err(|_| invalid())?
        .into_inner();
    RawTtlDirectory::decode(reply, binding, replicas)
}

#[cfg(test)]
mod tests;
