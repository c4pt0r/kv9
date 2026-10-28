use super::*;
use kv9_raft::configuration_origin::Action;
use kv9_raft::storage::configuration_timeline::ConfigurationState;
use std::collections::{BTreeMap, BTreeSet};

pub const MISSING_NATIVE: u16 = 1;
pub const LEGACY_CHANGE: u16 = 2;
pub const SNAPSHOT_ANCESTRY: u16 = 4;
pub const UNSUPPORTED_STATE: u16 = 8;
pub const AMBIGUOUS_POSITION: u16 = 16;
pub const TRANSITION_MISMATCH: u16 = 32;
pub const STORE_CONFLICT: u16 = 64;
pub const UNKNOWN_STORE: u16 = 128;
pub const SOURCE_HISTORY_GAPS: u16 = 256;
const ALL_GAPS: u16 = 511;
pub(super) const MAX_MEMBERS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub node: kv9_common::NodeId,
    /// SEEN_VOTER / SEEN_OUTGOING_VOTER / SEEN_LEARNER / SEEN_NEXT_LEARNER.
    pub roles: u8,
    pub store: Option<kv9_common::StoreIncarnation>,
}
pub(super) fn members(state: &ConfigurationState) -> Result<Vec<Member>> {
    state.encode()?;
    let mut roles = BTreeMap::new();
    for (nodes, role) in [
        (&state.voters, 1),
        (&state.outgoing, 2),
        (&state.learners, 4),
        (&state.learners_next, 8),
    ] {
        for node in nodes {
            *roles.entry(*node).or_insert(0) |= role;
        }
    }
    Ok(roles
        .into_iter()
        .map(|(node, roles)| Member {
            node: kv9_common::NodeId(node),
            roles,
            store: None,
        })
        .collect())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Boundary {
    pub ordinal: u64,
    pub kind: RecordKind,
    /// Initial begins at index zero. Ambiguous legacy indices remain None.
    pub index: Option<u64>,
    /// No term is invented for native initial or legacy apply records.
    pub term: Option<u64>,
}
pub(super) fn boundary(record: &ConfigurationRecord) -> Boundary {
    let (index, term) = match record.kind {
        RecordKind::Initial => (Some(0), None),
        RecordKind::Applied { index } => (
            (index != 0 && index != u64::MAX).then_some(index),
            record.origin.map(|o| o.at.term),
        ),
        RecordKind::Snapshot { at } => (Some(at.index), Some(at.term)),
    };
    Boundary {
        ordinal: record.ordinal,
        kind: record.kind,
        index,
        term,
    }
}
pub(super) fn project(
    record: &ConfigurationRecord,
    previous: Option<(&ConfigurationRecord, &Entry)>,
    native: Option<&initial::InitialOrigin>,
) -> Result<(Option<Vec<Member>>, u16)> {
    let Some(state) = &record.state else {
        return Ok((None, UNSUPPORTED_STATE | UNKNOWN_STORE));
    };
    let mut mapped = members(state)?;
    let mut gaps = 0;
    match record.kind {
        RecordKind::Initial => {
            if record.ordinal == 1
                && native.is_some_and(|n| {
                    state.outgoing.is_empty()
                        && state.learners.is_empty()
                        && state.learners_next.is_empty()
                        && !state.auto_leave
                        && n.replicas
                            .iter()
                            .map(|r| r.node.0)
                            .eq(state.voters.iter().copied())
                })
            {
                for (member, replica) in mapped.iter_mut().zip(&native.unwrap().replicas) {
                    member.store = Some(replica.incarnation);
                }
            } else {
                gaps |= MISSING_NATIVE;
            }
            if record.ordinal != 1 {
                gaps |= AMBIGUOUS_POSITION;
            }
        }
        RecordKind::Snapshot { .. } => {
            gaps |= SNAPSHOT_ANCESTRY;
        }
        RecordKind::Applied { .. } => {
            let current = boundary(record);
            let ordered = previous.is_some_and(|(p, _)| {
                let old = boundary(p);
                current.index.zip(old.index).is_some_and(|(a, b)| a > b)
                    && current.term.zip(old.term).is_none_or(|(a, b)| a >= b)
            });
            if !ordered {
                gaps |= AMBIGUOUS_POSITION;
            }
            match record.origin {
                None => gaps |= LEGACY_CHANGE,
                Some(origin) => {
                    let expected = previous
                        .and_then(|(p, _)| p.state.as_ref())
                        .map(|s| s.after_origin(origin.origin.action, origin.origin.node))
                        .transpose()?
                        .flatten();
                    if expected.as_ref() != Some(state) {
                        gaps |= TRANSITION_MISMATCH;
                    }
                    if gaps == 0 {
                        let old = previous.unwrap().1.members.as_deref().unwrap_or(&[]);
                        for member in &mut mapped {
                            member.store = old
                                .binary_search_by_key(&member.node, |m| m.node)
                                .ok()
                                .and_then(|i| old[i].store);
                        }
                        let subject = origin.origin.node;
                        if old.iter().any(|m| {
                            m.node == subject && m.store.is_some_and(|s| s != origin.origin.store)
                        }) {
                            gaps |= STORE_CONFLICT;
                        } else if matches!(origin.origin.action, Action::Attach | Action::Promote) {
                            if let Some(member) = mapped.iter_mut().find(|m| m.node == subject) {
                                member.store = Some(origin.origin.store);
                            }
                        }
                        let mut stores = BTreeSet::new();
                        if mapped
                            .iter()
                            .filter_map(|m| m.store)
                            .any(|s| !stores.insert(*s.as_bytes()))
                        {
                            gaps |= STORE_CONFLICT;
                        }
                        if gaps != 0 {
                            for member in &mut mapped {
                                member.store = None;
                            }
                        }
                    }
                }
            }
        }
    }
    if mapped.iter().any(|m| m.store.is_none()) {
        gaps |= UNKNOWN_STORE;
    }
    Ok((Some(mapped), gaps))
}
pub(super) fn validate_members(record: &ConfigurationRecord, entry: &Entry) -> Result<()> {
    if entry.gaps & !ALL_GAPS != 0 {
        return Err(invalid("configuration interval gap bits"));
    }
    match (&record.state, &entry.members) {
        (None, None) if entry.gaps & UNSUPPORTED_STATE != 0 && entry.gaps & UNKNOWN_STORE != 0 => {}
        (Some(state), Some(mapped)) => {
            let expected = members(state)?;
            let required = match record.kind {
                RecordKind::Snapshot { .. } => SNAPSHOT_ANCESTRY,
                RecordKind::Applied { .. } if record.origin.is_none() => LEGACY_CHANGE,
                RecordKind::Initial if record.ordinal != 1 => MISSING_NATIVE | AMBIGUOUS_POSITION,
                _ => 0,
            };
            let no_stores = MISSING_NATIVE
                | LEGACY_CHANGE
                | SNAPSHOT_ANCESTRY
                | AMBIGUOUS_POSITION
                | TRANSITION_MISMATCH
                | STORE_CONFLICT;
            let mut stores = BTreeSet::new();
            if mapped.len() > MAX_MEMBERS
                || mapped.len() != expected.len()
                || entry.gaps & UNSUPPORTED_STATE != 0
                || entry.gaps & required != required
                || mapped.iter().zip(&expected).any(|(m, e)| {
                    m.node != e.node
                        || m.roles != e.roles
                        || m.store.is_some_and(|s| {
                            s.as_bytes() == &[0; 16]
                                || !stores.insert(*s.as_bytes())
                                || entry.gaps & no_stores != 0
                        })
                })
                || (mapped.iter().any(|m| m.store.is_none()) && entry.gaps & UNKNOWN_STORE == 0)
            {
                return Err(invalid(
                    "configuration interval members differ from actual state",
                ));
            }
        }
        _ => return Err(invalid("configuration interval unsupported state differs")),
    }
    Ok(())
}
