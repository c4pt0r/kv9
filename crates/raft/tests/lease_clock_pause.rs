//! Actual process-stop check for the concrete clock and lease controllers.
//! This is not host-suspend, independent-clock calibration, RPC or Chaos Mesh.
#![cfg(all(target_os = "linux", feature = "experimental-leader-lease"))]

use kv9_raft::lease::{
    Authority, FixedConfiguration, LeaderLease, Progress, Refused, Timing, VoterLease,
};
use kv9_raft::lease_clock::{ClockBounds, LinuxBoottimeClock};
use kv9_raft::lease_policy::LeasePolicy;
use kv9_raft::rawnode::LeaseClock;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const CHILD_ENV: &str = "KV9_LEASE_CLOCK_PAUSE_CHILD";

#[test]
fn lease_clock_pause_child() {
    if std::env::var(CHILD_ENV).as_deref() != Ok("1") {
        return;
    }
    let p = LeasePolicy {
        node: 1,
        group: 0,
        configuration: 7,
        voters: vec![1],
        promise_ns: 100_000_000,
        drift_ppb: 1_000_000,
        margin_ns: 4_005,
    };
    let clock = LinuxBoottimeClock::new(
        &p,
        ClockBounds {
            drift_ppb: 1_000_000,
            error_ns: 1_000,
        },
    )
    .unwrap();
    let timing = Timing::new(p.promise_ns, p.drift_ppb, p.margin_ns).unwrap();
    let config = FixedConfiguration::new(p.configuration, p.voters.clone()).unwrap();
    let initial = clock.sample().unwrap();
    let mut voter = VoterLease::recover(1, 0, config.clone(), timing, initial, 5).unwrap();
    let mut leader = LeaderLease::new(
        config,
        Authority {
            group: 0,
            configuration: 7,
            leader: 1,
            term: 5,
            incarnation: 1,
        },
        timing,
        initial,
    )
    .unwrap();
    let progress = Progress {
        configuration: 7,
        leader: 1,
        term: 5,
        committed: 1,
        committed_term: 5,
    };
    let setup_deadline = Instant::now() + Duration::from_secs(3);
    while clock.sample().unwrap().nanos < initial.nanos + timing.recovery_ns() {
        assert!(
            Instant::now() < setup_deadline,
            "clock quarantine did not expire"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    let renewal = leader.start(clock.sample().unwrap(), progress).unwrap();
    let grant = voter
        .promise(clock.sample().unwrap(), 5, 1, renewal)
        .unwrap();
    assert!(leader.acknowledge(clock.sample().unwrap(), grant).unwrap());
    leader
        .publish(clock.sample().unwrap(), progress, renewal)
        .unwrap();
    let before = clock.sample().unwrap();
    let deadline = before.nanos + 10_000_000_000;
    let successful = leader.begin_read(before, progress, deadline).unwrap();
    leader
        .finish_read(clock.sample().unwrap(), progress, successful, 1)
        .unwrap();
    let paused = leader
        .begin_read(clock.sample().unwrap(), progress, deadline)
        .unwrap();
    println!("KV9CLOCK_READY {}", before.nanos);
    std::io::stdout().flush().unwrap();
    let mut resume = String::new();
    std::io::stdin().read_line(&mut resume).unwrap();
    assert_eq!(resume, "resume\n");
    let after = clock.sample().unwrap();
    assert_eq!(
        leader.finish_read(after, progress, paused, 1),
        Err(Refused::Expired),
        "process pause preserved expired local read authority"
    );
    assert!(
        matches!(
            leader.begin_read(after, progress, deadline),
            Err(Refused::Expired)
        ),
        "a new read reused authority after process pause"
    );
    voter.may_vote(after, 6).unwrap();
    println!("KV9CLOCK_RESULT {}", after.nanos - before.nanos);
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn tagged_line(lines: &Receiver<String>, tag: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let line = lines
            .recv_timeout(remaining)
            .expect("clock child output deadline");
        if let Some(start) = line.find(tag) {
            return line[start + tag.len()..].trim().to_owned();
        }
    }
}

#[test]
fn actual_process_stop_expires_existing_and_new_lease_reads() {
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lease_clock_pause_child", "--nocapture"])
        .env(CHILD_ENV, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut child = OwnedChild(child);
    let stdout = child.0.stdout.take().unwrap();
    let (sender, lines) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let _before: u64 = tagged_line(&lines, "KV9CLOCK_READY ").parse().unwrap();
    let pid = i32::try_from(child.0.id()).unwrap();
    // SAFETY: The PID is the live child owned above. Only that child receives
    // the signals, and the guard kills/reaps it on every assertion failure.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGSTOP) }, 0);
    let status_path = format!("/proc/{pid}/status");
    let stop_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let status = std::fs::read_to_string(&status_path).unwrap();
        if status.lines().any(|line| line.starts_with("State:\tT")) {
            break;
        }
        assert!(
            Instant::now() < stop_deadline,
            "child was not actually stopped"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(300));
    // SAFETY: Same exclusively owned child as the SIGSTOP above.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGCONT) }, 0);
    child
        .0
        .stdin
        .take()
        .unwrap()
        .write_all(b"resume\n")
        .unwrap();
    let elapsed: u64 = tagged_line(&lines, "KV9CLOCK_RESULT ").parse().unwrap();
    assert!(
        elapsed >= 200_000_000,
        "CLOCK_BOOTTIME did not include the stopped interval"
    );
    let exit_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "clock child failed: {status}");
            break;
        }
        assert!(Instant::now() < exit_deadline, "clock child did not exit");
        std::thread::sleep(Duration::from_millis(1));
    }
    reader.join().unwrap();
    println!(
        "Observed stopped child {pid}; elapsed clock nanoseconds: {elapsed}; expired reads refused"
    );
}
