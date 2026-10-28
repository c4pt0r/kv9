//! kv9-repl — an interactive shell for kv9 on the kv9-client SDK.
//!
//! ```text
//! KV9_CLIENT_TOKEN=... kv9-repl --addr 127.0.0.1:20161 [--peers 1=host:port,2=...] [--keyspace 100]
//! kv9> keyspaces
//! kv9> use 100
//! kv9[100]> put user/1 alice
//! applied (term=1, index=7)
//! kv9[100]> batch put user/2 bob, del user/1
//! kv9[100]> scan user/ user0
//! ```
//!
//! Keys and values are plain tokens; `"quoted strings"` may contain spaces and
//! `hex:6b31` writes raw bytes. Non-printable results are shown as `hex:…`.

use std::io::{self, BufRead, Write};
use std::time::Duration;

use kv9_client::{Client, Error, Keyspace, Mutation};

struct Session {
    client: Client,
    keyspace: Option<Keyspace>,
}

fn usage() -> ! {
    eprintln!(
        "usage: kv9-repl --addr <host:port> [--token <token>] [--peers <id=host:port,...>] \
         [--keyspace <id>] [--timeout-ms <n>]\n       the token may also come from KV9_CLIENT_TOKEN"
    );
    std::process::exit(2)
}

fn parse_peers(text: &str) -> Vec<(u64, String)> {
    text.split(',')
        .filter(|s| !s.is_empty())
        .map(|item| {
            let (node, address) = item.split_once('=').unwrap_or_else(|| usage());
            (node.parse().unwrap_or_else(|_| usage()), address.to_owned())
        })
        .collect()
}

/// Split a line into tokens: whitespace separated, `"…"` keeps spaces.
fn tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' if quoted => current.push(chars.next().ok_or("dangling escape")?),
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if quoted {
        return Err("unterminated quote".into());
    }
    if !current.is_empty() {
        out.push(current);
    }
    Ok(out)
}

/// A key/value token: `hex:6b31` is raw bytes, anything else is literal UTF-8.
fn bytes(token: &str) -> Result<Vec<u8>, String> {
    if let Some(hex) = token.strip_prefix("hex:") {
        if hex.len() % 2 != 0 {
            return Err("hex literal needs an even number of digits".into());
        }
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| e.to_string()))
            .collect()
    } else {
        Ok(token.as_bytes().to_vec())
    }
}

fn show(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text)
            if !text.is_empty()
                && text.chars().all(|c| !c.is_control())
                && !text.starts_with("hex:") =>
        {
            text.to_owned()
        }
        _ => format!(
            "hex:{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ),
    }
}

fn describe(error: &Error) -> String {
    match error {
        Error::NotLeader { leader } => format!(
            "not leader (hint: {}); add the node to --peers to follow it",
            leader.map_or("unknown".to_owned(), |n| format!("node {n}"))
        ),
        Error::ReadUnconfirmed { phase } => format!("read unconfirmed ({phase:?}); retry"),
        Error::AdmissionRefused { reason } => format!("admission refused ({reason:?}); back off"),
        Error::PartialWrite {
            committed_chunks,
            last_applied,
            cause,
        } => format!(
            "partial write: {committed_chunks} chunk(s) committed through (term={}, index={}): {cause}",
            last_applied.term, last_applied.index
        ),
        Error::Rpc { code, message } if error.is_precondition() => {
            format!("refused ({code:?}): {message} — stale epoch or a batch crossing a region")
        }
        Error::Rpc { code, message } => format!("rpc {code:?}: {message} (write outcome unknown)"),
        Error::Bounds(what) => format!("invalid request: {what}"),
        Error::Protocol(what) => format!("protocol error: {what}"),
        Error::Transport(what) => format!("transport: {what}"),
    }
}

const HELP: &str = "\
commands:
  keyspaces                    list keyspaces
  create <name> [tenant]       create a raw keyspace and switch to it
  use <id>                     select the keyspace for data commands
  info                         cluster counts
  region [key]                 region and epoch of the current keyspace
  refresh                      re-read the epoch of the current keyspace
  get <k>                      point read
  mget <k>...                  one quorum-confirmed multi-key read
  put <k> <v>                  single put
  del <k>                      single delete
  mput <k> <v> [<k> <v>]...    put-only atomic batch
  batch <op>, <op>...          atomic range batch; op = put <k> <v> | del <k>
  scan [start [end [limit]]]   one page of [start, end)
  delrange <start> <end>       chunked range delete
  help | quit
