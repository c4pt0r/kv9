//! One immutable, independently observed capture descriptor per migration.
//! Direct lookup avoids rescanning large historical descriptors. Replication
//! fixes protocol/image identity; object presence and all retention lifetimes
//! must still be checked by users of the descriptor.
use super::*;
use crate::endpoint::{node_endpoint, NodeEndpoint};
use kv9_common::RootDescriptor;
use kv9_engine::{ColumnFamily, ReadView, WriteBatch};
use kv9_raft::grpc::migration_capture::ConfirmedCapture;
use kv9_raft::snapshot_install::capture::{CaptureDescriptor, MAX_CAPTURE_DESCRIPTOR_BYTES};

const PREFIX: &[u8] = b"\0kv9\0migration-capture-v1\0";
const MAGIC: &[u8; 8] = b"KV9MCP01";
fn key(operation: [u8; 16]) -> Vec<u8> {
    let mut key = PREFIX.to_vec();
    key.extend(operation);
    key
}

#[derive(Debug, Clone, PartialEq)]
pub struct CommittedCapture {
    descriptor: CaptureDescriptor,
    endpoint_generation: u64,
}
impl CommittedCapture {
    pub fn descriptor(&self) -> &CaptureDescriptor {
        &self.descriptor
    }
    fn encode(&self) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(self.endpoint_generation.to_be_bytes());
        bytes.extend(self.descriptor.encode());
        bytes.extend(RootDigest::sha256(&bytes).as_bytes());
        bytes
    }
}
pub fn committed_capture(
    view: &dyn ReadView,
    root: &RootDescriptor,
    operation: [u8; 16],
) -> Result<Option<CommittedCapture>> {
    let Some(bytes) = view.get(ColumnFamily::Default, &key(operation))? else {
        return Ok(None);
    };
    if bytes.len() <= 48
        || bytes.len() > MAX_CAPTURE_DESCRIPTOR_BYTES + 48
        || &bytes[..8] != MAGIC
        || RootDigest::sha256(&bytes[..bytes.len() - 32])
            .as_bytes()
            .as_slice()
            != &bytes[bytes.len() - 32..]
    {
        return Err(invalid("invalid committed capture record"));
    }
    let record = CommittedCapture {
        descriptor: CaptureDescriptor::decode(&bytes[16..bytes.len() - 32])?,
        endpoint_generation: u64::from_be_bytes(bytes[8..16].try_into().unwrap()),
    };
    let request = record.descriptor.request();
    if request.operation != operation
        || request.range.root != root.digest()
        || request.source.cluster_id != root.cluster_id
        || record.encode() != bytes
    {
        return Err(invalid("committed capture binding differs"));
    }
    Ok(Some(record))
}

/// The runtime holds the catalog lock, drains prior proposals and commits this
/// whole batch in the same term. A decoded descriptor alone cannot authorize a
/// new record. Exact committed retries need no live source after its failure.
pub fn plan_record<E: Engine>(
    txn: MetaTxn<'_, E>,
    descriptor: &CaptureDescriptor,
    observed: Option<(&ConfirmedCapture, NodeEndpoint)>,
) -> Result<(WriteBatch, bool)> {
    let root = crate::root::certified_root(&txn)?
        .ok_or_else(|| invalid("capture requires certified root"))?;
    let request = descriptor.request();
    let migration = migration::committed_migrations_in_txn(&txn, root.digest())?
        .into_iter()
        .find(|m| m.operation() == request.operation)
        .ok_or_else(|| invalid("capture migration missing"))?;
    let range = ranges::read_in(&txn)?
        .into_iter()
        .find(|r| r.range().region == migration.region())
        .ok_or_else(|| invalid("capture range missing"))?;
    if range.creation().intent().task() != migration.creation_task()
        || &request.range != range.range()
        || request.range.root != root.digest()
        || request.source.cluster_id != root.cluster_id
        || request.destination.node_id != migration.destination().node
        || request.destination.store_incarnation != migration.destination().incarnation
    {
        return Err(invalid("capture differs from committed migration/range"));
    }
    for row in task_rows(&txn) {
        if abort::from_abort_row(&row?, root.digest())?
            .is_some_and(|a| a.operation() == request.operation)
        {
            return Err(invalid("aborted migration cannot record a capture"));
        }
    }
    let route = node_endpoint(&txn, request.source.node_id)?;
    let view = txn.into_view();
    if let Some(old) = committed_capture(view.as_ref(), &root, request.operation)? {
        if old.descriptor.encode() != descriptor.encode() {
            return Err(invalid("migration already binds a different capture"));
        }
        return Ok((WriteBatch::new(), false));
    }
    let (proof, expected) =
        observed.ok_or_else(|| invalid("capture requires independent source observation"))?;
    if !proof.matches(descriptor)
        || route != Some(expected)
        || !expected.active
        || expected.node != request.source.node_id
        || expected.incarnation != request.source.store_incarnation
        || root
            .voter(expected.node)
            .is_some_and(|v| v.store_incarnation != expected.incarnation)
    {
        return Err(invalid("capture source observation or endpoint changed"));
    }
    let record = CommittedCapture {
        descriptor: descriptor.clone(),
        endpoint_generation: expected.generation,
    };
    let mut batch = WriteBatch::new();
    batch.put(
        ColumnFamily::Default,
        key(request.operation),
        record.encode(),
    );
    Ok((batch, true))
}
