//! A MinIO / S3 backend for [`ObjectStore`](crate::object_store::ObjectStore).
//!
//! # Why a worker thread
//!
//! [`ObjectStore`](crate::object_store::ObjectStore) is a **synchronous** trait and stays
//! that way: `Engine` and the server core are synchronous contracts, and making the store
//! async would push an async boundary all the way into `Engine` while still needing a
//! bridge back for the synchronous read path.
//!
//! The upstream S3 client is async, so this backend owns **one dedicated worker thread**
//! holding a Tokio runtime. Synchronous calls hand a job to it over a bounded channel and
//! wait for the reply. Deliberately **not**:
//!
//! - a runtime built per call — that is a fresh reactor and thread for every `put`;
//! - `block_on` on the caller's thread — a caller already inside a runtime would then be
//!   driving two, and `block_on` from within a runtime context panics;
//! - a `Handle::try_current()` guard. That was proposed and rejected on measurement:
//!   `try_current()` *succeeds* inside `spawn_blocking` (the blocking pool enters the
//!   runtime context), so the guard would reject the one legitimate route while looking
//!   like it protected it.
//!
//! One worker per store instance, not one per process: two stores against two buckets is a
//! legitimate configuration, so a process-wide singleton would be wrong.
//!
//! # What this backend does NOT do
//!
//! **It never mints a `PreparedSst`.** An acknowledged upload is a necessary part of that
//! badge, not the whole of it, and its minting authority belongs to a sealed uploader
//! capability that does not exist yet. A backend that returned one here would put the trust
//! root back on a value the store reports about itself — the shape rejected when it was
//! proposed as `ObjectStore::visibility()`.
//!
//! # Credentials
//!
//! Read from the environment only. Never from the repository, a `Cargo.toml`, a test
//! constant, a log line, or an error message. Round one runs development credentials
//! against a local MinIO, and that is exactly why the habit is cheap to establish now:
//! once a credential reaches git history, deleting the commit does not delete the
//! credential.
//!
//! EdHuang's "skip TLS and credentials for now" ruling was about *internal node
//! communication*. MinIO authentication is not inside that ruling's scope and does not
//! inherit its exemption.

use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kv9_common::{Error, Result};
use s3_client::aws::AmazonS3Builder;
use s3_client::path::Path as S3Path;
// `get`/`delete` live on the extension trait; `put_opts`/`list` on the base trait. Both are
// needed, and both are aliased so that a bare `ObjectStore` in this file is unambiguously
// *ours*.
use s3_client::{
    ClientOptions, ObjectStore as S3ObjectStore, ObjectStoreExt as _, PutMode, PutOptions,
    PutPayload, RetryConfig,
};

use crate::object_store::{ObjectKey, ObjectStore};

/// Environment variables this backend reads. Named constants so an operator can be told
/// what to set without being told to read this file, and so a test cannot drift from the
/// production names by retyping them.
pub const ENV_ENDPOINT: &str = "KV9_OBJECT_STORE_ENDPOINT";
pub const ENV_BUCKET: &str = "KV9_OBJECT_STORE_BUCKET";
pub const ENV_ACCESS_KEY: &str = "KV9_OBJECT_STORE_ACCESS_KEY";
pub const ENV_SECRET_KEY: &str = "KV9_OBJECT_STORE_SECRET_KEY";

/// How long one operation may take before the caller gives up.
///
/// Deliberately a different quantity from any readiness wait a fixture performs. Sharing
/// one number would let a slow *start* present as an unresponsive *backend*, which turns a
/// timeout test green for the wrong reason.
const DEFAULT_OP_DEADLINE: Duration = Duration::from_secs(30);

/// Bounded, so a caller that outruns the backend blocks instead of growing a queue without
/// limit (DESIGN §13 principle 13).
const JOB_QUEUE_DEPTH: usize = 32;

