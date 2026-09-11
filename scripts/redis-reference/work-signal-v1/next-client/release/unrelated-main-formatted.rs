//! Bounded RESP2 reference client. One request per connection, no retries.
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    address: String,
    run_id: String,
    seed: u64,
    workers: usize,
    keys: usize,
    value_bytes: usize,
    get: u64,
    put: u64,
    delete: u64,
    warmup_operations: u64,
    measure_ms: u64,
    max_operations: u64,
}
fn mix(mut v: u64) -> u64 {
    v = v.wrapping_add(0x9e3779b97f4a7c15);
    v = (v ^ (v >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    v = (v ^ (v >> 27)).wrapping_mul(0x94d049bb133111eb);
    v ^ (v >> 31)
}
fn key(c: &Config, i: usize) -> Vec<u8> {
    format!("{}:{i:016x}", c.run_id).into_bytes()
}
fn value(c: &Config, nonce: u64) -> Vec<u8> {
    let mut v = vec![0; c.value_bytes];
    v[..8].copy_from_slice(&nonce.to_be_bytes());
    for (i, chunk) in v[8..].chunks_mut(8).enumerate() {
        chunk.copy_from_slice(&mix(c.seed ^ nonce ^ i as u64).to_be_bytes()[..chunk.len()]);
    }
    v
}
fn operation(c: &Config, nonce: u64) -> (usize, Vec<u8>) {
    let r = mix(c.seed ^ nonce);
    (
        if r % 100 < c.get {
            0
        } else if r % 100 < c.get + c.put {
            1
        } else {
            2
        },
        key(c, (mix(r) % c.keys as u64) as usize),
    )
}
fn invalid(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
type Connection = BufReader<TcpStream>;
async fn connect(c: &Config) -> io::Result<Connection> {
    let stream = TcpStream::connect(&c.address).await?;
    stream.set_nodelay(true)?;
    Ok(BufReader::new(stream))
}
async fn call(
    conn: &mut Connection,
    c: &Config,
    op: usize,
    k: &[u8],
    nonce: u64,
) -> io::Result<bool> {
    // Request construction and response integrity checks are inside logical latency.
    let v = if op == 1 { value(c, nonce) } else { Vec::new() };
    let cmd: &[u8] = [b"GET".as_slice(), b"SET", b"DEL"][op];
    let mut request = format!("*{}\r\n", if op == 1 { 3 } else { 2 }).into_bytes();
    for part in [cmd, k]
        .into_iter()
        .chain((op == 1).then_some(v.as_slice()))
    {
        request.extend_from_slice(format!("${}\r\n", part.len()).as_bytes());
        request.extend_from_slice(part);
        request.extend_from_slice(b"\r\n");
    }
    conn.get_mut().write_all(&request).await?;
    let mut line = Vec::new();
    loop {
        let b = conn.read_u8().await?;
        line.push(b);
        if b == b'\n' {
            break;
        }
        if line.len() > 128 {
            return Err(invalid("oversized RESP header"));
        }
    }
    if !line.ends_with(b"\r\n") {
        return Err(invalid("malformed RESP header"));
    }
    match op {
        0 => {
            if line == b"$-1\r\n" {
                if c.get == 100 {
                    return Err(invalid("read-only dataset unexpectedly missing"));
                }
                return Ok(false);
            }
            if line != format!("${}\r\n", c.value_bytes).as_bytes() {
                return Err(invalid("invalid GET length or response"));
            }
            let mut bytes = vec![0; c.value_bytes + 2];
            conn.read_exact(&mut bytes).await?;
            if !bytes.ends_with(b"\r\n") {
                return Err(invalid("invalid bulk terminator"));
            }
            let n = u64::from_be_bytes(bytes[..8].try_into().unwrap());
            if bytes[..c.value_bytes] != value(c, n) {
                return Err(invalid("invalid generated GET value"));
            }
        }
        1 if line != b"+OK\r\n" => return Err(invalid("SET did not acknowledge")),
        2 if line != b":0\r\n" && line != b":1\r\n" => return Err(invalid("invalid DEL result")),
        _ => (),
    }
    Ok(true)
}
async fn bounded_call(
    conn: &mut Connection,
    c: &Config,
    op: usize,
    k: &[u8],
    nonce: u64,
) -> io::Result<bool> {
    tokio::time::timeout(Duration::from_millis(1500), call(conn, c, op, k, nonce))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "request timed out; never retried"))?
}
#[derive(Serialize)]
struct Histogram {
    count: u64,
    sum_ns: u64,
    min_ns: u64,
    max_ns: u64,
    buckets: Vec<u64>,
}
impl Histogram {
    fn new() -> Self {
        Self {
            count: 0,
            sum_ns: 0,
            min_ns: u64::MAX,
            max_ns: 0,
            buckets: vec![0; 65],
        }
    }
    fn add(&mut self, n: u64) {
        self.count += 1;
        self.sum_ns += n;
        self.min_ns = self.min_ns.min(n);
        self.max_ns = self.max_ns.max(n);
        self.buckets[(64 - n.leading_zeros()) as usize] += 1;
    }
}
#[derive(Serialize)]
struct Worker {
    issued: u64,
    succeeded: u64,
    failed: u64,
    missing_gets: u64,
    errors: Vec<String>,
    latency: Vec<Histogram>,
}
fn proc_stat() -> String {
    fs::read_to_string("/proc/self/stat").unwrap_or_default()
}
async fn run(c: Config, output: &str) -> Result<bool, Box<dyn std::error::Error>> {
    if !(1..=64).contains(&c.workers)
        || !(1..=4096).contains(&c.keys)
        || !(16..=8192).contains(&c.value_bytes)
        || c.get + c.put + c.delete != 100
        || !(1..=60000).contains(&c.measure_ms)
        || !(1..=5_000_000).contains(&c.max_operations)
        || c.warmup_operations > 100000
        || c.run_id.is_empty()
        || c.run_id.len() > 64
        || !c
            .run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(invalid("invalid bounded reference configuration").into());
    }
    let c = Arc::new(c);
    let mut setup = connect(&c).await?;
    // Each trial receives a fresh Redis instance, so acknowledged SET establishes
    // the same populated hot set as KV9. The reserved sentinel is never traffic.
    for i in 0..=c.keys {
        bounded_call(&mut setup, &c, 1, &key(&c, i), i as u64).await?;
    }
    let initial_nonce = c.keys as u64 + 1;
    for i in 0..c.warmup_operations {
        let n = initial_nonce + i;
        let (op, k) = operation(&c, n);
        bounded_call(&mut setup, &c, op, &k, n).await?;
    }
    let mut connections = Vec::new();
    for _ in 0..c.workers {
        connections.push(connect(&c).await?);
    }
    let allocated = Arc::new(AtomicU64::new(0));
    let before = proc_stat();
    let start = Instant::now();
    let unix_start_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let deadline = start + Duration::from_millis(c.measure_ms);
    let mut tasks = Vec::new();
    for mut conn in connections {
        let c = c.clone();
        let allocated = allocated.clone();
        tasks.push(tokio::spawn(async move {
            let mut w = Worker {
                issued: 0,
                succeeded: 0,
                failed: 0,
                missing_gets: 0,
                errors: Vec::new(),
                latency: (0..3).map(|_| Histogram::new()).collect(),
            };
            while Instant::now() < deadline {
                let index =
                    match allocated.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |i| {
                        (i < c.max_operations).then_some(i + 1)
                    }) {
                        Ok(i) => i,
                        Err(_) => break,
                    };
                let n = initial_nonce + c.warmup_operations + index;
                let (op, k) = operation(&c, n);
                let begin = Instant::now();
                w.issued += 1;
                let result = bounded_call(&mut conn, &c, op, &k, n).await;
                w.latency[op].add(begin.elapsed().as_nanos() as u64);
                match result {
                    Ok(found) => {
                        w.succeeded += 1;
                        if op == 0 && !found {
                            w.missing_gets += 1;
                        }
                    }
                    Err(e) => {
                        w.failed += 1;
                        w.errors.push(format!("{e}"));
                        break;
                    }
                }
            }
            w
        }));
    }
    let mut workers = Vec::new();
    for task in tasks {
        workers.push(task.await?);
    }
    let elapsed_ns = start.elapsed().as_nanos() as u64;
    let after = proc_stat();
    let issued: u64 = workers.iter().map(|w| w.issued).sum();
    let succeeded: u64 = workers.iter().map(|w| w.succeeded).sum();
    let failed: u64 = workers.iter().map(|w| w.failed).sum();
    // A final sentinel check checks retained response/data framing outside timing.
    let verification = bounded_call(&mut setup, &c, 0, &key(&c, c.keys), 0).await;
    let complete = failed == 0 && verification.as_ref().is_ok_and(|v| *v) && issued > 0;
    let report = serde_json::json!({"version":1,"configuration":*c,"complete":complete,"verification_error":verification.err().map(|e| e.to_string()),
        "process_id":std::process::id(),"runtime_threads":2,"measurement_start_unix_ns":unix_start_ns,
        "cohort_elapsed_ns":elapsed_ns,"issued":issued,"succeeded":succeeded,"failed":failed,
        "stop_reason":if issued == c.max_operations {"operation_limit"} else {"duration"},
        "success_ops_per_second":succeeded as f64 * 1e9 / elapsed_ns as f64,
        "proc_stat_before":before,"proc_stat_after":after,"workers":workers});
    fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    Ok(complete)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: kv9-redis-reference CONFIG OUTPUT".into());
    }
    let c: Config = serde_json::from_slice(&fs::read(&args[1])?)?;
    let ok = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(run(c, &args[2]))?;
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}
