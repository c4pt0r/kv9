use super::*;
use crate::grpc::AuthContext;
use crate::point_test_support::{authenticator, get, Backend};
use std::sync::Mutex;
use tonic::{metadata::MetadataValue, Code};

type Observation = (usize, Vec<u8>, bool);

struct ObservedAuth {
    inner: Arc<dyn Authenticator>,
    seen: Mutex<Vec<Observation>>,
}

impl Authenticator for ObservedAuth {
    fn authenticate(&self, metadata: &MetadataMap) -> Result<AuthContext, Status> {
        let authorization = metadata.get("authorization").unwrap();
        self.seen.lock().unwrap().push((
            metadata.len(),
            authorization.as_encoded_bytes().to_vec(),
            authorization.is_sensitive(),
        ));
        self.inner.authenticate(metadata)
    }
}

fn handler() -> (Handler, Arc<ObservedAuth>) {
    let auth = Arc::new(ObservedAuth {
        inner: authenticator(),
        seen: Mutex::new(Vec::new()),
    });
    (
        Handler::new(Kv9Grpc::new(Arc::new(Backend::default())), auth.clone()),
        auth,
    )
}

#[test]
fn cached_frame_auth_sees_original_singleton_contents_and_flags() {
    let (handler, auth) = handler();
    let mut opening = Handler::authorization_map("Bearer secret").unwrap();
    opening
        .get_mut("authorization")
        .unwrap()
        .set_sensitive(true);
    opening.append("authorization", "Bearer second".parse().unwrap());
    opening.insert("opening-only", "extra".parse().unwrap());
    opening.insert_bin("opening-bin", MetadataValue::from_bytes(b"binary"));
    let cached = handler.for_stream(&opening);
    assert!(auth.seen.lock().unwrap().is_empty());
    let payload = get(b"key").encode_to_vec();
    for candidate in [&handler, &cached, &cached] {
        let request = candidate
            .request::<crate::proto::RawGetRequest>("Bearer secret", &payload)
            .unwrap();
        assert_eq!(request.get_ref().key, b"key");
        assert_eq!(
            request
                .extensions()
                .get::<AuthContext>()
                .unwrap()
                .principal
                .as_ref(),
            "alice"
        );
    }
    assert_eq!(
        *auth.seen.lock().unwrap(),
        vec![(1, b"Bearer secret".to_vec(), false); 3]
    );
    assert!(opening.get("authorization").unwrap().is_sensitive());
    assert_eq!(opening.get_all("authorization").iter().count(), 2);
}

#[test]
fn cached_and_fallback_frames_preserve_error_order_and_authentication_counts() {
    let (handler, auth) = handler();
    let cached = handler.for_stream(&Handler::authorization_map("Bearer secret").unwrap());
    let payload = get(b"key").encode_to_vec();
    let oversized = "x".repeat(4104);
    let cases = [
        ("Bearer secret", payload.as_slice(), Code::Ok, true),
        ("Bearer secret", &[0xff][..], Code::InvalidArgument, true),
        ("Bearer wrong", &[0xff][..], Code::Unauthenticated, true),
        (
            "bearer secret",
            payload.as_slice(),
            Code::Unauthenticated,
            true,
        ),
        ("", payload.as_slice(), Code::Unauthenticated, true),
        ("Bearer secret\n", &[0xff][..], Code::Unauthenticated, false),
        (
            oversized.as_str(),
            &[0xff][..],
            Code::ResourceExhausted,
            false,
        ),
    ];
    for candidate in [&handler, &cached] {
        for (authorization, bytes, expected, authenticated) in cases {
            let before = auth.seen.lock().unwrap().len();
            let result = candidate.request::<crate::proto::RawGetRequest>(authorization, bytes);
            assert_eq!(
                result.map_or_else(|error| error.code(), |_| Code::Ok),
                expected
            );
            assert_eq!(
                auth.seen.lock().unwrap().len() - before,
                usize::from(authenticated)
            );
        }
    }
    // A mismatched credential must neither replace nor grow the singleton.
    assert_eq!(cached.authorization_metadata.as_ref().unwrap().len(), 1);
    assert_eq!(
        cached
            .authorization_metadata
            .as_ref()
            .unwrap()
            .get("authorization")
            .unwrap(),
        "Bearer secret"
    );
    let oversized_payload = vec![0; crate::client::MAX_MESSAGE_BYTES + 1];
    let before = auth.seen.lock().unwrap().len();
    assert_eq!(
        cached
            .request::<crate::proto::RawGetRequest>("Bearer secret", &oversized_payload)
            .unwrap_err()
            .code(),
        Code::ResourceExhausted
    );
    assert_eq!(auth.seen.lock().unwrap().len(), before);
}

#[test]
fn ineligible_opening_disables_reuse_without_changing_frame_authentication() {
    let (handler, auth) = handler();
    let mut non_ascii = MetadataMap::new();
    non_ascii.insert(
        "authorization",
        MetadataValue::try_from(&[0xff][..]).unwrap(),
    );
    for opening in [
        MetadataMap::new(),
        non_ascii,
        Handler::authorization_map(&"x".repeat(4104)).unwrap(),
    ] {
        let before = auth.seen.lock().unwrap().len();
        let scoped = handler.for_stream(&opening);
        assert!(scoped.authorization_metadata.is_none());
        assert_eq!(auth.seen.lock().unwrap().len(), before);
        let request = scoped
            .request::<crate::proto::RawGetRequest>("Bearer secret", &get(b"key").encode_to_vec())
            .unwrap();
        assert_eq!(
            request.metadata().get("authorization").unwrap(),
            "Bearer secret"
        );
        assert_eq!(auth.seen.lock().unwrap().len(), before + 1);
    }
}