/// Thread name for the worker. Also the string the structural probe counts, so it is a
/// constant rather than a literal repeated in two places that could drift apart.
pub const WORKER_THREAD_NAME: &str = "kv9-objstore";

/// One queued request.
///
/// The deadline travels *with* the job because the caller stops waiting on its own clock.
/// Without it, a burst of timeouts leaves the worker executing requests nobody is listening
/// for: each later job waits behind abandoned ones, so the store falls further behind the
/// caller with every timeout, and [`Drop`] — which joins the worker — blocks for up to
/// `JOB_QUEUE_DEPTH` × deadline. Checking it before execution bounds both.
struct Job {
    deadline: Instant,
    kind: JobKind,
}

enum JobKind {
    Put {
        key: String,
        bytes: Vec<u8>,
        reply: SyncSender<Result<()>>,
    },
    Get {
        key: String,
        reply: SyncSender<Result<Option<Vec<u8>>>>,
    },
    Delete {
        key: String,
        reply: SyncSender<Result<()>>,
    },
    List {
        /// Segment-aligned prefix pushed down to the backend, or `None` for the whole
        /// bucket. See [`pushdown_prefix`].
        pushdown: Option<String>,
        /// The caller's literal string prefix, applied to what comes back.
        literal: String,
        reply: SyncSender<Result<Vec<String>>>,
    },
}

/// Connection settings.
///
/// Not `Debug`, and the credential fields are private with no accessor: a derived formatter
/// is the usual way a secret reaches a log line, and there is no reason for this struct to
/// have one.
pub struct MinioConfig {
    endpoint: String,
    bucket: String,
    access_key: String,
    secret_key: String,
    op_deadline: Duration,
}

impl MinioConfig {
    /// Build from the environment. Every variable is required: defaulting a credential to
    /// the empty string turns a missing-configuration mistake into an authentication
    /// failure reported far from its cause.
    pub fn from_env() -> Result<MinioConfig> {
        Ok(MinioConfig {
            endpoint: required_var(ENV_ENDPOINT)?,
            bucket: required_var(ENV_BUCKET)?,
            access_key: required_var(ENV_ACCESS_KEY)?,
            secret_key: required_var(ENV_SECRET_KEY)?,
            op_deadline: DEFAULT_OP_DEADLINE,
        })
    }

    /// Point at an explicit endpoint and bucket while still taking credentials from the
    /// environment. Used by the second-client visibility test, which must reach the same
    /// bucket through an independent session.
    pub fn from_env_at(endpoint: impl Into<String>, bucket: impl Into<String>) -> Result<Self> {
        Ok(MinioConfig {
            endpoint: endpoint.into(),
            bucket: bucket.into(),
            access_key: required_var(ENV_ACCESS_KEY)?,
            secret_key: required_var(ENV_SECRET_KEY)?,
            op_deadline: DEFAULT_OP_DEADLINE,
        })
    }

    /// Override the per-operation deadline. Tests drive this small so the timeout path is
    /// reachable without waiting out the production number.
    pub fn with_op_deadline(mut self, deadline: Duration) -> Self {
        self.op_deadline = deadline;
        self
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn bucket(&self) -> &str {
        &self.bucket
    }
}

/// Read a required environment variable.
///
/// The error names the *variable*, never the value — for the credential variables the value
/// is the secret, and "unset or empty" is the whole of what a reader needs.
fn required_var(name: &str) -> Result<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        _ => Err(Error::Config(format!(
            "object store: {name} is unset or empty"
        ))),
    }
}

/// Validate that a key survives the upstream path type unchanged.
///
/// `s3_client::path::Path` normalises: it strips a leading or trailing `/` and rejects
/// empty segments. A key that normalises to something else would still round-trip through
/// `put`/`get` (both normalise identically) but would come back from `list` under its
/// *normalised* name — so a caller would receive an [`ObjectKey`] it never wrote. Refusing
/// at the boundary turns that silent rename into a typed rejection.
fn checked_path(key: &str) -> Result<S3Path> {
    let path = S3Path::parse(key)
        .map_err(|e| Error::Config(format!("object store: key is not addressable: {e}")))?;
    if path.as_ref() != key {
        return Err(Error::Config(format!(
            "object store: key {key:?} would be stored as {:?}; leading or trailing slashes \
             and empty segments are not addressable",
            path.as_ref()
        )));
    }
    Ok(path)
}

