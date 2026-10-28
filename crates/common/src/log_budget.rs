//! Node-local log debt and pre-append admission. Persistence of accepted work
//! never fails merely because the threshold is reached. Sources retain their
//! charges after an owner closes; only verified reconciliation releases debt.
use crate::fs::DurableFile;
use crate::{Error, RegionId, Result};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const DEFAULT_LOG_BYTES: u64 = 4 * 1024 * 1024 * 1024;

pub mod headroom;
pub mod rewrite;
use headroom::{DiskHeadroom, DiskSpaceProbe, HeadroomSnapshot};
use rewrite::{RewriteSnapshot, DEFAULT_REWRITE_BYTES};

#[derive(Clone, Debug, serde::Serialize)]
pub struct Snapshot {
    pub limit_bytes: u64,
    pub file_bytes: u64,
    pub reserved_bytes: u64,
    pub peak_accounted_bytes: u64,
    pub refused_proposals: u64,
    pub sources: usize,
    pub pending_positions: usize,
    pub uncertain_sources: usize,
    pub disk_headroom: Option<HeadroomSnapshot>,
    pub rewrite_workspace: RewriteSnapshot,
}

#[derive(Debug)]
pub struct LogBudget {
    state: Mutex<State>,
    disk: Option<DiskHeadroom>,
}
#[derive(Debug, Default)]
struct Source {
    bytes: u64,
    uncertain: bool,
    pending: BTreeMap<(u64, u64), u64>, // (index, term), retained across owner drop
}
#[derive(Debug)]
struct State {
    limit: u64,
    files: u64,
    reserved: u64,
    peak: u64,
    refused: u64,
    uncertain: usize,
    sources: BTreeMap<PathBuf, Source>,
    write_started: u128,
    writing: u64,
    disk_observation: headroom::Observation,
    rewrite: RewriteSnapshot,
}
impl State {
    fn account_peak(&mut self) {
        self.peak = self.peak.max(
            self.files
                .saturating_add(self.reserved)
                .saturating_add(self.rewrite.reserved_bytes),
        );
    }
    fn has_capacity(&self, bytes: u64) -> bool {
        self.uncertain == 0
            && self
                .files
                .checked_add(self.reserved)
                .and_then(|used| used.checked_add(self.rewrite.reserved_bytes))
                .and_then(|used| used.checked_add(bytes))
                .is_some_and(|used| used <= self.limit)
    }
}

/// One exclusively owned log layout (Raft directory or engine WAL family).
/// Clones share the same accounting; they confer no writer/deletion authority.
#[derive(Clone, Debug)]
pub struct LogDebt {
    budget: Arc<LogBudget>,
    path: PathBuf,
}

/// Provisional capacity. Transfer to a named log position after append; only
/// an unsubmitted claim is released by Drop. Caller timeouts own no claim.
pub struct ProposalClaim {
    budget: Arc<LogBudget>,
    bytes: u64,
}

