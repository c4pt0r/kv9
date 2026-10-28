//! Local initial record only. A snapshot's selected configuration cannot
//! substitute for the native initial record or certify imported ancestry.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitialConfiguration {
    Missing,
    Ambiguous,
    Unsupported,
    Voters(Vec<NodeId>),
}
impl InitialConfiguration {
    pub(super) fn observe(state: &ConfState) -> Self {
        if state.get_unknown_fields().iter().next().is_some()
            || ![3, 5, 7].contains(&state.voters.len())
            || !state.voters_outgoing.is_empty()
            || !state.learners.is_empty()
            || !state.learners_next.is_empty()
            || state.auto_leave
        {
            return Self::Unsupported;
        }
        let mut voters: Vec<_> = state.voters.iter().copied().map(NodeId).collect();
        voters.sort_unstable();
        if voters.first() == Some(&NodeId(0)) || voters.windows(2).any(|w| w[0] == w[1]) {
            Self::Unsupported
        } else {
            Self::Voters(voters)
        }
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        let (tag, voters) = match self {
            Self::Missing => return Ok(vec![0]),
            Self::Ambiguous => return Ok(vec![1]),
            Self::Unsupported => return Ok(vec![2]),
            Self::Voters(v) => (3, v),
        };
        if ![3, 5, 7].contains(&voters.len())
            || voters.first() == Some(&NodeId(0))
            || voters.windows(2).any(|w| w[0] >= w[1])
        {
            return Err(Error::Raft("invalid native initial configuration".into()));
        }
        let mut b = vec![tag, voters.len() as u8];
        for node in voters {
            b.extend_from_slice(&node.0.to_be_bytes());
        }
        Ok(b)
    }
    pub fn decode(b: &[u8]) -> Result<Self> {
        let out = match b {
            [0] => Self::Missing,
            [1] => Self::Ambiguous,
            [2] => Self::Unsupported,
            [3, n, rest @ ..] if [3, 5, 7].contains(n) && rest.len() == *n as usize * 8 => {
                Self::Voters(
                    rest.chunks_exact(8)
                        .map(|v| NodeId(u64::from_be_bytes(v.try_into().unwrap())))
                        .collect(),
                )
            }
            _ => {
                return Err(Error::Raft(
                    "invalid native initial configuration encoding".into(),
                ))
            }
        };
        if out.encode()? != b {
            return Err(Error::Raft(
                "noncanonical native initial configuration".into(),
            ));
        }
        Ok(out)
    }
    pub fn is_unknown(&self) -> bool {
        !matches!(self, Self::Voters(_))
    }
}
