//! Exact-store, challenge-bound observation of an existing durable capture.
//! This confirms local retry data, not remote object presence or GC authority.
use super::*;
use kv9_common::{RegionId, StoreIdentity};
use prost::Message as _;
use std::time::Instant;

const MAX_BYTES: usize = 512;
const MAX_MICROS: u64 = 30_000_000;

#[derive(Debug)]
pub struct CaptureTicket {
    source: StoreIdentity,
    region: RegionId,
    operation: [u8; 16],
    digest: RootDigest,
    challenge: [u8; 16],
}
#[derive(Debug)]
/// Only a correlated response from the actual source RPC creates this value.
/// ```compile_fail
/// fn forge(d: kv9_raft::snapshot_install::capture::CaptureDescriptor)
///     -> kv9_raft::grpc::migration_capture::ConfirmedCapture { d.into() }
/// ```
pub struct ConfirmedCapture {
    source: StoreIdentity,
    region: RegionId,
    operation: [u8; 16],
    digest: RootDigest,
}

impl ConfirmedCapture {
    pub fn matches(
        &self,
        descriptor: &crate::snapshot_install::capture::CaptureDescriptor,
    ) -> bool {
        let request = descriptor.request();
        self.source == request.source
            && self.region == request.range.region
            && self.operation == request.operation
            && self.digest == descriptor.digest()
    }
}
impl CaptureTicket {
    pub fn new(
        descriptor: &crate::snapshot_install::capture::CaptureDescriptor,
    ) -> kv9_common::Result<Self> {
        Ok(Self {
            source: descriptor.request().source,
            region: descriptor.request().range.region,
            operation: descriptor.request().operation,
            digest: descriptor.digest(),
            challenge: *kv9_common::ProcessIncarnation::mint()?.as_bytes(),
        })
    }
    fn request(&self, budget: Duration) -> pb::MigrationCaptureRequest {
        pb::MigrationCaptureRequest {
            store_identity: self.source.canonical_bytes(),
            region_id: self.region.0,
            operation_id: self.operation.to_vec(),
            challenge: self.challenge.to_vec(),
            budget_micros: budget.as_micros() as u64,
        }
    }
    fn confirm(&self, reply: pb::MigrationCaptureResponse) -> kv9_common::Result<ConfirmedCapture> {
        if reply.encoded_len() > MAX_BYTES
            || reply.store_identity != self.source.canonical_bytes()
            || reply.region_id != self.region.0
            || reply.operation_id != self.operation
            || reply.challenge != self.challenge
            || reply.descriptor_digest.as_deref() != Some(self.digest.as_bytes().as_slice())
        {
            return Err(invalid());
        }
        Ok(ConfirmedCapture {
            source: self.source,
            region: self.region,
            operation: self.operation,
            digest: self.digest,
        })
    }
}
fn invalid() -> Error {
    Error::MetaNotReady("durable migration capture unconfirmed".into())
}

impl RaftGrpcService {
    pub(super) async fn inspect_migration_capture_request(
        &self,
        request: Request<pb::MigrationCaptureRequest>,
    ) -> std::result::Result<Response<pb::MigrationCaptureResponse>, Status> {
        let started = Instant::now();
        let caller = *request
            .extensions()
            .get::<NodeId>()
            .filter(|n| n.0 != 0)
            .ok_or_else(|| Status::unauthenticated("authenticated node identity missing"))?;
        let value = request.into_inner();
        if value.encoded_len() > MAX_BYTES
            || value.store_identity.len() != 80
            || value.operation_id.len() != 16
            || value.operation_id == [0; 16]
            || value.challenge.len() != 16
            || value.challenge == [0; 16]
            || value.region_id < 100
            || !(1..=MAX_MICROS).contains(&value.budget_micros)
        {
            return Err(Status::invalid_argument(
                "invalid capture observation bounds",
            ));
        }
        let identity = StoreIdentity::decode(&value.store_identity)
            .map_err(|_| Status::invalid_argument("invalid capture source identity"))?;
        let confirm = || {
            let mut request = Request::new(pb::RawTtlFormatRequest {
                root_digest: identity.root_digest.as_bytes().to_vec(),
                cluster_id: identity.cluster_id.as_bytes().to_vec(),
                node_id: identity.node_id.0,
                store_incarnation: identity.store_incarnation.as_bytes().to_vec(),
            });
            request.extensions_mut().insert(caller);
            self.confirm_raw_ttl_format_request(request)
        };
        confirm()?;
        let backend = self
            .registration
            .clone()
            .ok_or_else(|| Status::unimplemented("capture observation backend unavailable"))?;
        let permit = self
            .checkpoint_history_capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("capture observation reader busy"))?;
        let deadline = started + Duration::from_micros(value.budget_micros);
        let region = RegionId(value.region_id);
        let operation = value.operation_id.as_slice().try_into().unwrap();
        let job = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if Instant::now() >= deadline {
                return Err(invalid());
            }
            let digest =
                backend.migration_capture(identity.root_digest, region, operation, deadline)?;
            if Instant::now() >= deadline {
                return Err(invalid());
            }
            Ok(digest)
        });
        let digest = tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), job)
            .await
            .map_err(|_| Status::deadline_exceeded("capture observation deadline"))?
            .map_err(|_| Status::unavailable("capture observation worker stopped"))?
            .map_err(|_| Status::unavailable("capture observation unavailable"))?;
        confirm()?;
        Ok(Response::new(pb::MigrationCaptureResponse {
            store_identity: value.store_identity,
            region_id: value.region_id,
            operation_id: value.operation_id,
            challenge: value.challenge,
            descriptor_digest: digest.map(|d| d.to_vec()),
        }))
    }
}

