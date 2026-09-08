//! Behaviour of the MinIO backend against a **real** server.
//!
//! # How to run
//!
//! Every test here is `#[ignore]`d, so `cargo test` reports them in the *ignored* count
//! rather than silently passing without a server. To run them:
//!
//! ```text
//! export KV9_OBJECT_STORE_ENDPOINT=http://127.0.0.1:19000
//! export KV9_OBJECT_STORE_BUCKET=kv9-test
//! export KV9_OBJECT_STORE_ACCESS_KEY=...      # never committed, never logged
//! export KV9_OBJECT_STORE_SECRET_KEY=...
//! cargo test -p kv9-engine --test minio_backend -- --ignored
//! ```
//!
//! With `--ignored` and no endpoint configured these **fail**, they do not skip. A test that
//! quietly returns when its fixture is absent is indistinguishable from one that ran and
//! passed, and the whole point of this file is to distinguish those two.
//!
//! # Why a real server is required rather than a mock
//!
//! The properties under test are properties of the *endpoint*, not of this crate's code:
//!
//! - `put` enforces write-once by sending `If-None-Match: *`. A mock would answer however
//!   it was written to answer. A server that ignored the precondition would overwrite, and
//!   only a real request finds that out.
//! - `list` is a **string**-prefix operation in our trait and a **path-segment**-prefix
//!   operation upstream. The reconciliation is only witnessed against a real lister.
//! - Cross-session visibility is a property of the store, and a second handle inside one
//!   client can be served from that client's own state.

use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use kv9_common::Error;
use kv9_engine::minio::{
    Completion, MinioConfig, MinioObjectStore, ENV_BUCKET, ENV_ENDPOINT, WORKER_THREAD_NAME,
};
use kv9_engine::{ObjectKey, ObjectStore};

const SKIP_IS_NOT_A_PASS: &str = "\
this test was run with --ignored but the MinIO endpoint is not configured. \
It fails rather than skips: a quiet return would be indistinguishable from a pass. \
Set KV9_OBJECT_STORE_ENDPOINT / _BUCKET / _ACCESS_KEY / _SECRET_KEY and re-run.";

fn required(name: &str) -> String {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => v,
        _ => panic!("{name}: {SKIP_IS_NOT_A_PASS}"),
    }
}

/// A store against the configured endpoint.
fn store() -> MinioObjectStore {
    // Read through the public env path so the test exercises the same variables an operator
    // sets. A short deadline: every test here is local, and a hang should surface as a
    // failure inside the suite rather than as a suite that never returns.
    let config = MinioConfig::from_env()
        .unwrap_or_else(|e| panic!("{e}\n{SKIP_IS_NOT_A_PASS}"))
        .with_op_deadline(Duration::from_secs(10));
    MinioObjectStore::connect(config).expect("connect to the configured endpoint")
}

/// A key prefix unique to this process and call, so a rerun cannot collide with the objects
/// a previous run left behind. That matters more here than usual: `put` is write-once, so a
/// leftover object would make the mismatch test see a *different* first write than the one
/// it just performed.
fn unique_prefix(tag: &str) -> String {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("clock is after the epoch")
        .as_nanos();
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("it/{tag}/{nanos}-{}-{n}", std::process::id())
}

fn key(s: &str) -> ObjectKey {
    ObjectKey::new(s).expect("non-empty key")
}

