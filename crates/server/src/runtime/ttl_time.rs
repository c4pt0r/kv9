//! Prepare a qualified observation outside Raft/apply, then await its exact
//! effect. This does not itself authorize a read snapshot or enable a group.
use super::*;
use crate::ttl_clock::{ClockStatus, TtlClock};
use kv9_common::raw_ttl::clock::ClockRefusal;
use kv9_raft::driver::RawTtlSubmission;

fn submit_observation<
    S: kv9_raft::rawnode::PersistentRaftStorage,
    E: kv9_raft::ApplyStore + 'static,
>(
    driver: &NodeDriver<S, E>,
    expected: &kv9_common::data_range::DataRange,
    clock: &dyn TtlClock,
    deadline: Instant,
) -> Result<RawTtlSubmission> {
    if Instant::now() >= deadline {
        return Err(Error::MetaNotReady(
            "TTL observation deadline expired".into(),
        ));
    }
    let (range, committed) = driver.raw_ttl_state()?;
    if &range != expected {
        return Err(Error::StaleEpoch {
            region: expected.region,
        });
    }
    let sample = clock
        .sample()
        .map_err(|reason| Error::RawTtlClock { reason })?;
    let observed = sample.advance(committed).map_err(|_| Error::RawTtlClock {
        reason: ClockRefusal::BehindWatermark,
    })?;
    driver.propose_raw_ttl_observation(&range, observed, deadline)
}

impl NodeRuntime {
    pub fn raw_ttl_clock_status(&self) -> Option<ClockStatus> {
        self.raw_ttl_clock.as_ref().map(|clock| clock.status())
    }

    /// Blocking embedded control primitive. Callers must await the returned
    /// exact receipt before establishing a new ReadIndex/snapshot for TTL reads.
    /// The public RawTtl path performs its own scoped observation/read sequence.
    /// One absolute deadline is shared throughout this embedded primitive.
    pub fn observe_raw_ttl_time(
        &self,
        region: RegionId,
        deadline: Instant,
    ) -> Result<RawTtlSubmission> {
        self.rpc_backend.ensure_serving()?;
        let clock = self.raw_ttl_clock.as_ref().ok_or(Error::RawTtlClock {
            reason: ClockRefusal::Unavailable,
        })?;
        let group = self
            .rpc_backend
            .raw_directory
            .by_region(region)
            .ok_or(Error::RegionNotFound)?;
        let (driver, _, range) = group.capture_parts();
        let budget = deadline
            .checked_duration_since(Instant::now())
            .filter(|budget| !budget.is_zero())
            .ok_or_else(|| Error::MetaNotReady("TTL observation deadline expired".into()))?;
        driver.read_barrier(budget)?;
        submit_observation(driver, range, clock.as_ref(), deadline)
    }
}

#[cfg(test)]
mod tests;