pub async fn inspect_on_channel(
    channel: tonic::transport::Channel,
    ticket: &CaptureTicket,
    caller: NodeId,
    token: Option<String>,
    budget: Duration,
) -> kv9_common::Result<ConfirmedCapture> {
    if caller.0 == 0 || budget.as_micros() == 0 || budget.as_micros() > u128::from(MAX_MICROS) {
        return Err(invalid());
    }
    let mut request = Request::new(ticket.request(budget));
    request.set_timeout(budget);
    attach_auth(&mut request, &token, caller);
    let mut client = Kv9RaftClient::new(channel)
        .max_encoding_message_size(MAX_BYTES)
        .max_decoding_message_size(MAX_BYTES);
    let reply = tokio::time::timeout(budget, client.inspect_migration_capture(request))
        .await
        .map_err(|_| invalid())?
        .map_err(|_| invalid())?
        .into_inner();
    ticket.confirm(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ticket() -> CaptureTicket {
        CaptureTicket {
            source: StoreIdentity {
                root_digest: RootDigest::sha256(b"capture-root"),
                cluster_id: kv9_common::ClusterId::from_bytes([1; 16]),
                node_id: NodeId(1),
                store_incarnation: kv9_common::StoreIncarnation::from_bytes([2; 16]),
            },
            region: RegionId(100),
            operation: [3; 16],
            challenge: [4; 16],
            digest: RootDigest::sha256(b"exact record"),
        }
    }
    fn reply(t: &CaptureTicket) -> pb::MigrationCaptureResponse {
        pb::MigrationCaptureResponse {
            store_identity: t.source.canonical_bytes(),
            region_id: t.region.0,
            operation_id: t.operation.to_vec(),
            challenge: t.challenge.to_vec(),
            descriptor_digest: Some(t.digest.as_bytes().to_vec()),
        }
    }
    #[test]
    fn only_exact_identity_operation_challenge_and_record_digest_confirm() {
        let t = ticket();
        t.confirm(reply(&t)).unwrap();
        for field in 0..6 {
            let mut value = reply(&t);
            match field {
                0 => value.store_identity[32] ^= 1,
                1 => value.region_id += 1,
                2 => value.operation_id[0] ^= 1,
                3 => value.challenge[0] ^= 1,
                4 => value.descriptor_digest.as_mut().unwrap()[0] ^= 1,
                _ => value
                    .descriptor_digest
                    .as_mut()
                    .unwrap()
                    .pop()
                    .map(|_| ())
                    .unwrap(),
            }
            assert!(t.confirm(value).is_err(), "field {field}");
        }
    }
    #[test]
    fn absent_or_oversized_observation_never_confirms() {
        let t = ticket();
        let mut value = reply(&t);
        value.descriptor_digest = None;
        assert!(t.confirm(value).is_err());
        let mut value = reply(&t);
        value.store_identity.resize(MAX_BYTES + 1, 0);
        assert!(t.confirm(value).is_err());
    }
}