impl LogBudget {
    pub fn new(limit: u64) -> Result<Self> {
        if limit == 0 {
            return Err(Error::Config(
                "WAL/Raft byte threshold must be positive".into(),
            ));
        }
        Ok(Self {
            state: Mutex::new(State {
                limit,
                files: 0,
                reserved: 0,
                peak: 0,
                refused: 0,
                uncertain: 0,
                sources: BTreeMap::new(),
                write_started: 0,
                writing: 0,
                disk_observation: Default::default(),
                rewrite: RewriteSnapshot::new(DEFAULT_REWRITE_BYTES),
            }),
            disk: None,
        })
    }
    /// Production binds one retained filesystem handle for every node log.
    /// Libraries without a disk owner may explicitly retain byte-only admission.
    pub fn with_disk_headroom(
        limit: u64,
        minimum_free_bytes: u64,
        probe: Arc<dyn DiskSpaceProbe>,
    ) -> Result<Self> {
        let mut budget = Self::new(limit)?;
        budget.disk = Some(DiskHeadroom::new(minimum_free_bytes, probe)?);
        Ok(budget)
    }
    /// Cold inventory/attachment only; a source must use the monitored device.
    pub fn check_path(&self, path: &Path) -> Result<()> {
        if let Some(disk) = &self.disk {
            disk.probe.check_path(path).map_err(io)?;
        }
        Ok(())
    }
    pub fn tree_bytes(&self, path: &Path) -> Result<u64> {
        tree_bytes_checked(path, &|path| self.check_path(path))
    }
    pub fn snapshot(&self) -> Snapshot {
        let s = self.state.lock().expect("log budget");
        Snapshot {
            limit_bytes: s.limit,
            file_bytes: s.files,
            reserved_bytes: s.reserved,
            peak_accounted_bytes: s.peak,
            refused_proposals: s.refused,
            sources: s.sources.len(),
            pending_positions: s.sources.values().map(|v| v.pending.len()).sum(),
            uncertain_sources: s.sources.values().filter(|v| v.uncertain).count(),
            disk_headroom: self.disk.as_ref().map(|disk| {
                s.disk_observation
                    .snapshot(disk.minimum_free_bytes, s.writing)
            }),
            rewrite_workspace: s.rewrite.clone(),
        }
    }
    /// Cold retirement path; caller already drained the owning group.
    pub fn sources_under(self: &Arc<Self>, directory: &Path) -> Result<Vec<LogDebt>> {
        let prefix = std::path::absolute(directory).map_err(io)?;
        Ok(self
            .state
            .lock()
            .expect("log budget")
            .sources
            .keys()
            .filter(|p| p.starts_with(&prefix))
            .map(|p| LogDebt {
                budget: self.clone(),
                path: p.clone(),
            })
            .collect())
    }
    /// Observe a layout while its exclusive writer is stopped/locked. Include
    /// orphan and temporary files, not merely the selected live-log payload.
    pub fn track(self: &Arc<Self>, path: &Path, bytes: u64) -> Result<LogDebt> {
        let path = std::path::absolute(path).map_err(io)?;
        self.check_path(&path)?;
        let debt = LogDebt {
            budget: self.clone(),
            path,
        };
        debt.reconcile(bytes)?;
        Ok(debt)
    }
    pub fn reserve(self: &Arc<Self>, bytes: u64, region: RegionId) -> Result<ProposalClaim> {
        // No budget lock spans filesystem I/O. The sample carries a write
        // frontier so concurrent writes cannot disappear from its allowance.
        let sample = if let Some(disk) = &self.disk {
            let (started, writing) = {
                let mut s = self.state.lock().expect("log budget");
                if !s.has_capacity(bytes) {
                    s.refused = s.refused.saturating_add(1);
                    return Err(Error::WriteBackpressure { region });
                }
                (s.write_started, s.writing)
            };
            Some((started, writing, disk.sample()))
        } else {
            None
        };
        let mut s = self.state.lock().expect("log budget");
        let disk_ok = if let Some((started, writing, sample)) = sample {
            let overlap = (s.write_started - started) + u128::from(writing);
            let reserved = u128::from(s.reserved) + u128::from(s.rewrite.reserved_bytes);
            s.disk_observation.admit(
                self.disk.as_ref().unwrap().minimum_free_bytes,
                sample,
                overlap,
                reserved,
                bytes,
            )
        } else {
            true
        };
        if !disk_ok || !s.has_capacity(bytes) {
            s.refused = s.refused.saturating_add(1);
            return Err(Error::WriteBackpressure { region });
        }
        s.reserved = s.reserved.checked_add(bytes).ok_or_else(overflow)?;
        s.account_peak();
        Ok(ProposalClaim {
            budget: self.clone(),
            bytes,
        })
    }
    /// Fresh pressure evidence for a remote leader. No capacity is reserved,
    /// so this is not a grant; None means accounting/probe uncertainty.
    pub fn observe_data_capacity(&self) -> Option<u64> {
        let (started, writing) = {
            let s = self.state.lock().expect("log budget");
            if s.uncertain != 0 {
                return None;
            }
            if !s.has_capacity(1) {
                return Some(0);
            }
            (s.write_started, s.writing)
        };
        let sample = self.disk.as_ref().map(|disk| disk.sample());
        let mut s = self.state.lock().expect("log budget");
        if s.uncertain != 0 {
            return None;
        }
        let mut capacity = s.limit.saturating_sub(
            s.files
                .saturating_add(s.reserved)
                .saturating_add(s.rewrite.reserved_bytes),
        );
        if let (Some(disk), Some(sample)) = (&self.disk, sample) {
            let overlap = s.write_started - started + u128::from(writing);
            let available = s.disk_observation.observe(sample, overlap)?;
            let needed = (u128::from(disk.minimum_free_bytes)
                + u128::from(s.reserved)
                + u128::from(s.rewrite.reserved_bytes))
            .checked_add(overlap)?;
            capacity = capacity.min(u128::from(available).saturating_sub(needed) as u64);
        }
        Some(capacity)
    }
}