tokens: plain, \"quoted with spaces\", or hex:<bytes>";

async fn run(session: &mut Session, tokens: &[String]) -> Result<(), String> {
    let ks = || -> Result<&Keyspace, String> {
        session
            .keyspace
            .as_ref()
            .ok_or_else(|| "no keyspace selected: `use <id>` or `create <name>`".to_owned())
    };
    let rpc = |e: Error| describe(&e);
    let applied = |a: kv9_client::Applied| println!("applied (term={}, index={})", a.term, a.index);
    match tokens[0].as_str() {
        "help" | "?" => println!("{HELP}"),
        "keyspaces" => {
            for k in session.client.list_keyspaces().await.map_err(rpc)? {
                println!("{:>8}  tenant={}  {}", k.id, k.tenant_id, k.name);
            }
        }
        "info" => {
            let i = session.client.cluster_info().await.map_err(rpc)?;
            println!(
                "nodes={} keyspaces={} regions={} addressing={}",
                i.node_count,
                i.keyspace_count,
                i.region_count,
                session.client.address()
            );
        }
        "create" => {
            let name = tokens.get(1).ok_or("create <name> [tenant]")?;
            let tenant = tokens
                .get(2)
                .map_or(Ok(0), |t| t.parse::<u64>())
                .map_err(|e| e.to_string())?;
            let id = session
                .client
                .create_keyspace(name, tenant)
                .await
                .map_err(rpc)?;
            println!("keyspace {id} created; selected");
            session.keyspace = Some(session.client.keyspace(id));
        }
        "use" => {
            let id: u32 = tokens
                .get(1)
                .ok_or("use <id>")?
                .parse()
                .map_err(|e| format!("{e}"))?;
            session.keyspace = Some(session.client.keyspace(id));
            println!("keyspace {id} selected (epoch 1/1 until refreshed)");
        }
        "region" => {
            let ks = ks()?;
            let key = tokens.get(1).map_or(Ok(Vec::new()), |t| bytes(t))?;
            let r = session
                .client
                .get_region(ks.id(), &key)
                .await
                .map_err(rpc)?;
            println!(
                "region={} epoch=(conf_ver={}, version={}) leader={}",
                r.region_id,
                r.epoch.conf_ver,
                r.epoch.version,
                r.leader_node_id.map_or("unknown".into(), |n| n.to_string())
            );
        }
        "refresh" => {
            let e = ks()?.refresh_epoch(b"").await.map_err(rpc)?;
            println!("epoch=(conf_ver={}, version={})", e.conf_ver, e.version);
        }
        "get" => {
            let key = bytes(tokens.get(1).ok_or("get <k>")?)?;
            match ks()?.get(key).await.map_err(rpc)? {
                Some(v) => println!("{}", show(&v)),
                None => println!("(nil)"),
            }
        }
        "mget" => {
            let keys = tokens[1..]
                .iter()
                .map(|t| bytes(t))
                .collect::<Result<Vec<_>, _>>()?;
            if keys.is_empty() {
                return Err("mget <k>...".into());
            }
            for (key, value) in keys
                .iter()
                .zip(ks()?.batch_get(keys.clone()).await.map_err(rpc)?)
            {
                println!(
                    "{} => {}",
                    show(key),
                    value.map_or("(nil)".into(), |v| show(&v))
                );
            }
        }
        "put" => {
            let (k, v) = (
                tokens.get(1).ok_or("put <k> <v>")?,
                tokens.get(2).ok_or("put <k> <v>")?,
            );
            applied(ks()?.put(bytes(k)?, bytes(v)?).await.map_err(rpc)?);
        }
        "del" => {
            let key = bytes(tokens.get(1).ok_or("del <k>")?)?;
            applied(ks()?.delete(key).await.map_err(rpc)?);
        }
        "mput" => {
            let rest = &tokens[1..];
            if rest.is_empty() || !rest.len().is_multiple_of(2) {
                return Err("mput <k> <v> [<k> <v>]...".into());
            }
            let pairs = rest
                .chunks(2)
                .map(|p| Ok((bytes(&p[0])?, bytes(&p[1])?)))
                .collect::<Result<Vec<_>, String>>()?;
            applied(ks()?.batch_put(pairs).await.map_err(rpc)?);
        }
        "batch" => {
            let mut mutations = Vec::new();
            for op in tokens[1..].join(" ").split(',') {
                let parts = tokenize(op)?;
                match parts.as_slice() {
                    [p, k, v] if p == "put" => mutations.push(Mutation::Put {
                        key: bytes(k)?,
                        value: bytes(v)?,
                    }),
                    [d, k] if d == "del" => mutations.push(Mutation::Delete { key: bytes(k)? }),
                    _ => {
                        return Err(format!(
                            "bad batch op `{}`: use put <k> <v> | del <k>",
                            op.trim()
                        ))
                    }
                }
            }
            applied(ks()?.batch(mutations).await.map_err(rpc)?);
        }
        "scan" => {
            let start = tokens.get(1).map_or(Ok(Vec::new()), |t| bytes(t))?;
            let end = tokens.get(2).map_or(Ok(Vec::new()), |t| bytes(t))?;
            let limit: u32 = tokens
                .get(3)
                .map_or(Ok(64), |t| t.parse())
                .map_err(|e| format!("{e}"))?;
            let page = ks()?.scan(start, end, limit).await.map_err(rpc)?;
            for (k, v) in &page.pairs {
                println!("{} => {}", show(k), show(v));
            }
            if !page.resume_from.is_empty() {
                println!(
                    "(paused at a range boundary; continue from {})",
                    show(&page.resume_from)
                );
            } else {
                println!("({} row(s))", page.pairs.len());
            }
        }
        "delrange" => {
            let (s, e) = (
                tokens.get(1).ok_or("delrange <start> <end>")?,
                tokens.get(2).ok_or("delrange <start> <end>")?,
            );
            let r = ks()?
                .delete_range(bytes(s)?, bytes(e)?)
                .await
                .map_err(rpc)?;
            println!(
                "deleted: {} chunk(s), last applied (term={}, index={}){}",
                r.committed_chunks,
                r.last_applied.term,
                r.last_applied.index,
                if r.resume_from.is_empty() {
                    String::new()
                } else {
                    format!("; continue from {}", show(&r.resume_from))
                }
            );
        }
        other => return Err(format!("unknown command `{other}` (try `help`)")),
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let (mut addr, mut token, mut peers, mut keyspace, mut timeout_ms) = (
        None,
        std::env::var("KV9_CLIENT_TOKEN").ok(),
        Vec::new(),
        None,
        5000u64,
    );
    while let Some(flag) = args.next() {
        let value = args.next().unwrap_or_else(|| usage());
        match flag.as_str() {
            "--addr" => addr = Some(value),
            "--token" => token = Some(value),
            "--peers" => peers = parse_peers(&value),
            "--keyspace" => keyspace = Some(value.parse::<u32>().unwrap_or_else(|_| usage())),
            "--timeout-ms" => timeout_ms = value.parse().unwrap_or_else(|_| usage()),
            _ => usage(),
        }
    }
    let addr = addr.unwrap_or_else(|| usage());
    let token = token.filter(|t| !t.is_empty()).unwrap_or_else(|| usage());
    let client = match Client::new(addr, &token) {
        Ok(client) => client
            .with_peers(peers)
            .with_timeout(Duration::from_millis(timeout_ms)),
        Err(error) => {
            eprintln!("error: {}", describe(&error));
            std::process::exit(2);
        }
    };
    let mut session = Session {
        keyspace: keyspace.map(|id| client.keyspace(id)),
        client,
    };
    let interactive = std::io::IsTerminal::is_terminal(&io::stdin());
    if interactive {
        println!(
            "kv9-repl connected to {} (type `help`)",
            session.client.address()
        );
    }
    let stdin = io::stdin();
    let mut failures = 0u32;
    loop {
        if interactive {
            match &session.keyspace {
                Some(ks) => print!("kv9[{}]> ", ks.id()),
                None => print!("kv9> "),
            }
            io::stdout().flush().ok();
        }
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                eprintln!("stdin: {error}");
                break;
            }
        }
        let tokens = match tokenize(line.trim()) {
            Ok(tokens) => tokens,
            Err(error) => {
                eprintln!("error: {error}");
                failures += 1;
                continue;
            }
        };
        if tokens.is_empty() {
            continue;
        }
        if matches!(tokens[0].as_str(), "quit" | "exit") {
            break;
        }
        if let Err(error) = run(&mut session, &tokens).await {
            eprintln!("error: {error}");
            failures += 1;
        }
    }
    // Scripted use (`printf 'put a b\nget a\n' | kv9-repl …`) exits non-zero if any command failed.
    if !interactive && failures > 0 {
        std::process::exit(1);
    }
}