/// The longest segment-aligned prefix of `literal`, or `None` when there is none.
///
/// The trait's `list` takes a **string** prefix ("keys beginning with `prefix`"). Upstream's
/// takes a **path-segment** prefix: `foo/bar` is a prefix of `foo/bar/x` but not of
/// `foo/bar_baz/x`. Those are different sets, and handing the caller's string straight to
/// upstream would silently return fewer keys than `MemoryObjectStore` returns for the same
/// call — two backends of one trait disagreeing about the trait.
///
/// So push down only the part that means the same thing in both — everything up to the last
/// `/` — and apply the literal `starts_with` to what comes back. `sst/00` pushes down `sst`
/// and filters; `sst` pushes down nothing and scans, which is honest about the cost rather
/// than quietly returning a subset.
fn pushdown_prefix(literal: &str) -> Option<String> {
    let cut = literal.rfind('/')?;
    let head = &literal[..cut];
    if head.is_empty() {
        None
    } else {
        Some(head.to_string())
    }
}

/// A MinIO-backed object store.
///
/// Owns one worker thread and the channel to it. Dropping the store closes the channel,
/// which ends the worker loop, and joins the thread.
pub struct MinioObjectStore {
    /// `Option` only so that [`Drop`] can close the channel before joining. Every method
    /// runs while it is `Some`.
    tx: Option<SyncSender<Job>>,
    worker: Option<JoinHandle<()>>,
    op_deadline: Duration,
    endpoint: String,
    bucket: String,
}

impl std::fmt::Debug for MinioObjectStore {
    /// Hand-written rather than derived. A derive prints whatever fields exist *when it
    /// runs*, so the next person to add a credential-bearing field would leak it into every
    /// log line that formats a store, with nothing to notice. Endpoint and bucket are not
    /// secrets; nothing else is printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MinioObjectStore")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .finish_non_exhaustive()
    }
}

impl MinioObjectStore {
    /// Connect.
    ///
    /// The client is built *on* the worker thread, so the runtime that creates it is the
    /// runtime that drives it. Construction failure surfaces here rather than resurfacing
    /// later as an unrelated failure of whatever operation happens to be first.
    ///
    /// Note what this does **not** claim: a successful return means a client was
    /// constructed, not that the endpoint answered. The S3 builder performs no I/O. For
    /// "the store is reachable", do a round trip and observe it.
    pub fn connect(config: MinioConfig) -> Result<MinioObjectStore> {
        let (tx, rx) = sync_channel::<Job>(JOB_QUEUE_DEPTH);
        let (ready_tx, ready_rx) = sync_channel::<Result<()>>(1);

        let endpoint = config.endpoint.clone();
        let bucket = config.bucket.clone();
        let op_deadline = config.op_deadline;

        let worker = std::thread::Builder::new()
            .name(WORKER_THREAD_NAME.to_string())
            .spawn(move || worker_loop(config, rx, ready_tx))
            .map_err(|e| Error::Engine(format!("object store: cannot spawn worker: {e}")))?;

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = worker.join();
                return Err(e);
            }
            Err(_) => {
                let _ = worker.join();
                return Err(Error::Engine(
                    "object store: worker exited before signalling readiness".into(),
                ));
            }
        }

        Ok(MinioObjectStore {
            tx: Some(tx),
            worker: Some(worker),
            op_deadline,
            endpoint,
            bucket,
        })
    }

    fn submit<T>(&self, make: impl FnOnce(SyncSender<Result<T>>) -> JobKind) -> Result<T> {
        let (reply_tx, reply_rx) = sync_channel::<Result<T>>(1);
        let tx = self
            .tx
            .as_ref()
            .expect("the channel is dropped only in Drop, after which no method runs");
        // Stamped here, not on the worker: this is the moment the caller starts waiting,
        // and time spent queued is time the caller has already spent.
        let job = Job {
            deadline: Instant::now() + self.op_deadline,
            kind: make(reply_tx),
        };
        tx.send(job).map_err(|_| {
            Error::Engine("object store: worker is gone, cannot submit request".into())
        })?;
        match reply_rx.recv_timeout(self.op_deadline) {
            Ok(result) => result,
            // The worker may still be holding the job. The HTTP client's own timeout is
            // bound to this same deadline (see `worker_loop`), so it unwinds rather than
            // pinning the worker long after the caller stopped waiting.
            Err(RecvTimeoutError::Timeout) => Err(Error::Engine(format!(
                "object store: operation exceeded the {:?} deadline",
                self.op_deadline
            ))),
            Err(RecvTimeoutError::Disconnected) => Err(Error::Engine(
                "object store: worker dropped the request without replying".into(),
            )),
        }
    }
}

