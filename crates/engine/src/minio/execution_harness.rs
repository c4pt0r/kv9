//! Test-only gate around the actual worker queue, without opening any sockets.
use super::*;
use crate::object_execution::{Execution, JobLease};
use std::sync::{Condvar, Mutex};

#[derive(Default)]
struct State {
    released: bool,
    arrived: bool,
    execution: Option<Arc<Execution>>,
}
#[derive(Default)]
pub(super) struct Gate {
    state: Mutex<State>,
    changed: Condvar,
}
impl Gate {
    pub(super) fn arrive(&self, lease: Option<&JobLease>) {
        let mut state = self.state.lock().unwrap();
        if state.released {
            return;
        }
        state.arrived = true;
        state.execution = lease.map(JobLease::execution);
        self.changed.notify_all();
        while !state.released {
            state = self.changed.wait(state).unwrap();
        }
    }
}

/// Dropping this guard releases the worker, including when an assertion unwinds.
/// Available only with the testing feature; this is not remote durability evidence.
pub struct ObjectWorkerPause(Arc<Gate>);
impl ObjectWorkerPause {
    /// Observe an actual scoped job AND its caller closing the scope after the
    /// object deadline. A test cannot substitute elapsed time for those events.
    pub fn wait_for_closed_execution(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let state = self.0.state.lock().unwrap();
        let (state, _) = self
            .0
            .changed
            .wait_timeout_while(state, timeout, |s| !s.arrived)
            .unwrap();
        let execution = state.execution.clone();
        drop(state);
        execution.is_some_and(|e| e.wait_closed(deadline.saturating_duration_since(Instant::now())))
    }
}
impl Drop for ObjectWorkerPause {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().released = true;
        self.0.changed.notify_all();
    }
}

impl MinioObjectStore {
    /// Actual queue/deadline/future loop with upstream InMemory transport, held
    /// before the first job's deadline check. No network or credentials involved.
    #[doc(hidden)]
    pub fn with_paused_memory_worker(deadline: Duration) -> Result<(Self, ObjectWorkerPause)> {
        if wire_budget(deadline, configured_budget(deadline)).is_none() {
            return Err(Error::Config("test worker has no wire budget".into()));
        }
        let (tx, rx) = sync_channel(JOB_QUEUE_DEPTH);
        let (ready, receive) = sync_channel(1);
        let buffers = Arc::new(BufferBudget::default());
        let worker_buffers = buffers.clone();
        let worker = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            drive_jobs(
                &s3_client::memory::InMemory::new(),
                Bounded { runtime },
                configured_budget(deadline),
                None,
                &worker_buffers,
                rx,
                ready,
            );
        });
        receive.recv().unwrap()?;
        let gate = Arc::new(Gate::default());
        let store = Self {
            test_gate: Some(gate.clone()),
            #[cfg(test)]
            test_control: None,
            io_budget: None,
            buffer_budget: buffers,
            tx: Some(tx),
            worker: Some(worker),
            op_deadline: deadline,
            endpoint: "in-memory test transport".into(),
            bucket: "test".into(),
        };
        Ok((store, ObjectWorkerPause(gate)))
    }
}
