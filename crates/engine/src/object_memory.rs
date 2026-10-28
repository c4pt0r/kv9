//! Object-client owned payload buffers. The reservation belongs to the Bytes
//! owner, so SDK/queue/reply clones retain it beyond a caller's timeout.
use bytes::Bytes;
use kv9_common::{Error, Result};
use std::sync::{Arc, Mutex};

pub const DEFAULT_BUFFER_BYTES: u64 = 96 * 1024 * 1024;
#[derive(Clone, Debug, serde::Serialize)]
pub struct Snapshot {
    pub limit_bytes: u64,
    pub retained_bytes: u64,
    pub peak_bytes: u64,
    pub active_buffers: u64,
    pub admitted_buffers: u64,
    pub refused_buffers: u64,
}
#[derive(Debug)]
pub struct BufferBudget {
    state: Mutex<Snapshot>,
}
impl BufferBudget {
    pub fn new(limit_bytes: u64) -> Result<Self> {
        if limit_bytes == 0 {
            return Err(Error::Config("object buffer limit must be positive".into()));
        }
        Ok(Self {
            state: Mutex::new(Snapshot {
                limit_bytes,
                retained_bytes: 0,
                peak_bytes: 0,
                active_buffers: 0,
                admitted_buffers: 0,
                refused_buffers: 0,
            }),
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().expect("object buffer budget").clone()
    }
    pub(crate) fn reserve(self: &Arc<Self>, bytes: usize) -> Result<Reservation> {
        let mut state = self.state.lock().expect("object buffer budget");
        if bytes as u64 > state.limit_bytes - state.retained_bytes {
            state.refused_buffers = state.refused_buffers.saturating_add(1);
            return Err(Error::Engine("object buffer byte budget exhausted".into()));
        }
        state.retained_bytes += bytes as u64;
        state.peak_bytes = state.peak_bytes.max(state.retained_bytes);
        state.active_buffers += 1;
        state.admitted_buffers = state.admitted_buffers.saturating_add(1);
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
    pub(crate) fn copy(self: &Arc<Self>, source: &[u8]) -> Result<Bytes> {
        let reservation = self.reserve(source.len())?;
        Ok(reservation.finish(source.to_vec()))
    }
}
impl Default for BufferBudget {
    fn default() -> Self {
        Self::new(DEFAULT_BUFFER_BYTES).expect("positive default")
    }
}
pub(crate) struct Reservation {
    budget: Arc<BufferBudget>,
    bytes: usize,
}
impl Reservation {
    pub(crate) fn finish(self, bytes: Vec<u8>) -> Bytes {
        assert!(
            bytes.capacity() <= self.bytes,
            "object buffer exceeds its reservation"
        );
        Bytes::from_owner(Owned {
            bytes,
            _reservation: self,
        })
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock().expect("object buffer budget");
        state.retained_bytes -= self.bytes as u64;
        state.active_buffers -= 1;
    }
}
struct Owned {
    bytes: Vec<u8>,
    _reservation: Reservation,
}
impl AsRef<[u8]> for Owned {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
