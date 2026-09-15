//! Recover the actual published configuration at a committed state-image cut.
//! This is a recovery/anchor input, not a complete snapshot install capability.
use std::collections::BTreeMap;

use kv9_common::{AppliedPosition, Error, Result};
use protobuf::Message as _;
use raft::prelude::ConfState;
use raft::{GetEntriesContext, Storage};

use super::{DiskRaftStorage, FileSystem};

#[derive(Default)]
pub(super) struct ConfigurationHistory {
    initial: Option<ConfState>,
    applied: BTreeMap<u64, ConfState>,
    ambiguous: bool,
}
impl ConfigurationHistory {
    pub(super) fn initial(&mut self, state: &ConfState) {
        // A later unindexed record cannot establish when its membership began.
        if self.initial.is_some() || !self.applied.is_empty() {
            self.ambiguous = true;
        }
        self.initial.get_or_insert_with(|| state.clone());
    }
    pub(super) fn applied(&mut self, index: u64, state: &ConfState) {
        if index == 0
            || self
                .applied
                .last_key_value()
                .is_some_and(|(last, value)| *last > index || (*last == index && value != state))
        {
            self.ambiguous = true;
        }
        self.applied.entry(index).or_insert_with(|| state.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationUnavailable {
    InitialConfigurationMissing,
    UnindexedOrConflictingHistory,
    ProtocolHistoryCompacted,
    /// A configuration command is committed but has no selected applied record
    /// at this cut. Recovery must apply it or obtain certified snapshot state.
    ConfigurationNotApplied {
        index: u64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigurationLookup {
    Found(CommittedConfiguration),
    Unavailable(ConfigurationUnavailable),
}

/// Minted only by a successful lookup under the protocol writer lock. Private
/// fields prevent a plain descriptor from substituting for this checked view.
/// Root/range identity and manifest publication still need independent binding.
#[derive(Debug, Clone, PartialEq)]
pub struct CommittedConfiguration {
    cut: AppliedPosition,
    applied_at: Option<AppliedPosition>,
    state: ConfState,
}
impl CommittedConfiguration {
    pub fn cut(&self) -> AppliedPosition {
        self.cut
    }
    /// None names the initial configuration, not a fabricated Raft term/index.
    pub fn applied_at(&self) -> Option<AppliedPosition> {
        self.applied_at
    }
    pub fn state(&self) -> &ConfState {
        &self.state
    }

    /// Copy all known configuration sets into the bounded anchor representation.
    /// Reject unknown protobuf fields rather than silently losing future state.
    pub fn anchor_state(&self) -> Result<kv9_common::anchor::AnchorConfiguration> {
        if self.state.get_unknown_fields().iter().next().is_some() {
            return Err(Error::Raft(
                "unknown configuration fields cannot enter an anchor".into(),
            ));
        }
        let sorted = |nodes: &[u64]| -> Result<Vec<u64>> {
            if nodes.len() > kv9_common::anchor::MAX_ANCHOR_MEMBERS {
                return Err(Error::Raft(
                    "anchor configuration exceeds member bound".into(),
                ));
            }
            let mut nodes = nodes.to_vec();
            nodes.sort_unstable(); // Preserve duplicates for the anchor validator to reject.
            Ok(nodes)
        };
        Ok(kv9_common::anchor::AnchorConfiguration {
            voters: sorted(self.state.get_voters())?,
            voters_outgoing: sorted(self.state.get_voters_outgoing())?,
            learners: sorted(self.state.get_learners())?,
            learners_next: sorted(self.state.get_learners_next())?,
            auto_leave: self.state.get_auto_leave(),
        })
    }
}

impl<F: FileSystem> DiskRaftStorage<F> {
    /// Obtain the configuration actually effective at an exact committed cut.
    /// Does not substitute current/latest membership for an older cut.
    ///
    /// Recovery-only: the retained log after the selected configuration is
    /// scanned for a committed configuration missing from applied history.
    /// This holds the writer lock and may be O(retained entries); callers must
    /// not place it on the serving or periodic-checkpoint hot path. S05 needs a
    /// certified snapshot base/index before online bounded capture or truncation.
    pub fn configuration_at_committed(&self, cut: AppliedPosition) -> Result<ConfigurationLookup> {
        let writer = self.file.lock().expect("raft log file poisoned");
        if writer.is_none() {
            return Err(Error::Raft(
                "configuration lookup requires recovered durable storage".into(),
            ));
        }
        if cut.index == 0 || cut.term == 0 || cut.index == u64::MAX {
            return Err(Error::Raft("invalid configuration recovery cut".into()));
        }
        if self.committed_term(cut.index)? != cut.term {
            return Err(Error::Raft(
                "configuration cut term disagrees with committed history".into(),
            ));
        }
        if self.mem.first_index().map_err(raft_error)? != 1 {
            return Ok(ConfigurationLookup::Unavailable(
                ConfigurationUnavailable::ProtocolHistoryCompacted,
            ));
        }
        let history = self.conf_history.lock().expect("conf history poisoned");
        if history.ambiguous {
            return Ok(ConfigurationLookup::Unavailable(
                ConfigurationUnavailable::UnindexedOrConflictingHistory,
            ));
        }
        let Some(initial) = &history.initial else {
            return Ok(ConfigurationLookup::Unavailable(
                ConfigurationUnavailable::InitialConfigurationMissing,
            ));
        };
        let (index, state) = history
            .applied
            .range(..=cut.index)
            .next_back()
            .map(|(index, state)| (*index, state))
            .unwrap_or((0, initial));
        let applied_at = if index == 0 {
            None
        } else {
            let entry = self.configuration_entry(index)?;
            if !is_configuration(entry.get_entry_type()) {
                return Err(Error::Raft(
                    "indexed configuration does not name a configuration entry".into(),
                ));
            }
            if entry.term == 0 || entry.term > cut.term {
                return Err(Error::Raft(
                    "configuration entry term contradicts the recovery cut".into(),
                ));
            }
            Some(AppliedPosition {
                term: entry.term,
                index,
            })
        };
        // Latest *applied* configuration is insufficient when a newer config is
        // committed but has not finished ordered apply. Never certify the old one.
        for next in index + 1..=cut.index {
            if is_configuration(self.configuration_entry(next)?.get_entry_type()) {
                return Ok(ConfigurationLookup::Unavailable(
                    ConfigurationUnavailable::ConfigurationNotApplied { index: next },
                ));
            }
        }
        Ok(ConfigurationLookup::Found(CommittedConfiguration {
            cut,
            applied_at,
            state: state.clone(),
        }))
    }

    fn configuration_entry(&self, index: u64) -> Result<raft::prelude::Entry> {
        let mut entries = self
            .mem
            .entries(index, index + 1, Some(1), GetEntriesContext::empty(false))
            .map_err(raft_error)?;
        if entries.len() != 1 || entries[0].index != index {
            return Err(Error::Raft(
                "configuration lookup has incomplete protocol history".into(),
            ));
        }
        Ok(entries.remove(0))
    }
}
fn is_configuration(kind: raft::eraftpb::EntryType) -> bool {
    matches!(
        kind,
        raft::eraftpb::EntryType::EntryConfChange | raft::eraftpb::EntryType::EntryConfChangeV2
    )
}
fn raft_error(error: raft::Error) -> Error {
    Error::Raft(format!("configuration recovery: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rawnode::PersistentRaftStorage;
    use kv9_common::fs::testing::{Crash, Fault, ModelFs, Operation};
    use raft::prelude::{Entry, EntryType, HardState};
    use std::path::Path;

    const DIR: &str = "/configuration-cut/raft";
    fn cut(index: u64) -> AppliedPosition {
        AppliedPosition {
            term: if index < 4 { 4 } else { 5 },
            index,
        }
    }
    fn joint() -> ConfState {
        let mut cs = ConfState::from((vec![1, 2, 4], vec![5]));
        cs.set_voters_outgoing(vec![1, 2, 3]);
        cs.set_learners_next(vec![3]);
        cs.set_auto_leave(true);
        cs
    }
    fn stable() -> ConfState {
        ConfState::from((vec![1, 2, 4], vec![3, 5]))
    }
    fn opened(fs: &ModelFs) -> DiskRaftStorage<ModelFs> {
        DiskRaftStorage::open_on(fs.clone(), Path::new(DIR), &[1, 2, 3])
            .unwrap()
            .0
    }
    fn prepared(fs: &ModelFs) -> DiskRaftStorage<ModelFs> {
        let store = opened(fs);
        let entries: Vec<_> = (1..=6)
            .map(|i| Entry {
                index: i,
                term: cut(i).term,
                entry_type: match i {
                    2 => EntryType::EntryConfChange,
                    4 => EntryType::EntryConfChangeV2,
                    _ => EntryType::EntryNormal,
                },
                ..Default::default()
            })
            .collect();
        store
            .persist_ready(
                &entries,
                Some(&HardState {
                    term: 5,
                    vote: 2,
                    commit: 6,
                    ..Default::default()
                }),
            )
            .unwrap();
        store
    }
    fn found(store: &DiskRaftStorage<ModelFs>, index: u64) -> CommittedConfiguration {
        match store.configuration_at_committed(cut(index)).unwrap() {
            ConfigurationLookup::Found(value) => value,
            other => panic!("expected configuration, got {other:?}"),
        }
    }
    #[test]
    fn anchor_projection_preserves_joint_state_and_refuses_unknown_fields() {
        let mut state = joint();
        state.set_voters(vec![4, 1, 2]);
        let mut checked = CommittedConfiguration {
            cut: cut(5),
            applied_at: Some(cut(4)),
            state,
        };
        let projected = checked.anchor_state().unwrap();
        assert_eq!(projected.voters, vec![1, 2, 4]);
        assert_eq!(projected.voters_outgoing, vec![1, 2, 3]);
        assert_eq!(projected.learners, vec![5]);
        assert_eq!(projected.learners_next, vec![3]);
        assert!(projected.auto_leave);
        checked.state.mut_unknown_fields().add_varint(99, 1);
        assert!(checked.anchor_state().is_err());
    }

    #[test]
    fn historical_joint_and_initial_configuration_survive_newer_apply_and_restart() {
        let fs = ModelFs::default();
        let store = prepared(&fs);
        store.set_conf_state(&joint(), 2).unwrap();
        store.set_conf_state(&stable(), 4).unwrap();
        for recovered in [false, true] {
            let reopened;
            let current = if recovered {
                reopened = opened(&fs);
                &reopened
            } else {
                &store
            };
            let initial = found(current, 1);
            assert_eq!(initial.applied_at(), None);
            assert_eq!(initial.state(), &ConfState::from((vec![1, 2, 3], vec![])));
            let old = found(current, 3);
            assert_eq!(old.cut(), cut(3));
            assert_eq!(old.applied_at(), Some(cut(2)));
            assert_eq!(old.state(), &joint());
            let latest = found(current, 6);
            assert_eq!(latest.applied_at(), Some(cut(4)));
            assert_eq!(latest.state(), &stable());
        }
    }
    #[test]
    fn committed_but_unapplied_configuration_cannot_certify_the_previous_one() {
        let fs = ModelFs::default();
        let store = prepared(&fs);
        assert_eq!(
            store.configuration_at_committed(cut(3)).unwrap(),
            ConfigurationLookup::Unavailable(ConfigurationUnavailable::ConfigurationNotApplied {
                index: 2
            })
        );
        store.set_conf_state(&joint(), 2).unwrap();
        let old = found(&store, 3);
        assert_eq!(
            store.configuration_at_committed(cut(6)).unwrap(),
            ConfigurationLookup::Unavailable(ConfigurationUnavailable::ConfigurationNotApplied {
                index: 4
            })
        );
        store.set_conf_state(&stable(), 4).unwrap();
        assert_eq!(found(&store, 6).state(), &stable());
        assert_eq!(old.state(), &joint());
    }
    #[test]
    fn future_same_term_membership_cannot_replace_an_older_cut() {
        let fs = ModelFs::default();
        let store = opened(&fs);
        let entries: Vec<_> = (1..=4)
            .map(|index| Entry {
                index,
                term: 7,
                entry_type: if index == 2 || index == 3 {
                    EntryType::EntryConfChangeV2
                } else {
                    EntryType::EntryNormal
                },
                ..Default::default()
            })
            .collect();
        store
            .persist_ready(
                &entries,
                Some(&HardState {
                    term: 7,
                    vote: 2,
                    commit: 4,
                    ..Default::default()
                }),
            )
            .unwrap();
        store.set_conf_state(&joint(), 2).unwrap();
        store.set_conf_state(&stable(), 3).unwrap();
        let cut = AppliedPosition { term: 7, index: 2 };
        let ConfigurationLookup::Found(view) = store.configuration_at_committed(cut).unwrap()
        else {
            panic!("historical cut unavailable")
        };
        assert_eq!(view.cut(), cut);
        assert_eq!(view.applied_at(), Some(cut));
        assert_eq!(view.state(), &joint());
    }
    #[test]
    fn wrong_cut_or_nonconfiguration_record_cannot_mint_a_view() {
        let fs = ModelFs::default();
        let store = prepared(&fs);
        store.set_conf_state(&joint(), 2).unwrap();
        for at in [
            AppliedPosition { term: 6, index: 3 },
            cut(7),
            AppliedPosition { term: 0, index: 0 },
            cut(u64::MAX),
        ] {
            assert!(store.configuration_at_committed(at).is_err());
        }
        store.set_conf_state(&joint(), 3).unwrap();
        assert!(store.configuration_at_committed(cut(3)).is_err());
        store.append(&[]).unwrap(); // Invalid query input does not poison a healthy writer.
    }
    #[test]
    fn missing_initial_or_compacted_history_requires_independent_snapshot_authority() {
        let fs = ModelFs::default();
        let store = prepared(&fs);
        store.conf_history.lock().unwrap().initial = None;
        assert_eq!(
            store.configuration_at_committed(cut(1)).unwrap(),
            ConfigurationLookup::Unavailable(ConfigurationUnavailable::InitialConfigurationMissing)
        );
        let fs = ModelFs::default();
        let store = prepared(&fs);
        store.set_conf_state(&joint(), 2).unwrap();
        store.set_conf_state(&stable(), 4).unwrap();
        store.mem.wl().compact(3).unwrap();
        assert_eq!(
            store.configuration_at_committed(cut(6)).unwrap(),
            ConfigurationLookup::Unavailable(ConfigurationUnavailable::ProtocolHistoryCompacted)
        );
    }
    #[test]
    fn unindexed_or_conflicting_history_never_becomes_an_initial_guess() {
        use protobuf::Message;
        let fs = ModelFs::default();
        let store = prepared(&fs);
        store.set_conf_state(&joint(), 2).unwrap();
        store.set_conf_state(&stable(), 2).unwrap();
        assert_eq!(
            store.configuration_at_committed(cut(3)).unwrap(),
            ConfigurationLookup::Unavailable(
                ConfigurationUnavailable::UnindexedOrConflictingHistory
            )
        );
        drop(store);
        assert!(matches!(
            opened(&fs).configuration_at_committed(cut(3)).unwrap(),
            ConfigurationLookup::Unavailable(
                ConfigurationUnavailable::UnindexedOrConflictingHistory
            )
        ));
        let fs = ModelFs::default();
        let store = prepared(&fs);
        store
            .with_writer(|file| {
                DiskRaftStorage::<ModelFs>::write_record(
                    &store.io_metrics,
                    file,
                    super::super::REC_CONF_STATE,
                    &joint().write_to_bytes().unwrap(),
                )
            })
            .unwrap();
        drop(store);
        assert_eq!(
            opened(&fs).configuration_at_committed(cut(1)).unwrap(),
            ConfigurationLookup::Unavailable(
                ConfigurationUnavailable::UnindexedOrConflictingHistory
            )
        );
    }
    #[test]
    fn failed_configuration_persistence_never_exposes_live_query_authority() {
        let fs = ModelFs::default();
        let store = prepared(&fs);
        store.set_conf_state(&joint(), 2).unwrap();
        fs.clear_events();
        store.set_conf_state(&stable(), 4).unwrap();
        let cuts = fs.events();
        let mut cases = 0;
        for event in cuts {
            for errno in [5, 28] {
                let mut faults = vec![Fault::Before(errno), Fault::After(errno)];
                if event.operation == Operation::Write {
                    faults.push(Fault::ShortWrite { bytes: 4, errno });
                }
                for fault in faults {
                    for crash in [Crash::LoseUnsynced, Crash::KeepUnsynced] {
                        let fs = ModelFs::default();
                        let store = prepared(&fs);
                        store.set_conf_state(&joint(), 2).unwrap();
                        fs.clear_events();
                        fs.fail_at(event.number, fault);
                        assert!(store.set_conf_state(&stable(), 4).is_err());
                        assert!(fs.fault_arrived());
                        assert!(
                            store.configuration_at_committed(cut(3)).is_err(),
                            "failed writer granted a view"
                        );
                        drop(store);
                        fs.crash(crash);
                        let store = opened(&fs);
                        assert_eq!(found(&store, 3).state(), &joint());
                        match store.configuration_at_committed(cut(6)).unwrap() {
                            ConfigurationLookup::Found(view) => assert_eq!(view.state(), &stable()),
                            ConfigurationLookup::Unavailable(
                                ConfigurationUnavailable::ConfigurationNotApplied { index: 4 },
                            ) => {}
                            other => panic!("unexpected recovered authority: {other:?}"),
                        }
                        cases += 1;
                    }
                }
            }
        }
        assert_eq!(cases, 20);
    }
}