// ---------------------------------------------------------------------------------------
// Round trip and cross-session visibility
// ---------------------------------------------------------------------------------------

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn a_second_independent_client_sees_what_the_first_wrote() {
    let writer = store();
    let k = key(&format!("{}/object", unique_prefix("visibility")));
    writer.put(&k, b"the bytes").expect("put");

    // The writer reading its own write proves little: one client can answer from its own
    // state. The claim under test is that the bytes are in the STORE, so the reader is a
    // separate `MinioObjectStore` -- separate worker thread, separate HTTP client, separate
    // connection pool and session.
    let reader = MinioObjectStore::connect(
        MinioConfig::from_env_at(required(ENV_ENDPOINT), required(ENV_BUCKET))
            .expect("credentials from the environment")
            .with_op_deadline(Duration::from_secs(10)),
    )
    .expect("second client connects");

    assert_eq!(
        reader.get(&k).expect("second-client get").as_deref(),
        Some(&b"the bytes"[..]),
        "a write acknowledged to one client must be visible to an independent one"
    );

    writer.delete(&k).expect("cleanup");
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn absent_object_reads_as_none_not_error() {
    let s = store();
    let k = key(&format!("{}/never-written", unique_prefix("absent")));
    assert!(s
        .get(&k)
        .expect("get of an absent object is not an error")
        .is_none());
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn reput_of_identical_content_is_idempotent() {
    // An upload retried after a timeout, or reissued by a node that has since lost
    // leadership, is a normal event.
    let s = store();
    let k = key(&format!("{}/object", unique_prefix("idempotent")));
    s.put(&k, b"payload").expect("first put");
    s.put(&k, b"payload")
        .expect("identical re-put must be accepted");
    assert_eq!(s.get(&k).expect("get").as_deref(), Some(&b"payload"[..]));
    s.delete(&k).expect("cleanup");
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn reput_with_different_content_is_refused_by_the_real_endpoint() {
    // The load-bearing test of this file. `put` relies on a conditional create; if this
    // endpoint ignored `If-None-Match: *`, the second put would silently overwrite bytes a
    // committed manifest already points at, and nothing else here would notice.
    let s = store();
    let k = key(&format!("{}/object", unique_prefix("mismatch")));
    s.put(&k, b"original").expect("first put");

    let err = s
        .put(&k, b"different")
        .expect_err("a second, different object under one file-id must be refused");
    assert!(
        matches!(err, Error::ObjectContentMismatch { .. }),
        "expected a typed mismatch; got {err:?}. A success here means the endpoint ignored \
         the conditional write and write-once is NOT enforced."
    );

    // The refusal must not be a partial write: the original has to survive it.
    assert_eq!(s.get(&k).expect("get").as_deref(), Some(&b"original"[..]));
    s.delete(&k).expect("cleanup");
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn delete_is_idempotent_and_absent_delete_is_ok() {
    let s = store();
    let k = key(&format!("{}/object", unique_prefix("delete")));
    s.put(&k, b"payload").expect("put");
    s.delete(&k).expect("first delete");
    s.delete(&k).expect("second delete of the same key");
    s.delete(&key(&format!("{}/never", unique_prefix("delete"))))
        .expect("delete of a key that never existed");
    assert!(s.get(&k).expect("get").is_none());
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn list_is_a_string_prefix_and_sorted_not_a_segment_prefix() {
    let s = store();
    let base = unique_prefix("list");
    // `sst_other` is the discriminating case: it is a string-prefix match for `sst` but a
    // different path SEGMENT, so a backend that pushed the literal down unchanged would
    // return it for one query and not the other, or miss `sst/00...` entirely.
    let names = [
        format!("{base}/sst/000003"),
        format!("{base}/sst/000001"),
        format!("{base}/sst/000002"),
        format!("{base}/sst_other/1"),
        format!("{base}/snap/1"),
    ];
    for n in &names {
        s.put(&key(n), b"x").expect("put");
    }

    let listed = |prefix: &str| -> Vec<String> {
        s.list(prefix)
            .expect("list")
            .into_iter()
            .map(|k| k.to_string())
            .collect()
    };

    assert_eq!(
        listed(&format!("{base}/sst/")),
        [
            format!("{base}/sst/000001"),
            format!("{base}/sst/000002"),
            format!("{base}/sst/000003"),
        ],
        "sorted, and `sst_other` is not under `sst/`"
    );

    // A prefix that cuts INSIDE a segment. Upstream's lister cannot express this; the
    // backend pushes down `{base}/sst` and filters. Handing the literal down unchanged
    // returns nothing at all here.
    assert_eq!(
        listed(&format!("{base}/sst/00000")),
        [
            format!("{base}/sst/000001"),
            format!("{base}/sst/000002"),
            format!("{base}/sst/000003"),
        ]
    );
    assert_eq!(
        listed(&format!("{base}/sst/000002")),
        [format!("{base}/sst/000002")]
    );

    // And a prefix that cuts inside a segment ONE LEVEL UP, where the string-prefix rule
    // includes `sst_other` and the segment rule would not.
    let mut expected: Vec<String> = names
        .iter()
        .filter(|n| n.starts_with(&format!("{base}/sst")))
        .cloned()
        .collect();
    expected.sort();
    assert_eq!(listed(&format!("{base}/sst")), expected);
    assert!(
        expected.iter().any(|n| n.contains("sst_other")),
        "the fixture must actually contain the discriminating key, or this assertion is \
         vacuous"
    );

    for n in &names {
        s.delete(&key(n)).expect("cleanup");
    }
}

// ---------------------------------------------------------------------------------------
// Counter-cases: an unresponsive or absent endpoint must never read as success
// ---------------------------------------------------------------------------------------

/// A listener that accepts connections and then says nothing, ever.
///
/// Returns the address and a flag set once a connection has actually been accepted. The flag
/// is what makes the black-hole test conclusive: without it, a timeout is equally well
/// explained by "nothing was listening and the connect itself hung", and the test would pass
/// while proving nothing about how the backend handles a *silent* peer.
fn black_hole() -> (String, Arc<AtomicBool>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let accepted = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));

    let accepted_thread = Arc::clone(&accepted);
    let stop_thread = Arc::clone(&stop);
    std::thread::spawn(move || {
        let mut held = Vec::new();
        while !stop_thread.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut sock, _)) => {
                    // Read the request so the client's write completes and it is genuinely
                    // waiting on a RESPONSE, not on backpressure.
                    let mut buf = [0u8; 1024];
                    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
                    let _: std::io::Result<usize> = sock.read(&mut buf);
                    accepted_thread.store(true, Ordering::SeqCst);
                    // Hold the socket open: closing it would send FIN and turn this into a
                    // connection-reset case, which is a different counter-case.
                    held.push(sock);
                }
                Err(_) => break,
            }
        }
    });

    (format!("http://{addr}"), accepted, stop)
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn a_silent_endpoint_times_out_and_never_reports_success() {
    // Ordered deliberately: the real round trip runs FIRST, in this same process. Once it
    // has succeeded, "the environment had not come up yet" is structurally unavailable as an
    // explanation for the failure below.
    let real = store();
    let k = key(&format!("{}/probe", unique_prefix("silent")));
    real.put(&k, b"real round trip").expect("real put");
    assert_eq!(
        real.get(&k).expect("real get").as_deref(),
        Some(&b"real round trip"[..])
    );
    real.delete(&k).expect("cleanup");

    let (addr, accepted, stop) = black_hole();
    let s = MinioObjectStore::connect(
        MinioConfig::from_env_at(&addr, required(ENV_BUCKET))
            .expect("credentials from the environment")
            .with_op_deadline(Duration::from_secs(2)),
    )
    .expect("connect builds a client without I/O");

    let started = Instant::now();
    let err = s
        .put(&key("silent/object"), b"payload")
        .expect_err("a silent endpoint must never be reported as a successful write");
    let elapsed = started.elapsed();

    assert!(
        accepted.load(Ordering::SeqCst),
        "the black hole never accepted a connection, so this timeout says nothing about how \
         a silent peer is handled"
    );
    assert!(
        elapsed < Duration::from_secs(20),
        "the deadline did not bound the wait: {elapsed:?}"
    );
    // The message must say what happened. A bare "put failed" would send the reader looking
    // for a storage fault.
    let text = format!("{err}");
    assert!(
        text.contains("object store"),
        "the error must name the subsystem: {text}"
    );

    stop.store(true, Ordering::SeqCst);
    // Wake the accept loop so the helper thread can observe `stop`.
    let _ = TcpStream::connect(addr.trim_start_matches("http://"));
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn a_closed_port_is_an_error_not_a_silent_success() {
    // Distinct from the silent case: here the connection is refused outright. Both must
    // fail, and neither may be mistaken for the other by the code under test.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener); // nothing is listening now

    let s = MinioObjectStore::connect(
        MinioConfig::from_env_at(format!("http://{addr}"), required(ENV_BUCKET))
            .expect("credentials from the environment")
            .with_op_deadline(Duration::from_secs(5)),
    )
    .expect("connect performs no I/O, so it succeeds even against a dead port");

    // Worth stating: `connect` returning Ok above is itself part of the contract -- it
    // builds a client, it does not prove reachability. The failure has to come from the
    // operation.
    s.put(&key("refused/object"), b"payload")
        .expect_err("a refused connection must not read as a successful write");
    assert!(
        s.get(&key("refused/object")).is_err(),
        "a refused connection must not read as `absent`, which is a normal answer"
    );
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn credentials_never_appear_in_an_error_message() {
    // The failure path is where a secret most plausibly escapes: a client error carrying a
    // signed request, or a config error echoing what it was given.
    let secret = required("KV9_OBJECT_STORE_SECRET_KEY");
    let access = required("KV9_OBJECT_STORE_ACCESS_KEY");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);

    let s = MinioObjectStore::connect(
        MinioConfig::from_env_at(format!("http://{addr}"), required(ENV_BUCKET))
            .expect("credentials from the environment")
            .with_op_deadline(Duration::from_secs(5)),
    )
    .expect("connect");

    let err = s.put(&key("leak/object"), b"payload").unwrap_err();
    let text = format!("{err}");
    let debug = format!("{err:?}");
    for form in [&text, &debug] {
        assert!(
            !form.contains(&secret),
            "the secret key reached an error message"
        );
        assert!(
            !form.contains(&access),
            "the access key reached an error message"
        );
    }
    // Also the store's own Debug, which is what a log line would format.
    let store_debug = format!("{s:?}");
    assert!(!store_debug.contains(&secret) && !store_debug.contains(&access));
}

// ---------------------------------------------------------------------------------------
// Deadline budgets — three independent properties, bound to the SOURCE of the outcome
//
// Not to error text. The text is prose and gets reworded; which of the three ways an
// operation finished is the property under test. `contains("timed out")` keeps passing
// after a reword and stops meaning anything.
// ---------------------------------------------------------------------------------------

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn a_refused_endpoint_answers_from_the_worker_not_the_outer_net() {
    // Cell 1. A backend that fails fast must produce the WORKER's answer -- the specific one
    // naming what went wrong -- not the generic outer timeout.
    //
    // This was a coin flip before the internal budget was made strictly shorter: measured 8
    // caught / 2 missed over ten runs against a refused port, and the two misses were the
    // runs where the outer net fired first.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);

    let s = MinioObjectStore::connect(
        MinioConfig::from_env_at(format!("http://{addr}"), required(ENV_BUCKET))
            .expect("credentials from the environment")
            .with_op_deadline(Duration::from_secs(5)),
    )
    .expect("connect");

    // Repeated, because the defect this guards was intermittent. One run of a coin flip
    // proves nothing about the coin.
    for attempt in 0..5 {
        let (source, outcome) = s.put_traced(&key("refused/object"), b"payload");
        assert!(
            outcome.is_err(),
            "attempt {attempt}: refused port must fail"
        );
        assert_eq!(
            source,
            Completion::WorkerReply,
            "attempt {attempt}: the backend's own error must win the race with the outer net"
        );
    }
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn the_internal_budget_is_strictly_shorter_than_the_callers_deadline() {
    // Cell 2. The ORDERING is the property; the fraction is an implementation choice. An
    // assertion on the fraction would re-pin the constant and go red on a legitimate tuning
    // change while saying nothing about what matters.
    let s = store();
    assert!(
        s.internal_budget_for_test() < s.op_deadline_for_test(),
        "internal {:?} must be strictly under caller {:?}, or the two expire together and \
         which error the caller sees becomes a scheduling race",
        s.internal_budget_for_test(),
        s.op_deadline_for_test()
    );
    // Non-degenerate: an internal budget of zero would also satisfy `<` while making every
    // request fail instantly.
    assert!(s.internal_budget_for_test() > Duration::ZERO);
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn a_full_queue_refuses_admission_instead_of_parking_the_caller() {
    // Cell 3. A blocking send would park the caller for an unbounded time that is NOT
    // charged against its deadline -- it would then wait queue-time plus the full deadline.
    // Refusal is also the more useful answer: a full queue is backpressure a caller can act
    // on, a silent stall is not.
    let (addr, accepted, stop) = black_hole();
    let s = Arc::new(
        MinioObjectStore::connect(
            MinioConfig::from_env_at(&addr, required(ENV_BUCKET))
                .expect("credentials from the environment")
                .with_op_deadline(Duration::from_secs(4)),
        )
        .expect("connect"),
    );

    // Enough concurrent callers to overrun a 32-deep queue against a peer that never
    // answers. Each thread reports its own source so the assertion is about what the
    // STORE did, not about how many threads happened to be scheduled.
    let sources = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut handles = Vec::new();
    for i in 0..80 {
        let s = Arc::clone(&s);
        let sources = Arc::clone(&sources);
        handles.push(std::thread::spawn(move || {
            let (source, outcome) = s.put_traced(&key(&format!("full/{i}")), b"payload");
            assert!(outcome.is_err(), "a silent peer cannot succeed");
            sources.lock().expect("not poisoned").push(source);
        }));
    }
    for h in handles {
        h.join().expect("client thread");
    }
    assert!(
        accepted.load(Ordering::SeqCst),
        "the black hole never accepted, so the queue was never actually held open"
    );

    let sources = sources.lock().expect("not poisoned");
    let refused = sources
        .iter()
        .filter(|c| **c == Completion::AdmissionRefused)
        .count();
    assert!(
        refused > 0,
        "with 80 callers against a 32-deep queue and a peer that never answers, some must be \
         refused admission rather than parked; sources were {sources:?}"
    );

    let s = Arc::try_unwrap(s).expect("sole handle");
    drop(s);
    stop.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(addr.trim_start_matches("http://"));
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn queue_time_is_charged_to_the_callers_deadline() {
    // Cell 4. `submit` must wait only what REMAINS of the deadline, not a fresh full one
    // after the hand-off. Waiting a fresh one means a caller that queued behind a busy
    // worker waits queue-time PLUS the deadline -- so the deadline stops meaning "from when
    // I asked", which is the only thing a caller can reason about.
    //
    // This cell exists because the discrimination matrix found that reversing this property
    // reddened NOTHING: the three cells I had all passed with the bug reinstated. An
    // unwitnessed property is not a property, and the matrix is what surfaced it.
    let (addr, accepted, stop) = black_hole();
    let deadline = Duration::from_secs(3);
    let s = Arc::new(
        MinioObjectStore::connect(
            MinioConfig::from_env_at(&addr, required(ENV_BUCKET))
                .expect("credentials from the environment")
                .with_op_deadline(deadline),
        )
        .expect("connect"),
    );

    // Occupy the worker so the next caller genuinely queues. One is enough: the worker is
    // single-threaded by construction.
    let hog = {
        let s = Arc::clone(&s);
        std::thread::spawn(move || {
            let _ = s.put_traced(&key("queued/hog"), b"payload");
        })
    };
    // Let the hog reach the worker before timing the caller behind it.
    std::thread::sleep(Duration::from_millis(300));

    let started = Instant::now();
    let (_source, outcome) = s.put_traced(&key("queued/behind"), b"payload");
    let elapsed = started.elapsed();
    assert!(outcome.is_err(), "a silent peer cannot succeed");

    assert!(
        accepted.load(Ordering::SeqCst),
        "the black hole never accepted, so nothing was actually stalled and nothing queued"
    );
    // Under the bug this is queue-wait + a fresh full deadline, i.e. close to 2x. The bound
    // is generous on purpose: the failure it catches is a doubling, not a few hundred
    // milliseconds of scheduling noise.
    assert!(
        elapsed < deadline + deadline / 2,
        "a caller behind a busy worker waited {elapsed:?} against a {deadline:?} deadline; \
         queue time is not being charged to the caller"
    );

    hog.join().expect("hog thread");
    let s = Arc::try_unwrap(s).expect("sole handle");
    drop(s);
    stop.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(addr.trim_start_matches("http://"));
}

// ---------------------------------------------------------------------------------------
// Structural probes
// ---------------------------------------------------------------------------------------

/// Number of live threads in this process carrying `name`.
///
/// Linux-only, via `/proc/self/task`. This is an observation of the running process, not a
/// value the store reports about itself -- the distinction that matters, since a store
/// self-reporting "I use one worker" would be the same kind of unverifiable claim that was
/// rejected for object visibility.
#[cfg(target_os = "linux")]
fn threads_named(name: &str) -> usize {
    let mut count = 0;
    let dir = std::fs::read_dir("/proc/self/task").expect("procfs is mounted");
    for entry in dir.flatten() {
        if let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) {
            if comm.trim() == name {
                count += 1;
            }
        }
    }
    count
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn concurrent_operations_share_one_worker_thread_which_is_reaped_on_drop() {
    let before = threads_named(WORKER_THREAD_NAME);

    let s = Arc::new(store());
    assert_eq!(
        threads_named(WORKER_THREAD_NAME),
        before + 1,
        "one store must own exactly one worker"
    );

    // Positive control for the counter itself: a second store must move the number. Without
    // this, a counter that always returned `before + 1` -- or always zero plus a miscount --
    // would satisfy the assertion above while measuring nothing.
    let second = store();
    assert_eq!(
        threads_named(WORKER_THREAD_NAME),
        before + 2,
        "the thread counter must be able to distinguish one store from two"
    );
    drop(second);

    // Eight concurrent callers against the real endpoint. The count must not move: the
    // whole design is that concurrency is absorbed by the queue, not by more threads.
    let base = unique_prefix("worker");
    let peak = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for i in 0..8 {
        let s = Arc::clone(&s);
        let peak = Arc::clone(&peak);
        let base = base.clone();
        handles.push(std::thread::spawn(move || {
            let k = key(&format!("{base}/{i}"));
            s.put(&k, format!("payload {i}").as_bytes()).expect("put");
            peak.fetch_max(threads_named(WORKER_THREAD_NAME), Ordering::SeqCst);
            s.get(&k).expect("get");
            peak.fetch_max(threads_named(WORKER_THREAD_NAME), Ordering::SeqCst);
            s.delete(&k).expect("cleanup");
        }));
    }
    for h in handles {
        h.join().expect("worker thread test client");
    }

    assert_eq!(
        peak.load(Ordering::SeqCst),
        before + 1,
        "concurrent operations must be absorbed by the queue, not by additional threads"
    );

    let s = Arc::try_unwrap(s).expect("all clients joined, so this is the only handle");
    drop(s);
    assert_eq!(
        threads_named(WORKER_THREAD_NAME),
        before,
        "Drop must join the worker, not leak it"
    );
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn abandoned_requests_do_not_pin_the_worker_after_their_deadline() {
    // Queue several requests against a silent endpoint with a short deadline. Every caller
    // times out. Without the deadline carried on the job, the worker would then work through
    // the whole backlog at one deadline each and `Drop` -- which joins it -- would take
    // roughly N x deadline.
    let (addr, accepted, stop) = black_hole();
    let deadline = Duration::from_secs(2);
    let n = 6;

    let s = Arc::new(
        MinioObjectStore::connect(
            MinioConfig::from_env_at(&addr, required(ENV_BUCKET))
                .expect("credentials from the environment")
                .with_op_deadline(deadline),
        )
        .expect("connect"),
    );

    let mut handles = Vec::new();
    for i in 0..n {
        let s = Arc::clone(&s);
        handles.push(std::thread::spawn(move || {
            s.put(&key(&format!("abandoned/{i}")), b"payload")
                .expect_err("silent endpoint")
        }));
    }
    for h in handles {
        h.join().expect("client thread");
    }
    assert!(
        accepted.load(Ordering::SeqCst),
        "the black hole never accepted, so nothing was actually queued behind a stalled \
         request"
    );

    let s = Arc::try_unwrap(s).expect("sole handle");
    let started = Instant::now();
    drop(s);
    let join_took = started.elapsed();

    // Generous bound: the worker may still be inside the one request it had already issued.
    // The failure this catches is quadratic, not marginal -- 6 x 2s = 12s versus one.
    assert!(
        join_took < deadline * 2,
        "Drop joined after {join_took:?}; abandoned requests are still being executed"
    );

    stop.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(addr.trim_start_matches("http://"));
}

// ---------------------------------------------------------------------------------------
// Stop line
// ---------------------------------------------------------------------------------------

/// Every `.rs` file under `crates/engine/src/`, recursively.
///
/// The whole directory, not a named file. An earlier version read exactly
/// `include_str!("../src/minio.rs")`, and Cindy measured what that misses: declare a second
/// module in `lib.rs`, put `pub struct PreparedSst` in it, and the guard stays green. A
/// backend growing a second file -- upload retries, multipart, a signing helper -- is an
/// ordinary thing to happen, and the guard would quietly stop covering the new code.
///
/// Scoped to the whole crate rather than to a `minio` subtree, per Tess's rev-5 ruling and
/// Cindy's measurement behind it: every module in this workspace is a single file, so a
/// `src/minio/**` glob matches nothing today and the likely next helper is a *sibling*
/// (`src/minio_upload.rs`), which such a glob would miss. A whole-crate scan needs no naming
/// convention and no allowlist -- and it does not over-reach, because before Phase B there is
/// no legitimate place for this type anywhere in the engine.
///
/// `src/` and not `tests/`: this file necessarily contains the string it searches for, and a
/// guard that flags itself is not a guard. The exclusion is principled rather than
/// convenient -- test code cannot mint anything that ships.
///
/// EXPIRY, and it is deliberately wired to something that will refuse to be forgotten. The
/// marker below is the repo's existing mechanism (`scripts/unlanded-check.sh`): task #9's
/// Phase B is what introduces `PreparedSst`, and while this marker is live that task cannot
/// be closed. So the retirement of this tripwire is a checked precondition of the change
/// that makes it wrong, not a note somebody has to remember.
///
///   UNLANDED(task #9) — retire or replace this scan when Phase B lands `PreparedSst`;
///   from that day the whole-crate scope is a false alarm, and a guard that cries wolf
///   gets deleted rather than fixed.
fn engine_sources() -> Vec<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("the engine source tree must be readable: {dir:?}: {e}"));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    walk(&src, &mut out);
    out.sort();
    out
}

#[test]
fn the_backend_does_not_mint_a_prepared_sst() {
    // `PreparedSst` does not exist yet, so no type-level check can be written: you cannot
    // name a type in order to forbid it. What CAN be checked is that no engine source reaches
    // for the name, and this starts failing the moment one does.
    //
    // Stated plainly because it is weak evidence: a source-text check, not a type check. It
    // witnesses the NAME, not the property -- a function that did the minting under another
    // name would pass. When the sealed uploader capability lands, the real guard is that
    // minting is private to it, and this test should be REPLACED rather than kept alongside.
    let files = engine_sources();

    // Positive control for the scanner itself. Without it, a walk that returned an empty list
    // -- wrong directory, an `extension()` typo, a `read_dir` that silently yielded nothing --
    // would report "zero mentions" exactly like a clean tree does.
    assert!(
        files.iter().any(|p| p.ends_with("minio.rs")),
        "the scan must reach the backend it is guarding; it found {files:#?}"
    );
    let control: usize = files
        .iter()
        .map(|p| std::fs::read_to_string(p).expect("readable source"))
        .filter(|s| s.contains("MinioObjectStore"))
        .count();
    assert!(
        control > 0,
        "the scan must actually read file CONTENT, not just walk names"
    );

    // The rule is "no CODE mentions", not "exactly N mentions". Pinning a count was how the
    // single-file version was written, and widening the scope showed why that shape is
    // wrong: `sst.rs` legitimately cites `PreparedSst` in its module docs when explaining
    // whose job the key range is. A count would have to be bumped every time someone writes
    // a sentence, and a number that gets bumped routinely stops being read.
    //
    // Classification is deliberately crude and biased toward flagging: a line counts as
    // documentation only if it *starts* with `//`. A mention in a trailing comment on a line
    // of code is therefore reported. That direction is the safe one -- a false alarm costs a
    // reading, a false clear costs the stop line.
    let mut doc_mentions = Vec::new();
    let mut code_mentions = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).expect("readable source");
        for (i, line) in text.lines().enumerate() {
            if line.contains("PreparedSst") {
                let cited = format!("{}:{}: {}", path.display(), i + 1, line.trim());
                if line.trim_start().starts_with("//") {
                    doc_mentions.push(cited);
                } else {
                    code_mentions.push(cited);
                }
            }
        }
    }

    assert!(
        code_mentions.is_empty(),
        "{}",
        stop_line_failure(&code_mentions)
    );

    // Second positive control. Without it, a classifier that filed EVERYTHING as
    // documentation -- or a `contains` that matched nothing at all -- would leave
    // `code_mentions` empty and read exactly like a clean tree.
    assert!(
        !doc_mentions.is_empty(),
        "the scan found no mention of `PreparedSst` anywhere, not even the module docs that \
         state the stop line. The search is not working."
    );
}

/// The message this guard fails with.
///
/// A function rather than an inline string, so the disposal instruction can be asserted
/// without having to trip the guard. Unlike an ordinary assertion, **this one has an
/// expected, legitimate failure**: the day Phase B introduces `PreparedSst`, it fires and it
/// is right to fire. Whoever hits it that day has the card's ruling somewhere they are not
/// looking and one line of terminal output in front of them.
///
/// So the instruction goes in the output. The failure mode being defended against is not the
/// guard silently ossifying -- it is someone reading an expected red as an ordinary
/// regression and reaching for an allowlist, which is exactly the third source of truth this
/// scope was chosen to avoid.
fn stop_line_failure(found: &[String]) -> String {
    format!(
        "STOP LINE: no source under crates/engine/src may reach for `PreparedSst`.\n\
         Minting that badge belongs to the sealed uploader capability; an acknowledged \
         upload is a necessary part of it, not the whole of it, and a backend that returned \
         one would put the trust root back on a value the store reports about itself.\n\
         \n\
         IF YOU ARE SEEING THIS BECAUSE PHASE B (task #9) IS LEGITIMATELY INTRODUCING \
         `PreparedSst`, THIS RED IS EXPECTED AND THE CORRECT ACTION IS TO RETIRE OR REPLACE \
         THIS GUARD -- in the same reviewed change, together with its UNLANDED(task #9) \
         marker. DO NOT add an allowlist or an exception: this scan covers the whole crate \
         precisely so that there is no second place where the answer lives.\n\
         \n\
         Found {} code mention(s):\n{:#?}",
        found.len(),
        found
    )
}

#[test]
fn the_stop_line_failure_says_what_to_do_when_it_is_right_to_fail() {
    // Rev 6 of the card, and Cindy's reasoning for it: this guard's one legitimate failure
    // scenario WILL happen, and the disposition for it is written on the card -- somewhere
    // the person hitting the red is not looking. The terminal is where they are.
    let text = stop_line_failure(&["src/somewhere.rs:1: pub struct PreparedSst;".to_string()]);

    for required in [
        // that the red may be expected...
        "EXPECTED",
        // ...what to do about it...
        "RETIRE OR REPLACE",
        // ...what NOT to do...
        "DO NOT add an allowlist",
        // ...and where the retirement is tracked.
        "UNLANDED(task #9)",
        // Naming the phase, so the reader can tell whether their case is the expected one.
        "PHASE B",
    ] {
        assert!(
            text.contains(required),
            "the failure message must carry its own disposal instruction; it is missing \
             {required:?}.\nMessage was:\n{text}"
        );
    }

    // And it must still report the finding, or the instruction would have displaced the
    // evidence rather than accompanied it.
    assert!(text.contains("src/somewhere.rs:1"), "{text}");
}
