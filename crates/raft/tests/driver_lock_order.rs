//! Regression for the task-#20 acceptance flake: an AB-BA lock-order deadlock
//! between the pump's apply path and the status surface.
//!
//! `NodeDriver` guards two things every apply touches: the state machine
//! (`sm`) and the applied `(index, term)` ring (`applied`). The pump's
//! `step()` holds both across a batch so readers see watermark and ring move
//! together; `status()` and `wait_applied()` also hold both. Before the fix,
//! `step()` acquired `sm → applied` while the readers acquired `applied → sm`
//! — with the pump and the status writer on different threads (exactly the
//! production shape: the server's run loop polls `status()` every tick), one
//! unlucky interleaving deadlocks both threads. The node then freezes
//! *silently*: raft stops stepping, the FSM stops advancing, the status file
//! stops updating, and nothing reports an error. Under CPU load this hit
//! roughly half of all three-process acceptance runs.
//!
//! This test recreates the two-thread shape with tight loops and no sleeps to
//! maximize interleavings, and fails through a watchdog rather than hanging
//! CI forever. Sensitivity: with the pre-fix lock order (`sm → applied` in
//! `step()`) this test wedges reliably; with the declared order it completes
//! in well under the watchdog.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use kv9_common::{NodeId, RegionId};
use kv9_raft::driver::NodeDriver;
use kv9_raft::transport::{InProcHub, RaftTransport};
use kv9_raft::{cf_code, Command, MemStateMachine, RaftGroup, RaftPeer, Role};
use kv9_engine::ColumnFamily;

const PROPOSALS: u64 = 2_000;
/// Generous: the healthy run takes a few seconds; only a deadlock reaches it.
const WATCHDOG: Duration = Duration::from_secs(120);

#[test]
fn concurrent_status_and_apply_never_deadlock() {
    let (done_tx, done_rx) = mpsc::channel::<Result<(), String>>();

    // The whole scenario runs on detached threads so a deadlock leaves the
    // watchdog free to fail the test instead of hanging the harness.
    std::thread::spawn(move || {
        let result = run_scenario();
        let _ = done_tx.send(result);
    });

    match done_rx.recv_timeout(WATCHDOG) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!("scenario failed: {e}"),
        Err(_) => panic!(
            "watchdog fired after {WATCHDOG:?}: pump and status() are deadlocked \
             (AB-BA on the driver's applied/sm mutexes — see the declared lock order)"
        ),
    }
}

fn run_scenario() -> Result<(), String> {
    let hub = InProcHub::new();
    let peer = Arc::new(
        RaftPeer::new(NodeId(1), RegionId(1), &[NodeId(1)]).map_err(|e| e.to_string())?,
    );
    let endpoint = hub.endpoint(NodeId(1));
    let driver = NodeDriver::new(
        peer,
        Arc::new(endpoint) as Arc<dyn RaftTransport>,
        MemStateMachine::new(),
    );
    driver.peer().campaign().map_err(|e| e.to_string())?;
    for _ in 0..200 {
        driver.tick_and_step().map_err(|e| e.to_string())?;
        if driver.status().role == Role::Leader {
            break;
        }
    }
    if driver.status().role != Role::Leader {
        return Err("single node failed to elect itself".into());
    }

    // Reader thread: the production run loop's shape — status() in a loop.
    // Tight (no sleep) so its applied→sm window overlaps the pump constantly.
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let driver = Arc::clone(&driver);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let _ = driver.status();
            }
        })
    };

    // Writer thread (this one): propose and pump until each entry applies,
    // so step() keeps taking both locks with real committed entries.
    for i in 0..PROPOSALS {
        let cmd = Command::Put {
            cf: cf_code(ColumnFamily::Default),
            key: format!("lock-order-{i}").into_bytes(),
            value: vec![0u8; 16],
        };
        let at = driver.propose(&cmd).map_err(|e| e.to_string())?;
        loop {
            driver.step().map_err(|e| e.to_string())?;
            match driver.wait_applied(at, Duration::from_millis(0)) {
                Ok(true) => break,
                Ok(false) => return Err(format!("proposal {i} overwritten unexpectedly")),
                Err(_) => {} // pending: keep pumping
            }
        }
    }

    stop.store(true, Ordering::Relaxed);
    reader.join().map_err(|_| "reader panicked".to_string())?;
    Ok(())
}
