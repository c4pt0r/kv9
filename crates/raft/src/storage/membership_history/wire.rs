//! Canonical bounded transport descriptors, never source/store authority.
use super::*;

const CURSOR: &[u8; 8] = b"KV9MHC01";
const PAGE: &[u8; 8] = b"KV9MHP01";
fn invalid() -> Error {
    Error::Raft("invalid membership history descriptor".into())
}
impl MembershipCursor {
    pub fn encode(self) -> Vec<u8> {
        let mut b = CURSOR.to_vec();
        b.extend_from_slice(self.fingerprint.as_bytes());
        b.extend_from_slice(&self.after.to_be_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != 48 || &b[..8] != CURSOR || b[8..40] == [0; 32] {
            return Err(invalid());
        }
        let after = u64::from_be_bytes(b[40..48].try_into().unwrap());
        if after == 0 {
            return Err(invalid());
        }
        Ok(Self {
            fingerprint: RootDigest::from_bytes(b[8..40].try_into().unwrap()),
            after,
        })
    }
}
impl MembershipPage {
    fn validate(&self) -> Result<()> {
        if self.fingerprint.as_bytes() == &[0; 32]
            || self.known_nodes > MAX_KNOWN_MEMBERS
            || self.members.len() > MEMBERSHIP_HISTORY_PAGE
            || self.known_nodes < self.members.len()
            || (self.members.is_empty() && (self.known_nodes != 0 || !self.gaps.any()))
            || self
                .members
                .iter()
                .any(|m| m.node.0 == 0 || m.roles == 0 || m.roles & !15 != 0)
            || self.members.windows(2).any(|p| p[0].node >= p[1].node)
            || self.gaps.snapshot_prefix_unknown != self.last_snapshot.is_some()
            || self.last_snapshot.is_some_and(|s| {
                s.term == 0
                    || s.index == 0
                    || s.index == u64::MAX
                    || s.index > self.last_configuration_index
            })
            || self.next.is_some_and(|c| {
                c.fingerprint != self.fingerprint
                    || self.members.len() != MEMBERSHIP_HISTORY_PAGE
                    || self.known_nodes <= self.members.len()
                    || self.members.last().is_none_or(|m| m.node.0 != c.after)
            })
        {
            return Err(invalid());
        }
        Ok(())
    }
    /// Bind a response to the exact previous page and number of entries already
    /// observed. A terminal page must account for every reported known node.
    pub fn validate_progress(&self, cursor: Option<MembershipCursor>, seen: usize) -> Result<()> {
        self.validate()?;
        if (seen == 0) != cursor.is_none()
            || seen > MAX_KNOWN_MEMBERS
            || seen + self.members.len() > self.known_nodes
            || cursor.is_some_and(|c| {
                c.fingerprint != self.fingerprint
                    || self.members.first().is_none_or(|m| m.node.0 <= c.after)
            })
            || (self.next.is_none() && seen + self.members.len() != self.known_nodes)
            || (self.next.is_some() && seen + self.members.len() >= self.known_nodes)
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut b = Vec::with_capacity(112);
        b.extend_from_slice(PAGE);
        b.extend_from_slice(self.fingerprint.as_bytes());
        b.push(
            u8::from(self.gaps.missing_initial)
                | (u8::from(self.gaps.snapshot_prefix_unknown) << 1)
                | (u8::from(self.gaps.ambiguous_records) << 2)
                | (u8::from(self.gaps.unsupported_configuration) << 3)
                | (u8::from(self.gaps.member_limit_exceeded) << 4),
        );
        b.extend_from_slice(&self.last_configuration_index.to_be_bytes());
        b.extend_from_slice(&self.last_snapshot.map_or(0, |s| s.term).to_be_bytes());
        b.extend_from_slice(&self.last_snapshot.map_or(0, |s| s.index).to_be_bytes());
        b.extend_from_slice(&(self.known_nodes as u16).to_be_bytes());
        b.push(self.members.len() as u8);
        for m in &self.members {
            b.extend_from_slice(&m.node.0.to_be_bytes());
            b.push(m.roles);
        }
        b.extend_from_slice(&self.next.map_or(0, |c| c.after).to_be_bytes());
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        if !(76..=112).contains(&b.len()) || &b[..8] != PAGE || b[40] & !31 != 0 {
            return Err(invalid());
        }
        let n = b[67] as usize;
        if n > MEMBERSHIP_HISTORY_PAGE || b.len() != 76 + 9 * n {
            return Err(invalid());
        }
        let uint = |at| u64::from_be_bytes(b[at..at + 8].try_into().unwrap());
        let fingerprint = RootDigest::from_bytes(b[8..40].try_into().unwrap());
        let snapshot = AppliedPosition {
            term: uint(49),
            index: uint(57),
        };
        let after = uint(68 + n * 9);
        let page = Self {
            fingerprint,
            gaps: MembershipGaps {
                missing_initial: b[40] & 1 != 0,
                snapshot_prefix_unknown: b[40] & 2 != 0,
                ambiguous_records: b[40] & 4 != 0,
                unsupported_configuration: b[40] & 8 != 0,
                member_limit_exceeded: b[40] & 16 != 0,
            },
            last_configuration_index: uint(41),
            last_snapshot: (snapshot.term != 0 || snapshot.index != 0).then_some(snapshot),
            known_nodes: u16::from_be_bytes(b[65..67].try_into().unwrap()) as usize,
            members: (0..n)
                .map(|i| HistoricalMember {
                    node: NodeId(uint(68 + i * 9)),
                    roles: b[76 + i * 9],
                })
                .collect(),
            next: (after != 0).then_some(MembershipCursor { fingerprint, after }),
        };
        page.validate()?;
        if page.encode()? != b {
            return Err(invalid());
        }
        Ok(page)
    }
}
