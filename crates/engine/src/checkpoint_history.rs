//! Synchronize local checkpoint selection with protocol-history compaction.
//!
//! This is a restriction, never compaction or checkpoint publication authority.
//! The selected cut is recovered from the actual engine reference. A driver
//! binds this same object and observes its actual recovered Raft base before
//! starting. Lock order is peer -> history -> engine WAL -> Raft storage;
//! ordinary apply never acquires history or peer while holding the engine WAL.
use std::sync::{Arc, Mutex, MutexGuard};

use crate::checkpoint::{FlushJournal, PendingFlush, PlannedFlush};
use kv9_common::RootDigest;

use kv9_common::{AppliedPosition, Error, Result};

#[derive(Debug)]
pub struct CheckpointHistory {
    state: Mutex<State>,
}

#[derive(Debug)]
pub(crate) struct State {
    selected: Option<AppliedPosition>,
    compacted: Option<AppliedPosition>,
    uncertain: bool,
    candidate: Option<CandidateState>,
    next_candidate: u64,
}

impl CheckpointHistory {
    pub(crate) fn new(selected: Option<AppliedPosition>) -> Self {
        Self {
            state: Mutex::new(State {
                selected,
                compacted: None,
                uncertain: false,
                candidate: None,
                next_candidate: 0,
            }),
        }
    }

    /// The driver supplies its actual recovered compacted base. This only
    /// restricts future selection; it cannot authorize either kind of deletion.
    pub fn observe_compacted_base(&self, base: AppliedPosition) -> Result<()> {
        self.with_compaction(base, || Ok(()))
    }

    /// Keep the selected checkpoint fixed until the real compaction completes.
    /// Compaction through its cut is allowed: the exact cut/configuration stay
    /// in the Raft base, and its later ManifestChange stays in retained history.
    /// On ambiguous failure neither a later compaction nor a new selection is
    /// allowed until the actual files are recovered.
    pub fn with_compaction<T>(
        &self,
        floor: AppliedPosition,
        compact: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let mut state = self.state.lock().expect("checkpoint history poisoned");
        state.check()?;
        if floor.term == 0 || floor.index == 0 {
            return Err(invalid("invalid compaction floor"));
        }
        if state
            .selected
            .into_iter()
            .chain(state.candidate.as_ref().map(|c| c.cut))
            .any(|cut| {
                floor.index > cut.index
                    || floor.term > cut.term
                    || (floor.index == cut.index && floor.term != cut.term)
            })
        {
            return Err(invalid(
                "compaction would erase selected or pending checkpoint evidence",
            ));
        }
        let result = compact();
        if result.is_ok() {
            if state.compacted.is_none_or(|old| floor.index > old.index) {
                state.compacted = Some(floor);
            }
        } else {
            state.uncertain = true;
        }
        result
    }

    /// Scheduling ceiling only. A caller still needs the original committed
    /// compaction authority and the peer rechecks under its actual storage lock.
    pub fn compaction_ceiling(&self) -> Result<Option<AppliedPosition>> {
        let state = self.state.lock().expect("checkpoint history poisoned");
        state.check()?;
        Ok(state
            .selected
            .into_iter()
            .chain(state.candidate.as_ref().map(|c| c.cut))
            .min_by_key(|p| p.index))
    }