impl Drop for MinioObjectStore {
    fn drop(&mut self) {
        // Close the channel first: that is what ends the loop. Then join, so the runtime is
        // torn down here instead of racing process exit.
        self.tx = None;
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

fn worker_loop(config: MinioConfig, rx: Receiver<Job>, ready: SyncSender<Result<()>>) {
    // Current-thread, not multi-thread: this runtime exists to drive one operation at a
    // time on this one thread. A multi-thread runtime would spawn a pool per store and
    // contradict the one-worker property the structural probe checks.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            let _ = ready.send(Err(Error::Engine(format!(
                "object store: cannot build runtime: {e}"
            ))));
            return;
        }
    };

    // Bound the client's patience by the caller's. Upstream defaults to a three-minute
    // retry window, which would leave this thread inside `block_on` long after `submit`
    // returned a timeout — and `Drop` joins this thread, so that delay would resurface as a
    // store that takes minutes to drop.
    let client_options = ClientOptions::new()
        .with_timeout(config.op_deadline)
        .with_connect_timeout(config.op_deadline)
        // Round one runs against a local MinIO over plain HTTP. EdHuang ruled TLS out of
        // this round for internal traffic; stating it keeps it a decision rather than an
        // attempted-and-failed connection.
        .with_allow_http(true);

    let retry = RetryConfig {
        retry_timeout: config.op_deadline,
        ..Default::default()
    };

    let client = AmazonS3Builder::new()
        .with_endpoint(&config.endpoint)
        .with_bucket_name(&config.bucket)
        .with_access_key_id(&config.access_key)
        .with_secret_access_key(&config.secret_key)
        .with_allow_http(true)
        .with_client_options(client_options)
        .with_retry(retry)
        .build();

    let client = match client {
        Ok(client) => client,
        Err(e) => {
            // A builder/config error. Upstream's Display for it names missing or malformed
            // configuration, not credential values.
            let _ = ready.send(Err(Error::Engine(format!(
                "object store: cannot build S3 client: {e}"
            ))));
            return;
        }
    };

    if ready.send(Ok(())).is_err() {
        return;
    }

    while let Ok(job) = rx.recv() {
        // The caller has stopped waiting; issuing the request now would spend the worker on
        // an answer nobody receives while the next caller queues behind it. The reply is
        // still sent, because a caller that has not *quite* timed out deserves the reason
        // rather than a disconnect.
        if Instant::now() >= job.deadline {
            job.kind
                .decline("object store: request expired in the queue before it could be issued");
            continue;
        }
        match job.kind {
            JobKind::Put { key, bytes, reply } => {
                let result = runtime.block_on(do_put(&client, &key, bytes));
                let _ = reply.send(result);
            }
            JobKind::Get { key, reply } => {
                let result = runtime.block_on(do_get(&client, &key));
                let _ = reply.send(result);
            }
            JobKind::Delete { key, reply } => {
                let result = runtime.block_on(do_delete(&client, &key));
                let _ = reply.send(result);
            }
            JobKind::List {
                pushdown,
                literal,
                reply,
            } => {
                let result = runtime.block_on(do_list(&client, pushdown.as_deref(), &literal));
                let _ = reply.send(result);
            }
        }
    }
}

