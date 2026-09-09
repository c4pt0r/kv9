//! Versioned endpoint changes for an existing registered store.
//!
//! These are catalog planners, not an independent commit or authorization path.
//! The caller must authenticate, serialize catalog planning, drain ambiguous
//! proposals, and commit the resulting batch under the planning term. A returned
//! `Changed` value describes the staged batch until its exact Raft position applies.
//! Endpoint versions do not grant membership or authorize a replacement disk.

use std::net::SocketAddr;

use kv9_common::{ClusterId, Error, NodeId, Result, StoreIncarnation};
use kv9_engine::Engine;

use crate::codec::{memcmp_uint, ColumnValue};
use crate::schema::{ColumnId, NODES_DESC};
use crate::store::MetaTxn;

const ADDRESS: ColumnId = ColumnId(2);
const STATE: ColumnId = ColumnId(3);
const INCARNATION: ColumnId = ColumnId(5);
pub const ENDPOINT_GENERATION: ColumnId = ColumnId(6);
pub const ENDPOINT_PREVIOUS_ADDRESS: ColumnId = ColumnId(7);
const ACTIVE: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeEndpoint {
    pub node: NodeId,
    pub incarnation: StoreIncarnation,
    pub address: SocketAddr,
    pub generation: u64,
    /// The last transition's precondition; absent only at generation zero.
    pub previous_address: Option<SocketAddr>,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointChange {
    pub cluster: ClusterId,
    pub node: NodeId,
    pub incarnation: StoreIncarnation,
    pub expected_address: SocketAddr,
    pub expected_generation: u64,
    pub new_address: SocketAddr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointRefusal {
    WrongCluster,
    MissingNode,
    InactiveNode,
    InvalidIncarnation,
    Conflict,
    GenerationExhausted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointChangeOutcome {
    /// The batch stages one generation advance, including a same-address update.
    Changed(NodeEndpoint),
    /// The requested one-step result is already current. No batch was staged.
    /// This does not identify which caller committed that transition or recover
    /// its original Raft position. A response needs a fresh exact confirmation.
    Confirmed(NodeEndpoint),
    /// No endpoint mutation was staged by this call.
    Refused(EndpointRefusal),
}

/// Read from the transaction's one snapshot. Callers needing a current read must
/// establish their consensus read barrier before creating that snapshot.
pub fn node_endpoint<E: Engine>(
    txn: &MetaTxn<'_, E>,
    node: NodeId,
) -> Result<Option<NodeEndpoint>> {
    if node.0 == 0 {
        return Err(Error::Config("endpoint node id must be non-zero".into()));
    }
    let Some(row) = txn.get(&NODES_DESC, &[memcmp_uint(node.0)])? else {
        return Ok(None);
    };
    let address = match row.value.get(ADDRESS) {
        Some(ColumnValue::Text(address)) => address
            .parse()
            .map_err(|_| Error::Config("registered endpoint is not a socket address".into()))?,
        _ => {
            return Err(Error::Config(
                "registered endpoint address is missing".into(),
            ))
        }
    };
    let incarnation = match row.value.get(INCARNATION) {
        Some(ColumnValue::Bytes(bytes)) => {
            StoreIncarnation::from_bytes(bytes.as_slice().try_into().map_err(|_| {
                Error::Config("registered endpoint incarnation must have 16 bytes".into())
            })?)
        }
        _ => {
            return Err(Error::Config(
                "registered endpoint incarnation is missing".into(),
            ))
        }
    };
    // Existing v1 rows have no tag 6. Absence means the original generation;
    // a malformed present tag must never silently reset a version to zero.
    let generation = match row.value.get(ENDPOINT_GENERATION) {
        None => 0,
        Some(ColumnValue::Uint(generation)) => *generation,
        _ => {
            return Err(Error::Config(
                "registered endpoint generation is invalid".into(),
            ))
        }
    };
    let previous_address = match row.value.get(ENDPOINT_PREVIOUS_ADDRESS) {
        None if generation == 0 => None,
        Some(ColumnValue::Text(address)) if generation > 0 => Some(
            address
                .parse()
                .map_err(|_| Error::Config("previous endpoint is not a socket address".into()))?,
        ),
        _ => {
            return Err(Error::Config(
                "endpoint generation and previous address disagree".into(),
            ))
        }
    };
    let active = match row.value.get(STATE) {
        Some(ColumnValue::Uint(state)) => *state == ACTIVE,
        _ => return Err(Error::Config("registered endpoint state is missing".into())),
    };
    Ok(Some(NodeEndpoint {
        node,
        incarnation,
        address,
        generation,
        previous_address,
        active,
    }))
}

/// Stage an atomic address/version CAS. All refusals and confirmations leave
/// the transaction unchanged. Check the immutable binding before considering
/// either the initial change or a retry of its one-step result.
pub fn change_endpoint<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    request: EndpointChange,
) -> Result<EndpointChangeOutcome> {
    use EndpointChangeOutcome::{Changed, Confirmed, Refused};
    use EndpointRefusal::*;
    if crate::admission::cluster_id(txn)? != Some(request.cluster) {
        return Ok(Refused(WrongCluster));
    }
    let Some(current) = node_endpoint(txn, request.node)? else {
        return Ok(Refused(MissingNode));
    };
    if current.incarnation != request.incarnation {
        return Ok(Refused(InvalidIncarnation));
    }
    if !current.active {
        return Ok(Refused(InactiveNode));
    }
    let Some(next_generation) = request.expected_generation.checked_add(1) else {
        return Ok(Refused(GenerationExhausted));
    };
    if current.generation == request.expected_generation
        && current.address == request.expected_address
    {
        // A prior Pending/Consumed registration must not be able to reinstall
        // its old address after this operator transition. Read before staging
        // either row so corrupt admission state leaves the transaction intact.
        let revoke = crate::admission::admission(txn, request.node)?.is_some_and(|admission| {
            matches!(
                admission.state,
                crate::admission::AdmissionState::Pending
                    | crate::admission::AdmissionState::Consumed
            )
        });
        txn.update(
            &NODES_DESC,
            &[memcmp_uint(request.node.0)],
            vec![
                (ADDRESS, ColumnValue::Text(request.new_address.to_string())),
                (ENDPOINT_GENERATION, ColumnValue::Uint(next_generation)),
                (
                    ENDPOINT_PREVIOUS_ADDRESS,
                    ColumnValue::Text(current.address.to_string()),
                ),
            ],
        )?;
        if revoke {
            crate::admission::supersede_admission(txn, request.node)?;
        }
        return Ok(Changed(NodeEndpoint {
            address: request.new_address,
            generation: next_generation,
            previous_address: Some(current.address),
            ..current
        }));
    }
    if current.generation == next_generation
        && current.address == request.new_address
        && current.previous_address == Some(request.expected_address)
    {
        return Ok(Confirmed(current));
    }
    Ok(Refused(Conflict))
}

/// Update the endpoint of an already bound store while consuming a new
/// registration admission. The caller validates and consumes that admission
/// in this same transaction and serializes the entire plan through consensus.
/// Initial insertion remains generation zero. Address changes advance the
/// same version used by operator CAS; unchanged addresses stage nothing here.
pub fn refresh_registration_endpoint<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    node: NodeId,
    incarnation: StoreIncarnation,
    address: SocketAddr,
) -> Result<NodeEndpoint> {
    let current = node_endpoint(txn, node)?
        .ok_or_else(|| Error::Config("registered endpoint is missing".into()))?;
    if current.incarnation != incarnation {
        return Err(Error::Config(
            "registration cannot replace an endpoint's store incarnation".into(),
        ));
    }
    if current.address == address {
        return Ok(current);
    }
    let generation = current
        .generation
        .checked_add(1)
        .ok_or_else(|| Error::Config("endpoint generation is exhausted".into()))?;
    txn.update(
        &NODES_DESC,
        &[memcmp_uint(node.0)],
        vec![
            (ADDRESS, ColumnValue::Text(address.to_string())),
            (ENDPOINT_GENERATION, ColumnValue::Uint(generation)),
            (
                ENDPOINT_PREVIOUS_ADDRESS,
                ColumnValue::Text(current.address.to_string()),
            ),
        ],
    )?;
    Ok(NodeEndpoint {
        address,
        generation,
        previous_address: Some(current.address),
        ..current
    })
}
