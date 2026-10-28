use super::*;
use crate::grpc::Kv9Grpc;
use crate::point_test_support::{memory_channel, Backend};
use kv9_common::{KeyspaceId, RegionId, RootDigest, TenantId};
use std::sync::atomic::AtomicUsize;
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
fn op() -> Operation {
    Operation::Put {
        pairs: vec![(b"k".to_vec(), b"v".to_vec())],
        ttl_secs: Some(1),
    }
}

#[test]
fn mixed_duplicate_unknown_or_malformed_refusals_cannot_claim_no_write_effect() {
    let value = serde_json::to_string(&Refusal::NotEnabled).unwrap();
    let mut valid = Status::failed_precondition("refused");
    valid.metadata_mut().insert(REFUSED, value.parse().unwrap());
    assert!(matches!(classify(&valid), Failure::Refused { .. }));
    let mut duplicate = valid.clone();
    duplicate
        .metadata_mut()
        .append(REFUSED, value.parse().unwrap());
    let mut mixed = valid.clone();
    mixed
        .metadata_mut()
        .insert(NOT_LEADER_KEY, "true".parse().unwrap());
    let mut unknown = valid.clone();
    unknown
        .metadata_mut()
        .insert("kv9-other", "x".parse().unwrap());
    let mut bad = valid.clone();
    bad.metadata_mut().insert(REFUSED, "{}".parse().unwrap());
    let mut wrong_code = Status::unavailable("refused");
    *wrong_code.metadata_mut() = valid.metadata().clone();
    for status in [duplicate, mixed, unknown, bad, wrong_code] {
        assert_eq!(classify(&status), rpc(Reason::Protocol));
        assert!(matches!(
            data_outcome(Err(classify(&status)), false),
            TtlOutcome::UnknownWrite { .. }
        ));
    }
}

#[tokio::test]
async fn unsupported_capability_never_sends_the_write_or_falls_back_to_legacy() {
    let target = Arc::new(Backend::default());
    let api = Kv9Grpc::new(Arc::new(Backend {
        scoped: Some((range(), target.clone())),
        ..Backend::default()
    }));
    let (channel, server) = memory_channel(api).await;
    for transport in [TransportKind::TonicUnary, TransportKind::TonicStream] {
        let c = ScopedTtlClient::with_test_transport(range(), transport, channel.clone(), 1000);
        let report = c.call(op()).await;
        assert_eq!(report.attempts.len(), 1);
        assert!(report.attempts[0].capability);
        assert!(
            matches!(
                report.outcome,
                TtlOutcome::CapabilityFailure {
                    failure: Failure::Rpc {
                        reason: Reason::RpcStatus { code: 12 }
                    }
                }
            ),
            "{report:?}"
        );
        assert!(target.values.lock().unwrap().is_empty());
        assert!(target.calls.lock().unwrap().is_empty());
    }
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn lost_data_reply_never_retries_and_cancellation_retains_admission() {
    for (transport, read) in [
        (TransportKind::TonicUnary, false),
        (TransportKind::TonicStream, false),
        (TransportKind::TonicUnary, true),
        (TransportKind::TonicStream, true),
    ] {
        let writes = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Notify::new());
        let target = Arc::new(Backend {
            ttl_prepare: Some(Arc::new({
                let writes = writes.clone();
                let release = release.clone();
                move |_, op, _| {
                    let writes = writes.clone();
                    let release = release.clone();
                    Box::new(move || {
                        Ok(Box::pin(async move {
                            if matches!(op, Operation::Capabilities) {
                                return Ok(Reply::Capabilities);
                            }
                            writes.fetch_add(1, Ordering::SeqCst);
                            release.notified().await;
                            if op.is_read() {
                                Ok(Reply::Values { values: vec![None] })
                            } else {
                                Ok(Reply::Applied { term: 1, index: 2 })
                            }
                        }))
                    })
                }
            })),
            ..Backend::default()
        });
        let api = Kv9Grpc::new(Arc::new(Backend {
            scoped: Some((range(), target)),
            ..Backend::default()
        }));
        let admission = api.admission();
        let (channel, server) = memory_channel(api).await;
        let c = ScopedTtlClient::with_test_transport(range(), transport, channel, 250);
        let operation = if read {
            Operation::Get {
                keys: vec![b"k".to_vec()],
            }
        } else {
            op()
        };
        let report = c.call(operation).await;
        assert!(
            if read {
                matches!(report.outcome, TtlOutcome::ReadFailure { .. })
            } else {
                matches!(report.outcome, TtlOutcome::UnknownWrite { .. })
            },
            "{report:?}"
        );
        assert_eq!(report.attempts.len(), 2);
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.snapshot().in_flight,
            1,
            "detached completion still owns capacity"
        );
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), async {
            while admission.snapshot().in_flight != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        server.abort();
        let _ = server.await;
    }
}