impl LogDebt {
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Charge BEFORE an append or bounded layout rewrite. This is forced debt
    /// for committed/follower/control work, never a quota refusal in apply.
    pub fn grow(&self, bytes: u64) -> Result<()> {
        let mut s = self.budget.state.lock().expect("log budget");
        let total = s.files.checked_add(bytes).ok_or_else(overflow)?;
        let source = s.sources.entry(self.path.clone()).or_default();
        source.bytes = source.bytes.checked_add(bytes).ok_or_else(overflow)?;
        s.files = total;
        s.account_peak();
        Ok(())
    }
    fn begin_write(&self, bytes: u64) -> Result<Writing<'_>> {
        let mut s = self.budget.state.lock().expect("log budget");
        let files = s.files.checked_add(bytes).ok_or_else(overflow)?;
        let writing = s.writing.checked_add(bytes).ok_or_else(overflow)?;
        let started = s
            .write_started
            .checked_add(u128::from(bytes))
            .ok_or_else(overflow)?;
        let source = s.sources.entry(self.path.clone()).or_default();
        source.bytes = source.bytes.checked_add(bytes).ok_or_else(overflow)?;
        s.files = files;
        s.writing = writing;
        s.write_started = started;
        s.account_peak();
        Ok(Writing { debt: self, bytes })
    }
    /// Caller must hold the layout's exclusive writer through measurement.
    /// Failed/unconfirmed unlink or rename must not be credited as free space.
    pub fn reconcile(&self, bytes: u64) -> Result<()> {
        let mut s = self.budget.state.lock().expect("log budget");
        let old = s.sources.get(&self.path).map_or(0, |v| v.bytes);
        let total = s
            .files
            .checked_sub(old)
            .and_then(|v| v.checked_add(bytes))
            .ok_or_else(overflow)?;
        let source = s.sources.entry(self.path.clone()).or_default();
        let was_uncertain = source.uncertain;
        source.bytes = bytes;
        source.uncertain = false;
        s.uncertain -= usize::from(was_uncertain);
        s.files = total;
        s.account_peak();
        Ok(())
    }
    pub fn bytes(&self) -> u64 {
        self.budget.state.lock().expect("log budget").sources[&self.path].bytes
    }
    fn unwritten(&self, bytes: u64) {
        let mut s = self.budget.state.lock().expect("log budget");
        s.sources
            .get_mut(&self.path)
            .expect("registered log source")
            .bytes -= bytes;
        s.files -= bytes;
    }
    /// Measure only after a layout mutation has closed displaced writers and
    /// completed its required directory sync. On error, keep all charged debt.
    pub fn reconciliation(
        &self,
        measure: impl FnOnce() -> Result<u64> + 'static,
    ) -> Reconciliation {
        Reconciliation {
            debt: self.clone(),
            measure: Some(Box::new(measure)),
            completed: false,
        }
    }
    /// An accounting I/O error cannot be interpreted as a zero-length file.
    pub fn mark_uncertain(&self) {
        let mut s = self.budget.state.lock().expect("log budget");
        let source = s.sources.entry(self.path.clone()).or_default();
        let was_uncertain = source.uncertain;
        source.uncertain = true;
        s.uncertain += usize::from(!was_uncertain);
    }
    pub fn reserve(&self, bytes: u64, region: RegionId) -> Result<ProposalClaim> {
        self.budget.reserve(bytes, region)
    }
    /// Recovery/replication may arrive above a newly lowered threshold. Keep
    /// future apply debt without refusing already accepted protocol work.
    pub fn retain_position(&self, index: u64, term: u64, bytes: u64) -> Result<()> {
        let mut s = self.budget.state.lock().expect("log budget");
        let old = s
            .sources
            .get(&self.path)
            .and_then(|v| v.pending.get(&(index, term)))
            .copied()
            .unwrap_or(0);
        if old >= bytes {
            return Ok(());
        }
        let next = s.reserved.checked_add(bytes - old).ok_or_else(overflow)?;
        s.sources
            .entry(self.path.clone())
            .or_default()
            .pending
            .insert((index, term), bytes);
        s.reserved = next;
        s.account_peak();
        Ok(())
    }
    /// After exclusive recovery, or a successfully persisted replacement
    /// suffix, replace obsolete future-apply obligations atomically. Bytes of
    /// overwritten physical records remain in `file_bytes` until real rewrite.
    pub fn replace_pending_suffix(
        &self,
        first: u64,
        replacement: BTreeMap<(u64, u64), u64>,
    ) -> Result<()> {
        if replacement.keys().any(|(index, _)| *index < first) {
            return Err(Error::Engine(
                "log obligation suffix starts before its boundary".into(),
            ));
        }
        let mut s = self.budget.state.lock().expect("log budget");
        let source = s.sources.get(&self.path).expect("registered log source");
        let old: u64 = source.pending.range((first, 0)..).map(|(_, n)| n).sum();
        let next = replacement
            .values()
            .try_fold(0u64, |n, v| n.checked_add(*v))
            .ok_or_else(overflow)?;
        let total = s
            .reserved
            .checked_sub(old)
            .and_then(|n| n.checked_add(next))
            .ok_or_else(overflow)?;
        let source = s.sources.get_mut(&self.path).unwrap();
        source.pending.split_off(&(first, 0));
        source.pending.extend(replacement);
        s.reserved = total;
        s.account_peak();
        Ok(())
    }
    /// Called only after the actual driver has consumed the committed prefix;
    /// proposal futures and timeout handlers must never release these charges.
    pub fn applied_through(&self, index: u64) {
        let mut s = self.budget.state.lock().expect("log budget");
        let source = s
            .sources
            .get_mut(&self.path)
            .expect("registered log source");
        let mut released = 0;
        while source
            .pending
            .first_key_value()
            .is_some_and(|((i, _), _)| *i <= index)
        {
            released += source.pending.pop_first().unwrap().1;
        }
        s.reserved -= released;
    }
    /// Only after authorized deletion and final owner close. A tombstone in
    /// the actual group catalog, not this accounting map, prevents resurrection.
    pub fn forget_empty(&self) -> Result<()> {
        let mut s = self.budget.state.lock().expect("log budget");
        let source = s.sources.get(&self.path).expect("registered log source");
        if source.bytes != 0 || !source.pending.is_empty() || source.uncertain {
            return Err(Error::Engine("cannot forget unresolved log debt".into()));
        }
        s.sources.remove(&self.path);
        Ok(())
    }
}

