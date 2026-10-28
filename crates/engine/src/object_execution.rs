//! Local queue/future lifetime only. Draining this scope does not certify that
//! an ambiguous remote write stopped, nor authorize retention-owner release.
use std::sync::{Arc, Condvar, Mutex};

use kv9_common::retention::{OwnerId, OwnerToken};
use kv9_common::{Error, Result, RootDigest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionIdentity {
    pub root: RootDigest,
    pub pending: OwnerId,
    pub executor: OwnerToken,
}

/// Minted only after the enclosing scope closes and every actual job exits.
/// Counts describe this scope, never other generations or remote cancellation.
/// No Clone/constructor: the uploader hands this observation to its owner once.
pub struct DrainedExecution {
    identity: Option<ExecutionIdentity>,
    issued: u64,
    resolved: u64,
}
impl DrainedExecution {
    pub fn identity(&self) -> Option<ExecutionIdentity> {
        self.identity
    }
    pub fn issued_puts(&self) -> u64 {
        self.issued
    }
    pub fn resolved_puts(&self) -> u64 {
        self.resolved
    }
}

#[derive(Debug, Default)]
pub(crate) struct Execution {
    state: Mutex<State>,
    drained: Condvar,
}
#[derive(Debug, Default)]
struct State {
    jobs: usize,
    closed: bool,
    identity: Option<ExecutionIdentity>,
    issued: u64,
    resolved: u64,
}

impl Execution {
    pub(crate) fn require_binding(&self) -> Result<()> {
        let state = self.state.lock().expect("object execution poisoned");
        if state.closed || state.identity.is_none() {
            return Err(Error::Engine(
                "checkpoint read scope is unbound or closed".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn bind(&self, identity: ExecutionIdentity) -> Result<()> {
        let mut state = self.state.lock().expect("object execution poisoned");
        if state.closed || state.identity.is_some() || state.jobs != 0 || state.issued != 0 {
            return Err(Error::Engine(
                "checkpoint execution cannot be rebound or bound after I/O".into(),
            ));
        }
        if identity.root.as_bytes() == &[0; 32] {
            return Err(Error::Engine("checkpoint execution root is zero".into()));
        }
        state.identity = Some(identity);
        Ok(())
    }
    pub(crate) fn report(&self) -> Result<DrainedExecution> {
        let state = self.state.lock().expect("object execution poisoned");
        if !state.closed || state.jobs != 0 {
            return Err(Error::Engine("checkpoint execution has not drained".into()));
        }
        Ok(DrainedExecution {
            identity: state.identity,
            issued: state.issued,
            resolved: state.resolved,
        })
    }
    pub(crate) fn admit(self: &Arc<Self>) -> Result<JobLease> {
        let mut state = self.state.lock().expect("object execution poisoned");
        if state.closed {
            return Err(Error::Engine("object execution already closed".into()));
        }
        state.jobs += 1;
        Ok(JobLease(self.clone()))
    }

    fn close_and_drain(&self) {
        let mut state = self.state.lock().expect("object execution poisoned");
        state.closed = true;
        self.drained.notify_all();
        while state.jobs != 0 {
            state = self.drained.wait(state).expect("object execution poisoned");
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn wait_closed(&self, timeout: std::time::Duration) -> bool {
        let state = self.state.lock().unwrap();
        self.drained
            .wait_timeout_while(state, timeout, |s| !s.closed)
            .unwrap()
            .0
            .closed
    }
}

// Kept by the actual job, not the caller or its reply channel. Failed admission,
// queued-job discard, worker unwind and normal completion all drop this lease.
pub(crate) struct JobLease(Arc<Execution>);
impl JobLease {
    /// Immediately before the first poll of a potentially writing SDK call.
    /// Cancellation/panic/error leaves this write unresolved by default.
    pub(crate) fn start_put(&self) -> Result<PutAttempt> {
        let mut state = self.0.state.lock().expect("object execution poisoned");
        state.issued = state
            .issued
            .checked_add(1)
            .ok_or_else(|| Error::Engine("checkpoint PUT counter exhausted".into()))?;
        Ok(PutAttempt(self.0.clone()))
    }
}
pub(crate) struct PutAttempt(Arc<Execution>);
impl PutAttempt {
    /// Only for a single-attempt client and an actual success/precondition reply.
    pub(crate) fn resolved(self) {
        let mut state = self.0.state.lock().expect("object execution poisoned");
        state.resolved += 1;
        assert!(state.resolved <= state.issued);
    }
}
#[cfg(any(test, feature = "testing"))]
impl JobLease {
    pub(crate) fn execution(&self) -> Arc<Execution> {
        self.0.clone()
    }
}
impl Drop for JobLease {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().expect("object execution poisoned");
        state.jobs -= 1;
        if state.jobs == 0 {
            self.0.drained.notify_all();
        }
    }
}

pub(crate) struct DrainOnExit(pub(crate) Arc<Execution>);
impl Drop for DrainOnExit {
    fn drop(&mut self) {
        self.0.close_and_drain();
    }
}
