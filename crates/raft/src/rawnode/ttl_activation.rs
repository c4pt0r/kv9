//! One-use, configuration-bound format barrier for Raw TTL activation.
//! Format acknowledgements are durable facts, not quorum or serving receipts.
//! The controller must resolve exact stores from its authoritative directory.
use super::*;
use crate::command::{RawTtlAction, RawTtlCommand};
use crate::grpc::ConfirmedRawTtlFormat;
use crate::work::WorkSignal;
use kv9_common::data_range::DataRange;
use kv9_common::raw_ttl::ClockWatermark;
use kv9_common::StoreIdentity;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// Limits both held confirmation state and the duration of one collection pass.
pub const MAX_ACTIVATION_REPLICAS: usize = 1024;
pub const MAX_ACTIVATION_WINDOW: Duration = Duration::from_secs(30);

/// Captured only from a live leader and the driver's actual applied range.
/// No Clone/deserialize: each attempt is consumed even if submission fails.
pub struct RawTtlActivationProbe {
    owner: Arc<WorkSignal>,
    range: DataRange,
    term: u64,
    conf_applied: u64,
    configuration: ConfState,
    replicas: BTreeSet<NodeId>,
    deadline: Instant,
}

pub struct RawTtlActivationConfirmations {
    probe: RawTtlActivationProbe,
    stores: BTreeMap<NodeId, (StoreIdentity, bool)>,
}

/// Complete format coverage for one specific probe, not permission to serve TTL.
pub struct PreparedRawTtlActivation {
    probe: RawTtlActivationProbe,
}

fn retry(reason: &str) -> Error {
    Error::MetaNotReady(format!("Raw TTL activation must be recollected: {reason}"))
}

fn before_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(retry("collection deadline expired"));
    }
    Ok(())
}

impl RawTtlActivationProbe {
    pub fn range(&self) -> &DataRange {
        &self.range
    }

    pub fn replicas(&self) -> impl ExactSizeIterator<Item = NodeId> + '_ {
        self.replicas.iter().copied()
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Exact identities must come from one authoritative directory snapshot.
    /// No majority threshold, duplicate substitution, or implicit self vote.
    pub fn bind_stores(
        self,
        identities: &[StoreIdentity],
    ) -> Result<RawTtlActivationConfirmations> {
        before_deadline(self.deadline)?;
        if identities.len() != self.replicas.len() {
            return Err(retry(
                "store identities do not cover the full configuration",
            ));
        }
        let mut stores = BTreeMap::new();
        let cluster = identities.first().map(|s| s.cluster_id);
        for identity in identities {
            if !self.replicas.contains(&identity.node_id)
                || identity.root_digest != self.range.root
                || Some(identity.cluster_id) != cluster
                || identity.cluster_id.as_bytes() == &[0; 16]
                || identity.store_incarnation.as_bytes() == &[0; 16]
                || stores
                    .insert(identity.node_id, (*identity, false))
                    .is_some()
            {
                return Err(retry("invalid, duplicate or foreign store identity"));
            }
        }
        Ok(RawTtlActivationConfirmations {
            probe: self,
            stores,
        })
    }
}

impl RawTtlActivationConfirmations {
    pub fn confirm(&mut self, confirmed: ConfirmedRawTtlFormat) -> Result<()> {
        before_deadline(self.probe.deadline)?;
        let identity = confirmed.identity();
        let Some((expected, received)) = self.stores.get_mut(&identity.node_id) else {
            return Err(retry("confirmation names a non-member"));
        };
        if identity != *expected {
            return Err(retry(
                "confirmation names a different store incarnation or root",
            ));
        }
        *received = true; // Repeated replies cannot substitute for another member.
        Ok(())
    }

    pub fn finish(self) -> Result<PreparedRawTtlActivation> {
        before_deadline(self.probe.deadline)?;
        if self.stores.values().any(|(_, received)| !received) {
            return Err(retry("at least one replica has not confirmed the format"));
        }
        Ok(PreparedRawTtlActivation { probe: self.probe })
    }
}

impl PreparedRawTtlActivation {
    pub(crate) fn range(&self) -> &DataRange {
        &self.probe.range
    }

    pub(crate) fn deadline(&self) -> Instant {
        self.probe.deadline
    }
}

