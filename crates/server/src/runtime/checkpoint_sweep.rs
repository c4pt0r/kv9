//! Metadata-leader discovery, exact-history settlement and Held retirement.
//! One attempt per pass: either its owner handoff or one four-owner page. No
//! object I/O or per-attempt RAM map. Missing history cannot close admission;
//! settlement never releases negative Pending, old Version or remote PUT debt.
use super::*;
use crate::checkpoint_retention::{CheckpointLedger, CheckpointOwners};
use kv9_common::retention::{OwnerId, OwnerToken, PinPhase};
use kv9_meta::retention::checkpoint_admission::sweep::{self, SweepObservation};
use kv9_meta::retention::{encode_request, LedgerRequest};
use std::num::NonZeroU64;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const TICK: Duration = Duration::from_millis(250);
const PASS: Duration = Duration::from_secs(5);
pub(super) mod build_index;
mod intervals;
mod membership;
mod origins;
mod read_census;
mod remote;
mod settlement;
mod timeline;
pub(super) use settlement::local_history;

struct CheckedLedger<'a> {
    inner: &'a dyn CheckpointLedger,
    check: &'a (dyn Fn() -> Result<()> + Sync),
}
impl CheckpointLedger for CheckedLedger<'_> {
    fn apply(&self, request: Vec<u8>) -> Result<()> {
        (self.check)()?;
        self.inner.apply(request)
    }
    fn owner(&self, root: RootDigest, owner: OwnerId) -> Result<Option<Vec<u8>>> {
        (self.check)()?;
        self.inner.owner(root, owner)
    }
}

