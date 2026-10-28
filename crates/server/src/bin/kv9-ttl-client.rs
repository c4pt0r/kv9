//! Bounded NDJSON caller for explicit-scope or root-pinned routed TTL.
use kv9_server::{
    client::{
        routed::{
            ttl::{DeleteCursor, RoutedTtlClient, ScanCursor},
            RoutedConfig,
        },
        ttl::ScopedTtlClient,
        ClientConfig, TransportKind, MAX_MESSAGE_BYTES,
    },
    raw_ttl::Operation,
};
use serde::Deserialize;
use std::io::{self, BufRead, Read, Write};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopedConfig {
    client: ClientConfig,
    /// Canonical KV9RNG01 bytes from a root-pinned LookupRawRoute observation.
    binding: Vec<u8>,
    #[serde(default)]
    transport: TransportKind,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Routed {
    routing: RoutedConfig,
    #[serde(default)]
    transport: TransportKind,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Config {
    Scoped(ScopedConfig),
    Routed(Routed),
}
enum Client {
    Scoped(ScopedTtlClient),
    Routed(RoutedTtlClient),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    id: u64,
    operation: Option<Operation>,
    cursor: Option<ScanCursor>,
    delete_cursor: Option<DeleteCursor>,
    limit: Option<usize>,
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(args.len() == 2 || (args.len() == 3 && args[2] == "--activate")) || args[0] != "--config" {
        return Err("usage: kv9-ttl-client --config FILE [--activate]; token in KV9_CLIENT_TOKEN; stdin: {\"id\":1,\"operation\":{...}} per line".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&args[1])?
        .take((MAX_MESSAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err("TTL client configuration exceeds 1 MiB".into());
    }
    let config: Config = serde_json::from_slice(&bytes)?;
    if args.len() == 3 && matches!(config, Config::Routed(_)) {
        return Err("activation requires an explicit binding configuration".into());
    }
    let token = std::env::var("KV9_CLIENT_TOKEN").map_err(|_| "KV9_CLIENT_TOKEN is required")?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let client = runtime.block_on(async {
        Ok::<_, Box<dyn std::error::Error>>(match config {
            Config::Scoped(c) => Client::Scoped(ScopedTtlClient::new(
                c.client,
                &token,
                kv9_common::data_range::DataRange::decode(&c.binding)?,
                c.transport,
            )?),
            Config::Routed(c) => Client::Routed(RoutedTtlClient::new_with_transport(
                c.routing,
                &token,
                c.transport,
            )?),
        })
    })?;
    if args.len() == 3 {
        let Client::Scoped(client) = client else {
            unreachable!("activation configuration checked")
        };
        let report = runtime.block_on(client.activate());
        let mut output = io::stdout().lock();
        serde_json::to_writer(&mut output, &serde_json::json!({"activation":report}))?;
        writeln!(output)?;
        output.flush()?;
        return Ok(());
    }
    let mut stdin = io::stdin().lock();
    let mut last = 0;
    loop {
        let mut line = Vec::new();
        let count = (&mut stdin)
            .take((MAX_MESSAGE_BYTES * 6 + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        if count > MAX_MESSAGE_BYTES * 6 || !line.ends_with(b"\n") {
            return Err("TTL input exceeds byte bound or lacks newline".into());
        }
        let input: Input = serde_json::from_slice(&line)?;
        if input.id <= last {
            return Err("operation IDs must be positive and strictly increasing".into());
        }
        last = input.id;
        let report = runtime.block_on(async {
            Ok::<_, Box<dyn std::error::Error>>(
                match (
                    &client,
                    input.operation,
                    input.cursor,
                    input.delete_cursor,
                    input.limit,
                ) {
                    (Client::Scoped(c), Some(op), None, None, None) => {
                        serde_json::to_value(c.call(op).await)?
                    }
                    (Client::Routed(c), Some(op), None, None, None) => {
                        serde_json::to_value(c.call(op).await)?
                    }
                    (Client::Routed(c), None, Some(cursor), None, Some(limit)) => {
                        serde_json::to_value(c.continue_scan(&cursor, limit).await)?
                    }
                    (Client::Routed(c), None, None, Some(cursor), Some(limit)) => {
                        serde_json::to_value(c.continue_delete_range(&cursor, limit).await)?
                    }
                    _ => {
                        return Err(
                            "use operation OR one routed cursor/delete_cursor plus limit".into(),
                        )
                    }
                },
            )
        })?;
        let mut output = io::stdout().lock();
        serde_json::to_writer(
            &mut output,
            &serde_json::json!({"id":input.id,"report":report}),
        )?;
        writeln!(output)?;
        output.flush()?;
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("TTL client failed: {error}");
        std::process::exit(1);
    }
}
