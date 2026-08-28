//! Test doubles for exercising engine failure paths.
//!
//! With [`MemEngine`](crate::MemEngine) a write essentially cannot fail, so callers written
//! against it never had their error handling exercised — the failure branch was
//! unreachable, not merely untested. [`WalEngine`](crate::WalEngine) changes that: its
//! `write` performs real I/O, and `write_all`/`sync_all` fail on a full disk, a failed
//! fsync, a read-only mount, or an exceeded quota.
//!
//! Making durability real therefore *created* a live failure path through every caller of
//! `Engine::write`. [`FaultyEngine`] exists so those paths can be tested deliberately
//! rather than waiting for a disk to fill up in production.
//!
//! This is a test double and says so; it is exported normally (not behind `cfg(test)`)
//! because the callers that need it — the raft apply loop, the server runtime — live in
//! other crates and cannot see another crate's test-only items.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use kv9_common::{Error, Result, Value};

use crate::cf::ColumnFamily;
use crate::write_batch::WriteBatch;
use crate::{Durability, Engine, ReadView, ScanEntry};

/// An [`Engine`] wrapper whose writes can be made to fail on demand.
///
/// Reads always pass through to the inner engine, so a caller can inspect state after a
/// failed write and confirm nothing was applied.
#[derive(Debug)]
pub struct FaultyEngine<E: Engine> {
    inner: E,
    fail_writes: AtomicBool,
    write_attempts: AtomicU64,
}

impl<E: Engine> FaultyEngine<E> {
    /// Wrap `inner`. Writes succeed until [`FaultyEngine::start_failing_writes`] is called.
    pub fn new(inner: E) -> Self {
        FaultyEngine {
            inner,
            fail_writes: AtomicBool::new(false),
            write_attempts: AtomicU64::new(0),
        }
    }

    /// Every subsequent [`Engine::write`] fails, as a full disk or a failed fsync would.
    pub fn start_failing_writes(&self) {
        self.fail_writes.store(true, Ordering::SeqCst);
    }

    /// Resume accepting writes.
    pub fn stop_failing_writes(&self) {
        self.fail_writes.store(false, Ordering::SeqCst);
    }

    /// How many writes have been attempted, failed ones included.
    ///
    /// Useful for asserting a caller actually *tried* — distinguishing "handled the error"
    /// from "never got that far", which otherwise look identical from the outside.
    pub fn write_attempts(&self) -> u64 {
        self.write_attempts.load(Ordering::SeqCst)
    }

    /// The wrapped engine.
    pub fn inner(&self) -> &E {
        &self.inner
    }
}

impl<E: Engine> Engine for FaultyEngine<E> {
    fn get(&self, cf: ColumnFamily, key: &[u8]) -> Result<Option<Value>> {
        self.inner.get(cf, key)
    }

    fn write(&self, batch: WriteBatch) -> Result<()> {
        self.write_attempts.fetch_add(1, Ordering::SeqCst);
        if self.fail_writes.load(Ordering::SeqCst) {
            // Shaped like the real thing: WalEngine surfaces I/O failures as Error::Engine.
            return Err(Error::Engine(
                "injected write failure (simulating a full disk / failed fsync)".into(),
            ));
        }
        self.inner.write(batch)
    }

    fn scan(
        &self,
        cf: ColumnFamily,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<ScanEntry>> {
        self.inner.scan(cf, start, end, limit)
    }

    fn delete_range(&self, cf: ColumnFamily, start: &[u8], end: &[u8]) -> Result<()> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(Error::Engine("injected write failure".into()));
        }
        self.inner.delete_range(cf, start, end)
    }

    fn checksum(&self, cf: ColumnFamily, start: &[u8], end: &[u8]) -> Result<u64> {
        self.inner.checksum(cf, start, end)
    }

    fn snapshot(&self) -> Result<Box<dyn ReadView + '_>> {
        self.inner.snapshot()
    }

    fn durability(&self) -> Durability {
        self.inner.durability()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemEngine;

    fn put(e: &dyn Engine, k: &[u8], v: &[u8]) -> Result<()> {
        let mut b = WriteBatch::new();
        b.put(ColumnFamily::Default, k.to_vec(), v.to_vec());
        e.write(b)
    }

    #[test]
    fn writes_fail_only_while_armed_and_leave_no_trace() {
        let e = FaultyEngine::new(MemEngine::new());

        put(&e, b"a", b"1").expect("writes succeed before arming");
        assert_eq!(e.get(ColumnFamily::Default, b"a").unwrap(), Some(b"1".to_vec()));

        e.start_failing_writes();
        assert!(put(&e, b"b", b"2").is_err(), "an armed write must fail");
        assert_eq!(
            e.get(ColumnFamily::Default, b"b").unwrap(),
            None,
            "a failed write must not be partially applied"
        );
        // ...and the earlier write is untouched.
        assert_eq!(e.get(ColumnFamily::Default, b"a").unwrap(), Some(b"1".to_vec()));

        e.stop_failing_writes();
        put(&e, b"c", b"3").expect("writes succeed again after disarming");
        assert_eq!(e.get(ColumnFamily::Default, b"c").unwrap(), Some(b"3".to_vec()));
    }

    /// The counter is what separates "the caller handled the error" from "the caller never
    /// attempted the write" — two very different bugs that look the same from outside.
    #[test]
    fn attempts_are_counted_including_failures() {
        let e = FaultyEngine::new(MemEngine::new());
        put(&e, b"a", b"1").unwrap();
        e.start_failing_writes();
        let _ = put(&e, b"b", b"2");
        let _ = put(&e, b"c", b"3");
        assert_eq!(e.write_attempts(), 3);
    }

    #[test]
    fn reads_and_durability_pass_through() {
        let e = FaultyEngine::new(MemEngine::new());
        put(&e, b"k", b"v").unwrap();
        e.start_failing_writes();
        // Reads keep working while writes fail, so a test can inspect the aftermath.
        assert_eq!(e.get(ColumnFamily::Default, b"k").unwrap(), Some(b"v".to_vec()));
        assert_eq!(e.scan(ColumnFamily::Default, b"", b"z", 10).unwrap().len(), 1);
        assert_eq!(e.durability(), Durability::Volatile);
    }
}