#[derive(Clone, Default, serde::Serialize)]
pub(super) struct Stats {
    #[serde(skip)]
    membership_after: Option<kv9_meta::retention::data_store_history::SourceCursor>,
    #[serde(skip)]
    history_turn: u8,
    #[serde(skip)]
    origins_after: Option<kv9_meta::retention::data_store_history::SourceCursor>,
    #[serde(skip)]
    timeline_after: Option<kv9_meta::retention::data_store_history::SourceCursor>,
    #[serde(skip)]
    intervals_after: Option<kv9_meta::retention::data_store_history::SourceCursor>,
    pub interval_pages: u64,
    pub interval_failures: u64,
    pub interval_wraps: u64,
    pub interval_observed_rounds: u64,
    pub interval_known_gaps: u64,
    pub timeline_pages: u64,
    pub timeline_failures: u64,
    pub timeline_wraps: u64,
    pub timeline_observed_rounds: u64,
    pub timeline_known_gaps: u64,
    pub origin_pages: u64,
    pub origin_failures: u64,
    pub origin_wraps: u64,
    pub origin_observed_rounds: u64,
    pub origin_known_gaps: u64,
    pub membership_pages: u64,
    pub membership_failures: u64,
    pub membership_wraps: u64,
    pub membership_observed_rounds: u64,
    pub membership_known_gaps: u64,
    pub running: bool,
    pub in_flight: usize,
    pub peak_in_flight: usize,
    pub discovered: u64,
    pub wraps: u64,
    pub open: u64,
    pub positive_handoffs: u64,
    pub negative_closures: u64,
    pub negative_records: u64,
    pub history_unknown: u64,
    pub remote_queries: u64,
    pub remote_errors: u64,
    pub drain_reports_replayed: u64,
    pub build_index_pages: u64,
    pub build_index_rows: u64,
    pub build_index_upgrades: u64,
    pub build_index_catalog_complete: bool,
    pub reader_index_pages: u64,
    pub reader_index_rows: u64,
    pub image_publisher_index_pages: u64,
    pub image_publisher_index_rows: u64,
    pub image_publisher_index_catalog_complete: bool,
    pub data_store_history_pages: u64,
    pub data_store_history_rows: u64,
    /// Last observed coverage of retained placement tasks, not all readers or stores.
    pub data_store_history_catalog_complete: bool,
    pub data_store_history_source_revision: Option<u64>,
    pub data_store_history_uncertain: bool,
    pub metadata_image_write_gates_written: u64,
    pub metadata_image_write_gates_closed: u64,
    pub image_publisher_closure_pages: u64,
    pub image_publisher_closure_restarts: u64,
    pub image_publisher_closure_zero_put: u64,
    pub image_publisher_closure_blocked: u64,
    pub image_reader_index_pages: u64,
    pub image_reader_index_rows: u64,
    pub image_reader_index_catalog_complete: bool,
    pub image_reader_closure_pages: u64,
    pub image_reader_closure_restarts: u64,
    pub image_reader_closure_closed: u64,
    pub image_reader_closure_blocked: u64,
    pub reader_index_catalog_complete: bool,
    pub bootstrap_index_pages: u64,
    pub bootstrap_index_rows: u64,
    pub bootstrap_index_catalog_complete: bool,
    pub done: u64,
    pub closure_pages: u64,
    pub closure_restarts: u64,
    pub closure_zero_put: u64,
    pub closure_blocked: u64,
    pub reader_closure_pages: u64,
    pub reader_closure_restarts: u64,
    pub reader_closure_closed: u64,
    pub reader_closure_blocked: u64,
    pub metadata_read_gate_pages: u64,
    pub metadata_read_gate_waiting: u64,
    pub metadata_read_gate_closed: u64,
    pub metadata_image_gates_written: u64,
    pub metadata_image_gates_closed: u64,
    pub bootstrap_closure_pages: u64,
    pub bootstrap_closure_restarts: u64,
    pub bootstrap_closure_retired: u64,
    pub bootstrap_closure_blocked: u64,
    pub pages: u64,
    pub examined: u64,
    pub retirements_confirmed: u64,
    pub failures: u64,
    pub last_error: Option<String>,
}
#[derive(Default)]
pub(super) struct Reconciler {
    after: Option<OwnerId>,
    // One candidate node per full attempt sweep. Rotating both cursors on
    // every attempt could repeatedly miss the same attempt/node combinations.
    remote_node: Option<NodeId>,
    remote_after: Option<NodeId>,
    // Rotate priority per complete discovery sweep, not per attempt: every
    // alias gets a read-first turn even if settlement keeps timing out, and
    // census invalidation cannot indefinitely starve its publisher work.
    read_first: bool,
}
impl Reconciler {
    pub(super) fn pass(
        &mut self,
        backend: &RuntimeBackend,
        ledger: &impl CheckpointLedger,
        cancel: &CancellationToken,
        deadline: Instant,
        stats: &mut Stats,
    ) -> Result<()> {
        let check = || -> Result<()> {
            if cancel.is_cancelled() || Instant::now() >= deadline {
                return Err(Error::MetaNotReady(
                    "checkpoint sweep cancelled or expired".into(),
                ));
            }
            backend.ensure_serving()?;
            if backend.driver.status().role != Role::Leader {
                return Err(Error::NotLeader {
                    leader: backend.driver.status().leader_id,
                });
            }
            Ok(())
        };
        check()?;
        backend.driver.read_barrier(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_secs(1)),
        )?;
        let txn = backend.node.meta_raft.store.begin()?;
        let root = kv9_meta::root::certified_root(&txn)?
            .ok_or_else(|| Error::MetaNotReady("checkpoint sweep root unavailable".into()))?;
        if root.digest() != backend.ttl_context()?.root.digest() {
            return Err(Error::Config("checkpoint sweep root differs".into()));
        }
        let view = txn.into_view();
        let Some(pending) = sweep::next_attempt(view.as_ref(), &root, self.after)? else {
            self.after = None;
            self.remote_node = None;
            self.read_first = !self.read_first;
            stats.wraps = stats.wraps.saturating_add(1);
            return Ok(());
        };
        // Retain only this discovery key. A damaged attempt reports a failure
        // but later keys still get their turn; a future wrap revisits it.
        self.after = Some(pending);
        stats.discovered = stats.discovered.saturating_add(1);
        let observation = sweep::observe(view.as_ref(), &root, pending)?;
        let intent = kv9_meta::retention::checkpoint_intent::checkpoint_intent(
            view.as_ref(),
            &root,
            pending,
        )?
        .ok_or_else(|| Error::Engine("checkpoint sweep intent missing".into()))?;
        let pending_owner = kv9_meta::retention::retention_owner(view.as_ref(), &root, pending)?
            .ok_or_else(|| Error::Engine("checkpoint sweep pending missing".into()))?;
        let refused =
            kv9_meta::retention::checkpoint_refusal::refusal(view.as_ref(), &root, pending)?
                .is_some();
        let census_first = self.read_first
            && !matches!(observation, SweepObservation::Done)
            && matches!(
                kv9_meta::retention::metadata_image_gate::observe(view.as_ref(), &root, pending)?,
                kv9_meta::retention::metadata_image_gate::Work::Closed(_)
            );
        drop(view);
        if census_first && read_census::pass(backend, ledger, &root, pending, &check, stats)? {
            return Ok(());
        }
        let open = matches!(observation, SweepObservation::Open);
        // Closed is not a completed positive handoff. A crash after Close or
        // after Quiesce must be repairable even if the Held sweep reached Done.
        if !refused && (open || pending_owner.phase != PinPhase::Released) {
            check()?;
            let guarded = CheckedLedger {
                inner: ledger,
                check: &check,
            };
            let observed = match settlement::inspect(backend, &root, &intent)? {
                Some(observed) => Some(observed),
                None => remote::inspect(self, backend, &root, &intent, deadline, stats)?,
            };
            match observed {
                Some(settlement::Settlement::Applied) => {
                    check()?;
                    let owners =
                        CheckpointOwners::new(&root, &intent.manifest()?, intent.predecessor())?;
                    owners.after_positive_settlement(&guarded)?;
                    stats.positive_handoffs = stats.positive_handoffs.saturating_add(1);
                    // A fresh pass reloads gate/cursor/owners after the writes.
                    return Ok(());
                }
                Some(settlement::Settlement::WindowRefused { winner, source }) => {
                    check()?;
                    guarded.apply(encode_request(
                        root.digest(),
                        &LedgerRequest::RecordCheckpointRefusal {
                            pending: pending_owner.binding.token(),
                            source,
                            winner,
                        },
                    )?)?;
                    stats.negative_closures =
                        stats.negative_closures.saturating_add(u64::from(open));
                    stats.negative_records = stats.negative_records.saturating_add(1);
                    return Ok(());
                }
                None => stats.history_unknown = stats.history_unknown.saturating_add(1),
            }
        }
        let (cursor, page) = match observation {
            SweepObservation::Open => {
                stats.open = stats.open.saturating_add(1);
                return Ok(());
            }
            SweepObservation::Done => {
                stats.done = stats.done.saturating_add(1);
                check()?;
                let view = backend.node.meta_raft.store.begin()?.into_view();
                use kv9_meta::retention::metadata_read_gate::{self, Work as ReadGateWork};
                let read_gate = metadata_read_gate::observe(view.as_ref(), &root, pending)?;
                drop(view);
                match read_gate {
                    ReadGateWork::Advance(progress) => {
                        check()?;
                        ledger.apply(encode_request(root.digest(), &progress.request(pending))?)?;
                        stats.metadata_read_gate_pages =
                            stats.metadata_read_gate_pages.saturating_add(1);
                        return Ok(());
                    }
                    ReadGateWork::WaitingForSelection(_) => {
                        stats.metadata_read_gate_waiting =
                            stats.metadata_read_gate_waiting.saturating_add(1);
                    }
                    ReadGateWork::Closed(_) => {
                        stats.metadata_read_gate_closed =
                            stats.metadata_read_gate_closed.saturating_add(1);
                        let view = backend.node.meta_raft.store.begin()?.into_view();
                        use kv9_meta::retention::metadata_image_gate::{self, Work as ImageWork};
                        let image = metadata_image_gate::observe(view.as_ref(), &root, pending)?;
                        drop(view);
                        match image {
                            ImageWork::Ready => {
                                check()?;
                                ledger.apply(encode_request(
                                    root.digest(),
                                    &LedgerRequest::CloseMetadataImageReads(
                                        kv9_common::retention::OwnerToken {
                                            owner: pending,
                                            generation: std::num::NonZeroU64::new(1).unwrap(),
                                        },
                                    ),
                                )?)?;
                                stats.metadata_image_gates_written =
                                    stats.metadata_image_gates_written.saturating_add(1);
                                return Ok(());
                            }
                            ImageWork::Closed(_) => {
                                stats.metadata_image_gates_closed =
                                    stats.metadata_image_gates_closed.saturating_add(1);
                            }
                            ImageWork::Ineligible => {}
                        }
                    }
                    ReadGateWork::Ineligible => {}
                }
                check()?;
                let view = backend.node.meta_raft.store.begin()?.into_view();
                use kv9_meta::retention::checkpoint_closure::{self, Work};
                let work = checkpoint_closure::observe(view.as_ref(), &root, pending)?;
                drop(view);
                let zero_put = work.registered_zero_put_closed();
                match work {
                    Work::Advance { cursor, restart } => {
                        check()?;
                        ledger.apply(encode_request(root.digest(), &cursor.request(pending))?)?;
                        stats.closure_pages = stats.closure_pages.saturating_add(1);
                        stats.closure_restarts =
                            stats.closure_restarts.saturating_add(u64::from(restart));
                        return Ok(()); // One bounded aggregate page per pass.
                    }
                    Work::Complete(_) => {
                        stats.closure_zero_put =
                            stats.closure_zero_put.saturating_add(u64::from(zero_put));
                        stats.closure_blocked =
                            stats.closure_blocked.saturating_add(u64::from(!zero_put));
                    }
                    Work::Open | Work::WaitingForCatalog => {}
                }
                read_census::pass(backend, ledger, &root, pending, &check, stats)?;
                return Ok(());
            }
            SweepObservation::Page { cursor, page } => (cursor, page),
        };
        stats.examined = stats.examined.saturating_add(page.owners.len() as u64);
        for owner in page.owners {
            if owner.phase != PinPhase::Held {
                continue;
            }
            check()?;
            ledger.apply(encode_request(
                root.digest(),
                &LedgerRequest::RetireUnpublishedCheckpoint {
                    pending: OwnerToken {
                        owner: pending,
                        generation: NonZeroU64::new(1).unwrap(),
                    },
                    executor: owner.binding.token(),
                },
            )?)?;
            stats.retirements_confirmed = stats.retirements_confirmed.saturating_add(1);
        }
        check()?;
        ledger.apply(encode_request(
            root.digest(),
            &cursor.advance_request(pending, page.next),
        )?)?;
        stats.pages = stats.pages.saturating_add(1);
        Ok(())
    }
}