impl JobKind {
    /// Answer without issuing the request. Exhaustive on purpose: a new variant that forgot
    /// to answer would leave its caller waiting out the full deadline for a job the worker
    /// had already discarded.
    fn decline(self, why: &str) {
        match self {
            JobKind::Put { reply, .. } => {
                let _ = reply.send(Err(Error::Engine(why.to_string())));
            }
            JobKind::Get { reply, .. } => {
                let _ = reply.send(Err(Error::Engine(why.to_string())));
            }
            JobKind::Delete { reply, .. } => {
                let _ = reply.send(Err(Error::Engine(why.to_string())));
            }
            JobKind::List { reply, .. } => {
                let _ = reply.send(Err(Error::Engine(why.to_string())));
            }
        }
    }
}

/// Write an object, honouring the trait's write-once contract.
///
/// S3 `PUT` overwrites by default, which is exactly what the trait forbids. So the write
/// goes out as a conditional create (`If-None-Match: *`), and only when the object already
/// exists is the stored content read back and compared — keeping the extra round trip on
/// the conflict path instead of on every upload.
///
/// This is also why the mismatch test must run against a **real** server: if the endpoint
/// ignored the precondition, the create would silently succeed and the overwrite the trait
/// forbids would happen with nothing to report it. Nothing here assumes either way; that is
/// what the integration test observes.
async fn do_put(client: &impl S3ObjectStore, key: &str, bytes: Vec<u8>) -> Result<()> {
    let path = checked_path(key)?;
    let options = PutOptions {
        mode: PutMode::Create,
        ..Default::default()
    };
    match client
        .put_opts(&path, PutPayload::from(bytes.clone()), options)
        .await
    {
        Ok(_) => Ok(()),
        Err(s3_client::Error::AlreadyExists { .. }) => {
            // Retransmission after a timeout, or a reissue by a node that has since lost
            // leadership, is a normal event; identical content is not an error.
            match read_body(client, &path).await? {
                Some(existing) if existing == bytes => Ok(()),
                // Two distinct objects assigned one file-id. Refusing is the point; the key
                // is a file-id, not user data, so naming it leaks nothing.
                Some(_) => Err(Error::ObjectContentMismatch {
                    key: key.to_string(),
                }),
                // Present for the conditional put, absent for the read that followed: a
                // concurrent delete landed between them. Report it rather than inventing an
                // answer — in particular, do not report success, which would claim the
                // bytes are stored when they are not.
                None => Err(Error::Engine(format!(
                    "object store: {key} vanished between the conditional put and the \
                     read-back; retry"
                ))),
            }
        }
        Err(e @ s3_client::Error::NotImplemented { .. }) => Err(Error::Engine(format!(
            "object store: the endpoint does not support conditional writes, so write-once \
             cannot be enforced: {e}"
        ))),
        Err(e) => Err(Error::Engine(format!("object store: put failed: {e}"))),
    }
}

async fn do_get(client: &impl S3ObjectStore, key: &str) -> Result<Option<Vec<u8>>> {
    let path = checked_path(key)?;
    read_body(client, &path).await
}

/// Fetch and collect an object's bytes. `Ok(None)` for absent, which is a normal answer.
async fn read_body(client: &impl S3ObjectStore, path: &S3Path) -> Result<Option<Vec<u8>>> {
    match client.get(path).await {
        Ok(response) => match response.bytes().await {
            Ok(bytes) => Ok(Some(bytes.to_vec())),
            Err(e) => Err(Error::Engine(format!(
                "object store: reading body failed: {e}"
            ))),
        },
        Err(s3_client::Error::NotFound { .. }) => Ok(None),
        Err(e) => Err(Error::Engine(format!("object store: get failed: {e}"))),
    }
}