#[test]
fn replies_require_matching_scope_kind_version_and_monotonic_cursor() {
    let scan = Operation::Scan {
        start: b"a".to_vec(),
        end: b"z".to_vec(),
        limit: 2,
    };
    let r = range();
    let valid = proto::RawTtlResponse {
        binding_digest: r.digest().as_bytes().to_vec(),
        protocol_version: PROTOCOL_VERSION,
        result: Some(proto::raw_ttl_response::Result::Page(proto::RawTtlPage {
            pairs: vec![],
            next: Some(b"b".to_vec()),
            examined: 4096,
        })),
    };
    assert!(matches!(
        decode(tonic::Response::new(valid.clone()), &scan, &r),
        Ok(Reply::Page { next: Some(_), .. })
    ));
    let mut no_work = valid.clone();
    let Some(proto::raw_ttl_response::Result::Page(page)) = &mut no_work.result else {
        panic!()
    };
    page.examined = 0;
    assert!(decode(tonic::Response::new(no_work), &scan, &r).is_err());
    let mut wrong = valid.clone();
    wrong.binding_digest[0] ^= 1;
    assert!(decode(tonic::Response::new(wrong), &scan, &r).is_err());
    let mut wrong = valid.clone();
    wrong.protocol_version = 0;
    assert!(decode(tonic::Response::new(wrong), &scan, &r).is_err());
    assert!(decode(tonic::Response::new(valid.clone()), &op(), &r).is_err());
    for next in [
        b"a".to_vec(),
        b"z".to_vec(),
        vec![b'b'; crate::raw_ttl::MAX_RANGE_BOUND_BYTES + 1],
    ] {
        let mut wrong = valid.clone();
        let Some(proto::raw_ttl_response::Result::Page(p)) = &mut wrong.result else {
            panic!()
        };
        p.next = Some(next);
        assert!(decode(tonic::Response::new(wrong), &scan, &r).is_err());
    }
}

#[tokio::test]
async fn lost_activation_reply_never_retries_and_keeps_admission_until_completion() {
    // Activation always uses its separate unary RPC, regardless of data transport.
    for transport in [TransportKind::TonicUnary, TransportKind::TonicStream] {
        let submitted = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Notify::new());
        let backend = Arc::new(Backend {
            activate_prepare: Some(Arc::new({
                let submitted = submitted.clone();
                let release = release.clone();
                move |_, scope, _| {
                    assert_eq!(scope, range());
                    let submitted = submitted.clone();
                    let release = release.clone();
                    Box::new(move || {
                        submitted.fetch_add(1, Ordering::SeqCst);
                        Ok(Box::pin(async move {
                            release.notified().await;
                            Ok(crate::point_test_support::APPLIED)
                        }))
                    })
                }
            })),
            ..Backend::default()
        });
        let api = Kv9Grpc::new(backend.clone());
        let admission = api.admission();
        let (channel, server) = memory_channel(api).await;
        let client = ScopedTtlClient::with_test_transport(range(), transport, channel, 250);
        let report = client.activate().await;
        assert!(
            matches!(report.outcome, TtlOutcome::UnknownWrite { .. }),
            "{report:?}"
        );
        assert_eq!(report.attempts.len(), 1);
        assert_eq!(submitted.load(Ordering::SeqCst), 1);
        assert_eq!(admission.snapshot().in_flight, 1);
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), async {
            while admission.snapshot().in_flight != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(submitted.load(Ordering::SeqCst), 1);
        assert!(
            backend.calls.lock().unwrap().is_empty(),
            "no legacy fallback"
        );
        server.abort();
        let _ = server.await;
    }
}

#[test]
fn deletion_receipts_require_exact_scope_kind_count_position_and_forward_cursor() {
    let op = Operation::DeleteRange {
        start: b"a".to_vec(),
        end: b"z".to_vec(),
        limit: 2,
    };
    let r = range();
    let valid = proto::RawTtlResponse {
        binding_digest: r.digest().as_bytes().to_vec(),
        protocol_version: PROTOCOL_VERSION,
        result: Some(proto::raw_ttl_response::Result::Deleted(
            proto::RawTtlDeleted {
                processed_keys: 2,
                applied: Some(proto::RawWriteResponse {
                    applied_term: 1,
                    applied_index: 2,
                }),
                next: Some(b"c".to_vec()),
            },
        )),
    };
    assert!(decode(tonic::Response::new(valid.clone()), &op, &r).is_ok());
    for mode in 0..9 {
        let mut bad = valid.clone();
        let Some(proto::raw_ttl_response::Result::Deleted(p)) = &mut bad.result else {
            panic!()
        };
        match mode {
            0 => bad.protocol_version = 1,
            1 => bad.binding_digest[0] ^= 1,
            2 => p.processed_keys = 0,
            3 => p.processed_keys = 3,
            4 => p.applied = None,
            5 => p.applied.as_mut().unwrap().applied_index = 0,
            6 => p.next = Some(b"a".to_vec()),
            7 => p.next = Some(b"z".to_vec()),
            _ => p.next = Some(vec![b'b'; crate::raw_ttl::MAX_RANGE_BOUND_BYTES + 1]),
        }
        let result = decode(tonic::Response::new(bad), &op, &r);
        assert!(
            matches!(data_outcome(result, false), TtlOutcome::UnknownWrite { .. }),
            "mode {mode}"
        );
    }
    let empty = proto::RawTtlResponse {
        result: Some(proto::raw_ttl_response::Result::Deleted(
            proto::RawTtlDeleted {
                processed_keys: 0,
                applied: None,
                next: None,
            },
        )),
        ..valid
    };
    assert!(decode(tonic::Response::new(empty), &op, &r).is_ok());
}
