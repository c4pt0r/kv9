//! Object payload admission. Credits are spent before an attempted PUT/GET,
//! including conditional-PUT comparison reads. Errors never refund issued work.
//! This does not account for protocol headers, TCP retransmits, LIST or HEAD.
use std::sync::Mutex;
use std::time::{Duration, Instant};

use kv9_common::{Error, Result};

const SCALE: u128 = 1_000_000_000;

#[derive(Clone, Copy)]
pub(crate) enum Direction {
    Put,
    Get,
}
#[derive(Clone, Default, Debug, serde::Serialize)]
pub struct ObjectIoSnapshot {
    pub bytes_per_second: u64,
    pub burst_bytes: u64,
    pub admitted_put_bytes: u64,
    pub admitted_get_bytes: u64,
    pub admitted_operations: u64,
    pub waiting_operations: u64,
    pub peak_waiting_operations: u64,
    pub cancelled_before_admission: u64,
    pub oversized_operations: u64,
    pub waited_micros: u64,
}
struct Credits {
    scaled: u128,
    at: Instant,
}

/// One shared limiter for the node's object client, including checkpoint
/// maintenance and recovery. The async mutex orders waiters and is released
/// before network I/O. Cancellation while waiting spends no credit.
pub struct ObjectIoBudget {
    rate: u64,
    burst: u64,
    credits: tokio::sync::Mutex<Credits>,
    stats: Mutex<ObjectIoSnapshot>,
}
impl std::fmt::Debug for ObjectIoBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectIoBudget")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}
impl ObjectIoBudget {
    pub fn new(bytes_per_second: u64, burst_bytes: u64) -> Result<Self> {
        if bytes_per_second == 0 || burst_bytes == 0 {
            return Err(Error::Config(
                "object I/O rate and burst must be positive".into(),
            ));
        }
        Ok(Self {
            rate: bytes_per_second,
            burst: burst_bytes,
            credits: tokio::sync::Mutex::new(Credits {
                scaled: u128::from(burst_bytes) * SCALE,
                at: Instant::now(),
            }),
            stats: Mutex::new(ObjectIoSnapshot {
                bytes_per_second,
                burst_bytes,
                ..ObjectIoSnapshot::default()
            }),
        })
    }
    pub fn snapshot(&self) -> ObjectIoSnapshot {
        self.stats.lock().expect("object I/O stats").clone()
    }
    pub(crate) async fn admit(&self, direction: Direction, bytes: u64) -> Result<()> {
        if bytes > self.burst {
            let mut stats = self.stats.lock().expect("object I/O stats");
            stats.oversized_operations = stats.oversized_operations.saturating_add(1);
            return Err(Error::Engine(
                "object payload exceeds I/O burst limit".into(),
            ));
        }
        let mut waiting = Waiting::new(self);
        let mut credits = self.credits.lock().await;
        let required = u128::from(bytes) * SCALE;
        loop {
            let now = Instant::now();
            credits.scaled = (credits.scaled.saturating_add(
                now.saturating_duration_since(credits.at)
                    .as_nanos()
                    .saturating_mul(u128::from(self.rate)),
            ))
            .min(u128::from(self.burst) * SCALE);
            credits.at = now;
            if credits.scaled >= required {
                credits.scaled -= required;
                break;
            }
            let nanos = (required - credits.scaled).div_ceil(u128::from(self.rate));
            // Each wait also lies inside the object worker's actual deadline.
            tokio::time::sleep(Duration::from_nanos(nanos.min(u128::from(u64::MAX)) as u64)).await;
        }
        let mut stats = self.stats.lock().expect("object I/O stats");
        let counter = match direction {
            Direction::Put => &mut stats.admitted_put_bytes,
            Direction::Get => &mut stats.admitted_get_bytes,
        };
        *counter = counter.saturating_add(bytes);
        stats.admitted_operations = stats.admitted_operations.saturating_add(1);
        waiting.admitted = true;
        Ok(())
    }
}
struct Waiting<'a> {
    budget: &'a ObjectIoBudget,
    started: Instant,
    admitted: bool,
}
impl<'a> Waiting<'a> {
    fn new(budget: &'a ObjectIoBudget) -> Self {
        let mut stats = budget.stats.lock().expect("object I/O stats");
        stats.waiting_operations += 1;
        stats.peak_waiting_operations = stats.peak_waiting_operations.max(stats.waiting_operations);
        Self {
            budget,
            started: Instant::now(),
            admitted: false,
        }
    }
}
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let mut stats = self.budget.stats.lock().expect("object I/O stats");
        stats.waiting_operations -= 1;
        stats.waited_micros = stats
            .waited_micros
            .saturating_add(self.started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64);
        if !self.admitted {
            stats.cancelled_before_admission = stats.cancelled_before_admission.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests;
