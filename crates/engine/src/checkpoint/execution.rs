use super::*;
use crate::object_execution::{DrainOnExit, Execution};

#[cfg(test)]
mod tests;

/// A drained GET-only scope cannot be supplied as a publisher drain report.
/// ```compile_fail
/// fn forge(read: kv9_engine::checkpoint::DrainedRead)
///     -> kv9_engine::checkpoint::DrainedExecution { read }
/// ```
pub struct DrainedRead(DrainedExecution);
impl DrainedRead {
    pub fn identity(&self) -> Option<ExecutionIdentity> {
        self.0.identity()
    }
}
/// Borrowed GET-only interface. It cannot escape its draining scope or issue PUT.
/// ```compile_fail
/// fn upload(reader: &kv9_engine::checkpoint::CheckpointReader<'_>,
///           plan: kv9_engine::checkpoint::PlannedFlush) {
///     reader.upload_planned(plan);
/// }
/// ```
pub struct CheckpointReader<'a>(&'a RemoteUploader);
/// A recovery dependency which can only restore an image. Implementations may
/// admit, drain and persist a Reader report around each exact manifest restore.
pub trait CheckpointRestore {
    fn restore(&self, manifest: &CheckpointManifest) -> Result<MemEngine>;
}
impl CheckpointRestore for RemoteUploader {
    fn restore(&self, manifest: &CheckpointManifest) -> Result<MemEngine> {
        RemoteUploader::restore(self, manifest)
    }
}
impl CheckpointRestore for CheckpointReader<'_> {
    fn restore(&self, manifest: &CheckpointManifest) -> Result<MemEngine> {
        CheckpointReader::restore(self, manifest)
    }
}
impl CheckpointReader<'_> {
    pub fn bind_execution(&self, identity: ExecutionIdentity) -> Result<()> {
        self.0.bind_execution(identity)
    }
    pub fn restore(&self, manifest: &CheckpointManifest) -> Result<MemEngine> {
        self.0
            .execution
            .as_ref()
            .expect("read scope")
            .require_binding()?;
        self.0.restore(manifest)
    }
}

impl RemoteUploader {
    /// An independent GET-only child may run inside a publisher scope. It
    /// blocks its parent until its own actual jobs exit, and returns a distinct
    /// report type that cannot falsely drain the enclosing publisher.
    pub fn with_local_read_report<T>(
        &self,
        work: impl FnOnce(&CheckpointReader<'_>) -> T,
    ) -> Result<(T, DrainedRead)> {
        let execution = Arc::new(Execution::default());
        let drain = DrainOnExit(execution.clone());
        let scoped = Self {
            store: self.store.clone(),
            image_budget: self.image_budget.clone(),
            spool_budget: self.spool_budget.clone(),
            pending_budget: self.pending_budget.clone(),
            execution: Some(execution.clone()),
        };
        let result = work(&CheckpointReader(&scoped));
        drop(drain);
        Ok((result, DrainedRead(execution.report()?)))
    }
    /// Bind one exact ledger generation before its Share/Publish and any PUT.
    pub fn bind_execution(&self, identity: ExecutionIdentity) -> Result<()> {
        self.execution
            .as_ref()
            .ok_or_else(|| Error::Engine("checkpoint execution scope missing".into()))?
            .bind(identity)
    }

    /// Outermost scope only. Nested work cannot mint an early drain observation.
    pub fn with_local_execution_report<T>(
        &self,
        work: impl FnOnce(&Self) -> T,
    ) -> Result<(T, DrainedExecution)> {
        if self.execution.is_some() {
            return Err(Error::Engine("nested execution cannot report drain".into()));
        }
        let execution = Arc::new(Execution::default());
        let drain = DrainOnExit(execution.clone());
        let scoped = Self {
            store: self.store.clone(),
            image_budget: self.image_budget.clone(),
            spool_budget: self.spool_budget.clone(),
            pending_budget: self.pending_budget.clone(),
            execution: Some(execution.clone()),
        };
        let result = work(&scoped);
        drop(drain);
        Ok((result, execution.report()?))
    }
    /// Run synchronous checkpoint work, then close and drain its local object
    /// jobs before returning or unwinding. A caller timeout may leave a job in
    /// the queue or worker; it cannot end this execution's lifetime early.
    ///
    /// The caller keeps its maintenance permit and group state until this
    /// returns. Drain may outlast the operation deadline. It waits for local
    /// futures, not remote cancellation, and grants no object-release authority.
    /// Nested scopes share the enclosing execution. The borrowed uploader must
    /// be used for every operation in the closure.
    pub fn with_local_execution<T>(&self, work: impl FnOnce(&Self) -> T) -> T {
        if self.execution.is_some() {
            return work(self);
        }
        let execution = Arc::new(Execution::default());
        let _drain = DrainOnExit(execution.clone());
        let scoped = Self {
            store: self.store.clone(),
            image_budget: self.image_budget.clone(),
            spool_budget: self.spool_budget.clone(),
            pending_budget: self.pending_budget.clone(),
            execution: Some(execution),
        };
        work(&scoped)
    }
}

#[derive(Debug)]
pub(super) struct CheckpointStore<'a>(pub(super) &'a RemoteUploader);
impl ObjectStore for CheckpointStore<'_> {
    fn put(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        match &self.0.store {
            RemoteBackend::Minio(store) => {
                store.put_in_execution(key, bytes, self.0.execution.as_ref())
            }
            #[cfg(any(test, feature = "testing"))]
            RemoteBackend::Harness(store) => {
                let lease = self.0.execution.as_ref().map(|e| e.admit()).transpose()?;
                let _put = lease.as_ref().map(|lease| lease.start_put()).transpose()?;
                // An arbitrary test ObjectStore promises no remote terminality.
                // Even Ok cannot erase an issued write from this observation.
                store.put(key, bytes)
            }
        }
    }
    fn get_shared_bounded(
        &self,
        key: &ObjectKey,
        max_bytes: usize,
    ) -> Result<Option<bytes::Bytes>> {
        match &self.0.store {
            RemoteBackend::Minio(store) => {
                store.get_in_execution(key, max_bytes, self.0.execution.as_ref())
            }
            #[cfg(any(test, feature = "testing"))]
            RemoteBackend::Harness(store) => {
                let _lease = self.0.execution.as_ref().map(|e| e.admit()).transpose()?;
                store.get_shared_bounded(key, max_bytes)
            }
        }
    }
    fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>> {
        Ok(self
            .get_shared_bounded(key, MAX_CHECKPOINT_BYTES)?
            .map(|b| b.to_vec()))
    }
    fn get_bounded(&self, key: &ObjectKey, max_bytes: usize) -> Result<Option<Vec<u8>>> {
        Ok(self.get_shared_bounded(key, max_bytes)?.map(|b| b.to_vec()))
    }
    fn delete(&self, _: &ObjectKey) -> Result<()> {
        Err(Error::Engine("checkpoint I/O cannot delete objects".into()))
    }
    fn list(&self, _: &str) -> Result<Vec<ObjectKey>> {
        Err(Error::Engine("checkpoint I/O cannot list objects".into()))
    }
}
