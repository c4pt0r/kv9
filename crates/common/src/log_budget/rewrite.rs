//! Shared future output space for actual Raft reclamation jobs. File debt is
//! separate and remains charged after an unsuccessful rewrite or owner close.
use super::*;

pub const DEFAULT_REWRITE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, serde::Serialize)]
pub struct RewriteSnapshot {
    pub limit_bytes: u64,
    pub reserved_bytes: u64,
    pub peak_bytes: u64,
    pub active: u64,
    pub admitted: u64,
    pub deferred: u64,
}
impl RewriteSnapshot {
    pub(super) fn new(limit_bytes: u64) -> Self {
        Self {
            limit_bytes,
            reserved_bytes: 0,
            peak_bytes: 0,
            active: 0,
            admitted: 0,
            deferred: 0,
        }
    }
    fn fits(&self, bytes: u64) -> bool {
        self.reserved_bytes
            .checked_add(bytes)
            .is_some_and(|n| n <= self.limit_bytes)
    }
}

/// Owned by the synchronous rewrite, never by a caller's waiting future.
/// Dropping it returns only future workspace, not any written file bytes.
pub struct RewriteClaim {
    budget: Arc<LogBudget>,
    bytes: u64,
}
impl LogBudget {
    /// Configure before sharing/attaching the node budget. Consuming Self
    /// prevents replacing a limit beneath a live claim.
    pub fn with_rewrite_limit(mut self, bytes: u64) -> Result<Self> {
        if bytes == 0 {
            return Err(Error::Config(
                "Raft rewrite workspace must be positive".into(),
            ));
        }
        self.state
            .get_mut()
            .expect("log budget")
            .rewrite
            .limit_bytes = bytes;
        Ok(self)
    }
    fn reserve_rewrite(self: &Arc<Self>, bytes: u64) -> Option<RewriteClaim> {
        let (started, writing) = {
            let mut s = self.state.lock().expect("log budget");
            if bytes == 0 || s.uncertain != 0 || !s.rewrite.fits(bytes) {
                s.rewrite.deferred = s.rewrite.deferred.saturating_add(1);
                return None;
            }
            (s.write_started, s.writing)
        };
        // Never hold the node budget lock across a filesystem syscall. The
        // caller still owns the actual layout's exclusive writer.
        let sample = self.disk.as_ref().map(|disk| disk.sample());
        let mut s = self.state.lock().expect("log budget");
        let disk_ok = if let Some(sample) = sample {
            let overlap = s.write_started - started + u128::from(writing);
            let available = s.disk_observation.observe(sample, overlap);
            let needed = u128::from(self.disk.as_ref().unwrap().minimum_free_bytes)
                + u128::from(s.reserved)
                + u128::from(s.rewrite.reserved_bytes)
                + u128::from(bytes);
            available.is_some_and(|free| {
                overlap
                    .checked_add(needed)
                    .is_some_and(|n| n <= u128::from(free))
            })
        } else {
            true
        };
        if !disk_ok || s.uncertain != 0 || !s.rewrite.fits(bytes) {
            s.rewrite.deferred = s.rewrite.deferred.saturating_add(1);
            return None;
        }
        // Reclamation must be able to run while existing log debt exceeds the
        // data threshold. Its separate output budget and disk check still apply.
        s.rewrite.reserved_bytes += bytes;
        s.rewrite.active += 1;
        s.rewrite.admitted = s.rewrite.admitted.saturating_add(1);
        s.rewrite.peak_bytes = s.rewrite.peak_bytes.max(s.rewrite.reserved_bytes);
        s.account_peak();
        Some(RewriteClaim {
            budget: self.clone(),
            bytes,
        })
    }
}
impl LogDebt {
    /// None means defer without changing the layout. It authorizes neither
    /// compaction nor deletion; the caller must already own those authorities.
    pub fn reserve_rewrite(&self, bytes: u64) -> Option<RewriteClaim> {
        self.budget.reserve_rewrite(bytes)
    }
}
impl Drop for RewriteClaim {
    fn drop(&mut self) {
        let mut s = self.budget.state.lock().expect("log budget");
        s.rewrite.reserved_bytes -= self.bytes;
        s.rewrite.active -= 1;
    }
}

#[cfg(test)]
mod tests;
