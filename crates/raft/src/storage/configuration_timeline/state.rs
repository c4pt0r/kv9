use super::*;
pub const MAX_MEMBERS: usize = kv9_common::anchor::MAX_ANCHOR_MEMBERS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationState {
    pub voters: Vec<u64>,
    pub outgoing: Vec<u64>,
    pub learners: Vec<u64>,
    pub learners_next: Vec<u64>,
    pub auto_leave: bool,
}
impl ConfigurationState {
    /// Expected result of the single Auto change carried by a known origin.
    /// This computes Raft semantics only; it does not authenticate the origin.
    pub fn after_origin(
        &self,
        action: crate::configuration_origin::Action,
        node: kv9_common::NodeId,
    ) -> Result<Option<Self>> {
        self.validate()?;
        if node.0 == 0 {
            return Err(invalid());
        }
        // Known origins encode one Auto change. Raft refuses a simple change
        // while joint; legacy joint transitions stay descriptive/unknown.
        if !self.outgoing.is_empty() {
            return Ok(None);
        }
        let mut tracker = raft::ProgressTracker::new(1);
        for (nodes, kind) in [
            (&self.voters, raft::prelude::ConfChangeType::AddNode),
            (
                &self.learners,
                raft::prelude::ConfChangeType::AddLearnerNode,
            ),
        ] {
            for node in nodes {
                let seed = raft::prelude::ConfChangeSingle {
                    node_id: *node,
                    change_type: kind,
                    ..Default::default()
                };
                let Ok((configuration, changes)) = raft::Changer::new(&tracker).simple(&[seed])
                else {
                    return Ok(None);
                };
                tracker.apply_conf(configuration, changes, 1);
            }
        }
        let change = raft::prelude::ConfChangeSingle {
            node_id: node.0,
            change_type: action.change_type(),
            ..Default::default()
        };
        let Ok((configuration, _)) = raft::Changer::new(&tracker).simple(&[change]) else {
            return Ok(None);
        };
        Ok(Self::observe(&configuration.to_conf_state()))
    }
    fn sets(&self) -> [&[u64]; 4] {
        [
            &self.voters,
            &self.outgoing,
            &self.learners,
            &self.learners_next,
        ]
    }
    fn validate(&self) -> Result<()> {
        if self.sets().iter().any(|s| {
            s.len() > MAX_MEMBERS || s.first() == Some(&0) || s.windows(2).any(|w| w[0] >= w[1])
        }) || self.voters.is_empty()
            || self.learners.iter().any(|n| {
                self.voters.binary_search(n).is_ok() || self.outgoing.binary_search(n).is_ok()
            })
            || self.learners_next.iter().any(|n| {
                self.outgoing.binary_search(n).is_err()
                    || self.voters.binary_search(n).is_ok()
                    || self.learners.binary_search(n).is_ok()
            })
            || (self.outgoing.is_empty() && (self.auto_leave || !self.learners_next.is_empty()))
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) fn observe(cs: &ConfState) -> Option<Self> {
        if cs.get_unknown_fields().iter().next().is_some()
            || [
                &cs.voters,
                &cs.voters_outgoing,
                &cs.learners,
                &cs.learners_next,
            ]
            .iter()
            .any(|s| s.len() > MAX_MEMBERS)
        {
            return None;
        }
        let sorted = |s: &[u64]| {
            let mut s = s.to_vec();
            s.sort_unstable();
            s
        };
        let state = Self {
            voters: sorted(&cs.voters),
            outgoing: sorted(&cs.voters_outgoing),
            learners: sorted(&cs.learners),
            learners_next: sorted(&cs.learners_next),
            auto_leave: cs.auto_leave,
        };
        state.validate().ok()?;
        Some(state)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut b = Vec::new();
        b.push(u8::from(self.auto_leave));
        for nodes in self.sets() {
            b.extend_from_slice(&(nodes.len() as u16).to_be_bytes());
            for n in nodes {
                b.extend_from_slice(&n.to_be_bytes());
            }
        }
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() > 9 + 4 * MAX_MEMBERS * 8 {
            return Err(invalid());
        }
        let mut r = super::wire::Reader(b);
        let auto_leave = r.boolean()?;
        let mut set = || -> Result<Vec<u64>> {
            let count = u16::from_be_bytes(r.array()?) as usize;
            if count > MAX_MEMBERS || count > r.0.len() / 8 {
                return Err(invalid());
            }
            (0..count).map(|_| r.uint()).collect()
        };
        let out = Self {
            auto_leave,
            voters: set()?,
            outgoing: set()?,
            learners: set()?,
            learners_next: set()?,
        };
        if !r.0.is_empty() || out.encode()? != b {
            return Err(invalid());
        }
        Ok(out)
    }
}
