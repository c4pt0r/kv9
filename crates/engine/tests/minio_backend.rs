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

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use kv9_common::Error;
use kv9_engine::minio::{
    MinioConfig, MinioObjectStore, TestCompletion, ENV_BUCKET, ENV_ENDPOINT, REPLY_RESERVE,
    WORKER_THREAD_NAME,
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
            TestCompletion::WorkerReply,
            "attempt {attempt}: the backend's own error must win the race with the outer net"
        );
    }
}

/// Cell 2. The budget algebra, at its boundaries.
///
/// Needs no server, so it is not `#[ignore]`d — it is arithmetic, and arithmetic is exactly
/// where the previous versions were wrong. One asserted `internal < caller` on a single
/// store under a comment claiming the ordering held "at every input"; at 1ms and 0 it did
/// not. Another had two mutually-redundant refusal paths, so no single mutation could redden
/// it. This pins the four guarantees at the exact points where they change.
#[test]
fn the_budget_algebra_holds_at_its_boundaries() {
    let reserve = REPLY_RESERVE;
    let tick = Duration::from_nanos(1);
    let configured = MinioObjectStore::configured_budget_for_test(Duration::from_secs(10));

    // left <= reserve -> None. Both the exact boundary and below it.
    for left in [Duration::ZERO, reserve - tick, reserve] {
        assert_eq!(
            MinioObjectStore::wire_budget_for_test(left, configured),
            None,
            "left={left:?} is not more than the {reserve:?} reserve and must not be issued"
        );
    }

    // The first input that IS issuable — boundary + one tick. This is the cell that would
    // catch an off-by-one turning the comparison into `<`.
    let just_over = reserve + tick;
    let wire = MinioObjectStore::wire_budget_for_test(just_over, configured)
        .expect("one tick past the reserve must be issuable");
    assert!(wire > Duration::ZERO, "wire must be non-zero, got {wire:?}");
    assert!(
        wire + reserve <= just_over,
        "{wire:?} + {reserve:?} exceeds the {just_over:?} available"
    );

    // Across the whole usable range, all four guarantees at once.
    for left in [
        reserve + tick,
        Duration::from_millis(2),
        Duration::from_millis(50),
        Duration::from_secs(1),
        Duration::from_secs(10),
        Duration::from_secs(30),
    ] {
        let wire = MinioObjectStore::wire_budget_for_test(left, configured)
            .unwrap_or_else(|| panic!("{left:?} is past the reserve and must be issuable"));
        assert!(
            wire > Duration::ZERO,
            "left={left:?}: wire must be non-zero"
        );
        assert!(
            wire + reserve <= left,
            "left={left:?}: {wire:?} + {reserve:?} does not fit"
        );
        assert!(
            wire <= configured,
            "left={left:?}: {wire:?} exceeds the configured ceiling {configured:?}"
        );
    }

    // The ceiling binds for a job with lots of time, the remainder binds for a job with
    // little. Both branches of the `min` must be exercised, or one of them is unwitnessed.
    let small = Duration::from_millis(2);
    assert_eq!(
        MinioObjectStore::wire_budget_for_test(small, configured),
        Some(small - reserve),
        "with little left, the remainder binds"
    );
    assert_eq!(
        MinioObjectStore::wire_budget_for_test(Duration::from_secs(30), configured),
        Some(configured),
        "with plenty left, the configured ceiling binds"
    );
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
        .filter(|c| **c == TestCompletion::AdmissionRefused)
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

/// A peer that accepts, reads the request, and answers `503` immediately — forever.
///
/// Distinct from [`black_hole`] on purpose. A silent peer is bounded by the client's
/// *request* timeout; a peer that answers fast and retryably is bounded by its **retry**
/// budget, so this is the scenario that exercises the retry loop rather than one long wait.
/// That matters because the retry loop is where the overshoot lives: `retry_timeout` bounds
/// when the client stops STARTING attempts, so a backoff begun just inside the window runs
/// to completion outside it.
fn retry_storm() -> (String, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let addr = listener.local_addr().expect("local addr");
    let served = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));

    let served_thread = Arc::clone(&served);
    let stop_thread = Arc::clone(&stop);
    std::thread::spawn(move || {
        while !stop_thread.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut sock, _)) => {
                    let mut buf = [0u8; 4096];
                    let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
                    let _: std::io::Result<usize> = sock.read(&mut buf);
                    // 503 is in upstream's retryable class, so each one costs a backoff
                    // rather than ending the operation.
                    let _ = sock.write_all(
                        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    let _ = sock.flush();
                    served_thread.fetch_add(1, Ordering::SeqCst);
                }
                Err(_) => break,
            }
        }
    });

    (format!("http://{addr}"), served, stop)
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn a_retrying_backend_still_answers_within_the_callers_deadline() {
    // Cell 5, and the point of it is DETERMINISM. Cell 1 witnesses the same worker-bound
    // property against a refused port, but only 2 runs in 10 -- the overshoot it depends on
    // is a tail. Against a peer that answers 503 immediately and forever, the retry loop is
    // the whole cost, so the overshoot stops being rare.
    let (addr, served, stop) = retry_storm();
    let deadline = Duration::from_secs(3);
    let s = MinioObjectStore::connect(
        MinioConfig::from_env_at(&addr, required(ENV_BUCKET))
            .expect("credentials from the environment")
            .with_op_deadline(deadline),
    )
    .expect("connect");

    let started = Instant::now();
    let (source, outcome) = s.put_traced(&key("retry/object"), b"payload");
    let elapsed = started.elapsed();

    assert!(outcome.is_err(), "a permanently-503 peer cannot succeed");
    assert!(
        served.load(Ordering::SeqCst) > 1,
        "the peer served {} request(s); with fewer than two the retry loop was never \
         exercised and this cell witnesses nothing",
        served.load(Ordering::SeqCst)
    );
    assert_eq!(
        source,
        TestCompletion::WorkerReply,
        "the worker must answer within its own bound; reaching the outer net means the \
         operation outlived the caller's deadline"
    );
    assert!(
        elapsed < deadline,
        "answered after {elapsed:?}, past the {deadline:?} deadline"
    );

    stop.store(true, Ordering::SeqCst);
    let _ = TcpStream::connect(addr.trim_start_matches("http://"));
}

