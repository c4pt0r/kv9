//! Admission for live anonymous planning spools, not total filesystem debt.
//! Durable pending files, publication tmp files and resident indexes are separate.
use kv9_common::{Error, Result};
use std::sync::{Arc, Mutex};

pub const DEFAULT_SPOOL_BYTES: u64 = 512 * 1024 * 1024;
#[derive(Debug, Clone, serde::Serialize)]
pub struct Snapshot {
    pub limit_bytes: u64,
    pub reserved_bytes: u64,
    pub peak_bytes: u64,
    pub active_spools: u64,
    pub refused_spools: u64,
}
#[derive(Debug)]
pub struct SpoolBudget {
    state: Mutex<Snapshot>,
}
impl SpoolBudget {
    pub fn new(limit_bytes: u64) -> Result<Self> {
        if limit_bytes == 0 {
            return Err(Error::Config(
                "checkpoint spool limit must be positive".into(),
            ));
        }
        Ok(Self {
            state: Mutex::new(Snapshot {
                limit_bytes,
                reserved_bytes: 0,
                peak_bytes: 0,
                active_spools: 0,
                refused_spools: 0,
            }),
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().expect("spool budget").clone()
    }
    pub(crate) fn reserve(self: &Arc<Self>, bytes: u64) -> Result<Reservation> {
        let mut state = self.state.lock().expect("spool budget");
        if bytes > state.limit_bytes - state.reserved_bytes {
            state.refused_spools = state.refused_spools.saturating_add(1);
            return Err(Error::Engine(
                "checkpoint spool byte budget exhausted".into(),
            ));
        }
        state.reserved_bytes += bytes;
        state.peak_bytes = state.peak_bytes.max(state.reserved_bytes);
        state.active_spools += 1;
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
}
impl Default for SpoolBudget {
    fn default() -> Self {
        Self::new(DEFAULT_SPOOL_BYTES).expect("positive spool budget")
    }
}
#[derive(Debug)]
pub(crate) struct Reservation {
    budget: Arc<SpoolBudget>,
    bytes: u64,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock().expect("spool budget");
        state.reserved_bytes -= self.bytes;
        state.active_spools -= 1;
    }
}