    pub(crate) fn selecting(&self, cut: AppliedPosition) -> Result<MutexGuard<'_, State>> {
        let state = self.state.lock().expect("checkpoint history poisoned");
        state.check()?;
        state.check_cut(cut)?;
        Ok(state)
    }
    /// Keep selection and compaction fixed while the engine checks its live
    /// WAL. The callback must follow history -> WAL, never acquire a peer lock.
    pub(crate) fn observe_selected<T>(
        &self,
        observe: impl FnOnce(Option<AppliedPosition>) -> Result<T>,
    ) -> Result<T> {
        let state = self.state.lock().expect("checkpoint history poisoned");
        state.check()?;
        observe(state.selected)
    }
    pub(crate) fn reserve<T>(
        self: &Arc<Self>,
        freeze: impl FnOnce() -> Result<(T, AppliedPosition)>,
    ) -> Result<(T, CheckpointCandidate)> {
        let mut state = self.state.lock().expect("checkpoint history poisoned");
        state.check()?;
        if state.candidate.is_some() {
            return Err(invalid("checkpoint candidate already reserved"));
        }
        // The history lock spans the actual engine snapshot: compaction cannot
        // win between reading its position and installing the restriction.
        let (frozen, cut) = freeze()?;
        state.check_cut(cut)?;
        let serial = state.serial()?;
        state.candidate = Some(CandidateState {
            cut,
            operation: None,
            serial,
            claimed: true,
        });
        let lease = CheckpointCandidate {
            history: self.clone(),
            cut,
            serial,
            durable: false,
        };
        Ok((frozen, lease))
    }
    pub(crate) fn restore(&self, pending: &PendingFlush) -> Result<()> {
        let mut state = self.state.lock().expect("checkpoint history poisoned");
        state.check()?;
        let cut = pending.manifest().position();
        let operation = operation(pending.manifest(), pending.expected_generation())?;
        state.check_cut(cut)?;
        if let Some(current) = &state.candidate {
            if current.cut != cut || current.operation != Some(operation) || current.claimed {
                return Err(invalid(
                    "pending checkpoint reservation differs or is owned",
                ));
            }
            return Ok(());
        }
        let serial = state.serial()?;
        state.candidate = Some(CandidateState {
            cut,
            operation: Some(operation),
            serial,
            claimed: false,
        });
        Ok(())
    }
    pub(crate) fn claim(self: &Arc<Self>, pending: &PendingFlush) -> Result<CheckpointCandidate> {
        self.restore(pending)?;
        let mut state = self.state.lock().expect("checkpoint history poisoned");
        state.check()?;
        let serial = state.serial()?;
        let c = state
            .candidate
            .as_mut()
            .ok_or_else(|| invalid("candidate disappeared"))?;
        if c.claimed
            || c.cut != pending.manifest().position()
            || c.operation
                != Some(operation(
                    pending.manifest(),
                    pending.expected_generation(),
                )?)
        {
            return Err(invalid("candidate already owned or replaced"));
        }
        c.claimed = true;
        c.serial = serial;
        Ok(CheckpointCandidate {
            history: self.clone(),
            cut: c.cut,
            serial,
            durable: true,
        })
    }
}

impl State {
    fn serial(&mut self) -> Result<u64> {
        self.next_candidate = self
            .next_candidate
            .checked_add(1)
            .ok_or_else(|| invalid("candidate sequence exhausted"))?;
        Ok(self.next_candidate)
    }
    fn check_cut(&self, cut: AppliedPosition) -> Result<()> {
        if cut.term == 0 || cut.index == 0 {
            return Err(invalid("invalid checkpoint cut"));
        }
        if self.compacted.is_some_and(|floor| {
            cut.index < floor.index
                || cut.term < floor.term
                || (cut.index == floor.index && cut.term != floor.term)
        }) {
            return Err(invalid(
                "checkpoint protocol evidence was already compacted",
            ));
        }
        Ok(())
    }

    fn check(&self) -> Result<()> {
        if self.uncertain {
            return Err(invalid("ambiguous checkpoint/compaction requires recovery"));
        }
        Ok(())
    }

    pub(crate) fn selected(&mut self, cut: AppliedPosition, result: &Result<bool>) {
        match result {
            Ok(true) => self.selected = Some(cut),
            Ok(false) => {}
            Err(_) => self.uncertain = true,
        }
    }
}

fn invalid(message: &str) -> Error {
    Error::Engine(format!("checkpoint history: {message}"))
}

#[derive(Debug)]
struct CandidateState {
    cut: AppliedPosition,
    operation: Option<RootDigest>,
    serial: u64,
    claimed: bool,
}

