//! One process-wide limit on the multi-node clusters the unit tests start.
//!
//! Each in-process cluster runs a metadata group with 200 ms elections. With dozens of
//! them in parallel on one host they starve each other into election churn, and tests
//! fail on lost leadership instead of on what they check. A test thread takes one slot
//! when it starts its first cluster and keeps it until the thread (the test) ends.
//! `KV9_TEST_CLUSTER_SLOTS` (default 4) sets the limit.

use std::cell::RefCell;
use std::sync::{Condvar, Mutex, OnceLock};

struct Slots {
    used: Mutex<usize>,
    freed: Condvar,
    limit: usize,
}

fn slots() -> &'static Slots {
    static SLOTS: OnceLock<Slots> = OnceLock::new();
    SLOTS.get_or_init(|| Slots {
        used: Mutex::new(0),
        freed: Condvar::new(),
        limit: std::env::var("KV9_TEST_CLUSTER_SLOTS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(4),
    })
}

struct Held;

impl Drop for Held {
    fn drop(&mut self) {
        let s = slots();
        *s.used.lock().expect("cluster slots poisoned") -= 1;
        s.freed.notify_one();
    }
}

thread_local! {
    static HELD: RefCell<Option<Held>> = const { RefCell::new(None) };
}

/// Take this test thread's cluster slot (once; later calls are free).
pub(crate) fn take() {
    HELD.with(|held| {
        if held.borrow().is_some() {
            return;
        }
        let s = slots();
        let mut used = s.used.lock().expect("cluster slots poisoned");
        while *used >= s.limit {
            used = s.freed.wait(used).expect("cluster slots poisoned");
        }
        *used += 1;
        *held.borrow_mut() = Some(Held);
    });
}
