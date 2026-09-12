use super::super::{
    metrics::{Metrics, Sample},
    model::tests::config_json,
};
use super::*;
use serde_json::json;

fn config(replicas: u8, batch: bool) -> Config {
    let mut value = config_json();
    value["version"] = json!(4);
    value["read_api"] = json!(if batch { "mget" } else { "get" });
    value["write_api"] = json!(if batch { "mset" } else { "set" });
    value["write_confirmation"] = json!({"kind":"wait","replicas":replicas,"timeout_ms":50});
    value["read_percent"] = json!(0);
    value["batch_size"] = json!(if batch { 3 } else { 1 });
    serde_json::from_value(value).unwrap()
}

fn run(f: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(5), f)
                .await
                .expect("confirmation test deadline");
        });
}

#[test]
fn confirmation_requires_explicit_version_bounded_count_and_timeout() {
    let good = config(1, false);
    let bytes = good.confirmation_request().unwrap();
    assert_eq!(bytes, b"*3\r\n$4\r\nWAIT\r\n$1\r\n1\r\n$2\r\n50\r\n");
    assert_eq!(
        good.validate().unwrap().confirmation_request_bytes,
        Some(bytes.len())
    );
    for field in [
        json!(null),
        json!({"kind":"wait","replicas":0,"timeout_ms":50}),
        json!({"kind":"wait","replicas":3,"timeout_ms":50}),
        json!({"kind":"wait","replicas":1,"timeout_ms":0}),
        json!({"kind":"wait","replicas":1,"timeout_ms":1001}),
        json!({"kind":"async","replicas":1}),
    ] {
        let mut value = serde_json::to_value(&good).unwrap();
        value["write_confirmation"] = field.clone();
        assert!(
            !serde_json::from_value::<Config>(value).is_ok_and(|c| c.validate().is_ok()),
            "invalid confirmation accepted: {field}"
        );
    }
    for version in [1, 2, 3] {
        let mut bad = good.clone();
        bad.version = version;
        assert!(bad.validate().is_err());
    }
    let mut missing = good.clone();
    missing.write_confirmation = None;
    assert!(missing.validate().is_err());
    let mut asynchronous = good;
    asynchronous.write_confirmation = Some(WriteConfirmation::Async {});
    assert!(asynchronous.validate().is_ok());
    assert_eq!(asynchronous.confirmation_request(), None);
}

#[test]
fn set_and_mset_wait_on_the_write_connection_and_consume_both_replies() {
    run(async {
        for batch in [false, true] {
            for replicas in [1, 2] {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let mut c = config(replicas, batch);
                c.address = listener.local_addr().unwrap();
                c.validate().unwrap();
                let operations: Vec<_> = (1..=2)
                    .map(|n| Operation::for_traffic(&c, false, 0..c.batch_size, n))
                    .collect();
                let expected: Vec<_> = operations
                    .iter()
                    .map(|op| {
                        let mut bytes = encode(op).unwrap();
                        bytes.extend(c.confirmation_request().unwrap());
                        bytes
                    })
                    .collect();
                let peer = tokio::spawn(async move {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    for bytes in expected {
                        let mut got = vec![0; bytes.len()];
                        stream.read_exact(&mut got).await.unwrap();
                        assert_eq!(
                            got, bytes,
                            "write and confirmation did not share one connection"
                        );
                        stream.write_all(b"+OK\r\n:2\r\n").await.unwrap();
                    }
                });
                let mut connection = None;
                for (index, op) in operations.iter().enumerate() {
                    let result = call(&mut connection, &c, op).await;
                    assert!(matches!(result.result, Ok(Reply::Applied)));
                    assert_eq!(result.connection_attempts, u64::from(index == 0));
                    assert_eq!(
                        (
                            result.command_attempts,
                            result.confirmation_attempts,
                            result.confirmed_replicas
                        ),
                        (1, 1, Some(2))
                    );
                    assert!(connection.as_ref().unwrap().buffer().is_empty());
                }
                peer.await.unwrap();
            }
        }
    });
}

#[test]
fn short_confirmation_is_an_unknown_write_with_both_command_attempts() {
    run(async {
        for acknowledged in [0, 1] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut c = config(2, false);
            c.address = listener.local_addr().unwrap();
            let op = Operation::for_traffic(&c, false, 0..1, 1);
            let expected_len = encode(&op).unwrap().len() + c.confirmation_request().unwrap().len();
            let peer = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                stream.read_exact(&mut vec![0; expected_len]).await.unwrap();
                stream
                    .write_all(format!("+OK\r\n:{acknowledged}\r\n").as_bytes())
                    .await
                    .unwrap();
                let mut extra = [0u8; 1];
                assert_eq!(
                    stream.read(&mut extra).await.unwrap(),
                    0,
                    "uncertain write was replayed"
                );
            });
            let mut connection = None;
            let result = call(&mut connection, &c, &op).await;
            assert!(matches!(result.result, Err(Failure::ReplicationShortfall)));
            assert_eq!(result.confirmed_replicas, Some(acknowledged));
            assert!(connection.is_none());
            let mut metrics = Metrics::default();
            metrics.record(Sample {
                call: &result,
                read: false,
                items: 1,
                whole_call_ns: result.elapsed_ns,
                before_cutoff: true,
                scheduled_ns: None,
                lateness_ns: None,
                valid: true,
            });
            assert_eq!(metrics.operations[1].populations[0].calls, 0);
            assert_eq!(metrics.operations[1].populations[1].calls, 1);
            let report = metrics.report_with_confirmation(ReadApi::Get, WriteApi::Set, true);
            assert_eq!(
                report["statistics"][1]["total_resp_command_attempts"],
                json!(2)
            );
            assert_eq!(
                report["statistics"][1]["replica_confirmation_replies"][usize::from(acknowledged)],
                json!(1)
            );
            peer.await.unwrap();
        }
    });
}

