//! Shared admission for retained serialized checkpoint images. This accounts for
//! resident plan/pending bytes or one streamed SST buffer, not resident indexes
//! or network copies. The historical *_images counters count reservations;
//! a streaming image uses successive per-object reservations, never a whole one.
use std::sync::{Arc, Mutex};

use kv9_common::{Error, Result};

pub const DEFAULT_IMAGE_BYTES: u64 = 96 * 1024 * 1024;

#[derive(Clone, Debug, serde::Serialize)]
pub struct Snapshot {
    pub limit_bytes: u64,
    pub retained_bytes: u64,
    pub peak_bytes: u64,
    pub active_images: u64,
    pub admitted_images: u64,
    pub refused_images: u64,
}
#[derive(Debug)]
pub struct ImageBudget {
    state: Mutex<Snapshot>,
}
impl ImageBudget {
    pub fn new(limit_bytes: u64) -> Result<Self> {
        if limit_bytes == 0 {
            return Err(Error::Config(
                "checkpoint image byte limit must be positive".into(),
            ));
        }
        Ok(Self {
            state: Mutex::new(Snapshot {
                limit_bytes,
                retained_bytes: 0,
                peak_bytes: 0,
                active_images: 0,
                admitted_images: 0,
                refused_images: 0,
            }),
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().expect("checkpoint image budget").clone()
    }
    /// Nonblocking: no nested image acquisition can deadlock a running job.
    /// On pressure the existing journal and owner remain authoritative.
    pub(crate) fn reserve(self: &Arc<Self>, bytes: u64) -> Result<Reservation> {
        let mut state = self.state.lock().expect("checkpoint image budget");
        if bytes > state.limit_bytes - state.retained_bytes {
            state.refused_images = state.refused_images.saturating_add(1);
            return Err(Error::Engine(
                "checkpoint image byte budget exhausted".into(),
            ));
        }
        state.retained_bytes += bytes;
        state.peak_bytes = state.peak_bytes.max(state.retained_bytes);
        state.active_images += 1;
        state.admitted_images = state.admitted_images.saturating_add(1);
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
}
impl Default for ImageBudget {
    fn default() -> Self {
        Self::new(DEFAULT_IMAGE_BYTES).expect("positive default image budget")
    }
}
#[derive(Debug)]
pub(crate) struct Reservation {
    budget: Arc<ImageBudget>,
    bytes: u64,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock().expect("checkpoint image budget");
        state.retained_bytes -= self.bytes;
        state.active_images -= 1;
    }
}
