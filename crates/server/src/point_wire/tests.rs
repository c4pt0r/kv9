use super::*;
use crate::admission::{PublicApiLimits, WorkClass};
use crate::grpc::{proto, proto::kv9_server::Kv9, AuthKind, ADMISSION_REFUSED_KEY};
use crate::point_test_support::{authenticator, context_message, get, put, Backend};
use kv9_common::NodeId;
use tonic::{Code, Request};

struct FixedIdentity(AuthContext);

impl Authenticator for FixedIdentity {
    fn authenticate(&self, _: &MetadataMap) -> Result<AuthContext, Status> {
        Ok(self.0.clone())
    }
}

fn identity(kind: AuthKind, node_id: Option<NodeId>) -> AuthContext {
    AuthContext {
        principal: "alice".into(),
        auth_kind: kind,
        node_id,
    }
}

async fn send(handler: &Handler, operation: u8, payload: Vec<u8>) -> WireReply {
    handler
        .clone()
        .dispatch(
            operation,
            WireRequest {
                authorization: "Bearer secret".into(),
                payload,
            },
        )
        .await
}

fn batch_get(keys: Vec<Vec<u8>>) -> proto::RawBatchGetRequest {
    proto::RawBatchGetRequest {
        context: Some(context_message()),
        keys,
    }
}

#[tokio::test]
async fn typed_dispatch_rejects_nonclient_identities_before_admission_for_all_operations() {
    let backend = Arc::new(Backend::default());
    let api = Kv9Grpc::with_limits(
        backend.clone(),
        PublicApiLimits {
            max_requests: 1,
            max_encoded_bytes: 4096,
        },
    )
    .unwrap();
    let held = api.admission().reserve(WorkClass::RawRead, 1).unwrap();
    let payloads = [
        get(b"key").encode_to_vec(),
        put(b"key", b"value").encode_to_vec(),
        proto::RawDeleteRequest {
            context: Some(context_message()),
            key: b"key".to_vec(),
        }
        .encode_to_vec(),
        batch_get(vec![b"key".to_vec()]).encode_to_vec(),
        proto::RawBatchPutRequest {
            context: Some(context_message()),
            pairs: vec![proto::KeyValue {
                key: b"key".to_vec(),
                value: b"value".to_vec(),
            }],
        }
        .encode_to_vec(),
    ];
    for auth in [
        identity(AuthKind::Node, None),
        identity(AuthKind::Client, Some(NodeId(1))),
    ] {
        let handler = Handler {
            api: api.clone(),
            authenticator: Arc::new(FixedIdentity(auth)),
        };
        for (operation, payload) in payloads.iter().enumerate() {
            let reply = send(&handler, operation as u8, payload.clone()).await;
            assert_eq!(reply.code, Code::PermissionDenied as i32);
            assert_eq!(reply.message, "client identity required");
            assert!(reply.metadata.is_empty());
        }
    }
    assert!(backend.calls.lock().unwrap().is_empty());
    drop(held);
    assert!(api.admission().reserve(WorkClass::RawRead, 4096).is_ok());
}

#[tokio::test]
async fn typed_dispatch_keeps_stream_batch_bounds_before_role_checks_and_out_of_unary() {
    let backend = Arc::new(Backend::default());
    let api = Kv9Grpc::new(backend.clone());
    let handler = Handler {
        api: api.clone(),
        authenticator: Arc::new(FixedIdentity(identity(AuthKind::Node, Some(NodeId(1))))),
    };
    for (operation, payload, message) in [
        (
            3,
            batch_get(vec![]).encode_to_vec(),
            "invalid batch read bounds",
        ),
        (
            4,
            proto::RawBatchPutRequest {
                context: Some(context_message()),
                pairs: vec![],
            }
            .encode_to_vec(),
            "invalid batch write bounds",
        ),
    ] {
        let reply = send(&handler, operation, payload).await;
        assert_eq!(reply.code, Code::InvalidArgument as i32);
        assert_eq!(reply.message, message);
    }
    let malformed = send(&handler, 3, vec![0xff]).await;
    assert_eq!(malformed.code, Code::InvalidArgument as i32);
    assert_eq!(malformed.message, "invalid point request");
    assert!(backend.calls.lock().unwrap().is_empty());

    // The unary adapter does not impose the stream's batch bounds. Preserve
    // this existing distinction; common typed dispatch must not introduce it.
    let mut unary = Request::new(batch_get(vec![]));
    unary
        .extensions_mut()
        .insert(identity(AuthKind::Client, None));
    assert!(api
        .raw_batch_get(unary)
        .await
        .unwrap()
        .into_inner()
        .values
        .is_empty());
    assert_eq!(backend.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn typed_dispatch_charges_decoded_size_and_keeps_admission_before_context_validation() {
    let backend = Arc::new(Backend::default());
    let message = get(b"key");
    let api = Kv9Grpc::with_limits(
        backend.clone(),
        PublicApiLimits {
            max_requests: 1,
            max_encoded_bytes: message.encoded_len(),
        },
    )
    .unwrap();
    let handler = Handler {
        api: api.clone(),
        authenticator: authenticator(),
    };
    let mut payload = message.encode_to_vec();
    // Unknown protobuf field 100, varint 1, increases wire length without
    // changing the decoded message or its admission charge.
    payload.extend_from_slice(&[0xa0, 0x06, 0x01]);
    assert_eq!(send(&handler, 0, payload).await.code, Code::Ok as i32);
    assert_eq!(backend.calls.lock().unwrap().len(), 1);

    let held = api.admission().reserve(WorkClass::RawRead, 0).unwrap();
    let missing_context = proto::RawGetRequest {
        context: None,
        key: b"key".to_vec(),
    }
    .encode_to_vec();
    let refused = send(&handler, 0, missing_context.clone()).await;
    let status = refused.decode::<proto::RawGetResponse>().unwrap_err();
    assert_eq!(status.code(), Code::ResourceExhausted);
    assert!(status.metadata().contains_key(ADMISSION_REFUSED_KEY));
    drop(held);
    let invalid = send(&handler, 0, missing_context).await;
    assert_eq!(invalid.code, Code::InvalidArgument as i32);
    assert_eq!(invalid.message, "request context is required");
    assert_eq!(backend.calls.lock().unwrap().len(), 1);
    assert!(api
        .admission()
        .reserve(WorkClass::RawRead, message.encoded_len())
        .is_ok());
}
