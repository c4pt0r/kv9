//! Bounded remote pressure polling. Routing comes from the applied node
//! directory, while each actual Raft peer uses its own full configuration.
use super::*;
use kv9_raft::replica_pressure::ReplicaPressure;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const INTERVAL: Duration = Duration::from_millis(250);
const DEADLINE: Duration = Duration::from_secs(1);
const WORKERS: usize = 2;
#[derive(Default, Clone, serde::Serialize)]
pub(super) struct Stats {
    pub running: bool,
    pub in_flight: usize,
    pub peak: usize,
    pub completed: u64,
    pub errors: u64,
}
pub(super) struct Maintenance {
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
    stats: Arc<Mutex<Stats>>,
}
impl Maintenance {
    pub(super) fn start(
        backend: Arc<RuntimeBackend>,
        pressure: Arc<ReplicaPressure>,
        handle: &tokio::runtime::Handle,
    ) -> Self {
        let cancel = CancellationToken::new();
        let stats = Arc::new(Mutex::new(Stats::default()));
        let task = handle.spawn(run(backend, pressure, cancel.clone(), stats.clone()));
        Self {
            cancel,
            task: Some(task),
            stats,
        }
    }
    pub(super) fn snapshot(&self) -> Stats {
        self.stats.lock().expect("pressure stats").clone()
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

async fn run(
    backend: Arc<RuntimeBackend>,
    pressure: Arc<ReplicaPressure>,
    cancel: CancellationToken,
    stats: Arc<Mutex<Stats>>,
) {
    let mut lane = backend
        .maintenance
        .lane(crate::maintenance::Class::ReplicaPressure);
    let mut ticker = tokio::time::interval(INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    type Reply = (
        NodeId,
        kv9_raft::replica_pressure::PressureTicket,
        Result<kv9_raft::grpc::log_pressure::ConfirmedPressure>,
    );
    let mut jobs = tokio::task::JoinSet::<Reply>::new();
    let mut busy = std::collections::BTreeSet::new();
    let mut after = None;
    stats.lock().unwrap().running = true;
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            result = jobs.join_next(), if !jobs.is_empty() => {
                match result {
                    Some(Ok((node, ticket, reply))) => {
                        busy.remove(&node);
                        let failed = reply.is_err();
                        pressure.finish(ticket, reply);
                        let mut s = stats.lock().unwrap();
                        s.completed = s.completed.saturating_add(1);
                        s.errors = s.errors.saturating_add(u64::from(failed));
                        s.in_flight = jobs.len();
                    }
                    // A worker panic cannot silently shrink the pool or leave
                    // stale positive observations masquerading as active polling.
                    _ => break,
                }
            }
            _ = ticker.tick() => {
                let nodes = pressure.polling_nodes();
                let mut eligible = false;
                let mut waiting = false;
                let cursor = after;
                for &node in nodes.iter().filter(|n| cursor.is_none_or(|a| **n > a))
                    .chain(nodes.iter().filter(|n| cursor.is_some_and(|a| **n <= a))).take(16) {
                    if jobs.len() >= WORKERS { break; }
                    if busy.contains(&node) { continue; }
                    eligible = true;
                    let Some(permit) = lane.try_acquire() else { waiting = true; break; };
                    after = Some(node);
                    let ticket = match pressure.begin(node) {
                        Ok(Some(ticket)) => ticket,
                        _ => { drop(permit); continue; }
                    };
                    let context = backend.ttl_context();
                    busy.insert(node);
                    jobs.spawn(async move {
                        let _permit = permit;
                        let reply = match context {
                            Ok(context) => {
                                let deadline = Instant::now() + DEADLINE;
                                match context.channel(ticket.route().address, deadline).await {
                                    Ok(channel) => match deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
                                        Some(left) => kv9_raft::grpc::log_pressure::inspect_on_channel(
                                            channel, &ticket, context.local.identity().node_id, Some(context.token.clone()), left,
                                        ).await,
                                        None => Err(Error::MetaNotReady("pressure request deadline elapsed".into())),
                                    },
                                    Err(error) => Err(error),
                                }
                            }
                            Err(error) => Err(error),
                        };
                        (node, ticket, reply)
                    });
                }
                if !waiting || !eligible { lane.withdraw(); }
                let mut s = stats.lock().unwrap();
                s.in_flight = jobs.len(); s.peak = s.peak.max(s.in_flight);
            }
        }
    }
    lane.withdraw();
    // Only local network futures are cancelled here. A remote service retains
    // its own blocking probe permit until that actual syscall returns.
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    let mut s = stats.lock().unwrap();
    s.running = false;
    s.in_flight = 0;
}

pub(super) fn status(pressure: &ReplicaPressure) -> Vec<serde_json::Value> {
    pressure
        .snapshot()
        .into_iter()
        .map(|s| {
            serde_json::json!({
                "node": s.node, "generation": s.generation, "incarnation": s.incarnation,
                "address": s.address, "polling": s.polling, "observed": s.observed,
                "uncertain": s.uncertain, "available_bytes": s.available_bytes,
                "age_millis": s.age_millis, "errors": s.errors,
            })
        })
        .collect()
}