pub struct Reconciliation {
    debt: LogDebt,
    measure: Option<Box<dyn FnOnce() -> Result<u64>>>,
    completed: bool,
}
impl Reconciliation {
    pub fn completed(&mut self) {
        self.completed = true;
    }
}
impl Drop for Reconciliation {
    fn drop(&mut self) {
        if !self.completed {
            return;
        }
        match self.measure.take().unwrap()().and_then(|bytes| self.debt.reconcile(bytes)) {
            Ok(()) => {}
            Err(_) => self.debt.mark_uncertain(),
        }
    }
}

/// Append-only writer adapter. Charge before I/O, and retain the whole attempted
/// growth on ambiguous failure. A short successful write releases only bytes
/// explicitly reported unwritten. Existing-frame rewrites need reconciliation.
pub struct BudgetWriter<'a, F: DurableFile> {
    file: &'a mut F,
    debt: Option<&'a LogDebt>,
}
struct Writing<'a> {
    debt: &'a LogDebt,
    bytes: u64,
}
impl Drop for Writing<'_> {
    fn drop(&mut self) {
        self.debt.budget.state.lock().expect("log budget").writing -= self.bytes;
    }
}
impl<'a, F: DurableFile> BudgetWriter<'a, F> {
    pub fn new(file: &'a mut F, debt: Option<&'a LogDebt>) -> Self {
        Self { file, debt }
    }
}
impl<F: DurableFile> Write for BudgetWriter<'_, F> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let _writing = self
            .debt
            .map(|debt| debt.begin_write(bytes.len() as u64))
            .transpose()
            .map_err(std::io::Error::other)?;
        let result = self.file.write(bytes);
        if let (Some(debt), Ok(n)) = (self.debt, &result) {
            debt.unwritten((bytes.len() - n) as u64);
        }
        result
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
impl<F: DurableFile> Read for BudgetWriter<'_, F> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(bytes)
    }
}
impl<F: DurableFile> Seek for BudgetWriter<'_, F> {
    fn seek(&mut self, at: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(at)
    }
}
impl<F: DurableFile> DurableFile for BudgetWriter<'_, F> {
    fn sync_data(&self) -> std::io::Result<()> {
        self.file.sync_data()
    }
    fn set_len(&self, _len: u64) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "resize requires exclusive layout reconciliation",
        ))
    }
}

