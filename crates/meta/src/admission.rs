//! Cluster identity + node-admission catalog operations (task #24).
//!
//! The three-gate membership contract's data half:
//!
//! - **Gate 2 (identity)**: [`initialize_cluster`] records the ClusterId the
//!   bootstrap winner minted — as ordinary committed catalog rows, so identity
//!   is crash-safe and replicated like everything else. Written once, never
//!   updated; a second initialization attempt is a typed error.
//! - **Gate 3 (admission)**: [`admit_node`] is the leader-committed approval
//!   binding `(cluster_id, node_id, address, role)` BEFORE a node may join the
//!   raft group; [`consume_admission`] flips it exactly once when the join
//!   completes. A valid cluster token is never admission by itself.
//!
//! Everything here produces or reads catalog rows inside a [`MetaTxn`];
//! replication happens by the caller committing that txn through raft (the
//! same propose → wait-applied path as `CreateKeyspace`). NOTHING here
//! mutates state outside the transaction — a "locally admitted" node the log
//! doesn't know about is exactly the fake-Registered state the #24 contract
//! forbids.
//!
//! The join-ticket seam (`nonce_sha256`) is NOT implemented in this block:
//! the column exists for the schema's final shape, no code writes or compares
//! it, and the minimal admission mode is a mandatory `--cluster-id`. When
//! implemented, the hash must be SHA-256 (not a checksum hash), the compare
//! constant-time, and neither ticket nor digest ever logged (Tess's review).

use kv9_common::{ClusterId, Error, NodeId, Result};
use kv9_engine::Engine;

use crate::codec::{decode_uint_component, memcmp_uint, ColumnValue, RowValue};
use crate::schema::{ColumnId, CLUSTER_META_DESC, NODE_ADMISSIONS_DESC};
use crate::store::MetaTxn;

/// The role an admission grants. Only learner exists in Phase-1 dynamic
/// membership: promotion to voter is a separate leader decision through raft
/// ConfChange, not an admission state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmittedRole {
    Learner = 1,
}

/// Admission lifecycle. `Pending` → `Consumed` on successful join (exactly
/// once); `Revoked` closes an admission that must no longer be usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionState {
    Pending = 1,
    Consumed = 2,
    Revoked = 3,
}

impl AdmissionState {
    fn from_code(code: u64) -> Result<AdmissionState> {
        match code {
            1 => Ok(AdmissionState::Pending),
            2 => Ok(AdmissionState::Consumed),
            3 => Ok(AdmissionState::Revoked),
            other => Err(Error::Config(format!(
                "unknown admission state code {other}"
            ))),
        }
    }
}

/// One admission record, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admission {
    pub node_id: NodeId,
    pub cluster_id: ClusterId,
    pub addr: String,
    pub role: AdmittedRole,
    pub state: AdmissionState,
    pub expires_unix: u64,
}

const CM_PK: u64 = 0; // cluster_meta singleton row id
const CM_CLUSTER_ID: ColumnId = ColumnId(2);
const CM_CREATED: ColumnId = ColumnId(3);

const NA_CLUSTER_ID: ColumnId = ColumnId(2);
const NA_ADDR: ColumnId = ColumnId(3);
const NA_ROLE: ColumnId = ColumnId(4);
const NA_STATE: ColumnId = ColumnId(5);
const NA_EXPIRES: ColumnId = ColumnId(7);

/// Record the freshly minted cluster identity (bootstrap winner, exactly
/// once). Fails if an identity already exists — a ClusterId is immutable, and
/// a second write is either a replayed init (harmless to refuse: apply is
/// idempotent via the watermark, this guard is for the PROPOSE path) or a bug.
pub fn initialize_cluster<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    cluster_id: ClusterId,
    created_unix: u64,
) -> Result<()> {
    if txn.get(&CLUSTER_META_DESC, &[memcmp_uint(CM_PK)])?.is_some() {
        return Err(Error::Config(
            "cluster identity already initialized; ClusterId is immutable".into(),
        ));
    }
    let mut row = RowValue::new();
    row.set(
        CM_CLUSTER_ID,
        ColumnValue::Bytes(cluster_id.as_bytes().to_vec()),
    );
    row.set(CM_CREATED, ColumnValue::Uint(created_unix));
    txn.insert(&CLUSTER_META_DESC, &[memcmp_uint(CM_PK)], row)
}

/// Read the cluster identity, if initialized.
pub fn cluster_id<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<Option<ClusterId>> {
    let Some(row) = txn.get(&CLUSTER_META_DESC, &[memcmp_uint(CM_PK)])? else {
        return Ok(None);
    };
    let Some(ColumnValue::Bytes(b)) = row.value.get(CM_CLUSTER_ID) else {
        return Err(Error::Config("cluster_meta row missing cluster_id".into()));
    };
    let bytes: [u8; 16] = b.as_slice().try_into().map_err(|_| {
        Error::Config(format!("cluster_id must be 16 bytes, got {}", b.len()))
    })?;
    Ok(Some(ClusterId::from_bytes(bytes)))
}

