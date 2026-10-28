use super::*;
use crate::client::{
    ttl::{ScopedTtlClient, TtlOutcome},
    TransportKind,
};
use crate::grpc::{proto, proto::kv9_server::Kv9, Kv9Grpc};
use crate::point_test_support::{authenticator, memory_channel, Backend};
use crate::point_wire::Handler;
use crate::ttl_clock::{ClockResult, ClockStatus, TtlClock};
use kv9_common::raw_ttl::{ClockSample, ExpiryRecord};
use kv9_common::{RootDigest, StoreIncarnation, TenantId};
use kv9_raft::{
    transport::{InProcHub, RaftTransport},
    MemStateMachine, RaftGroup, RaftPeer,
};
use kv9_region::RegionEpoch;
use prost::Message;
use std::path::PathBuf;

mod headroom;
mod maintenance;

#[tokio::test]
async fn public_ttl_reopened_above_node_byte_threshold_refuses_before_append() {
    let mut f = Fixture::new(Some(Sample::at(1000)));
    f.put(b"kept", b"value", None).await;
    f.stop();
    let path = f.path.clone();
    drop(f);
    let mut f = Fixture::open_range_budget(path.clone(), Some(Sample::at(1000)), range(), 1);
    let before = f.group.driver.status().log_last_index;
    for operation in [
        Operation::Put {
            pairs: vec![(b"refused".to_vec(), b"v".to_vec())],
            ttl_secs: Some(10),
        },
        Operation::Get {
            keys: vec![b"kept".to_vec()],
        },
        Operation::Delete {
            key: b"kept".to_vec(),
        },
    ] {
        let error = f.call(operation).await.unwrap_err();
        assert!(matches!(
            crate::client::ttl::classify(&error),
            crate::client::ttl::Failure::Refused {
                refusal: crate::raw_ttl::Refusal::Backpressure
            }
        ));
        assert_eq!(f.group.driver.status().log_last_index, before);
        assert_eq!(f.group.driver.async_apply_snapshot().in_flight, 0);
    }
    f.call(Operation::Capabilities).await.unwrap();
    f.stop();
    drop(f);
    let f = Fixture::open(path, Some(Sample::at(1000)));
    assert_eq!(
        f.values(&[b"kept", b"refused"]).await,
        vec![Some(b"value".to_vec()), None]
    );
    f.close();
}

#[tokio::test]
async fn public_ttl_backpressure_refuses_observations_and_writes_without_append() {
    let f = Fixture::new(Some(Sample::at(1000)));
    let status = f.group.driver.status();
    f.group
        .driver
        .peer()
        .set_max_write_log_entries(status.log_last_index - status.log_first_index + 2);
    f.put(b"kept", b"value", None).await;
    let before = f.group.driver.status().log_last_index;
    for op in [
        Operation::Put {
            pairs: vec![(b"refused".to_vec(), b"v".to_vec())],
            ttl_secs: Some(10),
        },
        Operation::Get {
            keys: vec![b"kept".to_vec()],
        },
        Operation::Delete {
            key: b"kept".to_vec(),
        },
    ] {
        let error = f.call(op).await.unwrap_err();
        assert!(matches!(
            crate::client::ttl::classify(&error),
            crate::client::ttl::Failure::Refused {
                refusal: crate::raw_ttl::Refusal::Backpressure
            }
        ));
        assert_eq!(f.group.driver.status().log_last_index, before);
    }
    f.call(Operation::Capabilities).await.unwrap();
    let replacement = RawGroup::new(
        f.group.binding.clone(),
        f.group.engine.clone(),
        f.group.driver.clone(),
        f.group.clock.clone(),
    );
    assert!(matches!(
        replacement.write_pressure(),
        Err(Error::WriteBackpressure { .. })
    ));
    drop(replacement);
    f.group.driver.peer().set_max_write_log_entries(0);
    assert_eq!(
        f.values(&[b"kept", b"refused"]).await,
        vec![Some(b"value".to_vec()), None]
    );
    f.close();
}

