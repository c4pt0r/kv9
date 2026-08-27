//! Cluster membership (DESIGN §5.1). Stored as ordinary KV in the system keyspace.

use std::collections::HashMap;

use kv9_common::NodeId;

/// A node's lifecycle state (DESIGN §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    Up,
    Leaving,
    Down,
}

/// One membership record: node id → address, state, heartbeat, capacity (DESIGN §5.1).
#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: NodeId,
    pub address: String,
    pub state: NodeState,
    /// Last heartbeat time (nanos since epoch).
    pub last_heartbeat_nanos: u64,
    /// Advertised capacity units (input to consumption-aware placement, DESIGN §10).
    pub capacity_units: u64,
}

/// The membership table (DESIGN §5.1).
#[derive(Debug, Default)]
pub struct Membership {
    nodes: HashMap<NodeId, NodeInfo>,
}

impl Membership {
    pub fn new() -> Self {
        Membership::default()
    }

    /// Register / update a node (a write to the system keyspace — DESIGN §5.2).
    pub fn upsert(&mut self, info: NodeInfo) {
        self.nodes.insert(info.id, info);
    }

    pub fn get(&self, id: NodeId) -> Option<&NodeInfo> {
        self.nodes.get(&id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &NodeInfo> {
        self.nodes.values()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}
