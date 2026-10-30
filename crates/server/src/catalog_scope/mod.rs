//! The scoped metadata mode (docs/META-REPLICA-MODES.md): the change ring and the
//! scope stream a full node serves, and the slice a scoped node keeps.

pub mod ring;
pub mod serve;
pub mod subscribe;

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use kv9_common::AppliedPosition;

/// The position a scoped node's slice has applied, for code that must wait for a
/// committed metadata entry to be locally visible (the metadata driver's
/// `wait_applied` on a full node). Unused on full nodes.
#[derive(Default)]
pub struct SliceWatch {
    scoped: std::sync::atomic::AtomicBool,
    position: Mutex<Option<AppliedPosition>>,
    moved: Condvar,
}

impl SliceWatch {
    pub fn is_scoped(&self) -> bool {
        self.scoped.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn mark_scoped(&self) {
        self.scoped.store(true, std::sync::atomic::Ordering::Release);
    }
    pub fn advance(&self, at: AppliedPosition) {
        *self.position.lock().expect("slice watch poisoned") = Some(at);
        self.moved.notify_all();
    }
    pub fn position(&self) -> Option<AppliedPosition> {
        *self.position.lock().expect("slice watch poisoned")
    }
    /// Wait until the slice is at or past `index`; `false` on timeout.
    pub fn wait_index(&self, index: u64, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        let mut position = self.position.lock().expect("slice watch poisoned");
        loop {
            if position.is_some_and(|p| p.index >= index) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            position = self.moved.wait_timeout(position, deadline - now).expect("slice watch poisoned").0;
        }
    }
}
