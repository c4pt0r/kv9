//! Bounded, persistent-channel read pressure for the Chaos Mesh acceptance gate.
//! This fixture is not linked into the production binary. It never writes keys.
use kv9_server::grpc::{admission_refusal, proto};
use prost::Message;
use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tonic::Request;

fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

async fn read(
    mut client: proto::kv9_client::Kv9Client<tonic::transport::Channel>,
    token: Arc<str>,
    keyspace: u32,
    key_bytes: usize,
    sequence: usize,
    barrier: Option<Arc<tokio::sync::Barrier>>,
) -> &'static str {
    let mut request = Request::new(proto::RawGetRequest {
        context: Some(proto::RequestContext {
            keyspace_id: keyspace,
            region_epoch: Some(proto::RegionEpoch {
                conf_ver: 1,
                version: 1,
            }),
        }),
        key: vec![0xad; key_bytes],
    });
    let encoded = request.get_ref().encoded_len();
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    if let Some(barrier) = barrier {
        barrier.wait().await;
    }
    let begin = now();
    let (outcome, code) =
        match tokio::time::timeout(Duration::from_secs(5), client.raw_get(request)).await {
            Ok(Ok(response)) => {
                let value = response
                    .into_inner()
                    .value
                    .expect("read response needs OptionalValue");
                assert!(
                    !value.found && value.value.is_empty(),
                    "fixture key must remain absent"
                );
                ("missing", 0)
            }
            Ok(Err(status)) => (
                admission_refusal(&status).unwrap_or("unexpected_status"),
                status.code() as i32,
            ),
            Err(_) => ("timeout", -1),
        };
    println!("{{\"type\":\"read\",\"sequence\":{sequence},\"start_ns\":{begin},\"end_ns\":{},\"key_bytes\":{key_bytes},\"key_byte\":173,\"encoded_bytes\":{encoded},\"outcome\":\"{outcome}\",\"code\":{code}}}", now());
    assert!(
        !matches!(outcome, "unexpected_status" | "timeout"),
        "pressure observed an unclassified outcome"
    );
    outcome
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(
        args.len(),
        5,
        "usage: admission-pressure ADDRESS KEYSPACE READY_FILE STOP_FILE"
    );
    let address: std::net::SocketAddr = args[1].parse().unwrap();
    let keyspace: u32 = args[2].parse().unwrap();
    let token: Arc<str> = std::env::var("KV9_CLIENT_TOKEN")
        .expect("client token required")
        .into();
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
        .unwrap()
        .connect_timeout(Duration::from_secs(5))
        .connect()
        .await
        .unwrap();
    let client = proto::kv9_client::Kv9Client::new(channel);
    println!("{{\"type\":\"start\",\"time_ns\":{},\"address\":\"{address}\",\"keyspace\":{keyspace},\"concurrency\":64}}", now());
    assert_eq!(
        read(
            client.clone(),
            token.clone(),
            keyspace,
            2 * 1024 * 1024 + 1,
            0,
            None
        )
        .await,
        "request_too_large",
        "oversized request must be refused before execution"
    );
    let deadline = Instant::now() + Duration::from_secs(45);
    let (mut sequence, mut refused, mut completed) = (1, 0, 0);
    let mut ready = false;
    loop {
        assert!(
            Instant::now() < deadline && sequence < 100_000,
            "harness did not stop bounded pressure"
        );
        let barrier = Arc::new(tokio::sync::Barrier::new(65));
        let mut jobs = tokio::task::JoinSet::new();
        for _ in 0..64 {
            jobs.spawn(read(
                client.clone(),
                token.clone(),
                keyspace,
                32,
                sequence,
                Some(barrier.clone()),
            ));
            sequence += 1;
        }
        barrier.wait().await;
        while let Some(result) = jobs.join_next().await {
            match result.unwrap() {
                "request_count" => refused += 1,
                "missing" => completed += 1,
                other => panic!("unexpected small-request outcome: {other}"),
            }
        }
        if !ready && refused > 0 && completed > 0 {
            std::fs::write(&args[3], format!("{}\n", now())).unwrap();
            ready = true;
        }
        if std::path::Path::new(&args[4]).exists() {
            assert!(ready, "pressure never reached its positive precondition");
            break;
        }
        // Bounded waves leave opportunities for the independent writer/readers.
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    println!("{{\"type\":\"complete\",\"time_ns\":{},\"requests\":{sequence},\"count_refusals\":{refused},\"missing\":{completed}}}", now());
}
