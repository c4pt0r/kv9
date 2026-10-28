//! One fair catalog serialization point, with deadline-bounded background wait.
//! Planning and ordered apply still share one guard. Preserve std mutex poisoning:
//! a panicking planner cannot silently hand a possibly incomplete contract onward.
use parking_lot::{FairMutex, FairMutexGuard};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

pub(crate) struct CatalogLock {
    inner: FairMutex<()>,
    poisoned: AtomicBool,
    waiting: AtomicUsize,
}
impl CatalogLock {
    pub(crate) fn new() -> Self {
        Self {
            inner: FairMutex::new(()),
            poisoned: AtomicBool::new(false),
            waiting: AtomicUsize::new(0),
        }
    }
    fn guard<'a>(&'a self, inner: FairMutexGuard<'a, ()>) -> CatalogGuard<'a> {
        assert!(
            !self.poisoned.load(Ordering::Acquire),
            "catalog transaction lock poisoned"
        );
        CatalogGuard {
            lock: self,
            _inner: inner,
        }
    }
    pub(crate) fn lock(&self) -> CatalogGuard<'_> {
        let _waiting = Waiting::new(self);
        self.guard(self.inner.lock())
    }
    #[cfg(test)]
    pub(crate) fn try_lock(&self) -> Option<CatalogGuard<'_>> {
        self.inner.try_lock().map(|inner| self.guard(inner))
    }
    pub(crate) fn until(&self, deadline: Instant) -> Option<CatalogGuard<'_>> {
        if Instant::now() >= deadline {
            return None;
        }
        let _waiting = Waiting::new(self);
        let inner = self.inner.try_lock_until(deadline)?;
        // A wakeup or a preempted fast acquisition can run past the deadline.
        // It must not turn expiration into a new proposal capability.
        if Instant::now() >= deadline {
            return None;
        }
        Some(self.guard(inner))
    }
    #[cfg(test)]
    pub(crate) fn waiting(&self) -> usize {
        self.waiting.load(Ordering::Acquire)
    }
}
pub(crate) struct CatalogGuard<'a> {
    lock: &'a CatalogLock,
    _inner: FairMutexGuard<'a, ()>,
}
impl Drop for CatalogGuard<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.lock.poisoned.store(true, Ordering::Release);
        }
    }
}
struct Waiting<'a>(&'a CatalogLock);
impl<'a> Waiting<'a> {
    fn new(lock: &'a CatalogLock) -> Self {
        lock.waiting.fetch_add(1, Ordering::AcqRel);
        Self(lock)
    }
}
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.waiting.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests;