/// Exclusive local scheduling reservation. It grants no publication, object
/// ownership or compaction authority. Only one candidate exists per engine.
/// Before staging, drop cancels the restriction. Once staging starts, drop
/// relinquishes execution ownership but preserves the cut until durable journal
/// settlement or file recovery. No engine, driver, FD or frozen image is held.
#[derive(Debug)]
pub struct CheckpointCandidate {
    history: Arc<CheckpointHistory>,
    cut: AppliedPosition,
    serial: u64,
    durable: bool,
}
impl CheckpointCandidate {
    pub fn position(&self) -> AppliedPosition {
        self.cut
    }
    pub fn is_durable(&self) -> bool {
        self.durable
    }
    pub fn stage(
        &mut self,
        journal: &mut FlushJournal,
        plan: &PlannedFlush,
        generation: u64,
    ) -> Result<()> {
        let operation = operation(plan.manifest(), generation)?;
        self.stage_with(journal, plan.manifest().position(), operation, |journal| {
            journal.stage_planned(plan, generation)
        })
    }
    pub fn stage_spooled(
        &mut self,
        journal: &mut FlushJournal,
        plan: &mut crate::checkpoint::SpooledFlush,
        generation: u64,
    ) -> Result<()> {
        let operation = operation(plan.manifest(), generation)?;
        self.stage_with(journal, plan.manifest().position(), operation, |journal| {
            journal.stage_spooled(plan, generation)
        })
    }
    fn stage_with(
        &mut self,
        journal: &mut FlushJournal,
        position: AppliedPosition,
        operation: RootDigest,
        stage: impl FnOnce(&mut FlushJournal) -> Result<()>,
    ) -> Result<()> {
        if self.durable || position != self.cut {
            return Err(invalid("candidate stage differs"));
        }
        {
            let mut state = self
                .history
                .state
                .lock()
                .expect("checkpoint history poisoned");
            state.check()?;
            let c = self.current(&mut state)?;
            c.operation = Some(operation);
            self.durable = true; // Before any potentially ambiguous publication.
        }
        match stage(journal) {
            Ok(()) => Ok(()),
            Err(error) => {
                // This capability has never permitted remote I/O or submission.
                // If staging left no selected file, fsync that absence before
                // cancelling. A present/uncertain file retains its reservation.
                if matches!(journal.load(), Ok(None)) && journal.clear_settled().is_ok() {
                    let mut state = self
                        .history
                        .state
                        .lock()
                        .expect("checkpoint history poisoned");
                    self.current(&mut state)?;
                    state.candidate = None;
                    self.durable = false;
                }
                Err(error)
            }
        }
    }
    /// Call after typed manifest settlement, and after positive adoption/handoff
    /// if applicable. Durable local clearing precedes lifting the restriction.
    /// On failure keep this capability and retry; neither timeout nor drop clears.
    pub fn finish(&mut self, journal: &mut FlushJournal) -> Result<()> {
        let mut state = self
            .history
            .state
            .lock()
            .expect("checkpoint history poisoned");
        state.check()?;
        let c = self.current(&mut state)?;
        if !self.durable {
            return Err(invalid("candidate has no durable plan"));
        }
        if let Some(pending) = journal.load()? {
            if pending.manifest().position() != self.cut
                || Some(operation(
                    pending.manifest(),
                    pending.expected_generation(),
                )?) != c.operation
            {
                return Err(invalid("settled journal differs from reserved candidate"));
            }
        }
        journal.clear_settled()?;
        state.candidate = None;
        self.durable = false;
        Ok(())
    }
    fn current<'a>(&self, state: &'a mut State) -> Result<&'a mut CandidateState> {
        state
            .candidate
            .as_mut()
            .filter(|c| c.serial == self.serial && c.claimed && c.cut == self.cut)
            .ok_or_else(|| invalid("candidate ownership differs"))
    }
}
impl Drop for CheckpointCandidate {
    fn drop(&mut self) {
        let mut state = self
            .history
            .state
            .lock()
            .expect("checkpoint history poisoned");
        if let Some(c) = state.candidate.as_mut().filter(|c| c.serial == self.serial) {
            if self.durable {
                c.claimed = false;
            } else {
                state.candidate = None;
            }
        }
    }
}
fn operation(
    manifest: &crate::checkpoint::CheckpointManifest,
    generation: u64,
) -> Result<RootDigest> {
    Ok(RootDigest::from_bytes(
        manifest
            .change_id(generation)?
            .try_into()
            .map_err(|_| invalid("invalid candidate identity"))?,
    ))
}