/// raft-rs may emit sets in varying order. Compare canonical full state, while
/// retaining the applied configuration index to reject change-away-and-back.
fn configuration<S: PersistentRaftStorage>(
    g: &PeerInner<S>,
) -> Result<(ConfState, BTreeSet<NodeId>)> {
    g.check_fatal()?;
    if !g.alive || g.raw.raft.state != StateRole::Leader {
        let leader = g.raw.raft.leader_id;
        return Err(Error::NotLeader {
            leader: (leader != raft::INVALID_ID).then_some(NodeId(leader)),
        });
    }
    // Use raft-rs's apply frontier: a new leader conservatively sets this to
    // last_index even if the tail has no ConfChange. Comparing conf_applied
    // instead would permanently reject such a healthy leader.
    if g.raw.raft.has_pending_conf() {
        return Err(retry(
            "a configuration change or recovered tail is not applied",
        ));
    }
    let mut state = g.raw.raft.prs().conf().to_conf_state();
    if state.get_unknown_fields().iter().next().is_some() {
        return Err(retry("unknown configuration fields"));
    }
    let mut replicas = BTreeSet::new();
    for ids in [
        &mut state.voters,
        &mut state.voters_outgoing,
        &mut state.learners,
        &mut state.learners_next,
    ] {
        if ids.len() > MAX_ACTIVATION_REPLICAS {
            return Err(retry("configuration exceeds replica bound"));
        }
        ids.sort_unstable();
        if ids.first() == Some(&0) || ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(retry("invalid configuration members"));
        }
        replicas.extend(ids.iter().copied().map(NodeId));
    }
    if state.voters.is_empty() || replicas.len() > MAX_ACTIVATION_REPLICAS {
        return Err(retry("empty or oversized configuration"));
    }
    Ok((state, replicas))
}

impl<S: PersistentRaftStorage> RaftPeer<S> {
    pub(crate) fn begin_raw_ttl_activation(
        &self,
        range: DataRange,
        deadline: Instant,
    ) -> Result<RawTtlActivationProbe> {
        let now = Instant::now();
        if deadline <= now || deadline.duration_since(now) > MAX_ACTIVATION_WINDOW {
            return Err(retry("collection deadline must be within 30 seconds"));
        }
        if range.region != self.region || range.sealed {
            return Err(retry("range does not belong to this active group"));
        }
        range.validate()?;
        let g = self.lock();
        let (configuration, replicas) = configuration(&g)?;
        Ok(RawTtlActivationProbe {
            owner: self.work_signal.clone(),
            range,
            term: g.raw.raft.term,
            conf_applied: g.conf_applied,
            configuration,
            replicas,
            deadline,
        })
    }

    pub(crate) fn propose_raw_ttl_activation(
        &self,
        prepared: PreparedRawTtlActivation,
    ) -> Result<ProposedAt> {
        // Serialize outside the peer lock; no RPC or engine read occurs in it.
        // Activation establishes format only, and cannot invent a clock sample.
        let data = crate::Command::RawTtl(RawTtlCommand::new(
            prepared.range(),
            RawTtlAction::Enable {
                observed: ClockWatermark::default(),
            },
        )?)
        .encode();
        self.propose_format_confirmed(prepared, data, false)
    }

    /// Shared final append check. Only typed activation/transfer wrappers can
    /// pair a confirmation set with their pre-bound command.
    pub(super) fn propose_format_confirmed(
        &self,
        prepared: PreparedRawTtlActivation,
        data: Vec<u8>,
        requires_write_capacity: bool,
    ) -> Result<ProposedAt> {
        let probe = prepared.probe;
        let mut g = self.lock();
        let (current, _) = configuration(&g)?;
        before_deadline(probe.deadline)?;
        if !Arc::ptr_eq(&self.work_signal, &probe.owner)
            || self.region != probe.range.region
            || g.raw.raft.term != probe.term
            || g.conf_applied != probe.conf_applied
            || current != probe.configuration
        {
            return Err(retry(
                "peer incarnation, leader term or configuration changed",
            ));
        }
        if requires_write_capacity {
            g.check_write_pressure(self.region)?;
        }
        let bytes = data.len();
        let claim = g.reserve_log_bytes(self.region, bytes, requires_write_capacity)?;
        // A filesystem-space probe can consume the confirmation's remaining
        // lifetime even while this peer's term/configuration stay locked.
        before_deadline(probe.deadline)?;
        g.raw.propose(Vec::new(), data).map_err(raft_err)?;
        let at = ProposedAt {
            term: g.raw.raft.term,
            index: LogIndex(g.raw.raft.raft_log.last_index()),
        };
        g.retain_proposed_bytes(at, bytes, claim)?;
        drop(g);
        self.work_signal.notify();
        Ok(at)
    }
}

#[cfg(test)]
mod tests;