async fn do_delete(client: &impl S3ObjectStore, key: &str) -> Result<()> {
    let path = checked_path(key)?;
    match client.delete(&path).await {
        Ok(()) => Ok(()),
        // A late delete from a deposed leader, or a retried GC pass, is expected.
        Err(s3_client::Error::NotFound { .. }) => Ok(()),
        Err(e) => Err(Error::Engine(format!("object store: delete failed: {e}"))),
    }
}

async fn do_list(
    client: &impl S3ObjectStore,
    pushdown: Option<&str>,
    literal: &str,
) -> Result<Vec<String>> {
    use futures_util::StreamExt;

    let pushdown = match pushdown {
        Some(prefix) => Some(
            S3Path::parse(prefix)
                .map_err(|e| Error::Config(format!("object store: bad list prefix: {e}")))?,
        ),
        None => None,
    };

    let mut stream = client.list(pushdown.as_ref());
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(meta) => {
                let name = meta.location.as_ref();
                if name.starts_with(literal) {
                    out.push(name.to_string());
                }
            }
            Err(e) => return Err(Error::Engine(format!("object store: list failed: {e}"))),
        }
    }
    // Upstream documents that list order is *not* guaranteed; the trait promises sorted.
    out.sort();
    Ok(out)
}

impl ObjectStore for MinioObjectStore {
    fn put(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        let key = key.as_str().to_string();
        let bytes = bytes.to_vec();
        self.submit(|reply| JobKind::Put { key, bytes, reply })
    }

    fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>> {
        let key = key.as_str().to_string();
        self.submit(|reply| JobKind::Get { key, reply })
    }

    fn delete(&self, key: &ObjectKey) -> Result<()> {
        let key = key.as_str().to_string();
        self.submit(|reply| JobKind::Delete { key, reply })
    }