pub(super) struct Maintenance {
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
    stats: Arc<Mutex<Stats>>,
}
impl Maintenance {
    pub(super) fn start(backend: Arc<RuntimeBackend>, runtime: &tokio::runtime::Handle) -> Self {
        let cancel = CancellationToken::new();
        let stats = Arc::new(Mutex::new(Stats::default()));
        let task = runtime.spawn(run(backend, cancel.clone(), stats.clone()));
        Self {
            cancel,
            task: Some(task),
            stats,
        }
    }
    pub(super) fn snapshot(&self) -> Stats {
        self.stats.lock().expect("checkpoint sweep stats").clone()
    }
    pub(super) fn stopped_unexpectedly(&self) -> bool {
        !self.cancel.is_cancelled() && self.task.as_ref().is_some_and(JoinHandle::is_finished)
    }
    pub(super) fn cancel(&self) {
        self.cancel.cancel();
    }
    pub(super) async fn drain(mut self) {
        self.cancel.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for Maintenance {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
async fn run(backend: Arc<RuntimeBackend>, cancel: CancellationToken, shared: Arc<Mutex<Stats>>) {
    let mut lane = backend
        .maintenance
        .lane(crate::maintenance::Class::CheckpointRetention);
    let mut state = Reconciler::default();
    let mut stats = Stats {
        running: true,
        ..Stats::default()
    };
    *shared.lock().expect("checkpoint sweep stats") = stats.clone();
    let mut ticker = tokio::time::interval(TICK);
    let mut prefer_report = true;
    let mut prefer_build_index = true;
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = ticker.tick() => {}
        }
        let outbox = backend
            .ttl_context()
            .ok()
            .and_then(|c| c.drain_outbox.clone());
        let pending_report = outbox.as_ref().is_some_and(|o| o.has_pending());
        let leader = backend.driver.status().role == Role::Leader;
        if backend.ensure_serving().is_err() || (!leader && !pending_report) {
            lane.withdraw();
            state.after = None;
            continue;
        }
        let Some(permit) = lane.try_acquire() else {
            continue;
        };
        stats.in_flight = 1;
        stats.peak_in_flight = 1;
        *shared.lock().expect("checkpoint sweep stats") = stats.clone();
        let job_backend = backend.clone();
        let stopped = cancel.clone();
        let replay_report = pending_report && (!leader || prefer_report);
        prefer_report = !replay_report;
        let census = !replay_report && leader && prefer_build_index;
        if !replay_report {
            prefer_build_index = !prefer_build_index;
        }
        // Await the actual blocking job even after cancellation. Permit, cursor
        // and captured backend stay alive until its bounded writes return.
        let job = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let deadline = Instant::now() + PASS;
            let ledger = super::checkpoint_retention::RoutedLedger::until(
                &job_backend,
                tokio::runtime::Handle::current(),
                deadline,
            );
            let result = if replay_report {
                let check = || {
                    job_backend.ensure_serving()?;
                    if stopped.is_cancelled() || Instant::now() >= deadline {
                        return Err(Error::MetaNotReady(
                            "drain replay cancelled or expired".into(),
                        ));
                    }
                    Ok(())
                };
                check().and_then(|_| {
                    let guarded = CheckedLedger {
                        inner: &ledger,
                        check: &check,
                    };
                    let replayed = outbox.as_ref().unwrap().replay_one(&guarded)?;
                    stats.drain_reports_replayed = stats
                        .drain_reports_replayed
                        .saturating_add(u64::from(replayed));
                    Ok(())
                })
            } else if census {
                build_index::pass(&job_backend, &ledger, &stopped, deadline, &mut stats)
            } else {
                state.pass(&job_backend, &ledger, &stopped, deadline, &mut stats)
            };
            (state, stats, result)
        })
        .await;
        match job {
            Ok((next, observed, result)) => {
                state = next;
                stats = observed;
                stats.in_flight = 0;
                if let Err(error) = result {
                    stats.failures = stats.failures.saturating_add(1);
                    stats.last_error = Some(error.to_string().chars().take(160).collect());
                }
                *shared.lock().expect("checkpoint sweep stats") = stats.clone();
            }
            Err(error) => {
                let mut stats = shared.lock().expect("checkpoint sweep stats");
                stats.failures = stats.failures.saturating_add(1);
                stats.last_error = Some(
                    format!("checkpoint sweep job: {error}")
                        .chars()
                        .take(160)
                        .collect(),
                );
                stats.in_flight = 0;
                stats.running = false;
                return;
            }
        }
    }
    stats.running = false;
    *shared.lock().expect("checkpoint sweep stats") = stats;
}
