//! Persistent serial public client for routing/fault histories. This runner
//! records every unknown outcome; it never resubmits a failed logical operation.
use kv9_server::client::{
    routed::{RoutedConfig, RoutedRawClient},
    RawOperation,
};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Input {
    id: u64,
    operation: RawOperation,
}
fn now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time before Unix epoch")
        .as_nanos()
}
fn emit(value: serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, &value)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 2 || args[0] != "--config" {
        return Err("usage: kv9-routed-workload --config FILE; token in KV9_CLIENT_TOKEN".into());
    }
    let config: RoutedConfig = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    config.validate()?;
    let token = std::env::var("KV9_CLIENT_TOKEN").map_err(|_| "KV9_CLIENT_TOKEN is required")?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let client = runtime.block_on(async { RoutedRawClient::new(config.clone(), &token) })?;
    emit(
        serde_json::json!({"kind":"start","pid":std::process::id(),"config":config,"started_unix_ns":now()}),
    )?;
    let mut last_id = 0;
    let mut reader = io::stdin().lock();
    loop {
        // Bound an input line before JSON allocation, including malformed input.
        let mut line = Vec::new();
        let count = (&mut reader)
            .take((kv9_server::client::MAX_MESSAGE_BYTES * 6 + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        if count > kv9_server::client::MAX_MESSAGE_BYTES * 6 || !line.ends_with(b"\n") {
            return Err("workload input line exceeds bound or lacks newline".into());
        }
        let input: Input = serde_json::from_slice(&line)?;
        if input.id <= last_id {
            return Err("operation IDs must be positive and strictly increasing".into());
        }
        last_id = input.id;
        let invoked = now();
        let report = runtime.block_on(client.call(input.operation.clone()));
        emit(
            serde_json::json!({"kind":"operation","id":input.id,"operation":input.operation,
            "invoked_unix_ns":invoked,"finished_unix_ns":now(),"report":report}),
        )?;
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("routed workload failed: {error}");
        std::process::exit(1);
    }
}