    fn list(&self, prefix: &str) -> Result<Vec<ObjectKey>> {
        let literal = prefix.to_string();
        let pushdown = pushdown_prefix(prefix);
        let names = self.submit(|reply| JobKind::List {
            pushdown,
            literal,
            reply,
        })?;
        names.into_iter().map(ObjectKey::new).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A variable name used by no other test, so this test's env mutation cannot race one.
    const PROBE_VAR: &str = "KV9_TEST_MINIO_REQUIRED_VAR_PROBE";

    #[test]
    fn required_var_names_the_variable_and_treats_empty_as_missing() {
        std::env::remove_var(PROBE_VAR);
        let err = required_var(PROBE_VAR).unwrap_err();
        let text = format!("{err}");
        assert!(text.contains(PROBE_VAR), "must name the variable: {text}");

        // Empty is refused like unset. Accepting it would send an empty secret key and
        // surface as an authentication failure with no hint that configuration is missing.
        std::env::set_var(PROBE_VAR, "");
        assert!(required_var(PROBE_VAR).is_err());

        std::env::set_var(PROBE_VAR, "a-value");
        assert_eq!(required_var(PROBE_VAR).unwrap(), "a-value");
        std::env::remove_var(PROBE_VAR);
    }

    #[test]
    fn required_var_error_does_not_echo_the_value() {
        // The failing cases above are unset and empty, where there is no value to leak.
        // This is the case that could leak: a value exists, and something else about it is
        // wrong. Today the only rejection is emptiness, so this test would pass vacuously
        // on the current code -- it is here to fail the day a validation rule is added that
        // formats the value into the message.
        std::env::set_var(PROBE_VAR, "AKIAsecretvalue0000");
        let text = match required_var(PROBE_VAR) {
            Ok(_) => String::new(),
            Err(e) => format!("{e}"),
        };
        assert!(
            !text.contains("AKIAsecretvalue0000"),
            "a credential value must never reach an error message: {text}"
        );
        std::env::remove_var(PROBE_VAR);
    }

    #[test]
    fn keys_the_path_type_would_rename_are_refused() {
        // Each of these round-trips through put/get -- both normalise identically -- but
        // comes back from `list` under a different name. Handing back an ObjectKey the
        // caller never wrote is the failure this refuses.
        for bad in ["/leading", "trailing/", "double//segment", "/"] {
            assert!(
                checked_path(bad).is_err(),
                "{bad:?} normalises to something else and must be refused"
            );
        }
        for good in ["sst/000001", "a", "a/b/c", "sst/000001.sst"] {
            assert_eq!(checked_path(good).expect("addressable").as_ref(), good);
        }
    }

    #[test]
    fn refusing_a_renamed_key_does_not_echo_a_second_key() {
        // The message names the key, which is a file-id rather than user data. Pin that it
        // reports the ONE key it was given and its normalised form -- not, say, the whole
        // listing it might otherwise have reached for.
        let err = checked_path("trailing/").unwrap_err();
        let text = format!("{err}");
        assert!(text.contains("trailing/"), "{text}");
        assert!(text.contains("\"trailing\""), "{text}");
    }

    #[test]
    fn pushdown_is_segment_aligned() {
        assert_eq!(pushdown_prefix("sst/000001"), Some("sst".to_string()));
        assert_eq!(pushdown_prefix("sst/00"), Some("sst".to_string()));
        assert_eq!(pushdown_prefix("sst/"), Some("sst".to_string()));
        assert_eq!(pushdown_prefix("a/b/c"), Some("a/b".to_string()));
        // No segment boundary: nothing can be pushed down, so the whole bucket is scanned
        // and filtered. Expensive and correct, rather than cheap and wrong.
        assert_eq!(pushdown_prefix("sst"), None);
        assert_eq!(pushdown_prefix(""), None);
        // A leading slash leaves an empty head, which is not a usable prefix.
        assert_eq!(pushdown_prefix("/sst"), None);
    }

    #[test]
    fn pushdown_then_filter_selects_exactly_what_the_literal_selects() {
        // The assertions above are about the function's return value; this one is about
        // what the PAIR of steps selects, which is the property that matters. `sst/00` is
        // the case where handing the literal straight to a segment-based backend returns
        // nothing at all.
        let keys = ["sst/000001", "sst/000002", "sst_other/1", "snap/1", "sst"];
        for literal in ["sst/00", "sst/", "sst", "a/b"] {
            let by_literal: Vec<&str> = keys
                .iter()
                .copied()
                .filter(|k| k.starts_with(literal))
                .collect();

            let via_backend: Vec<&str> = match pushdown_prefix(literal) {
                // Upstream's rule: segment-aligned, recursive.
                Some(p) => keys
                    .iter()
                    .copied()
                    .filter(|k| *k == p || k.starts_with(&format!("{p}/")))
                    .filter(|k| k.starts_with(literal))
                    .collect(),
                None => by_literal.clone(),
            };

            assert_eq!(via_backend, by_literal, "literal {literal:?}");
        }
    }

    #[test]
    fn debug_prints_the_endpoint_and_bucket_and_nothing_else() {
        // Built by hand rather than by `connect`, so this needs no server: the subject is
        // the formatter, not the connection.
        let store = MinioObjectStore {
            tx: None,
            worker: None,
            op_deadline: Duration::from_secs(1),
            endpoint: "http://127.0.0.1:1".into(),
            bucket: "a-bucket".into(),
        };
        let text = format!("{store:?}");
        assert!(text.contains("http://127.0.0.1:1"), "{text}");
        assert!(text.contains("a-bucket"), "{text}");
        assert!(
            text.contains(".."),
            "must be finish_non_exhaustive so added fields are not printed: {text}"
        );
    }
}
