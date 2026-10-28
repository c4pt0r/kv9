//! Identity-bound remote pressure observations. These are advisory capacity
//! samples, never grants or Raft acknowledgements. Missing reports preserve
//! quorum availability; errors never clear previously observed pressure.
use kv9_common::{Error, NodeId, Result, StoreIdentity};
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const MAX_PEERS: usize = 1024;
const MAX_REPLY_AGE: Duration = Duration::from_secs(2);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PressureRoute {
    pub identity: StoreIdentity,
    pub address: SocketAddr,
    pub generation: u64,
}
#[derive(Debug)]
struct Slot {
    route: PressureRoute,
    enabled: bool,
    epoch: Arc<()>,
    sequence: u64,
    capacity: Option<Option<u64>>, // unobserved / uncertain / measured
    observed: Option<Instant>,
    errors: u64,
}
#[derive(Debug)]
pub struct ReplicaPressure {
    local: StoreIdentity,
    slots: Mutex<BTreeMap<NodeId, Slot>>,
}
/// One current-route request. The observer owns its ordering and challenge;
/// wire replies cannot select either or revive a replaced route.
#[derive(Debug)]
pub struct PressureTicket {
    pub(crate) route: PressureRoute,
    pub(crate) challenge: [u8; 16],
    epoch: Arc<()>,
    sequence: u64,
    issued: Instant,
}
impl PressureTicket {
    pub fn route(&self) -> PressureRoute {
        self.route
    }
}
#[derive(Debug, Clone)]
pub struct PressureStatus {
    pub node: u64,
    pub generation: u64,
    pub incarnation: String,
    pub address: String,
    pub polling: bool,
    pub observed: bool,
    pub uncertain: bool,
    pub available_bytes: Option<u64>,
    pub age_millis: Option<u64>,
    pub errors: u64,
}
impl ReplicaPressure {
    pub fn new(local: StoreIdentity) -> Self {
        Self {
            local,
            slots: Mutex::default(),
        }
    }
    /// A serialized applied-directory snapshot updates routes, not membership.
    /// Disappeared routes stop polling but retain their last pressure; only a
    /// peer's real ConfState decides whether that node still restricts writes.
    pub fn set_routes(&self, routes: Vec<PressureRoute>) -> Result<()> {
        if routes.len() > MAX_PEERS {
            return Err(invalid());
        }
        let mut unique = BTreeSet::new();
        for route in &routes {
            let identity = route.identity;
            if identity.cluster_id != self.local.cluster_id
                || identity.root_digest != self.local.root_digest
                || identity.node_id.0 == 0
                || identity.node_id == self.local.node_id
                || identity.store_incarnation.as_bytes() == &[0; 16]
                || route.address.port() == 0
                || route.address.ip().is_unspecified()
                || !unique.insert(identity.node_id)
            {
                return Err(invalid());
            }
        }
        let mut slots = self.slots.lock().expect("replica pressure");
        if slots.len()
            + routes
                .iter()
                .filter(|r| !slots.contains_key(&r.identity.node_id))
                .count()
            > MAX_PEERS
        {
            return Err(invalid());
        }
        // Validate the full update before mutation. Same-store route history
        // cannot roll backwards or assign two addresses to one generation.
        for route in &routes {
            if let Some(old) = slots.get(&route.identity.node_id) {
                if old.route.identity == route.identity
                    && (route.generation < old.route.generation
                        || (route.generation == old.route.generation
                            && route.address != old.route.address))
                {
                    return Err(invalid());
                }
            }
        }
        for (node, slot) in slots.iter_mut() {
            if !unique.contains(node) && slot.enabled {
                slot.enabled = false;
                slot.epoch = Arc::new(());
            }
        }
        for route in routes {
            let slot = slots.entry(route.identity.node_id).or_insert_with(|| Slot {
                route,
                enabled: true,
                epoch: Arc::new(()),
                sequence: 0,
                capacity: None,
                observed: None,
                errors: 0,
            });
            if slot.route != route || !slot.enabled {
                // A different disk cannot inherit either the old disk's spare
                // capacity or its pressure. Same-disk endpoint moves keep it.
                if slot.route.identity != route.identity {
                    slot.capacity = None;
                    slot.observed = None;
                }
                slot.epoch = Arc::new(());
                slot.route = route;
                slot.enabled = true;
            }
        }
        Ok(())
    }
    pub fn polling_nodes(&self) -> Vec<NodeId> {
        self.slots
            .lock()
            .expect("replica pressure")
            .iter()
            .filter_map(|(node, slot)| slot.enabled.then_some(*node))
            .collect()
    }
    pub fn begin(&self, node: NodeId) -> Result<Option<PressureTicket>> {
        let challenge = *kv9_common::ProcessIncarnation::mint()?.as_bytes();
        let mut slots = self.slots.lock().expect("replica pressure");
        let Some(slot) = slots.get_mut(&node).filter(|slot| slot.enabled) else {
            return Ok(None);
        };
        slot.sequence = slot.sequence.checked_add(1).ok_or_else(invalid)?;
        Ok(Some(PressureTicket {
            route: slot.route,
            challenge,
            epoch: slot.epoch.clone(),
            sequence: slot.sequence,
            issued: Instant::now(),
        }))
    }
    pub fn finish(
        &self,
        ticket: PressureTicket,
        reply: Result<crate::grpc::log_pressure::ConfirmedPressure>,
    ) {
        let mut slots = self.slots.lock().expect("replica pressure");
        let Some(slot) = slots.get_mut(&ticket.route.identity.node_id) else {
            return;
        };
        if !slot.enabled
            || !Arc::ptr_eq(&slot.epoch, &ticket.epoch)
            || slot.sequence != ticket.sequence
        {
            return;
        }
        match reply {
            Ok(reply) if ticket.issued.elapsed() <= MAX_REPLY_AGE && reply.matches(&ticket) => {
                slot.capacity = Some(reply.capacity());
                slot.observed = Some(Instant::now());
            }
            _ => {
                slot.errors = slot.errors.saturating_add(1);
            }
        }
    }
    /// Called while the peer's append lock protects the actual current full
    /// configuration. No RPC, directory/engine lock or file I/O occurs here.
    pub fn refuses(&self, members: impl IntoIterator<Item = u64>, bytes: u64) -> bool {
        let slots = self.slots.lock().expect("replica pressure");
        members.into_iter().any(|node| {
            slots.get(&NodeId(node)).is_some_and(|slot| {
                matches!(slot.capacity, Some(None))
                    || slot
                        .capacity
                        .flatten()
                        .is_some_and(|capacity| capacity < bytes)
            })
        })
    }
    pub fn snapshot(&self) -> Vec<PressureStatus> {
        self.slots
            .lock()
            .expect("replica pressure")
            .values()
            .map(|slot| PressureStatus {
                node: slot.route.identity.node_id.0,
                generation: slot.route.generation,
                incarnation: slot.route.identity.store_incarnation.to_string(),
                address: slot.route.address.to_string(),
                polling: slot.enabled,
                observed: slot.capacity.is_some(),
                uncertain: slot.capacity == Some(None),
                available_bytes: slot.capacity.flatten(),
                age_millis: slot
                    .observed
                    .map(|at| at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64),
                errors: slot.errors,
            })
            .collect()
    }
}
fn invalid() -> Error {
    Error::Config("invalid, regressing or oversized replica pressure directory".into())
}

#[cfg(test)]
mod tests;
