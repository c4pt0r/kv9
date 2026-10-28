use super::*;
use crate::client::ttl::{classify, Failure};
use crate::raw_ttl::Refusal;
use kv9_common::raw_ttl::clock::ClockRefusal;
use kv9_common::RegionId;

#[test]
fn definitive_refusals_are_typed_and_unknown_errors_are_not_marked() {
    for (error, refusal) in [
        (
            Error::RawTtlClock {
                reason: ClockRefusal::Fenced,
            },
            Refusal::Clock {
                reason: ClockRefusal::Fenced,
            },
        ),
        (
            Error::RawTtlRejected {
                region: RegionId(100),
                reason: TtlRejection::NotEnabled,
            },
            Refusal::NotEnabled,
        ),
        (
            Error::StaleEpoch {
                region: RegionId(100),
            },
            Refusal::StaleScope,
        ),
        (
            Error::WriteBackpressure {
                region: RegionId(100),
            },
            Refusal::Backpressure,
        ),
    ] {
        assert_eq!(classify(&ttl_error(error)), Failure::Refused { refusal });
    }
    let unknown = ttl_error(Error::Raft("accepted but response unconfirmed".into()));
    assert!(!unknown.metadata().contains_key(REFUSED));
}

#[test]
fn protobuf_presence_distinguishes_clear_immediate_and_positive_expiry() {
    for ttl_secs in [None, Some(0), Some(1), Some(u64::MAX)] {
        let request = proto::RawTtlPut {
            pairs: vec![proto::KeyValue {
                key: b"k".to_vec(),
                value: vec![],
            }],
            ttl_secs,
        };
        assert_eq!(
            proto::RawTtlPut::decode(request.encode_to_vec().as_slice())
                .unwrap()
                .ttl_secs,
            ttl_secs
        );
    }
    assert!(Operation::Put {
        pairs: vec![(b"k".to_vec(), vec![])],
        ttl_secs: Some(u64::MAX)
    }
    .validate()
    .is_err());
}

#[tokio::test]
async fn activation_accepts_full_catalog_bounds_and_v1_cannot_authorize_v2_operations() {
    use crate::point_test_support::{memory_channel, request, Backend, APPLIED};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let binding = DataRange {
        root: kv9_common::RootDigest::from_bytes([1; 32]),
        creation: kv9_common::RootDigest::from_bytes([2; 32]),
        region: RegionId(100),
        keyspace: KeyspaceId(100),
        tenant: TenantId(1),
        conf_ver: 1,
        version: 1,
        start: vec![1; kv9_common::data_range::MAX_BOUND],
        end: vec![2; kv9_common::data_range::MAX_BOUND],
        sealed: false,
    };
    let api = Kv9Grpc::new(Arc::new(Backend {
        activate_prepare: Some(Arc::new({
            let binding = binding.clone();
            move |_, actual, _| {
                assert_eq!(actual, binding);
                Box::new(|| Ok(Box::pin(async { Ok(APPLIED) })))
            }
        })),
        route_target: Some(Arc::new({
            let calls = calls.clone();
            move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(Error::NotImplemented("unexpected V1 dispatch"))
            }
        })),
        ..Backend::default()
    }));
    let (channel, server) = memory_channel(api).await;
    let client = crate::client::ttl::ScopedTtlClient::with_test_transport(
        binding.clone(),
        crate::client::TransportKind::TonicUnary,
        channel.clone(),
        5000,
    );
    assert!(matches!(
        client.activate().await.outcome,
        crate::client::ttl::TtlOutcome::Success { .. }
    ));
    let mut old = proto::kv9_client::Kv9Client::new(channel);
    for operation in [
        Operation::Capabilities,
        Operation::DeleteRange {
            start: binding.start.clone(),
            end: binding.end.clone(),
            limit: 1,
        },
    ] {
        let error = old
            .raw_ttl(request(proto::RawTtlRequest {
                binding: binding.encode(),
                protocol_version: 1,
                budget_micros: 1_000_000,
                operation: Some(operation.into()),
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    server.abort();
    let _ = server.await;
}