#[test]
fn missing_or_invalid_wait_reply_never_turns_write_ok_into_success() {
    run(async {
        for (reply, failure) in [
            (b"+OK\r\n".as_slice(), Failure::Io),
            (b"+OK\r\n:3\r\n", Failure::Protocol),
            (b"+OK\r\n:-1\r\n", Failure::Protocol),
            (b"+OK\r\n+OK\r\n", Failure::Protocol),
            (b"+OK\r\n-ERR unavailable\r\n", Failure::ServerError),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut c = config(1, false);
            c.address = listener.local_addr().unwrap();
            let op = Operation::for_traffic(&c, false, 0..1, 1);
            let len = encode(&op).unwrap().len() + c.confirmation_request().unwrap().len();
            let peer = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                stream.read_exact(&mut vec![0; len]).await.unwrap();
                stream.write_all(reply).await.unwrap();
            });
            let mut connection = None;
            let result = call(&mut connection, &c, &op).await;
            assert_eq!(result.result.err(), Some(failure));
            assert_eq!(result.confirmation_attempts, 1);
            assert!(connection.is_none());
            peer.await.unwrap();
        }
    });
}

#[test]
fn confirmation_does_not_restart_the_original_call_deadline() {
    run(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut c = config(1, false);
        c.address = listener.local_addr().unwrap();
        c.deadline_ms = 200;
        let op = Operation::for_traffic(&c, false, 0..1, 1);
        let len = encode(&op).unwrap().len() + c.confirmation_request().unwrap().len();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            stream.read_exact(&mut vec![0; len]).await.unwrap();
            tokio::time::sleep(Duration::from_millis(130)).await;
            stream.write_all(b"+OK\r\n").await.unwrap();
            tokio::time::sleep(Duration::from_millis(130)).await;
            // This ACK is after the original deadline but before a new budget
            // incorrectly started after +OK. A late delivery may hit a closed
            // socket, which is the expected original-deadline outcome.
            let _ = stream.write_all(b":1\r\n").await;
        });
        let mut connection = None;
        let result = call(&mut connection, &c, &op).await;
        assert_eq!(result.result.err(), Some(Failure::Deadline));
        assert_eq!(
            (result.command_attempts, result.confirmation_attempts),
            (1, 1)
        );
        assert!(connection.is_none());
        peer.await.unwrap();
    });
}

#[test]
fn reads_and_explicit_async_writes_do_not_issue_wait() {
    run(async {
        for read in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let mut c = config(1, false);
            c.address = listener.local_addr().unwrap();
            if !read {
                c.write_confirmation = Some(WriteConfirmation::Async {});
            }
            let op = Operation::for_traffic(&c, read, 0..1, 1);
            let expected = encode(&op).unwrap();
            let value = c.value(0, 1);
            let peer = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = vec![0; expected.len()];
                stream.read_exact(&mut bytes).await.unwrap();
                assert_eq!(bytes, expected);
                if read {
                    stream
                        .write_all(format!("${}\r\n", value.len()).as_bytes())
                        .await
                        .unwrap();
                    stream.write_all(&value).await.unwrap();
                    stream.write_all(b"\r\n").await.unwrap();
                } else {
                    stream.write_all(b"+OK\r\n").await.unwrap();
                }
                let mut extra = [0u8; 1];
                assert_eq!(
                    stream.read(&mut extra).await.unwrap(),
                    0,
                    "read/async call sent WAIT"
                );
            });
            let mut connection = None;
            let result = call(&mut connection, &c, &op).await;
            assert!(result.result.is_ok());
            assert_eq!(
                (
                    result.command_attempts,
                    result.confirmation_attempts,
                    result.confirmed_replicas
                ),
                (1, 0, None)
            );
            drop(connection);
            peer.await.unwrap();
        }
    });
}

#[test]
fn replication_metrics_are_explicit_and_legacy_report_shape_is_preserved() {
    let metrics = Metrics::default();
    let legacy = metrics.report_for_apis(ReadApi::Get, WriteApi::Set);
    assert_eq!(legacy["reasons"].as_array().unwrap().len(), 6);
    for op in legacy["statistics"].as_array().unwrap() {
        assert_eq!(op["reasons"].as_array().unwrap().len(), 6);
        let fields = op.as_object().unwrap();
        assert_eq!(fields.len(), 7);
        for field in [
            "confirmation_attempts",
            "replica_confirmation_replies",
            "total_resp_command_attempts",
        ] {
            assert!(!fields.contains_key(field));
        }
    }
    let replicated = metrics.report_with_confirmation(ReadApi::Get, WriteApi::Set, true);
    assert_eq!(replicated["reasons"][6], json!("replication_shortfall"));
    for op in replicated["statistics"].as_array().unwrap() {
        assert_eq!(op["reasons"].as_array().unwrap().len(), 7);
        assert_eq!(op["total_resp_command_attempts"], json!(0));
        assert_eq!(op["replica_confirmation_replies"], json!([0, 0, 0]));
    }
}
