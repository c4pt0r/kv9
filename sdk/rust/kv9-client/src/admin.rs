//! The administrative surface of the SDK (docs/KV9-ADMIN.md): the node directory,
//! a node's status document, and the metadata-group membership verbs. These are the
//! RPCs `kv9-admin` is built on; applications normally never need them.

use crate::{proto, Applied, Client, Error};

/// One node's status document: the lines of its status file, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeStatus {
    pub fields: Vec<(String, String)>,
    /// The node's local, unbarriered view of the directory: `(node_id, address)`.
    pub peers: Vec<(u64, String)>,
}

impl NodeStatus {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn u64(&self, name: &str) -> Option<u64> {
        self.get(name).and_then(|v| v.parse().ok())
    }

    /// A comma-separated id list field (`meta_voters`, `meta_learners`, ...).
    pub fn ids(&self, name: &str) -> Vec<u64> {
        self.get(name)
            .unwrap_or_default()
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect()
    }
}

/// One row of the node directory as the metadata leader sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeRow {
    pub node_id: u64,
    pub address: String,
    pub store_incarnation: Vec<u8>,
    pub endpoint_generation: u64,
    pub active: bool,
    pub last_heartbeat_unix: u64,
    /// `voter`, `learner` or `none`.
    pub raft_role: String,
    pub matched_index: u64,
    pub recent_active: bool,
    /// `pending`, `consumed`, `revoked`, `superseded` or empty.
    pub admission_state: String,
    pub admission_expires_unix: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeDirectory {
    pub leader_node_id: u64,
    pub raft_committed: u64,
    pub nodes: Vec<NodeRow>,
}

/// The receipt of a membership verb: the applied position and the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipChange {
    pub applied: Applied,
    pub voters: Vec<u64>,
    pub learners: Vec<u64>,
    /// Present only on `admit_node`: the one-time join ticket, returned once.
    pub join_ticket: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeEndpoint {
    pub node_id: u64,
    pub store_incarnation: Vec<u8>,
    pub address: String,
    pub generation: u64,
    pub previous_address: Option<String>,
    pub active: bool,
}

fn membership(r: proto::MembershipChangeResponse) -> MembershipChange {
    MembershipChange {
        applied: Applied {
            term: r.applied_term,
            index: r.applied_index,
        },
        voters: r.voters,
        learners: r.learners,
        join_ticket: Some(r.join_ticket).filter(|t| !t.is_empty()),
    }
}

impl Client {
    /// The same client addressed at another node: same token, peers, timeout and
    /// redirect policy. `NodeStatus` is answered by the node asked, so a fan-out
    /// over the directory builds one of these per address.
    pub fn at(&self, address: impl Into<String>) -> Client {
        let address = address.into();
        self.clone().rebuild(|inner| {
            *inner.address.get_mut().expect("address") = address;
        })
    }

    /// This node's status document. Served by every node about itself; never
    /// redirected.
    pub async fn node_status(&self) -> Result<NodeStatus, Error> {
        let r = self
            .invoke(true, proto::NodeStatusRequest {}, |mut c, r| async move {
                c.node_status(r).await
            })
            .await?;
        Ok(NodeStatus {
            fields: r.fields.into_iter().map(|f| (f.name, f.value)).collect(),
            peers: r.peers.into_iter().map(|p| (p.node_id, p.address)).collect(),
        })
    }

    /// The node directory, linearizably, from the metadata leader (follows a
    /// `NotLeader` hint through the peers map).
    pub async fn list_nodes(&self) -> Result<NodeDirectory, Error> {
        let r = self
            .invoke(true, proto::ListNodesRequest {}, |mut c, r| async move {
                c.list_nodes(r).await
            })
            .await?;
        Ok(NodeDirectory {
            leader_node_id: r.leader_node_id,
            raft_committed: r.raft_committed,
            nodes: r
                .nodes
                .into_iter()
                .map(|n| NodeRow {
                    node_id: n.node_id,
                    address: n.address,
                    store_incarnation: n.store_incarnation,
                    endpoint_generation: n.endpoint_generation,
                    active: n.active,
                    last_heartbeat_unix: n.last_heartbeat_unix,
                    raft_role: n.raft_role,
                    matched_index: n.matched_index,
                    recent_active: n.recent_active,
                    admission_state: n.admission_state,
                    admission_expires_unix: n.admission_expires_unix,
                })
                .collect(),
        })
    }

    pub async fn node_endpoint(&self, node_id: u64) -> Result<Option<NodeEndpoint>, Error> {
        let r = self
            .invoke(
                true,
                proto::GetNodeEndpointRequest { node_id },
                |mut c, r| async move { c.get_node_endpoint(r).await },
            )
            .await?;
        Ok(r.endpoint.map(|e| NodeEndpoint {
            node_id: e.node_id,
            store_incarnation: e.store_incarnation,
            address: e.address,
            generation: e.generation,
            previous_address: e.previous_address,
            active: e.active,
        }))
    }

    /// Admit `node_id` at `addr` as a learner of the metadata group. The returned
    /// join ticket is shown exactly once; the catalog keeps only its digest.
    pub async fn admit_node(
        &self,
        node_id: u64,
        addr: &str,
        ttl_seconds: u64,
    ) -> Result<MembershipChange, Error> {
        let r = self
            .invoke(
                false,
                proto::AdmitNodeRequest {
                    node_id,
                    addr: addr.to_owned(),
                    ttl_seconds,
                },
                |mut c, r| async move { c.admit_node(r).await },
            )
            .await?;
        Ok(membership(r))
    }

    pub async fn revoke_admission(&self, node_id: u64) -> Result<MembershipChange, Error> {
        let r = self
            .invoke(
                false,
                proto::RevokeAdmissionRequest { node_id },
                |mut c, r| async move { c.revoke_admission(r).await },
            )
            .await?;
        Ok(membership(r))
    }

    pub async fn promote_node(&self, node_id: u64) -> Result<MembershipChange, Error> {
        let r = self
            .invoke(
                false,
                proto::PromoteNodeRequest { node_id },
                |mut c, r| async move { c.promote_node(r).await },
            )
            .await?;
        Ok(membership(r))
    }

    /// Remove a voter. The server refuses below three voters and never removes the
    /// leader itself; promote a replacement first.
    pub async fn remove_node(&self, node_id: u64) -> Result<MembershipChange, Error> {
        let r = self
            .invoke(
                false,
                proto::RemoveNodeRequest { node_id },
                |mut c, r| async move { c.remove_node(r).await },
            )
            .await?;
        Ok(membership(r))
    }
}
