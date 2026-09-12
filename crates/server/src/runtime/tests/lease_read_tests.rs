//! Real runtime and RPC coverage with an explicitly controlled lease clock.
//! These tests do not qualify host clocks or replace actual Chaos Mesh histories.

use super::*;
use crate::api::RawReadJob;
use crate::grpc::proto;
use kv9_raft::driver::{LeaseReadView, ReadPreparation};
use std::sync::atomic::AtomicU64;

const PROMISE: u64 = 1_000_000_000;

#[derive(Default)]
struct TestClock(AtomicU64);

impl kv9_raft::rawnode::LeaseClock for TestClock {
    fn sample(&self) -> kv9_raft::lease::Result<kv9_raft::lease::ClockReading> {
        Ok(kv9_raft::lease::ClockReading {
            domain: 1,
            nanos: self.0.load(Ordering::SeqCst),
        })
    }
}

struct Fixture {
    rts: Vec<NodeRuntime>,
    root: RootDescriptor,
    base: PathBuf,
    leader: usize,
    clock: Arc<TestClock>,
    executor: tokio::runtime::Runtime,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.rts.clear();
        fs::remove_dir_all(&self.base).unwrap();
    }
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let clock = Arc::new(TestClock::default());
        let (mut rts, root, _, base) = unformed_trio_with_lease(tag, false, Some(clock.clone()));
        // Finish each durable installation's recovery quarantine. Time remains
        // controlled; no inference about physical nanoseconds follows.
        clock.0.store(PROMISE, Ordering::SeqCst);
        wait_for(&mut rts, 60, "lease-enabled trio Serving", |rts| {
            rts.iter()
                .all(|rt| rt.node.meta.lock().unwrap().bootstrap.is_serving())
        });
        let leader = cluster_leader(&rts).unwrap();
        Self {
            rts,
            root,
            base,
            leader,
            clock,
            executor: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
        }
    }

    fn backend(&self) -> Arc<RuntimeBackend> {
        Arc::new(backend_view(&self.rts[self.leader], &self.root))
    }

    fn view(&self, backend: &RuntimeBackend) -> LeaseReadView {
        let until = Instant::now() + Duration::from_secs(5);
        for _ in 0..500 {
            let prepared = self
                .executor
                .block_on(backend.driver.read_preparation_async(READ_BARRIER_DEADLINE))
                .unwrap();
            if let ReadPreparation::Lease(view) = prepared {
                return view;
            }
            assert!(
                Instant::now() < until,
                "runtime never acquired a lease read view"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("runtime lease read attempt limit reached");
    }

    fn expire(&self) {
        self.clock.0.fetch_add(PROMISE, Ordering::SeqCst);
    }
}

fn context(backend: &RuntimeBackend, name: &str) -> RequestContext {
    let created = backend
        .create_keyspace(
            "acceptance",
            name,
            TenantId::DEFAULT,
            ApiType::Raw,
            TxnGroupId(0),
        )
        .unwrap();
    let location = backend
        .get_region("acceptance", created.keyspace, b"alpha")
        .unwrap();
    RequestContext {
        keyspace: created.keyspace,
        region_epoch: location.epoch,
        origin: crate::api::RequestOrigin::from_transport("acceptance"),
    }
}

fn wire_context(ctx: &RequestContext) -> Option<proto::RequestContext> {
    Some(proto::RequestContext {
        keyspace_id: ctx.keyspace.0,
        region_epoch: Some(proto::RegionEpoch {
            conf_ver: ctx.region_epoch.conf_ver,
            version: ctx.region_epoch.version,
        }),
    })
}

fn authenticated<T>(body: T) -> tonic::Request<T> {
    let mut request = tonic::Request::new(body);
    request.metadata_mut().insert(
        "authorization",
        // RuntimeAuth stores (token, principal), in that order.
        "Bearer acceptance".parse().unwrap(),
    );
    request.set_timeout(Duration::from_secs(5));
    request
}

fn bump_epoch(backend: &RuntimeBackend, ctx: &RequestContext, column: u16) -> RequestContext {
    use kv9_meta::codec::{memcmp_uint, ColumnValue};
    use kv9_meta::schema::{ColumnId, REGIONS_DESC};
    let region = Tables::new(&backend.node.meta_raft.store)
        .region_for_key(ctx.keyspace, b"alpha")
        .unwrap()
        .unwrap();
    let mut next_ctx = ctx.clone();
    let next = if column == 5 {
        next_ctx.region_epoch.conf_ver += 1;
        next_ctx.region_epoch.conf_ver
    } else {
        next_ctx.region_epoch.version += 1;
        next_ctx.region_epoch.version
    };
    let term = backend.prepare_catalog().unwrap();
    let mut change = backend.node.meta_raft.store.begin().unwrap();
    change
        .update(
            &REGIONS_DESC,
            &[memcmp_uint(region.id.0)],
            vec![(ColumnId(column), ColumnValue::Uint(next))],
        )
        .unwrap();
    backend
        .commit_catalog(&Command::from_batch(&change.into_batch()), term)
        .unwrap();
    next_ctx
}

#[test]
fn lease_get_and_batch_get_traverse_real_unary_and_stream_rpc() {
    let fixture = Fixture::new("lease-rpc");
    let backend = fixture.backend();
    let ctx = context(&backend, "lease-rpc");
    backend
        .raw_batch_put(
            &ctx,
            &[
                (b"alpha".to_vec(), b"before".to_vec()),
                (b"empty".to_vec(), Vec::new()),
            ],
        )
        .unwrap();
    drop(fixture.view(&backend));
    fixture.executor.block_on(async {
        let address = fixture.rts[fixture.leader].addr;
        let mut unary = proto::kv9_client::Kv9Client::connect(format!("http://{address}"))
            .await
            .unwrap();
        let stream = crate::point_stream::StreamClient::new(address, 16);
        for streaming in [false, true] {
            for batch in [false, true] {
                let until = Instant::now() + Duration::from_secs(5);
                let mut hit = false;
                for _ in 0..500 {
                    let before = backend.driver.lease_read_hits();
                    let groups = backend.driver.async_read_snapshot().group_attempts;
                    if batch {
                        let request = authenticated(proto::RawBatchGetRequest {
                            context: wire_context(&ctx),
                            keys: [b"missing".as_slice(), b"empty", b"alpha", b"alpha"]
                                .into_iter()
                                .map(<[u8]>::to_vec)
                                .collect(),
                        });
                        let response = if streaming {
                            stream
                                .raw_batch_get(
                                    request,
                                    (Instant::now() + Duration::from_secs(5)).into(),
                                )
                                .await
                        } else {
                            unary.raw_batch_get(request).await
                        }
                        .unwrap()
                        .into_inner();
                        assert_eq!(
                            response.values,
                            vec![
                                proto::OptionalValue {
                                    found: false,
                                    value: Vec::new()
                                },
                                proto::OptionalValue {
                                    found: true,
                                    value: Vec::new()
                                },
                                proto::OptionalValue {
                                    found: true,
                                    value: b"before".to_vec()
                                },
                                proto::OptionalValue {
                                    found: true,
                                    value: b"before".to_vec()
                                },
                            ],
                            "lease RPC lost ordered duplicates, absence, or empty values"
                        );
                    } else {
                        let request = authenticated(proto::RawGetRequest {
                            context: wire_context(&ctx),
                            key: b"alpha".to_vec(),
                        });
                        let response = if streaming {
                            stream
                                .raw_get(request, (Instant::now() + Duration::from_secs(5)).into())
                                .await
                        } else {
                            unary.raw_get(request).await
                        }
                        .unwrap()
                        .into_inner();
                        assert_eq!(
                            response.value,
                            Some(proto::OptionalValue {
                                found: true,
                                value: b"before".to_vec()
                            })
                        );
                    }
                    if backend.driver.lease_read_hits() == before + 1 {
                        assert_eq!(
                            backend.driver.async_read_snapshot().group_attempts,
                            groups,
                            "lease RPC also submitted a quorum read"
                        );
                        hit = true;
                        break;
                    }
                    assert!(
                        Instant::now() < until,
                        "RPC never entered the lease read path"
                    );
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                assert!(hit, "RPC lease hit attempt limit reached");
            }
        }
    });
    assert_eq!(backend.driver.async_read_snapshot().in_flight, 0);
}

#[test]
fn deferred_lease_get_checks_metadata_and_values_on_its_retained_view() {
    deferred_epoch_case(false, false);
}

#[test]
fn deferred_lease_batch_checks_metadata_and_values_on_its_retained_view() {
    deferred_epoch_case(true, false);
}

#[test]
fn large_lease_batch_defers_copy_without_changing_the_authorized_view() {
    deferred_epoch_case(true, true);
}

fn deferred_epoch_case(batch: bool, large: bool) {
    let fixture = Fixture::new("lease-deferred-epoch");
    let backend = fixture.backend();
    for column in [5, 6] {
        let ctx = context(&backend, &format!("lease-epoch-{column}"));
        let value = if large {
            vec![b'x'; MAX_RESIDENT_BATCH_READ_BYTES / 4 + 1]
        } else {
            b"before".to_vec()
        };
        backend
            .raw_put(&ctx, b"alpha".to_vec(), value.clone())
            .unwrap();
        let view = fixture.view(&backend);
        // Force lifecycle deferral only after the exact lease view was captured.
        // Large values instead enter the existing bounded inline-copy branch.
        let held = (!large).then(|| backend.node.meta.lock().unwrap());
        let keys = vec![b"alpha".to_vec(); if large { 5 } else { 2 }];
        let job = if batch {
            backend
                .clone()
                .finish_lease_batch_get(ctx.clone(), keys.clone(), view)
                .unwrap()
        } else {
            match backend
                .clone()
                .finish_lease_get(ctx.clone(), b"alpha".to_vec(), view)
                .unwrap()
            {
                RawReadJob::Blocking(job) => {
                    RawReadJob::Blocking(Box::new(move || job().map(|v| vec![v])))
                }
                RawReadJob::Completed(_) => {
                    panic!("lease GET failed to defer under lifecycle contention")
                }
            }
        };
        drop(held);
        assert!(
            matches!(job, RawReadJob::Blocking(_)),
            "lease batch exceeded its inline copy budget"
        );
        let next_ctx = bump_epoch(&backend, &ctx, column);
        backend
            .raw_put(&next_ctx, b"alpha".to_vec(), b"after".to_vec())
            .unwrap();
        fixture.expire();
        assert_eq!(
            job.run()
                .expect("deferred lease read lost its retained metadata authorization"),
            vec![Some(value); if batch { keys.len() } else { 1 }],
            "deferred lease read replaced its authorized metadata/data view"
        );

        let stale_view = fixture.view(&backend);
        let stale = if batch {
            backend
                .clone()
                .finish_lease_batch_get(ctx.clone(), keys.clone(), stale_view)
                .and_then(RawReadJob::run)
        } else {
            backend
                .clone()
                .finish_lease_get(ctx.clone(), b"alpha".to_vec(), stale_view)
                .and_then(RawReadJob::run)
                .map(|v| vec![v])
        };
        assert!(
            matches!(stale, Err(Error::StaleEpoch { .. })),
            "fresh lease read accepted stale metadata: {stale:?}"
        );
        let current = fixture
            .executor
            .block_on(backend.clone().prepare_raw_get(next_ctx, b"alpha".to_vec()))
            .unwrap()
            .run()
            .unwrap();
        assert_eq!(current, Some(b"after".to_vec()));
    }
    assert_eq!(backend.driver.async_read_snapshot().in_flight, 0);
}

#[test]
fn expired_lease_with_unavailable_peers_returns_typed_rpc_refusal() {
    let mut fixture = Fixture::new("lease-expired-rpc");
    let backend = fixture.backend();
    let ctx = context(&backend, "lease-expired-rpc");
    backend
        .raw_put(&ctx, b"alpha".to_vec(), b"old".to_vec())
        .unwrap();
    drop(fixture.view(&backend));
    for (i, rt) in fixture.rts.iter_mut().enumerate() {
        if i != fixture.leader {
            rt.driver.stop();
            rt.driver_thread.take().unwrap().join().unwrap();
        }
    }
    fixture.expire();
    let hits = backend.driver.lease_read_hits();
    fixture.executor.block_on(async {
        let mut unary = proto::kv9_client::Kv9Client::connect(format!(
            "http://{}",
            fixture.rts[fixture.leader].addr
        ))
        .await
        .unwrap();
        for batch in [false, true] {
            let status = if batch {
                unary
                    .raw_batch_get(authenticated(proto::RawBatchGetRequest {
                        context: wire_context(&ctx),
                        keys: vec![b"alpha".to_vec()],
                    }))
                    .await
                    .unwrap_err()
            } else {
                unary
                    .raw_get(authenticated(proto::RawGetRequest {
                        context: wire_context(&ctx),
                        key: b"alpha".to_vec(),
                    }))
                    .await
                    .unwrap_err()
            };
            let quorum = status.code() == tonic::Code::Unavailable
                && status
                    .metadata()
                    .get(crate::grpc::READ_UNCONFIRMED_KEY)
                    .is_some_and(|v| v == "quorum");
            let deposed = status.code() == tonic::Code::FailedPrecondition
                && status
                    .metadata()
                    .get(crate::grpc::NOT_LEADER_KEY)
                    .is_some_and(|v| v == "true");
            assert!(
                quorum || deposed,
                "expired lease refusal lost its typed RPC meaning: {status:?}"
            );
        }
    });
    assert_eq!(
        backend.driver.lease_read_hits(),
        hits,
        "expired lease returned a successful local read"
    );
}
