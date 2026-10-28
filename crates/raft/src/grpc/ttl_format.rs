//! Authenticated, exact-store confirmation of the durable TTL format floor.
//! No cached connection/version hint is sufficient, and no legacy RPC fallback
//! is permitted. One confirmation is not group activation authority.
use super::*;
use kv9_common::store_lifecycle::RAW_TTL_FORMAT_VERSION;
use kv9_common::StoreIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmedRawTtlFormat {
    identity: StoreIdentity,
}

impl ConfirmedRawTtlFormat {
    /// The local equivalent of the authenticated RPC, minted only from a
    /// successfully published, active StoreGuard format fence.
    pub fn from_local(fence: kv9_common::store_lifecycle::RawTtlFormatFence) -> Self {
        Self {
            identity: fence.identity(),
        }
    }

    pub fn identity(self) -> StoreIdentity {
        self.identity
    }

    fn from_receipt(
        expected: StoreIdentity,
        receipt: pb::RawTtlFormatReceipt,
    ) -> kv9_common::Result<Self> {
        if receipt.format_version != RAW_TTL_FORMAT_VERSION
            || receipt.node_id != expected.node_id.0
            || receipt.root_digest.as_slice() != expected.root_digest.as_bytes()
            || receipt.cluster_id.as_slice() != expected.cluster_id.as_bytes()
            || receipt.store_incarnation.as_slice() != expected.store_incarnation.as_bytes()
        {
            return Err(Error::MetaNotReady(
                "Raw TTL format reply does not confirm the exact store and format".into(),
            ));
        }
        Ok(Self { identity: expected })
    }
}

impl RaftGrpcService {
    pub(super) fn confirm_raw_ttl_format_request(
        &self,
        request: Request<pb::RawTtlFormatRequest>,
    ) -> std::result::Result<Response<pb::RawTtlFormatReceipt>, Status> {
        request
            .extensions()
            .get::<NodeId>()
            .copied()
            .filter(|node| node.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        if value.node_id != self.me.0
            || value.root_digest.as_slice() != self.discovery.root_identity().root_digest.as_bytes()
        {
            return Err(Status::failed_precondition(
                "Raw TTL confirmation targets another node or root",
            ));
        }
        if !self.discovery.raft_receive_allowed() {
            return Err(Status::failed_precondition(
                "local replica has no Raft authority",
            ));
        }
        let fence = self.discovery.raw_ttl_format_fence().ok_or_else(|| {
            Status::failed_precondition("durable Raw TTL format fence is unavailable")
        })?;
        let identity = fence.identity();
        if identity.node_id != self.me
            || identity.root_digest != self.discovery.root_identity().root_digest
        {
            return Err(Status::internal(
                "local Raw TTL format evidence has a foreign identity",
            ));
        }
        if value.cluster_id.as_slice() != identity.cluster_id.as_bytes()
            || value.store_incarnation.as_slice() != identity.store_incarnation.as_bytes()
        {
            return Err(Status::failed_precondition(
                "Raw TTL confirmation targets another cluster or disk incarnation",
            ));
        }
        Ok(Response::new(pb::RawTtlFormatReceipt {
            root_digest: identity.root_digest.as_bytes().to_vec(),
            cluster_id: identity.cluster_id.as_bytes().to_vec(),
            node_id: identity.node_id.0,
            store_incarnation: identity.store_incarnation.as_bytes().to_vec(),
            format_version: RAW_TTL_FORMAT_VERSION,
        }))
    }
}

/// One bounded read-only confirmation on the V11 service. Endpoint replacement,
/// unsupported protocol, timeout and malformed responses all remain unconfirmed.
pub async fn grpc_confirm_raw_ttl_format(
    address: SocketAddr,
    expected: StoreIdentity,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<ConfirmedRawTtlFormat> {
    if budget.is_zero()
        || caller.0 == 0
        || expected.node_id.0 == 0
        || expected.root_digest.as_bytes() == &[0; 32]
        || expected.cluster_id.as_bytes() == &[0; 16]
        || expected.store_incarnation.as_bytes() == &[0; 16]
        || address.port() == 0
        || address.ip().is_unspecified()
    {
        return Err(Error::Config(
            "invalid Raw TTL format confirmation request".into(),
        ));
    }
    let unconfirmed =
        |error| Error::MetaNotReady(format!("Raw TTL format remains unconfirmed: {error}"));
    let receipt = tokio::time::timeout(budget, async {
        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
            .map_err(|e| unconfirmed(e.to_string()))?
            .connect_timeout(budget)
            .timeout(budget);
        let channel = endpoint
            .connect()
            .await
            .map_err(|e| unconfirmed(e.to_string()))?;
        confirm_on_channel(channel, expected, caller, token, budget).await
    })
    .await
    .map_err(|e| unconfirmed(e.to_string()))??;
    Ok(receipt)
}

/// Confirm the exact durable store over a supplied channel; no cached version
/// or connection observation is a format receipt.
pub async fn confirm_on_channel(
    channel: tonic::transport::Channel,
    expected: StoreIdentity,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<ConfirmedRawTtlFormat> {
    if budget.is_zero()
        || budget > Duration::from_secs(30)
        || caller.0 == 0
        || expected.node_id.0 == 0
        || expected.root_digest.as_bytes() == &[0; 32]
        || expected.cluster_id.as_bytes() == &[0; 16]
        || expected.store_incarnation.as_bytes() == &[0; 16]
    {
        return Err(Error::Config("invalid Raw TTL format query".into()));
    }
    let mut client = Kv9RaftClient::new(channel)
        .max_decoding_message_size(256)
        .max_encoding_message_size(256);
    let mut request = Request::new(pb::RawTtlFormatRequest {
        root_digest: expected.root_digest.as_bytes().to_vec(),
        cluster_id: expected.cluster_id.as_bytes().to_vec(),
        node_id: expected.node_id.0,
        store_incarnation: expected.store_incarnation.as_bytes().to_vec(),
    });
    attach_auth(&mut request, &token, caller);
    let receipt = tokio::time::timeout(budget, client.confirm_raw_ttl_format(request))
        .await
        .map_err(|_| Error::MetaNotReady("Raw TTL format confirmation timed out".into()))?
        .map_err(|_| Error::MetaNotReady("Raw TTL format remains unconfirmed".into()))?
        .into_inner();
    ConfirmedRawTtlFormat::from_receipt(expected, receipt)
}

#[cfg(test)]
mod tests;
