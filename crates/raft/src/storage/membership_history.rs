//! Bounded inventory of node IDs observed in local durable configuration
//! records, including ancestors of installed snapshots. This is not a store
//! census: configurations do not identify disk incarnations, and snapshots
//! do not certify the source's preceding membership history.
use super::configuration_timeline::{
    ConfigurationTimeline, RecordKind, TimelineCursor, TimelinePage,
};
use super::*;
use kv9_common::{AppliedPosition, NodeId, RootDigest};
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};

mod initial;
mod wire;
pub use initial::InitialConfiguration;

pub const MEMBERSHIP_HISTORY_PAGE: usize = 4;
pub const MAX_KNOWN_MEMBERS: usize = 4096;
pub const SEEN_VOTER: u8 = 1;
pub const SEEN_OUTGOING_VOTER: u8 = 2;
pub const SEEN_LEARNER: u8 = 4;
pub const SEEN_NEXT_LEARNER: u8 = 8;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MembershipGaps {
    pub missing_initial: bool,
    pub snapshot_prefix_unknown: bool,
    pub ambiguous_records: bool,
    pub unsupported_configuration: bool,
    pub member_limit_exceeded: bool,
}
impl MembershipGaps {
    pub fn any(self) -> bool {
        self.missing_initial
            || self.snapshot_prefix_unknown
            || self.ambiguous_records
            || self.unsupported_configuration
            || self.member_limit_exceeded
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoricalMember {
    pub node: NodeId,
    /// Union of SEEN_* roles; removal never erases an observed role.
    pub roles: u8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MembershipCursor {
    fingerprint: RootDigest,
    after: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipPage {
    pub fingerprint: RootDigest,
    pub gaps: MembershipGaps,
    /// Applied configuration record position, NOT an all-entries applied cut.
    pub last_configuration_index: u64,
    pub last_snapshot: Option<AppliedPosition>,
    pub known_nodes: usize,
    pub members: Vec<HistoricalMember>,
    pub next: Option<MembershipCursor>,
}

pub(super) struct MembershipHistory {
    members: BTreeMap<u64, u8>,
    initial: bool,
    timeline: ConfigurationTimeline,
    initial_observation: InitialConfiguration,
    gaps: MembershipGaps,
    last_change: Option<(u64, RootDigest)>,
    snapshot: Option<AppliedPosition>,
    fingerprint: RootDigest,
}
impl Default for MembershipHistory {
    fn default() -> Self {
        let mut history = Self {
            members: BTreeMap::new(),
            initial: false,
            timeline: ConfigurationTimeline::default(),
            initial_observation: InitialConfiguration::Missing,
            gaps: MembershipGaps {
                missing_initial: true,
                ..MembershipGaps::default()
            },
            last_change: None,
            snapshot: None,
            fingerprint: RootDigest::from_bytes([0; 32]),
        };
        history.rehash();
        history
    }
}
impl MembershipHistory {
    fn observe(&mut self, cs: &ConfState) {
        let sets = [
            (cs.get_voters(), SEEN_VOTER),
            (cs.get_voters_outgoing(), SEEN_OUTGOING_VOTER),
            (cs.get_learners(), SEEN_LEARNER),
            (cs.get_learners_next(), SEEN_NEXT_LEARNER),
        ];
        // Unsupported input remains a sticky gap, not a shortened complete
        // inventory. Bound work even for a legacy oversized configuration.
        if cs.get_unknown_fields().iter().next().is_some()
            || sets
                .iter()
                .any(|(nodes, _)| nodes.len() > kv9_common::anchor::MAX_ANCHOR_MEMBERS)
        {
            self.gaps.unsupported_configuration = true;
            return;
        }
        if cs.voters.is_empty()
            || cs
                .learners
                .iter()
                .any(|n| cs.voters.contains(n) || cs.voters_outgoing.contains(n))
            || cs.learners_next.iter().any(|n| {
                !cs.voters_outgoing.contains(n) || cs.voters.contains(n) || cs.learners.contains(n)
            })
            || (cs.voters_outgoing.is_empty() && (cs.auto_leave || !cs.learners_next.is_empty()))
        {
            self.gaps.unsupported_configuration = true;
        }
        for (nodes, role) in sets {
            for (i, &node) in nodes.iter().enumerate() {
                if node == 0 || nodes[..i].contains(&node) {
                    self.gaps.unsupported_configuration = true;
                    continue;
                }
                if let Some(roles) = self.members.get_mut(&node) {
                    *roles |= role;
                } else if self.members.len() < MAX_KNOWN_MEMBERS {
                    self.members.insert(node, role);
                } else {
                    self.gaps.member_limit_exceeded = true;
                    // Keep a deterministic subset across rewrite/replay order.
                    // The gap forbids treating this subset as complete coverage.
                    if self
                        .members
                        .last_key_value()
                        .is_some_and(|(last, _)| node < *last)
                    {
                        self.members.pop_last();
                        self.members.insert(node, role);
                    }
                }
            }
        }
    }
    pub(super) fn initial(&mut self, cs: &ConfState) {
        self.timeline.observe(RecordKind::Initial, cs, None);
        if self.initial || self.last_change.is_some() || self.snapshot.is_some() {
            self.gaps.ambiguous_records = true;
            self.initial_observation = InitialConfiguration::Ambiguous;
        } else {
            self.initial_observation = InitialConfiguration::observe(cs);
        }
        self.initial = true;
        self.gaps.missing_initial = false;
        self.observe(cs);
        self.rehash();
    }
    pub(super) fn applied(&mut self, index: u64, cs: &ConfState) {
        self.applied_origin(index, cs, None);
    }
    pub(super) fn applied_origin(
        &mut self,
        index: u64,
        cs: &ConfState,
        origin: Option<configuration_origins::AppliedConfigurationOrigin>,
    ) {
        self.timeline
            .observe(RecordKind::Applied { index }, cs, origin);
        // An oversized/unknown legacy state becomes a gap without making an
        // additional full protobuf copy. Only bounded known fields are hashed.
        let digest = if cs.get_unknown_fields().iter().next().is_none()
            && [
                cs.get_voters(),
                cs.get_voters_outgoing(),
                cs.get_learners(),
                cs.get_learners_next(),
            ]
            .iter()
            .all(|nodes| nodes.len() <= kv9_common::anchor::MAX_ANCHOR_MEMBERS)
        {
            RootDigest::sha256(&cs.write_to_bytes().expect("bounded ConfState encodes"))
        } else {
            RootDigest::from_bytes([0; 32])
        };
        if index == 0
            || index == u64::MAX
            || self
                .last_change
                .is_some_and(|(last, value)| last > index || (last == index && value != digest))
            || self.snapshot.is_some_and(|s| index <= s.index)
        {
            self.gaps.ambiguous_records = true;
        }
        if self.last_change.is_none_or(|(last, _)| index >= last) {
            self.last_change = Some((index, digest));
        }
        self.observe(cs);
        self.rehash();
    }
    pub(super) fn installed(&mut self, cut: AppliedPosition, cs: &ConfState) {
        self.timeline
            .observe(RecordKind::Snapshot { at: cut }, cs, None);
        self.gaps.snapshot_prefix_unknown = true;
        self.snapshot = Some(cut);
        self.last_change = None;
        self.observe(cs);
        self.rehash();
    }
    pub(super) fn retained_base(&mut self, cs: &ConfState) {
        // Rewrite places the base BEFORE the verbatim initial/indexed history.
        // It contributes possible members but cannot establish that initial.
        self.observe(cs);
        self.rehash();
    }
    pub(super) fn timeline_page(&self, after: Option<TimelineCursor>) -> Result<TimelinePage> {
        self.timeline.page(self.fingerprint, self.gaps, after)
    }
    pub(super) fn initial_observation(&self) -> InitialConfiguration {
        self.initial_observation.clone()
    }
    fn rehash(&mut self) {
        let mut bytes = Vec::with_capacity(64 + self.members.len() * 9);
        bytes.extend_from_slice(b"KV9MEM01");
        bytes.extend_from_slice(self.timeline.fingerprint().as_bytes());
        bytes.extend_from_slice(
            &self
                .initial_observation
                .encode()
                .expect("bounded initial record"),
        );
        bytes.extend_from_slice(&[
            self.gaps.missing_initial as u8,
            self.gaps.snapshot_prefix_unknown as u8,
            self.gaps.ambiguous_records as u8,
            self.gaps.unsupported_configuration as u8,
            self.gaps.member_limit_exceeded as u8,
        ]);
        bytes.extend_from_slice(&self.last_index().to_be_bytes());
        if let Some(cut) = self.snapshot {
            bytes.extend_from_slice(&cut.term.to_be_bytes());
            bytes.extend_from_slice(&cut.index.to_be_bytes());
        } else {
            bytes.extend_from_slice(&[0; 16]);
        }
        for (node, roles) in &self.members {
            bytes.extend_from_slice(&node.to_be_bytes());
            bytes.push(*roles);
        }
        self.fingerprint = RootDigest::sha256(&bytes);
    }
    fn last_index(&self) -> u64 {
        self.last_change
            .map_or(0, |(i, _)| i)
            .max(self.snapshot.map_or(0, |s| s.index))
    }
    pub(super) fn same_observations(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
    }
    pub(super) fn page(&self, cursor: Option<MembershipCursor>) -> Result<MembershipPage> {
        let after = match cursor {
            Some(c) if c.fingerprint == self.fingerprint && self.members.contains_key(&c.after) => {
                c.after
            }
            Some(_) => return Err(Error::Raft("membership history cursor changed".into())),
            None => 0,
        };
        let mut iter = self.members.range((Excluded(after), Unbounded));
        let members: Vec<_> = iter
            .by_ref()
            .take(MEMBERSHIP_HISTORY_PAGE)
            .map(|(&node, &roles)| HistoricalMember {
                node: NodeId(node),
                roles,
            })
            .collect();
        let next = iter.next().map(|_| MembershipCursor {
            fingerprint: self.fingerprint,
            after: members
                .last()
                .expect("nonempty page before lookahead")
                .node
                .0,
        });
        Ok(MembershipPage {
            fingerprint: self.fingerprint,
            gaps: self.gaps,
            last_configuration_index: self.last_index(),
            last_snapshot: self.snapshot,
            known_nodes: self.members.len(),
            members,
            next,
        })
    }
}
impl<F: FileSystem> DiskRaftStorage<F> {
    /// Inventory local durable configuration observations. An empty next page
    /// does not clear gaps or certify global membership, store incarnations,
    /// committed-but-unapplied changes, or sources predating a snapshot.
    /// Callers must bind the result to their group/store and cut independently.
    pub fn membership_history_page(
        &self,
        cursor: Option<MembershipCursor>,
    ) -> Result<MembershipPage> {
        let writer = self.file.lock().expect("raft log file poisoned");
        if writer.is_none() {
            return Err(Error::Raft(
                "membership history requires recovered storage".into(),
            ));
        }
        self.membership_history
            .lock()
            .expect("membership history poisoned")
            .page(cursor)
    }
}

#[cfg(test)]
mod tests;