/// Leader-committed admission: approve `node_id@addr` to join THIS cluster as
/// `role`. Refuses if the cluster identity is missing (admission is a gate-3
/// artifact — it cannot precede gate 2) or if ANY admission row already exists
/// for the node (re-admission requires an explicit revoke first: silently
/// replacing a consumed record would let one approval be used twice).
pub fn admit_node<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    node_id: NodeId,
    addr: &str,
    role: AdmittedRole,
    expires_unix: u64,
) -> Result<()> {
    let Some(cid) = cluster_id(txn)? else {
        return Err(Error::Config(
            "cannot admit a node before the cluster identity is initialized".into(),
        ));
    };
    if txn
        .get(&NODE_ADMISSIONS_DESC, &[memcmp_uint(node_id.0)])?
        .is_some()
    {
        return Err(Error::Config(format!(
            "an admission record for node {} already exists (revoke it first)",
            node_id.0
        )));
    }
    let mut row = RowValue::new();
    row.set(NA_CLUSTER_ID, ColumnValue::Bytes(cid.as_bytes().to_vec()));
    row.set(NA_ADDR, ColumnValue::Text(addr.to_string()));
    row.set(NA_ROLE, ColumnValue::Uint(role as u64));
    row.set(NA_STATE, ColumnValue::Uint(AdmissionState::Pending as u64));
    row.set(NA_EXPIRES, ColumnValue::Uint(expires_unix));
    txn.insert(&NODE_ADMISSIONS_DESC, &[memcmp_uint(node_id.0)], row)
}

/// Consume a pending admission at join time — exactly once. Verifies, in
/// order: the record exists; it binds THIS cluster (a record replayed into the
/// wrong environment must admit nobody); it is still `Pending`
/// (consumed/revoked are terminal for this call); and it has not expired
/// (`now_unix` comes from the caller — catalog code takes no clocks).
pub fn consume_admission<E: Engine>(
    txn: &mut MetaTxn<'_, E>,
    node_id: NodeId,
    expected_cluster: ClusterId,
    now_unix: u64,
) -> Result<Admission> {
    let adm = admission(txn, node_id)?
        .ok_or_else(|| Error::Config(format!("no admission record for node {}", node_id.0)))?;
    if adm.cluster_id != expected_cluster {
        return Err(Error::Config(format!(
            "admission for node {} binds a different cluster",
            node_id.0
        )));
    }
    if adm.state != AdmissionState::Pending {
        return Err(Error::Config(format!(
            "admission for node {} is {:?}, not pending",
            node_id.0, adm.state
        )));
    }
    if now_unix > adm.expires_unix {
        return Err(Error::Config(format!(
            "admission for node {} expired at {}",
            node_id.0, adm.expires_unix
        )));
    }
    let changes = vec![(
        NA_STATE,
        ColumnValue::Uint(AdmissionState::Consumed as u64),
    )];
    txn.update(&NODE_ADMISSIONS_DESC, &[memcmp_uint(node_id.0)], changes)?;
    Ok(Admission {
        state: AdmissionState::Consumed,
        ..adm
    })
}

/// Read one admission record.
pub fn admission<E: Engine>(
    txn: &MetaTxn<'_, E>,
    node_id: NodeId,
) -> Result<Option<Admission>> {
    let Some(row) = txn.get(&NODE_ADMISSIONS_DESC, &[memcmp_uint(node_id.0)])? else {
        return Ok(None);
    };
    decode_admission(node_id, &row.value).map(Some)
}

/// All admissions still pending (the status surface's `pending_admissions`).
pub fn pending_admissions<E: Engine>(txn: &MetaTxn<'_, E>) -> Result<Vec<Admission>> {
    let rows = txn.scan(&NODE_ADMISSIONS_DESC, usize::MAX)?;
    let mut out = Vec::new();
    for row in rows {
        let pk = row
            .pk
            .first()
            .ok_or_else(|| Error::Config("admission row without pk".into()))?;
        let node_id = NodeId(decode_uint_component(pk)?);
        let adm = decode_admission(node_id, &row.value)?;
        if adm.state == AdmissionState::Pending {
            out.push(adm);
        }
    }
    Ok(out)
}

fn decode_admission(node_id: NodeId, row: &RowValue) -> Result<Admission> {
    let cluster_id = match row.get(NA_CLUSTER_ID) {
        Some(ColumnValue::Bytes(b)) => {
            let bytes: [u8; 16] = b.as_slice().try_into().map_err(|_| {
                Error::Config(format!(
                    "admission cluster_id must be 16 bytes, got {}",
                    b.len()
                ))
            })?;
            ClusterId::from_bytes(bytes)
        }
        _ => return Err(Error::Config("admission row missing cluster_id".into())),
    };
    let addr = match row.get(NA_ADDR) {
        Some(ColumnValue::Text(t)) => t.clone(),
        _ => return Err(Error::Config("admission row missing addr".into())),
    };
    let role = match row.get(NA_ROLE) {
        Some(ColumnValue::Uint(1)) => AdmittedRole::Learner,
        Some(ColumnValue::Uint(other)) => {
            return Err(Error::Config(format!("unknown admission role code {other}")))
        }
        _ => return Err(Error::Config("admission row missing role".into())),
    };
    let state = match row.get(NA_STATE) {
        Some(ColumnValue::Uint(code)) => AdmissionState::from_code(*code)?,
        _ => return Err(Error::Config("admission row missing state".into())),
    };
    let expires_unix = match row.get(NA_EXPIRES) {
        Some(ColumnValue::Uint(v)) => *v,
        _ => return Err(Error::Config("admission row missing expiry".into())),
    };
    Ok(Admission {
        node_id,
        cluster_id,
        addr,
        role,
        state,
        expires_unix,
    })
}
