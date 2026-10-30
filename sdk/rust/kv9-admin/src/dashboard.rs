//! `kv9-admin dashboard`: a read-only web view of the cluster, served by the
//! tool itself. A poller re-resolves the metadata leader and gathers the same
//! reports the CLI verbs print (`cluster info`, `cluster health`, `node list`,
//! `cluster status`, `group list`, `keyspace list`, `backup list`) into one JSON
//! snapshot; a tiny HTTP/1.1 server answers `GET /` with the page and
//! `GET /api/state` with the snapshot. The token never leaves the process; the
//! listener binds 127.0.0.1 unless told otherwise.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

use crate::{backup_list, cluster_health, cluster_info, cluster_status, group_list, keyspace_list, node_list, Cluster, Fail, Global};

const PAGE: &str = include_str!("dashboard.html");
const MAX_REQUEST_BYTES: usize = 8192;

/// The latest snapshot and how it was obtained.
struct Shared {
    state: RwLock<Value>,
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// One poll: every report the page needs, each failure recorded instead of fatal.
async fn snapshot(global: &Global) -> Value {
    let mut errors: Vec<String> = Vec::new();
    let mut cluster = match Cluster::connect(global).await {
        Ok(cluster) => cluster,
        Err(fail) => {
            return json!({
                "updated_unix": now_unix(),
                "connected": false,
                "errors": [format!("connect: {}", fail.message())],
            });
        }
    };
    let mut out = serde_json::Map::new();
    out.insert("updated_unix".into(), json!(now_unix()));
    out.insert("connected".into(), json!(true));
    out.insert("resolved_leader".into(), cluster.resolved());
    macro_rules! section {
        ($name:literal, $call:expr) => {
            match $call.await {
                Ok(report) => {
                    if !report.partial.is_empty() {
                        errors.extend(report.partial.iter().map(|p| format!("{}: {p}", $name)));
                    }
                    out.insert($name.into(), report.result);
                }
                Err(fail) => {
                    errors.push(format!("{}: {}", $name, fail.message()));
                    out.insert($name.into(), Value::Null);
                }
            }
        };
    }
    section!("info", cluster_info(&mut cluster));
    section!("health", cluster_health(&mut cluster));
    section!("nodes", node_list(&mut cluster));
    section!("status", cluster_status(&mut cluster, None));
    section!("groups", group_list(&mut cluster));
    section!("keyspaces", keyspace_list(&mut cluster));
    section!("backups", backup_list(&mut cluster, false));
    out.insert("errors".into(), json!(errors));
    Value::Object(out)
}

async fn respond(mut stream: TcpStream, shared: Arc<Shared>, refresh_secs: u64) {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        match stream.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                buffer.extend_from_slice(&chunk[..n]);
                if buffer.windows(4).any(|w| w == b"\r\n\r\n") || buffer.len() > MAX_REQUEST_BYTES {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let head = String::from_utf8_lossy(&buffer);
    let mut words = head.lines().next().unwrap_or("").split_whitespace();
    let (method, path) = (words.next().unwrap_or(""), words.next().unwrap_or("/"));
    let path = path.split('?').next().unwrap_or("/");
    let (status, content_type, body): (&str, &str, Vec<u8>) = match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => (
            "200 OK",
            "text/html; charset=utf-8",
            PAGE.replace("__REFRESH_SECS__", &refresh_secs.to_string()).into_bytes(),
        ),
        ("GET", "/api/state") => (
            "200 OK",
            "application/json",
            serde_json::to_vec(&*shared.state.read().await).unwrap_or_else(|_| b"{}".to_vec()),
        ),
        ("GET", "/healthz") => ("200 OK", "text/plain", b"ok".to_vec()),
        _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
    };
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.shutdown().await;
}

/// Serve until the process is stopped. Returns only when the listener cannot be bound.
pub(crate) async fn serve(global: &Global, listen: &str, refresh: Duration) -> Result<(), Fail> {
    let listener = TcpListener::bind(listen)
        .await
        .map_err(|e| Fail::Usage(format!("cannot listen on {listen}: {e}")))?;
    let shared = Arc::new(Shared {
        state: RwLock::new(json!({"updated_unix": 0, "connected": false, "errors": ["first poll pending"]})),
    });
    eprintln!(
        "kv9-admin dashboard: http://{listen}/  (refresh every {}s; JSON at /api/state; Ctrl-C to stop)",
        refresh.as_secs()
    );
    let poller = {
        let shared = shared.clone();
        let global = global.clone();
        tokio::spawn(async move {
            loop {
                let next = snapshot(&global).await;
                *shared.state.write().await = next;
                tokio::time::sleep(refresh).await;
            }
        })
    };
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let shared = shared.clone();
                let secs = refresh.as_secs().max(1);
                tokio::spawn(async move { respond(stream, shared, secs).await });
            }
            Err(error) => {
                if global.verbose {
                    eprintln!("dashboard: accept failed: {error}");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        if poller.is_finished() {
            return Err(Fail::Transport("dashboard poller stopped".into()));
        }
    }
}