/// A store pointed at an endpoint that is never contacted.
///
/// The controlled cells below never touch the network: a `Hold` job sleeps inside the worker
/// for a duration the test chose. So the endpoint only has to be syntactically valid.
fn controlled_store(deadline: Duration) -> MinioObjectStore {
    MinioObjectStore::connect(
        MinioConfig::from_env_at("http://127.0.0.1:1", required(ENV_BUCKET))
            .expect("credentials from the environment")
            .with_op_deadline(deadline),
    )
    .expect("connect performs no I/O")
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn connect_refuses_a_deadline_too_small_to_split() {
    // Blocker 1. The algebra cell proves the PREDICATE is right; it does not prove production
    // uses it. Tess demonstrated the gap by replacing the worker's
    // `wire_budget(config.op_deadline, configured)` with an unconditional `Some(configured)`
    // — every no-fixture test stayed green, because none of them went through `connect`.
    //
    // This one does. It is the call site, not the helper.
    for too_small in [
        Duration::ZERO,
        Duration::from_micros(500),
        REPLY_RESERVE, // exactly the reserve: nothing left for the wire
    ] {
        let result = MinioObjectStore::connect(
            MinioConfig::from_env_at("http://127.0.0.1:1", required(ENV_BUCKET))
                .expect("credentials from the environment")
                .with_op_deadline(too_small),
        );
        let err = match result {
            Ok(_) => panic!("{too_small:?} cannot yield a wire budget and must be refused"),
            Err(e) => e,
        };
        // Typed, and typed as CONFIGURATION — this is a bad setting, not a storage fault, and
        // a caller distinguishing the two acts differently on each.
        assert!(
            matches!(err, Error::Config(_)),
            "{too_small:?} must be refused as a config error, got {err:?}"
        );
    }

    // The OTHER side of the boundary, and it is not the same check as the 50ms control.
    // Credit: Cindy. A 50ms control only rules out "refuses everything"; it does not rule out
    // "refuses a little too much". An off-by-one leaning toward rejection -- say the gate
    // became `d <= RESERVE + 1ms` -- still rejects 0/500us/RESERVE and still accepts 50ms, so
    // every value above would stay green while the gate turned away legitimate deadlines.
    //
    // This is Tess's blocker 1 one level down: the predicate's boundary being right does not
    // make the production gate's boundary right. Same values as the helper's boundary cell,
    // applied at the call site.
    for acceptable in [
        REPLY_RESERVE + Duration::from_nanos(1),
        Duration::from_millis(50),
    ] {
        let result = MinioObjectStore::connect(
            MinioConfig::from_env_at("http://127.0.0.1:1", required(ENV_BUCKET))
                .expect("credentials from the environment")
                .with_op_deadline(acceptable),
        );
        assert!(
            result.is_ok(),
            "{acceptable:?} is past the reserve and must connect; got {:?}",
            result.err()
        );
    }
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn the_worker_cuts_short_an_operation_that_outlives_its_budget() {
    // Cell 4, deterministic at last. Every earlier attempt at this used a real peer --
    // refused, silent, 503 -- and every one witnessed the bound only as a tail (2 in 10 at
    // best), because real peers have probabilistic timing. A future that sleeps for a
    // duration the TEST picked removes the probability: hold longer than the budget and the
    // worker must cut it short, or it must not.
    let deadline = Duration::from_secs(2);
    let s = controlled_store(deadline);

    // Comfortably longer than any budget derivable from a 2s deadline.
    let (source, outcome) = s.hold_traced(Duration::from_secs(60));

    assert!(
        outcome.is_err(),
        "an operation that never finishes cannot succeed"
    );
    assert_eq!(
        source,
        TestCompletion::WorkerReply,
        "the WORKER must cut it short and answer; reaching the outer net means the bound did \
         not hold"
    );
}

#[test]
#[ignore = "requires a MinIO endpoint; see the module docs"]
fn a_job_whose_deadline_ran_out_while_queued_is_not_issued() {
    // Cell 3. Deterministic, and the arithmetic is the point.
    //
    // ONE occupying job can never near-expire the next: the occupant is itself bounded, so
    // the job behind it still has a fraction of its own deadline.
    //
    // The derivation, under the CURRENT formula `min(configured, left - reserve)` with
    // D=200ms, configured=160ms, reserve=1ms:
    //
    //     job 1  left=200ms  issued, budget=min(160, 199)=160ms
    //     job 2  left= 40ms  issued, budget=min(160,  39)= 39ms
    //     job 3  left=  1ms  DECLINED  (left <= reserve)
    //
    // THREE jobs suffice, and arithmetically must — but only if their deadlines share an
    // origin. An earlier version of this comment carried the sequence 200/40/8/1.6/0.32ms,
    // which was computed under the OLD `0.8*left` rule and would send the next reader to
    // add jobs. The count is not the problem (credit: Cindy recomputed it).
    //
    // KNOWN LIMITATION, and it is the real one: this is a SCHEDULING fixture. `Hold` controls
    // how long an issued job runs; it does not control when each caller stamps its deadline
    // and enqueues, so there is no common origin and therefore no genuine "behind" relation.
    // That, not the job count, is why `declined` is a frequency here rather than a certainty.
    // The replacement is a barrier that completes every stamp/enqueue before the worker is
    // released; after it, `declined >= 1` should hold every run, and if it still flakes then
    // something else is uncontrolled.
    let deadline = Duration::from_millis(200);
    let s = Arc::new(controlled_store(deadline));

    let (issued_before, declined_before) = MinioObjectStore::issued_counts();

    // Each asks to be held far longer than any budget it could receive, so every issued job
    // is cut short at exactly its budget and the decay is driven by the formula, not by how
    // long a peer happened to take.
    let mut handles = Vec::new();
    for _ in 0..4 {
        let s = Arc::clone(&s);
        handles.push(std::thread::spawn(move || {
            s.hold_traced(Duration::from_secs(30))
        }));
    }
    let sources: Vec<TestCompletion> = handles
        .into_iter()
        .map(|h| h.join().expect("client thread").0)
        .collect();

    // Counters are read AFTER the store is dropped, and that ordering is load-bearing.
    // `Drop` closes the channel and joins the worker, which drains whatever is still queued.
    // Reading before that gave "2 issued, 0 declined" -- six jobs had not been reached yet,
    // and the earlier version of this test read the counters there and failed 9 runs in 12.
    let s = Arc::try_unwrap(s).expect("all client threads joined");
    drop(s);

    let (issued_after, declined_after) = MinioObjectStore::issued_counts();
    let issued = issued_after - issued_before;
    let declined = declined_after - declined_before;

    // Positive control: something WAS issued. Without it, "at least one declined" is also
    // satisfied by a store that issues nothing, and nothing-happened is what a broken fixture
    // produces too.
    assert!(
        issued >= 1,
        "nothing was issued ({issued} issued, {declined} declined); the fixture never \
         exercised the worker, so the assertion below would be vacuous"
    );
    assert!(
        sources.contains(&TestCompletion::WorkerReply),
        "no job was issued and answered; sources were {sources:?}"
    );

    assert!(
        declined >= 1,
        "no job ran out of deadline while queued ({issued} issued, {declined} declined); \
         with four jobs behind one another the tail must fall under the reserve"
    );

    // NOT asserted: that a CALLER saw `NotIssued`.
    //
    // It is nearly unobservable from the caller by construction, and that is a property of
    // the design rather than a gap in the fixture. A job is declined exactly when at most
    // `REPLY_RESERVE` of its deadline is left -- which is also the instant its caller stops
    // waiting. So the caller has at most 1ms to receive the answer, and usually gets its own
    // `OuterFallback` first. Requiring it here is asking a 1ms race to be won every run.
    //
    // The worker-side counter is the honest witness: it records the decision regardless of
    // whether anyone is still listening. The `NotIssued` variant still earns its place --
    // without it the decline would be reported as `WorkerReply`, i.e. "someone answered"
    // about a case where nobody did -- but the acceptance for it is the counter.
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

// Phase B replaces the temporary source-name ban with the capability's
// compile-fail documentation tests and real upload/recovery tests in checkpoint.rs.