struct Sample(Mutex<ClockResult<ClockSample>>);
impl Sample {
    fn at(ms: u64) -> Arc<Self> {
        Arc::new(Self(Mutex::new(Ok(
            ClockSample::with_uncertainty(ms, 10).unwrap()
        ))))
    }
    fn set(&self, ms: u64) {
        *self.0.lock().unwrap() = Ok(ClockSample::with_uncertainty(ms, 10).unwrap());
    }
}
impl TtlClock for Sample {
    fn sample(&self) -> ClockResult<ClockSample> {
        *self.0.lock().unwrap()
    }
    fn status(&self) -> ClockStatus {
        panic!("status cannot authorize a sample")
    }
}
fn range() -> DataRange {
    DataRange {
        root: RootDigest::from_bytes([1; 32]),
        creation: RootDigest::from_bytes([2; 32]),
        region: RegionId(100),
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: vec![],
        end: vec![],
        sealed: false,
    }
}
struct Fixture {
    path: PathBuf,
    group: Arc<RawGroup>,
    owner: Option<std::thread::JoinHandle<()>>,
    api: Kv9Grpc,
}
impl Fixture {
    fn new(clock: Option<Arc<dyn TtlClock>>) -> Self {
        let path = std::env::temp_dir().join(format!(
            "kv9-ttl-public-{}",
            StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self::open(path, clock)
    }
    fn open(path: PathBuf, clock: Option<Arc<dyn TtlClock>>) -> Self {
        Self::open_range(path, clock, range())
    }
    fn open_range(path: PathBuf, clock: Option<Arc<dyn TtlClock>>, binding: DataRange) -> Self {
        Self::open_range_budget(
            path,
            clock,
            binding,
            kv9_common::log_budget::DEFAULT_LOG_BYTES,
        )
    }
    fn open_range_budget(
        path: PathBuf,
        clock: Option<Arc<dyn TtlClock>>,
        binding: DataRange,
        limit: u64,
    ) -> Self {
        let budget = Arc::new(kv9_common::log_budget::LogBudget::new(limit).unwrap());
        Self::open_with_log_budget(path, clock, binding, budget)
    }
    fn open_with_log_budget(
        path: PathBuf,
        clock: Option<Arc<dyn TtlClock>>,
        binding: DataRange,
        budget: Arc<kv9_common::log_budget::LogBudget>,
    ) -> Self {
        let engine = Arc::new(WalEngine::open(path.join("engine.wal")).unwrap().0);
        engine.attach_log_budget(&budget).unwrap();
        let fresh = engine
            .get(ColumnFamily::Default, RANGE_KEY)
            .unwrap()
            .is_none();
        let storage = DiskRaftStorage::open(&path.join("raft"), &[1]).unwrap().0;
        let debt = storage.attach_log_budget(&budget).unwrap();
        let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
        sm.set_data_group(binding.root, binding.region, binding.creation)
            .unwrap();
        let hub = InProcHub::new();
        let driver = NodeDriver::new(
            Arc::new(RaftPeer::with_storage(NodeId(1), binding.region, storage).unwrap()),
            Arc::new(hub.endpoint(NodeId(1))) as Arc<dyn RaftTransport>,
            sm,
        )
        .unwrap();
        driver.attach_log_debt(debt).unwrap();
        driver.peer().campaign().unwrap();
        for _ in 0..10 {
            driver.step().unwrap();
        }
        if fresh {
            driver
                .propose(&Command::DataRange {
                    expected: None,
                    next: binding.clone(),
                })
                .unwrap();
            for _ in 0..10 {
                driver.step().unwrap();
            }
            // Explicit fixture injection, not production activation authority.
            driver
                .peer()
                .propose_raw_for_harness(
                    Command::RawTtl(
                        RawTtlCommand::new(
                            &binding,
                            RawTtlAction::Enable {
                                observed: ClockWatermark::default(),
                            },
                        )
                        .unwrap(),
                    )
                    .encode(),
                )
                .unwrap();
            for _ in 0..10 {
                driver.step().unwrap();
            }
        }
        let owner = driver.spawn(Duration::from_millis(2)).unwrap();
        let group = RawGroup::new(binding.clone(), engine, driver, clock);
        let api = Kv9Grpc::new(Arc::new(Backend {
            scoped: Some((binding, group.clone())),
            ..Backend::default()
        }));
        Self {
            path,
            group,
            owner: Some(owner),
            api,
        }
    }
    async fn call(
        &self,
        op: Operation,
    ) -> std::result::Result<proto::RawTtlResponse, tonic::Status> {
        let body = proto::RawTtlRequest {
            binding: self.group.binding.encode(),
            protocol_version: crate::raw_ttl::PROTOCOL_VERSION,
            budget_micros: 5_000_000,
            operation: Some(op.into()),
        };
        let handler = Handler {
            api: self.api.clone(),
            authenticator: authenticator(),
        };
        self.api
            .raw_ttl(
                handler
                    .request("Bearer secret", &body.encode_to_vec())
                    .unwrap(),
            )
            .await
            .map(tonic::Response::into_inner)
    }
    async fn put(&self, key: &[u8], value: &[u8], ttl_secs: Option<u64>) -> AppliedPosition {
        let result = self
            .call(Operation::Put {
                pairs: vec![(key.to_vec(), value.to_vec())],
                ttl_secs,
            })
            .await
            .unwrap();
        let Some(proto::raw_ttl_response::Result::Applied(at)) = result.result else {
            panic!("receipt expected")
        };
        AppliedPosition {
            term: at.applied_term,
            index: at.applied_index,
        }
    }
    async fn values(&self, keys: &[&[u8]]) -> Vec<Option<Vec<u8>>> {
        let result = self
            .call(Operation::Get {
                keys: keys.iter().map(|k| k.to_vec()).collect(),
            })
            .await
            .unwrap();
        let Some(proto::raw_ttl_response::Result::Values(v)) = result.result else {
            panic!("values expected")
        };
        v.values
            .into_iter()
            .map(|v| v.found.then_some(v.value))
            .collect()
    }
    fn expiry(&self, key: &[u8]) -> Option<ExpiryRecord> {
        self.group
            .engine
            .get(
                ColumnFamily::Write,
                &kv9_common::codec::encode_key(
                    kv9_common::codec::KeyMode::Raw,
                    range().keyspace,
                    key,
                )
                .unwrap(),
            )
            .unwrap()
            .map(|v| ExpiryRecord::decode(&v).unwrap())
    }
    fn stop(&mut self) {
        self.group.driver.stop();
        if let Some(owner) = self.owner.take() {
            owner.join().unwrap();
        }
    }
    fn close(mut self) {
        self.stop();
        let path = self.path.clone();
        drop(self);
        std::fs::remove_dir_all(path).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop();
    }
}

#[tokio::test]
async fn public_deadlines_presence_receipts_and_duplicate_reads_survive_wal_reopen() {
    let clock = Sample::at(1000);
    let mut f = Fixture::new(Some(clock.clone()));
    let at = f.put(b"k", b"KV9TTL01 opaque", Some(1)).await;
    let record = f.expiry(b"k").unwrap();
    assert_eq!(
        record,
        ExpiryRecord::new(range().creation, at, 2010).unwrap()
    );
    f.put(b"empty", b"", None).await;
    let before = f.group.engine.snapshot().unwrap();
    clock.set(2015); // earliest = 2005 < deadline: no early expiry.
    assert_eq!(
        f.values(&[b"k", b"empty", b"k", b"missing"]).await,
        vec![
            Some(b"KV9TTL01 opaque".to_vec()),
            Some(vec![]),
            Some(b"KV9TTL01 opaque".to_vec()),
            None
        ]
    );
    clock.set(2020);
    assert_eq!(f.values(&[b"k", b"empty"]).await, vec![None, Some(vec![])]);
    assert_eq!(
        TtlRead::from_leader(&LeaderRead::new(before.as_ref(), true, None).unwrap())
            .unwrap()
            .get(range().keyspace, b"k")
            .unwrap(),
        Some(b"KV9TTL01 opaque".to_vec())
    );
    drop(before);
    let path = f.path.clone();
    f.stop();
    drop(f);
    let f = Fixture::open(path, Some(clock.clone()));
    assert_eq!(f.expiry(b"k"), Some(record));
    clock.set(1500);
    let before = f.group.driver.driver_applied();
    let refusal = f
        .call(Operation::Get {
            keys: vec![b"k".to_vec()],
        })
        .await
        .unwrap_err();
    assert!(matches!(
        crate::client::ttl::classify(&refusal),
        crate::client::ttl::Failure::Refused {
            refusal: crate::raw_ttl::Refusal::Clock {
                reason: ClockRefusal::BehindWatermark
            }
        }
    ));
    assert_eq!(f.group.driver.driver_applied(), before);
    clock.set(2100);
    assert_eq!(f.values(&[b"k"]).await, vec![None]);
    f.close();
}

#[tokio::test]
async fn permanent_zero_and_delete_need_no_clock_but_positive_writes_and_reads_do() {
    let f = Fixture::new(None);
    f.put(b"k", b"zero", Some(0)).await;
    assert!(f
        .expiry(b"k")
        .unwrap()
        .expired_at(ClockWatermark::default()));
    f.put(b"k", b"permanent", None).await;
    assert!(f.expiry(b"k").is_none());
    let before = f.group.driver.driver_applied();
    for op in [
        Operation::Put {
            pairs: vec![(b"k".to_vec(), b"forbidden".to_vec())],
            ttl_secs: Some(1),
        },
        Operation::Get {
            keys: vec![b"k".to_vec()],
        },
    ] {
        assert!(matches!(
            crate::client::ttl::classify(&f.call(op).await.unwrap_err()),
            crate::client::ttl::Failure::Refused {
                refusal: crate::raw_ttl::Refusal::Clock {
                    reason: ClockRefusal::Unavailable
                }
            }
        ));
        assert_eq!(f.group.driver.driver_applied(), before);
    }
    f.call(Operation::Delete { key: b"k".to_vec() })
        .await
        .unwrap();
    assert!(f.expiry(b"k").is_none());
    let view = f.group.engine.snapshot().unwrap();
    assert_eq!(
        TtlRead::from_leader(&LeaderRead::new(view.as_ref(), true, None).unwrap())
            .unwrap()
            .get(range().keyspace, b"k")
            .unwrap(),
        None
    );
    // The legacy reader still refuses the SAME activated engine.
    let ctx = RequestContext {
        keyspace: range().keyspace,
        region_epoch: RegionEpoch {
            conf_ver: 1,
            version: 1,
        },
        origin: crate::api::RequestOrigin::from_transport("test"),
    };
    assert!(matches!(
        f.group.authorize(view.as_ref(), &ctx, KeySpan::Point(b"k")),
        Err(Error::RawTtlRejected {
            reason: TtlRejection::LegacyCommand,
            ..
        })
    ));
    drop(view);
    f.close();
}

#[tokio::test]
async fn public_scan_continues_after_a_full_expired_page_and_maximum_key_cursor() {
    let f = Fixture::new(Some(Sample::at(1000)));
    for start in (0..MAX_SCAN_EXAMINED).step_by(crate::client::MAX_BATCH_ITEMS) {
        let pairs = (start..start + crate::client::MAX_BATCH_ITEMS)
            .map(|i| ((i as u32).to_be_bytes().to_vec(), b"expired".to_vec()))
            .collect();
        f.call(Operation::Put {
            pairs,
            ttl_secs: Some(0),
        })
        .await
        .unwrap();
    }
    f.put(b"z", b"live", None).await;
    let page = f
        .call(Operation::Scan {
            start: vec![],
            end: vec![],
            limit: 10,
        })
        .await
        .unwrap();
    let Some(proto::raw_ttl_response::Result::Page(page)) = page.result else {
        panic!()
    };
    assert!(page.pairs.is_empty());
    assert_eq!(page.examined, MAX_SCAN_EXAMINED as u32);
    let next = f
        .call(Operation::Scan {
            start: page.next.unwrap(),
            end: vec![],
            limit: 10,
        })
        .await
        .unwrap();
    let Some(proto::raw_ttl_response::Result::Page(page)) = next.result else {
        panic!()
    };
    assert_eq!(
        page.pairs,
        vec![proto::KeyValue {
            key: b"z".to_vec(),
            value: b"live".to_vec()
        }]
    );
    assert!(page.next.is_none());
    let max = vec![0xff; crate::client::MAX_KEY_BYTES];
    f.put(&max, b"last", None).await;
    let page = f
        .call(Operation::Scan {
            start: max,
            end: vec![],
            limit: 1,
        })
        .await
        .unwrap();
    let Some(proto::raw_ttl_response::Result::Page(page)) = page.result else {
        panic!()
    };
    let cursor = page.next.unwrap();
    assert_eq!(cursor.len(), crate::client::MAX_KEY_BYTES + 1);
    let eof = f
        .call(Operation::Scan {
            start: cursor,
            end: vec![],
            limit: 1,
        })
        .await
        .unwrap();
    let Some(proto::raw_ttl_response::Result::Page(page)) = eof.result else {
        panic!()
    };
    assert!(page.pairs.is_empty() && page.next.is_none());
    f.close();
}

#[tokio::test]
async fn both_http2_adapters_execute_the_scoped_client_against_real_raft() {
    let clock = Sample::at(1000);
    let f = Fixture::new(Some(clock.clone()));
    let (channel, server) = memory_channel(f.api.clone()).await;
    for transport in [TransportKind::TonicUnary, TransportKind::TonicStream] {
        let c = ScopedTtlClient::with_test_transport(range(), transport, channel.clone(), 5000);
        let put = c
            .call(Operation::Put {
                pairs: vec![(b"k".to_vec(), b"v".to_vec())],
                ttl_secs: Some(0),
            })
            .await;
        assert!(
            matches!(
                put.outcome,
                TtlOutcome::Success {
                    reply: Reply::Applied { .. }
                }
            ),
            "{put:?}"
        );
        assert_eq!(put.attempts.len(), 2);
        assert!(put.attempts[0].capability);
        assert!(!put.attempts[1].capability);
        let get = c
            .call(Operation::Get {
                keys: vec![b"k".to_vec(), b"k".to_vec()],
            })
            .await;
        assert_eq!(
            get.outcome,
            TtlOutcome::Success {
                reply: Reply::Values {
                    values: vec![None, None]
                }
            }
        );
        *clock.0.lock().unwrap() = Err(ClockRefusal::Fenced);
        let fail = c
            .call(Operation::Put {
                pairs: vec![(b"k".to_vec(), b"bad".to_vec())],
                ttl_secs: Some(1),
            })
            .await;
        assert!(matches!(
            fail.outcome,
            TtlOutcome::Refused {
                failure: crate::client::ttl::Failure::Refused {
                    refusal: crate::raw_ttl::Refusal::Clock {
                        reason: ClockRefusal::Fenced
                    }
                }
            }
        ));
        assert_eq!(fail.attempts.len(), 2);
        clock.set(1000);
    }
    drop(channel);
    server.abort();
    let _ = server.await;
    f.close();
}

#[tokio::test]
async fn public_scan_byte_budget_keeps_the_first_unreturned_row() {
    let f = Fixture::new(Some(Sample::at(1000)));
    let value = vec![3; crate::client::MAX_VALUE_BYTES];
    for i in 0..20u8 {
        f.put(&[i], &value, None).await;
    }
    let mut cursor = Vec::new();
    let mut seen = Vec::new();
    let mut pages = 0;
    loop {
        let response = f
            .call(Operation::Scan {
                start: cursor,
                end: vec![],
                limit: 256,
            })
            .await
            .unwrap();
        assert!(response.encoded_len() <= crate::client::MAX_MESSAGE_BYTES);
        let Some(proto::raw_ttl_response::Result::Page(page)) = response.result else {
            panic!()
        };
        seen.extend(page.pairs.into_iter().map(|p| p.key));
        pages += 1;
        match page.next {
            Some(next) => cursor = next,
            None => break,
        }
    }
    assert_eq!(seen, (0..20u8).map(|i| vec![i]).collect::<Vec<_>>());
    assert_eq!(pages, 2);
    f.close();
}

#[tokio::test]
async fn sampling_delay_cannot_reset_the_deadline_or_append_after_it() {
    struct Slow;
    impl TtlClock for Slow {
        fn sample(&self) -> ClockResult<ClockSample> {
            std::thread::sleep(Duration::from_millis(30));
            Ok(ClockSample::with_uncertainty(1000, 10).unwrap())
        }
        fn status(&self) -> ClockStatus {
            panic!("not sample authority")
        }
    }
    let f = Fixture::new(Some(Arc::new(Slow)));
    let before = f.group.driver.driver_applied();
    let ctx = RequestContext {
        keyspace: range().keyspace,
        region_epoch: RegionEpoch {
            conf_ver: 1,
            version: 1,
        },
        origin: crate::api::RequestOrigin::from_transport("deadline"),
    };
    let preparation = f.group.clone().prepare_raw_ttl(
        ctx,
        Operation::Put {
            pairs: vec![(b"k".to_vec(), b"v".to_vec())],
            ttl_secs: Some(1),
        },
        Instant::now() + Duration::from_millis(20),
    );
    assert!(tokio::task::spawn_blocking(preparation)
        .await
        .unwrap()
        .is_err());
    assert_eq!(f.group.driver.driver_applied(), before);
    assert!(f.expiry(b"k").is_none());
    f.close();
}

#[tokio::test]
async fn delete_range_removes_both_column_families_in_bounded_chunks_without_a_clock_and_survives_reopen(
) {
    let clock = Sample::at(1000);
    let mut f = Fixture::new(Some(clock.clone()));
    for (key, ttl) in [
        (b"a".as_slice(), Some(0)),
        (b"a\0", Some(60)),
        (b"b", None),
        (b"z", Some(60)),
    ] {
        f.put(key, b"v", ttl).await;
    }
    *clock.0.lock().unwrap() = Err(ClockRefusal::Fenced);
    let (channel, server) = memory_channel(f.api.clone()).await;
    let client =
        ScopedTtlClient::with_test_transport(range(), TransportKind::TonicStream, channel, 5000);
    let before = f.group.engine.snapshot().unwrap();
    let report = client
        .call(Operation::DeleteRange {
            start: vec![],
            end: b"z".to_vec(),
            limit: 2,
        })
        .await;
    let TtlOutcome::Success {
        reply:
            Reply::Deleted {
                processed_keys: 2,
                applied: Some(at),
                next: Some(next),
            },
    } = report.outcome
    else {
        panic!("{report:?}")
    };
    assert_eq!(next, b"a\0\0");
    let actual = f.group.driver.driver_applied().unwrap();
    assert_eq!((actual.term, actual.index), (at.term, at.index));
    for key in [b"a".as_slice(), b"a\0"] {
        let physical =
            kv9_common::codec::encode_key(kv9_common::codec::KeyMode::Raw, range().keyspace, key)
                .unwrap();
        assert!(before
            .get(ColumnFamily::Default, &physical)
            .unwrap()
            .is_some());
        assert!(f
            .group
            .engine
            .get(ColumnFamily::Default, &physical)
            .unwrap()
            .is_none());
        assert!(f.expiry(key).is_none());
    }
    drop(before);
    server.abort();
    let _ = server.await;
    drop(client);
    f.stop();
    let path = f.path.clone();
    drop(f);
    let f = Fixture::open(path, None);
    let result = f
        .call(Operation::DeleteRange {
            start: next,
            end: b"z".to_vec(),
            limit: 2,
        })
        .await
        .unwrap();
    assert!(matches!(
        result.result,
        Some(proto::raw_ttl_response::Result::Deleted(
            proto::RawTtlDeleted {
                processed_keys: 1,
                applied: Some(_),
                next: None
            }
        ))
    ));
    let before_empty = f.group.driver.driver_applied();
    let empty = f
        .call(Operation::DeleteRange {
            start: vec![],
            end: b"z".to_vec(),
            limit: 2,
        })
        .await
        .unwrap();
    assert!(matches!(
        empty.result,
        Some(proto::raw_ttl_response::Result::Deleted(
            proto::RawTtlDeleted {
                processed_keys: 0,
                applied: None,
                next: None
            }
        ))
    ));
    assert_eq!(f.group.driver.driver_applied(), before_empty);
    assert!(f.expiry(b"z").is_some());
    assert!(f.expiry(b"a").is_none());
    f.close();
}

#[tokio::test]
async fn scoped_scan_and_delete_accept_full_catalog_bounds_in_both_http2_transports() {
    let mut binding = range();
    binding.start = vec![1; kv9_common::data_range::MAX_BOUND];
    binding.end = vec![2; kv9_common::data_range::MAX_BOUND];
    let path = std::env::temp_dir().join(format!(
        "kv9-ttl-long-bounds-{}",
        StoreIncarnation::mint().unwrap()
    ));
    let f = Fixture::open_range(path, Some(Sample::at(1000)), binding.clone());
    let (channel, server) = memory_channel(f.api.clone()).await;
    for transport in [TransportKind::TonicUnary, TransportKind::TonicStream] {
        f.put(&[1, 2], b"v", None).await;
        let c =
            ScopedTtlClient::with_test_transport(binding.clone(), transport, channel.clone(), 5000);
        let result = c
            .call(Operation::Scan {
                start: binding.start.clone(),
                end: binding.end.clone(),
                limit: 10,
            })
            .await;
        assert_eq!(
            result.outcome,
            TtlOutcome::Success {
                reply: Reply::Page {
                    rows: vec![(vec![1, 2], b"v".to_vec())],
                    next: None,
                    examined: 1
                }
            }
        );
        let result = c
            .call(Operation::DeleteRange {
                start: binding.start.clone(),
                end: binding.end.clone(),
                limit: 10,
            })
            .await;
        assert!(
            matches!(
                result.outcome,
                TtlOutcome::Success {
                    reply: Reply::Deleted {
                        processed_keys: 1,
                        applied: Some(_),
                        next: None
                    }
                }
            ),
            "{result:?}"
        );
    }
    server.abort();
    let _ = server.await;
    drop(channel);
    f.close();
}
