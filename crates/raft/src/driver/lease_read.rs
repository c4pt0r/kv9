//! Lease reads return an exact owned view, never a fabricated ReadBarrier.

use super::*;
use kv9_engine::{ReadView, WalEngine};
use std::sync::TryLockError;

/// Successful final lease validation applies only to this retained view. It
/// cannot be exchanged for another engine snapshot or reused as a read barrier.
pub struct LeaseReadView {
    view: Box<dyn ReadView>,
    applied_through: u64,
}

impl LeaseReadView {
    pub fn into_view(self) -> Box<dyn ReadView> {
        self.view
    }

    /// Diagnostic only; this scalar cannot construct another authorized view.
    pub fn applied_through(&self) -> u64 {
        self.applied_through
    }
}

pub enum ReadPreparation {
    Lease(LeaseReadView),
    Quorum(ReadBarrier),
}

impl<S: PersistentRaftStorage> NodeDriver<S, WalEngine> {
    /// Try a local lease under the same admission limit as asynchronous
    /// ReadIndex. Any refusal/contended lock transfers the exact reservation
    /// and original absolute deadline to Safe ReadIndex. Cancellation keeps
    /// its existing RAII release path. This never changes raft-rs read mode.
    pub async fn read_preparation_async(
        &self,
        budget: Duration,
    ) -> std::result::Result<ReadPreparation, ReadIndexError> {
        let timer = self.metrics.read_establishment.start();
        let started = Instant::now();
        let result = async {
            let deadline = started.checked_add(budget).ok_or_else(|| {
                ReadIndexError::Failed(Error::Raft("read deadline overflow".into()))
            })?;
            let context = self.mint_read_context()?;
            let reservation = self.async_reads.reserve_local(context, started, deadline)?;
            if let Some(view) = self.try_lease_read_view(started, deadline)? {
                self.lease_read_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(ReadPreparation::Lease(view));
            }
            let index = reservation.submit()?.wait().await?;
            if Instant::now() >= deadline {
                return Err(expired(started));
            }
            Ok(ReadPreparation::Quorum(ReadBarrier { index }))
        }
        .await;
        timer.finish(classify_read(&result));
        result
    }

    pub fn lease_read_hits(&self) -> u64 {
        self.lease_read_hits.load(Ordering::Relaxed)
    }

    fn try_lease_read_view(
        &self,
        started: Instant,
        deadline: Instant,
    ) -> std::result::Result<Option<LeaseReadView>, ReadIndexError> {
        // A successful owner turn is the only point at which its unified
        // applied watermark describes a complete publication. Never wait for
        // an owner or hold sm/applied locks across any peer call.
        let _owner = match self.pump_gate.try_lock() {
            Ok(owner) => owner,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(_)) => panic!("pump gate poisoned"),
        };
        if self.stop.load(Ordering::Acquire) {
            return Err(ReadIndexError::Failed(Error::Raft(
                "read owner is stopped".into(),
            )));
        }
        {
            let fatal = match self.fatal.try_lock() {
                Ok(fatal) => fatal,
                Err(TryLockError::WouldBlock) => return Ok(None),
                Err(TryLockError::Poisoned(_)) => panic!("fatal poisoned"),
            };
            if let Some(cause) = fatal.as_ref() {
                return Err(ReadIndexError::Failed(Error::Raft(cause.clone())));
            }
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| expired(started))?;
        let remaining_ns = u64::try_from(remaining.as_nanos()).map_err(|_| {
            ReadIndexError::Failed(Error::Raft("read budget exceeds lease clock range".into()))
        })?;
        let Some(read) = self
            .peer
            .try_begin_lease_read(remaining_ns)
            .map_err(ReadIndexError::Failed)?
        else {
            return Ok(None);
        };
        let through = {
            let applied = match self.driver_applied.try_lock() {
                Ok(applied) => applied,
                Err(TryLockError::WouldBlock) => return Ok(None),
                Err(TryLockError::Poisoned(_)) => panic!("driver_applied poisoned"),
            };
            let Some(through) = *applied else {
                return Ok(None);
            };
            through
        };
        let snapshot = {
            let sm = match self.sm.try_lock() {
                Ok(sm) => sm,
                Err(TryLockError::WouldBlock) => return Ok(None),
                Err(TryLockError::Poisoned(_)) => panic!("sm poisoned"),
            };
            // The engine comes from this exact state machine. Callers cannot
            // substitute a stale/foreign view or pair one with a fresh number.
            let Some(snapshot) = sm.engine().try_positioned_resident_snapshot() else {
                return Ok(None);
            };
            let command = snapshot.position().map_or(0, |position| position.index);
            if command != sm.applied_index().0 || command > through.index {
                return Err(ReadIndexError::Failed(Error::Engine(
                    "lease snapshot disagrees with successful driver application".into(),
                )));
            }
            snapshot
        };
        // No driver leaf/state-machine lock is held here. pump_gate excludes
        // concurrent apply, so intervening no-ops do not label an older data
        // version with a newer command's position. Peer authority is rechecked
        // after acquisition of the exact snapshot; expiry discards that view.
        if !self
            .peer
            .try_finish_lease_read(read, through.index)
            .map_err(ReadIndexError::Failed)?
        {
            return Ok(None);
        }
        if Instant::now() >= deadline {
            return Err(expired(started));
        }
        if self.stop.load(Ordering::Acquire) {
            return Err(ReadIndexError::Failed(Error::Raft(
                "read owner is stopped".into(),
            )));
        }
        Ok(Some(LeaseReadView {
            view: snapshot.into_view(),
            applied_through: through.index,
        }))
    }
}

fn expired(started: Instant) -> ReadIndexError {
    ReadIndexError::Unconfirmed {
        phase: BarrierPhase::QuorumConfirmation,
        waited: started.elapsed(),
    }
}

#[allow(dead_code)]
const _: () = {
    struct Probe<T>(core::marker::PhantomData<T>);
    trait Fallback {
        const CHECK: () = ();
    }
    impl<T> Fallback for Probe<T> {}
    impl<T: Clone> Probe<T> {
        const CHECK: () = panic!("LeaseReadView must be neither Clone nor Copy");
    }
    Probe::<LeaseReadView>::CHECK
};

#[cfg(test)]
mod tests;