impl ProposalClaim {
    pub fn accept(mut self, debt: &LogDebt, index: u64, term: u64) {
        assert!(
            Arc::ptr_eq(&self.budget, &debt.budget),
            "foreign log budget claim"
        );
        let mut s = self.budget.state.lock().expect("log budget");
        let source = s.sources.entry(debt.path.clone()).or_default();
        // The peer's append lock assigns a fresh exact position. A preexisting
        // same-position obligation (e.g. recovered) is conservatively merged.
        let old = source.pending.get(&(index, term)).copied().unwrap_or(0);
        source.pending.insert((index, term), self.bytes.max(old));
        s.reserved -= old.min(self.bytes);
        self.bytes = 0;
    }
}
impl Drop for ProposalClaim {
    fn drop(&mut self) {
        self.budget.state.lock().expect("log budget").reserved -= self.bytes;
    }
}

fn io(e: std::io::Error) -> Error {
    Error::Engine(format!("log accounting: {e}"))
}
fn overflow() -> Error {
    Error::Engine("log accounting byte overflow".into())
}

/// Count an owned tree without following directory or file symlinks. A bad
/// layout is still debt, not recovery authority. Missing paths cost zero;
/// permission and other metadata errors are never swallowed.
pub fn tree_bytes(path: &Path) -> Result<u64> {
    tree_bytes_checked(path, &|_| Ok(()))
}
fn tree_bytes_checked(path: &Path, check: &impl Fn(&Path) -> Result<()>) -> Result<u64> {
    check(path)?;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(io(e)),
    };
    if metadata.file_type().is_symlink() {
        return Err(Error::Engine(
            "log accounting refuses a symlink layout".into(),
        ));
    }
    if !metadata.is_dir() {
        return Ok(metadata.len());
    }
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(path).map_err(io)? {
        bytes = bytes
            .checked_add(tree_bytes_checked(&entry.map_err(io)?.path(), check)?)
            .ok_or_else(overflow)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
