//! kv9-admin — cluster administration for kv9 (docs/KV9-ADMIN.md), increment 1.
//!
//! A pure gRPC client on the Rust SDK: it opens no data directory and holds no state
//! of its own. It finds the metadata leader from the endpoints it is given (every node
//! answers `NodeStatus` about itself; the directory read and the membership verbs
//! follow `NotLeader` hints), shows the cluster as the catalog sees it, drives the
//! membership verbs with `--wait` on the committed configuration, and prints JSON
//! with a fixed exit-code table for scripts.
//!
//! ```text
//! kv9-admin [--context <name> | --endpoints <[id=]host:port,...>] [--token <t>]
//!           [--output table|json] [--timeout <secs>] [--yes] [--no-redirect] [-v]
//!   cluster  info | status [--node <id>] | health | endpoints
//!   node     list | describe <id> | admit <id> --addr <host:port> [--ttl <secs>]
//!            | revoke <id> | promote <id> [--wait] | remove <id> [--wait]
//!   keyspace list | describe <id|name> | create --name <n> [--tenant <id>]
//!   version
//! ```
//!
//! Exit codes: 0 ok · 1 refused by the server with a typed reason · 2 usage ·
//! 3 transport (dial, deadline, unauthenticated) · 4 partial (some nodes unreachable)
//! · 6 `--wait` deadline passed with the change proposed but not visible yet.

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kv9_client::{Client, DataGroup, Error, NodeDirectory, NodeStatus};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: u64 = 1;
const DEFAULT_WAIT: Duration = Duration::from_secs(30);
/// Pending admissions that end within this many seconds are a health warning.
const ADMISSION_EXPIRY_WARNING_SECS: u64 = 3600;
/// Entries between the leader's commit index and a node's applied index that count
/// as lag.
const APPLY_LAG_WARNING: u64 = 1024;

fn usage() -> String {
    "usage: kv9-admin [--context <name> | --endpoints <[id=]host:port,...>] [--token <t>]\n\
     \u{20}                [--output table|json] [--timeout <secs>] [--yes] [--no-redirect] [-v]\n\
     \u{20}                <noun> <verb> [args]\n\
     \n\
     cluster  info                         counters, ids, metadata leader\n\
     \u{20}        status [--node <id>]         every node's status document (fan-out), or one\n\
     \u{20}        health                       verdicts: quorum margin, lost voters, fatal, lag, ...\n\
     \u{20}        endpoints                    the node directory: id, address, generation, active\n\
     node     list                         directory + admissions + the leader's raft view\n\
     \u{20}        describe <id>                one node: directory row and status document\n\
     \u{20}        admit <id> --addr <a> [--ttl <secs>]   prints the one-time join ticket\n\
     \u{20}        revoke <id>\n\
     \u{20}        promote <id> [--wait [--wait-secs <n>]]\n\
     \u{20}        remove <id>  [--wait [--wait-secs <n>]] refuses below three voters\n\
     \u{20}        transfer-leader --to <id> [--wait]     hand metadata leadership to a caught-up voter\n\
     keyspace list | describe <id|name> | create --name <n> [--tenant <id>] [--group <creation-task>]\n\
     group    list                         committed data groups: task, region, replicas, keyspaces\n\
     \u{20}        describe <creation-task>     rows + every replica's live view of the group\n\
     \u{20}        create --voters <id,id,id> [--operation <hex> | --label <text>]\n\
     \u{20}        add-replica <creation-task> --to <node> [--operation <hex> | --label <text>]\n\
     \u{20}        move-replica <creation-task> --from <node> --to <node> [--operation <hex> | --label <text>]\n\
     \u{20}        transfer-leader <creation-task> --to <node> [--wait]\n\
     task     list [--kind <n>] [--after <id>] [--limit <n>]   the committed TASKS history\n\
     \u{20}        describe <id>\n\
     version\n\
     \n\
     The token comes from --token, KV9_ADMIN_TOKEN, KV9_CLIENT_TOKEN or the context's\n\
     token_env. Contexts live in ~/.config/kv9/admin.toml:\n\
     \u{20} default = \"local\"\n\
     \u{20} [context.local]\n\
     \u{20} endpoints = [\"1=127.0.0.1:20161\", \"2=127.0.0.1:20162\", \"3=127.0.0.1:20163\"]\n\
     \u{20} token_env = \"KV9_ADMIN_TOKEN\"\n\
     Exit codes: 0 ok, 1 refused (typed), 2 usage, 3 transport, 4 partial, 6 wait deadline."
        .to_string()
}

// ------------------------------------------------------------------ failures

#[derive(Debug)]
enum Fail {
    Usage(String),
    Refused(String),
    Transport(String),
    WaitDeadline(String),
}

impl Fail {
    fn code(&self) -> u8 {
        match self {
            Fail::Usage(_) => 2,
            Fail::Refused(_) => 1,
            Fail::Transport(_) => 3,
            Fail::WaitDeadline(_) => 6,
        }
    }
    fn kind(&self) -> &'static str {
        match self {
            Fail::Usage(_) => "usage",
            Fail::Refused(_) => "refused",
            Fail::Transport(_) => "transport",
            Fail::WaitDeadline(_) => "wait_deadline",
        }
    }
    fn message(&self) -> &str {
        match self {
            Fail::Usage(m) | Fail::Refused(m) | Fail::Transport(m) | Fail::WaitDeadline(m) => m,
        }
    }
}

impl From<Error> for Fail {
    fn from(error: Error) -> Self {
        use tonic_code::*;
        match &error {
            Error::Transport(_) => Fail::Transport(error.to_string()),
            Error::Rpc { code, .. } if is_transport(*code) => Fail::Transport(error.to_string()),
            Error::Bounds(_) => Fail::Usage(error.to_string()),
            _ => Fail::Refused(error.to_string()),
        }
    }
}

/// tonic's `Code` is re-exported through the SDK error; keep the mapping in one place.
mod tonic_code {
    pub fn is_transport(code: kv9_client::Code) -> bool {
        matches!(
            code,
            kv9_client::Code::Unavailable
                | kv9_client::Code::DeadlineExceeded
                | kv9_client::Code::Unauthenticated
                | kv9_client::Code::Cancelled
        )
    }
}

type Outcome = Result<Report, Fail>;

/// What a command produced: the JSON result and its table rendering.
struct Report {
    command: String,
    result: Value,
    table: String,
    /// Non-fatal problems (unreachable nodes); exit 4 when non-empty.
    partial: Vec<String>,
}

// ------------------------------------------------------------------ arguments

#[derive(Clone, Copy, PartialEq, Eq)]
enum Output {
    Table,
    Json,
}

struct Global {
    context: Option<String>,
    endpoints: Vec<(Option<u64>, String)>,
    token: Option<String>,
    output: Output,
    timeout: Duration,
    yes: bool,
    no_redirect: bool,
    verbose: bool,
}

struct Args {
    global: Global,
    words: Vec<String>,
}

fn parse_endpoints(text: &str) -> Result<Vec<(Option<u64>, String)>, Fail> {
    let mut out = Vec::new();
    for item in text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (id, address) = match item.split_once('=') {
            Some((id, address)) => (
                Some(
                    id.parse::<u64>()
                        .map_err(|_| Fail::Usage(format!("endpoint {item:?}: node id is not a number")))?,
                ),
                address,
            ),
            None => (None, item),
        };
        if !address.contains(':') {
            return Err(Fail::Usage(format!("endpoint {address:?} is not host:port")));
        }
        out.push((id, address.to_owned()));
    }
    if out.is_empty() {
        return Err(Fail::Usage("no endpoints given".into()));
    }
    Ok(out)
}

fn parse_args(argv: Vec<String>) -> Result<Args, Fail> {
    let mut global = Global {
        context: None,
        endpoints: Vec::new(),
        token: None,
        output: if std::io::stdout().is_terminal() {
            Output::Table
        } else {
            Output::Json
        },
        timeout: Duration::from_secs(5),
        yes: false,
        no_redirect: false,
        verbose: false,
    };
    let mut words = Vec::new();
    let mut it = argv.into_iter().peekable();
    let need = |flag: &str, it: &mut std::iter::Peekable<std::vec::IntoIter<String>>| {
        it.next()
            .ok_or_else(|| Fail::Usage(format!("{flag} needs a value")))
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--context" => global.context = Some(need("--context", &mut it)?),
            "--endpoints" => global.endpoints = parse_endpoints(&need("--endpoints", &mut it)?)?,
            "--token" => global.token = Some(need("--token", &mut it)?),
            "--output" => {
                global.output = match need("--output", &mut it)?.as_str() {
                    "table" => Output::Table,
                    "json" => Output::Json,
                    other => return Err(Fail::Usage(format!("--output {other:?}: table or json"))),
                }
            }
            "--timeout" => {
                let secs: u64 = need("--timeout", &mut it)?
                    .parse()
                    .map_err(|_| Fail::Usage("--timeout must be whole seconds".into()))?;
                global.timeout = Duration::from_secs(secs.clamp(1, 30));
            }
            "--yes" | "-y" => global.yes = true,
            "--no-redirect" => global.no_redirect = true,
            "-v" | "--verbose" => global.verbose = true,
            "-h" | "--help" | "help" if words.is_empty() => {
                return Err(Fail::Usage(usage()));
            }
            _ => words.push(arg),
        }
    }
    Ok(Args { global, words })
}

/// `--flag value` pairs after the verb, plus positional words.
struct VerbArgs {
    positional: Vec<String>,
    flags: BTreeMap<String, String>,
    switches: Vec<String>,
}

fn verb_args(words: &[String], switches: &[&str]) -> Result<VerbArgs, Fail> {
    let mut out = VerbArgs {
        positional: Vec::new(),
        flags: BTreeMap::new(),
        switches: Vec::new(),
    };
    let mut it = words.iter();
    while let Some(word) = it.next() {
        if let Some(flag) = word.strip_prefix("--") {
            if switches.contains(&flag) {
                out.switches.push(flag.to_string());
                continue;
            }
            let value = it
                .next()
                .ok_or_else(|| Fail::Usage(format!("--{flag} needs a value")))?;
            out.flags.insert(flag.to_string(), value.clone());
        } else {
            out.positional.push(word.clone());
        }
    }
    Ok(out)
}

fn parse_id(text: &str, what: &str) -> Result<u64, Fail> {
    text.parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| Fail::Usage(format!("{what} must be a positive integer, got {text:?}")))
}

// ------------------------------------------------------------------ contexts

struct Context {
    endpoints: Vec<(Option<u64>, String)>,
    token_env: Option<String>,
    timeout: Option<Duration>,
}

/// The subset of TOML the contexts file uses: `default = "name"` at top level,
/// `[context.<name>]` tables with `endpoints = ["..."]`, `token_env = "..."`,
/// `timeout_secs = n`. Anything else is an error, not a guess.
fn load_contexts(path: &std::path::Path) -> Result<(Option<String>, BTreeMap<String, Context>), Fail> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Fail::Usage(format!("read {}: {e}", path.display())))?;
    let mut default = None;
    let mut contexts: BTreeMap<String, Context> = BTreeMap::new();
    let mut current: Option<String> = None;
    let unquote = |v: &str, line: usize| -> Result<String, Fail> {
        v.strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .map(str::to_owned)
            .ok_or_else(|| Fail::Usage(format!("{}:{line}: expected a quoted string", path.display())))
    };
    for (number, raw) in text.lines().enumerate() {
        let line = number + 1;
        let content = raw.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        if let Some(section) = content.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let name = section
                .strip_prefix("context.")
                .ok_or_else(|| Fail::Usage(format!("{}:{line}: only [context.<name>] tables are understood", path.display())))?;
            contexts.entry(name.to_string()).or_insert(Context {
                endpoints: Vec::new(),
                token_env: None,
                timeout: None,
            });
            current = Some(name.to_string());
            continue;
        }
        let (key, value) = content
            .split_once('=')
            .map(|(k, v)| (k.trim(), v.trim()))
            .ok_or_else(|| Fail::Usage(format!("{}:{line}: expected key = value", path.display())))?;
        match (&current, key) {
            (None, "default") => default = Some(unquote(value, line)?),
            (Some(name), "endpoints") => {
                let inner = value
                    .strip_prefix('[')
                    .and_then(|v| v.strip_suffix(']'))
                    .ok_or_else(|| Fail::Usage(format!("{}:{line}: endpoints must be an array", path.display())))?;
                let mut items = Vec::new();
                for item in inner.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    items.push(unquote(item, line)?);
                }
                contexts.get_mut(name).expect("current context").endpoints =
                    parse_endpoints(&items.join(","))?;
            }
            (Some(name), "token_env") => {
                contexts.get_mut(name).expect("current context").token_env = Some(unquote(value, line)?)
            }
            (Some(name), "timeout_secs") => {
                let secs: u64 = value
                    .parse()
                    .map_err(|_| Fail::Usage(format!("{}:{line}: timeout_secs must be a number", path.display())))?;
                contexts.get_mut(name).expect("current context").timeout = Some(Duration::from_secs(secs));
            }
            _ => {
                return Err(Fail::Usage(format!(
                    "{}:{line}: unknown key {key:?}",
                    path.display()
                )))
            }
        }
    }
    Ok((default, contexts))
}

fn contexts_path() -> Option<std::path::PathBuf> {
    if let Ok(path) = std::env::var("KV9_ADMIN_CONFIG") {
        return Some(path.into());
    }
    let home = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .ok()?;
    Some(home.join("kv9").join("admin.toml"))
}

/// Fill endpoints, token and timeout from the context when the flags left them open.
fn resolve_context(global: &mut Global) -> Result<(), Fail> {
    let wants_context = global.context.is_some() || global.endpoints.is_empty();
    if !wants_context {
        return Ok(());
    }
    let name = global
        .context
        .clone()
        .or_else(|| std::env::var("KV9_ADMIN_CONTEXT").ok());
    let Some(path) = contexts_path() else {
        return Err(Fail::Usage("no --endpoints and no contexts file".into()));
    };
    if !path.exists() {
        return Err(Fail::Usage(format!(
            "no --endpoints given and {} does not exist\n\n{}",
            path.display(),
            usage()
        )));
    }
    let (default, contexts) = load_contexts(&path)?;
    let name = name.or(default).ok_or_else(|| {
        Fail::Usage(format!("{} has no default context; pass --context", path.display()))
    })?;
    let context = contexts.get(&name).ok_or_else(|| {
        Fail::Usage(format!("context {name:?} is not in {}", path.display()))
    })?;
    if global.endpoints.is_empty() {
        global.endpoints = context.endpoints.clone();
    }
    if global.token.is_none() {
        if let Some(var) = &context.token_env {
            global.token = std::env::var(var).ok();
        }
    }
    if let Some(timeout) = context.timeout {
        global.timeout = timeout.clamp(Duration::from_secs(1), Duration::from_secs(30));
    }
    Ok(())
}

// ------------------------------------------------------------------ the cluster

struct Cluster {
    /// Addressed at the metadata leader; follows hints through `peers`.
    leader: Client,
    leader_id: u64,
    peers: BTreeMap<u64, String>,
    token: String,
    timeout: Duration,
    verbose: bool,
    unreachable: Vec<String>,
}

impl Cluster {
    async fn connect(global: &Global) -> Result<Self, Fail> {
        let token = global
            .token
            .clone()
            .or_else(|| std::env::var("KV9_ADMIN_TOKEN").ok())
            .or_else(|| std::env::var("KV9_CLIENT_TOKEN").ok())
            .filter(|t| !t.is_empty())
            .ok_or_else(|| Fail::Usage("no token: --token, KV9_ADMIN_TOKEN or KV9_CLIENT_TOKEN".into()))?;
        // Every node answers NodeStatus about itself: that is how the endpoints given
        // become a peers map and name the leader, without any node-local file.
        let mut peers = BTreeMap::new();
        let mut leader_id = 0;
        let mut unreachable = Vec::new();
        let mut refused: Option<Fail> = None;
        let mut answered = false;
        let mut hints: BTreeMap<u64, String> = BTreeMap::new();
        for (id, address) in &global.endpoints {
            let client = Client::new(address.clone(), &token)?.with_timeout(global.timeout);
            match client.node_status().await {
                Ok(status) => {
                    answered = true;
                    let node = status.u64("node_id").unwrap_or(id.unwrap_or(0));
                    if node > 0 {
                        peers.insert(node, address.clone());
                    }
                    if leader_id == 0 {
                        leader_id = status.u64("leader_id").unwrap_or(0);
                    }
                    for (peer, peer_address) in &status.peers {
                        hints.entry(*peer).or_insert_with(|| peer_address.clone());
                    }
                }
                Err(error) => {
                    if let Some(id) = id {
                        peers.insert(*id, address.clone());
                    }
                    let fail = Fail::from(error);
                    if matches!(fail, Fail::Refused(_)) && refused.is_none() {
                        refused = Some(Fail::Refused(format!("{address}: {}", fail.message())));
                    }
                    unreachable.push(format!("{address}: {}", fail.message()));
                }
            }
        }
        if !answered {
            // A typed refusal (not an administrator, for instance) is not a transport
            // problem and must not be reported as one.
            if let Some(refused) = refused {
                return Err(refused);
            }
            return Err(Fail::Transport(format!(
                "no endpoint answered: {}",
                unreachable.join("; ")
            )));
        }
        // Endpoints given explicitly win; the nodes' own directory view fills the rest,
        // which is how one follower address is enough to reach the leader.
        for (peer, address) in hints {
            peers.entry(peer).or_insert(address);
        }
        let leader_address = match peers.get(&leader_id) {
            Some(address) => address.clone(),
            None if leader_id == 0 => {
                return Err(Fail::Refused(
                    "no node reports a metadata leader yet (the group is electing or not serving)".into(),
                ))
            }
            None => {
                return Err(Fail::Refused(format!(
                    "the metadata leader is node {leader_id}, whose address is not among the endpoints given; add it"
                )))
            }
        };
        let mut leader = Client::new(leader_address, &token)?
            .with_timeout(global.timeout)
            .with_peers(peers.clone());
        if global.no_redirect {
            leader = leader.with_max_redirects(0);
        }
        if global.verbose {
            eprintln!("resolved leader: node {leader_id} at {}", leader.address());
        }
        Ok(Self {
            leader,
            leader_id,
            peers,
            token,
            timeout: global.timeout,
            verbose: global.verbose,
            unreachable,
        })
    }

    /// A client aimed at one replica of a data group (the first one whose node is
    /// in the directory); the SDK follows the NotLeader hint to the group leader.
    async fn group_leader_client(&mut self, region: u64) -> Result<Client, Fail> {
        let groups = self.leader.list_data_groups().await?;
        let group = groups
            .iter()
            .find(|g| g.region_id == region)
            .ok_or_else(|| Fail::Refused(format!("region {region} is not a committed data group")))?;
        let _ = self.directory().await;
        for replica in group.current_members() {
            if let Some(address) = self.peers.get(&replica.node_id) {
                return Ok(Client::new(address.clone(), &self.token)?
                    .with_timeout(self.timeout)
                    .with_peers(self.peers.clone()));
            }
        }
        Err(Fail::Refused(format!("no replica of region {region} has a known address")))
    }

    /// The certified metadata root digest, from the leader's status document.
    async fn root_digest(&self) -> Result<[u8; 32], Fail> {
        let status = self.leader.node_status().await?;
        let text = status
            .get("root_digest")
            .ok_or_else(|| Fail::Refused("the leader reports no root_digest".into()))?;
        let bytes = parse_hex(text, "root_digest", 32).map_err(|_| Fail::Refused(format!("unreadable root_digest {text:?}")))?;
        Ok(bytes.try_into().expect("32 bytes"))
    }

    fn resolved(&self) -> Value {
        json!({ "node_id": self.leader_id, "address": self.leader.address() })
    }

    fn client_at(&self, address: &str) -> Result<Client, Fail> {
        Ok(Client::new(address.to_owned(), &self.token)?.with_timeout(self.timeout))
    }

    /// The directory, and the peers map refreshed from it (every registered node).
    async fn directory(&mut self) -> Result<NodeDirectory, Fail> {
        let directory = self.leader.list_nodes().await?;
        for node in &directory.nodes {
            if !node.address.is_empty() {
                self.peers.insert(node.node_id, node.address.clone());
            }
        }
        self.leader_id = directory.leader_node_id;
        Ok(directory)
    }

    /// `NodeStatus` from every known peer, in parallel; unreachable ones are reported,
    /// not fatal.
    async fn fan_out(&self, only: Option<u64>) -> Vec<(u64, String, Result<NodeStatus, String>)> {
        let mut tasks = Vec::new();
        for (node, address) in &self.peers {
            if only.is_some_and(|id| id != *node) {
                continue;
            }
            let (node, address) = (*node, address.clone());
            let client = self.client_at(&address);
            tasks.push(tokio::spawn(async move {
                let result = match client {
                    Ok(client) => client.node_status().await.map_err(|e| e.to_string()),
                    Err(e) => Err(e.message().to_string()),
                };
                (node, address, result)
            }));
        }
        let mut out = Vec::new();
        for task in tasks {
            if let Ok(item) = task.await {
                out.push(item);
            }
        }
        out.sort_by_key(|(node, _, _)| *node);
        out
    }

    /// Poll the leader's status document until `done` holds.
    async fn wait_for(
        &self,
        what: &str,
        deadline: Duration,
        done: impl Fn(&NodeStatus) -> bool,
    ) -> Result<NodeStatus, Fail> {
        let start = Instant::now();
        let mut last_error = String::new();
        loop {
            match self.leader.node_status().await {
                Ok(status) if done(&status) => return Ok(status),
                Ok(_) => {}
                Err(error) => last_error = error.to_string(),
            }
            if start.elapsed() >= deadline {
                return Err(Fail::WaitDeadline(format!(
                    "{what} not visible after {:.0?}{}",
                    deadline,
                    if last_error.is_empty() { String::new() } else { format!(" (last error: {last_error})") }
                )));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

// ------------------------------------------------------------------ rendering

fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
    }
    let line = |cells: Vec<String>| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{:<width$}", c, width = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let mut out = line(headers.iter().map(|h| h.to_string()).collect());
    for row in rows {
        out.push('\n');
        out.push_str(&line(row.clone()));
    }
    out
}

fn ids(list: &[u64]) -> String {
    list.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn status_json(status: &NodeStatus) -> Value {
    let mut map = serde_json::Map::new();
    for (name, value) in &status.fields {
        // JSON-valued lines stay structured; everything else is text.
        let parsed = if value.starts_with('{') || value.starts_with('[') {
            serde_json::from_str::<Value>(value).ok()
        } else {
            None
        };
        map.insert(name.clone(), parsed.unwrap_or_else(|| Value::String(value.clone())));
    }
    Value::Object(map)
}

fn membership_report(command: &str, change: &kv9_client::MembershipChange, extra: Value) -> Report {
    let mut result = json!({
        "applied": { "term": change.applied.term, "index": change.applied.index },
        "voters": change.voters,
        "learners": change.learners,
    });
    if let Some(ticket) = &change.join_ticket {
        result["join_ticket"] = Value::String(ticket.clone());
    }
    if let Value::Object(extra) = extra {
        for (k, v) in extra {
            result[k] = v;
        }
    }
    let mut lines = vec![
        format!("applied: term={} index={}", change.applied.term, change.applied.index),
        format!("voters:   {}", ids(&change.voters)),
        format!("learners: {}", ids(&change.learners)),
    ];
    if let Some(ticket) = &change.join_ticket {
        lines.push(format!("join_ticket: {ticket}"));
        lines.push("(shown once; the catalog keeps only its digest)".into());
    }
    Report {
        command: command.to_string(),
        result,
        table: lines.join("\n"),
        partial: Vec::new(),
    }
}

/// Mutating verbs print their plan and ask on a terminal unless `--yes`.
fn confirm(global: &Global, plan: &str, typed: Option<&str>) -> Result<(), Fail> {
    eprintln!("{plan}");
    if global.yes || !std::io::stdin().is_terminal() {
        return Ok(());
    }
    let prompt = match typed {
        Some(word) => format!("type {word} to continue: "),
        None => "continue? [yes/N] ".to_string(),
    };
    eprint!("{prompt}");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).ok();
    let answer = line.trim();
    let accepted = match typed {
        Some(word) => answer == word,
        None => answer.eq_ignore_ascii_case("yes") || answer.eq_ignore_ascii_case("y"),
    };
    if accepted {
        Ok(())
    } else {
        Err(Fail::Usage("not confirmed".into()))
    }
}

// ------------------------------------------------------------------ commands

async fn cluster_info(cluster: &mut Cluster) -> Outcome {
    let info = cluster.leader.cluster_info().await?;
    let status = cluster.leader.node_status().await?;
    let result = json!({
        "cluster_id": status.get("cluster_id").unwrap_or(""),
        "root_digest": status.get("root_digest").unwrap_or(""),
        "bootstrap_generation": status.get("bootstrap_generation").unwrap_or(""),
        "leader_node_id": cluster.leader_id,
        "leader_address": cluster.leader.address(),
        "term": status.u64("term").unwrap_or(0),
        "meta_voters": status.ids("meta_voters"),
        "meta_learners": status.ids("meta_learners"),
        "node_count": info.node_count,
        "keyspace_count": info.keyspace_count,
        "region_count": info.region_count,
    });
    let table = [
        ("cluster_id", result["cluster_id"].as_str().unwrap_or("").to_string()),
        ("root_digest", result["root_digest"].as_str().unwrap_or("").to_string()),
        ("leader", format!("node {} at {}", cluster.leader_id, cluster.leader.address())),
        ("term", result["term"].to_string()),
        ("meta_voters", status.get("meta_voters").unwrap_or("").to_string()),
        ("meta_learners", status.get("meta_learners").unwrap_or("").to_string()),
        ("nodes", info.node_count.to_string()),
        ("keyspaces", info.keyspace_count.to_string()),
        ("regions", info.region_count.to_string()),
    ]
    .iter()
    .map(|(k, v)| format!("{k:<14} {v}"))
    .collect::<Vec<_>>()
    .join("\n");
    Ok(Report {
        command: "cluster info".into(),
        result,
        table,
        partial: Vec::new(),
    })
}

async fn cluster_endpoints(cluster: &mut Cluster) -> Outcome {
    let directory = cluster.directory().await?;
    let rows: Vec<Vec<String>> = directory
        .nodes
        .iter()
        .map(|n| {
            vec![
                n.node_id.to_string(),
                n.address.clone(),
                n.endpoint_generation.to_string(),
                if n.active { "yes".into() } else { "no".into() },
                hex(&n.store_incarnation),
            ]
        })
        .collect();
    Ok(Report {
        command: "cluster endpoints".into(),
        result: json!(directory
            .nodes
            .iter()
            .map(|n| json!({
                "node_id": n.node_id,
                "address": n.address,
                "generation": n.endpoint_generation,
                "active": n.active,
                "store_incarnation": hex(&n.store_incarnation),
            }))
            .collect::<Vec<_>>()),
        table: table(&["NODE", "ADDRESS", "GEN", "ACTIVE", "STORE_INCARNATION"], &rows),
        partial: Vec::new(),
    })
}

async fn cluster_status(cluster: &mut Cluster, only: Option<u64>) -> Outcome {
    // The directory gives every registered address; a follower-only endpoint list
    // would otherwise miss nodes. A failed directory read is not fatal here.
    let _ = cluster.directory().await;
    let statuses = cluster.fan_out(only).await;
    let mut partial = Vec::new();
    let mut nodes = Vec::new();
    let mut rows = Vec::new();
    for (node, address, result) in &statuses {
        match result {
            Ok(status) => {
                rows.push(vec![
                    node.to_string(),
                    address.clone(),
                    status.get("role").unwrap_or("").into(),
                    status.get("leader_id").unwrap_or("").into(),
                    status.get("term").unwrap_or("").into(),
                    format!(
                        "{}/{}",
                        status.get("applied_index").unwrap_or("?"),
                        status.get("raft_committed").unwrap_or("?")
                    ),
                    status.get("bootstrap_state").unwrap_or("").into(),
                    status.get("fatal").unwrap_or("").into(),
                ]);
                nodes.push(json!({
                    "node_id": node,
                    "address": address,
                    "status": status_json(status),
                }));
            }
            Err(error) => {
                partial.push(format!("node {node} at {address}: {error}"));
                rows.push(vec![
                    node.to_string(),
                    address.clone(),
                    "unreachable".into(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    error.clone(),
                ]);
                nodes.push(json!({ "node_id": node, "address": address, "error": error }));
            }
        }
    }
    if only.is_some() && statuses.is_empty() {
        return Err(Fail::Usage(format!(
            "node {} is not in the directory or the endpoints given",
            only.unwrap_or(0)
        )));
    }
    Ok(Report {
        command: "cluster status".into(),
        result: json!(nodes),
        table: table(
            &["NODE", "ADDRESS", "ROLE", "LEADER", "TERM", "APPLIED/COMMITTED", "BOOTSTRAP", "FATAL"],
            &rows,
        ),
        partial,
    })
}

struct Verdict {
    name: &'static str,
    level: &'static str,
    detail: String,
}

async fn cluster_health(cluster: &mut Cluster) -> Outcome {
    let directory = cluster.directory().await?;
    let statuses = cluster.fan_out(None).await;
    let leader_status = statuses
        .iter()
        .find(|(node, _, _)| *node == cluster.leader_id)
        .and_then(|(_, _, r)| r.as_ref().ok())
        .cloned();
    let mut verdicts = Vec::new();

    // Voter health, from the leader's own verdicts.
    let voters = leader_status
        .as_ref()
        .map(|s| s.ids("meta_voters"))
        .unwrap_or_default();
    let replacement: Option<Value> = leader_status
        .as_ref()
        .and_then(|s| s.get("voter_replacement"))
        .and_then(|v| serde_json::from_str(v).ok());
    let (lost, suspected): (Vec<u64>, Vec<u64>) = replacement
        .as_ref()
        .and_then(|r| r["voters"].as_array())
        .map(|rows| {
            let mut lost = Vec::new();
            let mut suspected = Vec::new();
            for row in rows {
                let node = row["node"].as_u64().unwrap_or(0);
                match row["state"].as_str() {
                    Some("lost") => lost.push(node),
                    Some("suspected") => suspected.push(node),
                    _ => {}
                }
            }
            (lost, suspected)
        })
        .unwrap_or_default();
    let majority = voters.len() / 2 + 1;
    let live = voters.len().saturating_sub(lost.len());
    let margin = live as i64 - majority as i64;
    verdicts.push(Verdict {
        name: "quorum_margin",
        level: if voters.is_empty() || margin < 0 {
            "fail"
        } else if margin == 0 {
            "warn"
        } else {
            "ok"
        },
        detail: format!(
            "{} voters, {} live, majority {} (margin {margin})",
            voters.len(),
            live,
            majority
        ),
    });
    verdicts.push(Verdict {
        name: "voter_lost",
        level: if !lost.is_empty() {
            "fail"
        } else if !suspected.is_empty() {
            "warn"
        } else {
            "ok"
        },
        detail: if lost.is_empty() && suspected.is_empty() {
            "every voter active".into()
        } else {
            format!("lost {} suspected {}", ids(&lost), ids(&suspected))
        },
    });
    let auto_replace = replacement
        .as_ref()
        .and_then(|r| r["enabled"].as_bool())
        .unwrap_or(false);
    verdicts.push(Verdict {
        name: "auto_replacement",
        level: "ok",
        detail: if auto_replace { "enabled".into() } else { "disabled (KV9_AUTO_REPLACE_VOTER unset)".into() },
    });

    // Per-node verdicts from the fan-out.
    let mut fatal = Vec::new();
    let mut unreachable = Vec::new();
    let mut lagging = Vec::new();
    let mut control_errors = Vec::new();
    let committed = leader_status
        .as_ref()
        .and_then(|s| s.u64("raft_committed"))
        .unwrap_or(0);
    for (node, address, result) in &statuses {
        match result {
            Ok(status) => {
                if let Some(message) = status.get("fatal").filter(|m| !m.is_empty()) {
                    fatal.push(format!("node {node}: {message}"));
                }
                if let Some(applied) = status.u64("applied_index") {
                    if committed.saturating_sub(applied) > APPLY_LAG_WARNING {
                        lagging.push(format!("node {node}: applied {applied} of {committed}"));
                    }
                }
                if let Some(error) = status
                    .get("data_group_control_error")
                    .filter(|e| !e.is_empty() && *e != "none" && *e != "null")
                {
                    control_errors.push(format!("node {node}: {error}"));
                }
            }
            Err(error) => unreachable.push(format!("node {node} at {address}: {error}")),
        }
    }
    for (name, items, level) in [
        ("fatal", fatal, "fail"),
        ("unreachable", unreachable, "fail"),
        ("apply_lag", lagging, "warn"),
        ("group_control_error", control_errors, "warn"),
    ] {
        verdicts.push(Verdict {
            name,
            level: if items.is_empty() { "ok" } else { level },
            detail: if items.is_empty() { "none".into() } else { items.join("; ") },
        });
    }

    // Admissions about to expire without a join.
    let now = unix_now();
    let expiring: Vec<String> = directory
        .nodes
        .iter()
        .filter(|n| n.admission_state == "pending")
        .filter(|n| n.admission_expires_unix.saturating_sub(now) < ADMISSION_EXPIRY_WARNING_SECS)
        .map(|n| format!("node {} (expires in {} s)", n.node_id, n.admission_expires_unix.saturating_sub(now)))
        .collect();
    verdicts.push(Verdict {
        name: "admissions_expiring",
        level: if expiring.is_empty() { "ok" } else { "warn" },
        detail: if expiring.is_empty() { "none".into() } else { expiring.join("; ") },
    });

    let overall = if verdicts.iter().any(|v| v.level == "fail") {
        "fail"
    } else if verdicts.iter().any(|v| v.level == "warn") {
        "warn"
    } else {
        "ok"
    };
    let rows: Vec<Vec<String>> = verdicts
        .iter()
        .map(|v| vec![v.name.to_string(), v.level.to_string(), v.detail.clone()])
        .collect();
    let mut report = Report {
        command: "cluster health".into(),
        result: json!({
            "overall": overall,
            "verdicts": verdicts.iter().map(|v| json!({"name": v.name, "level": v.level, "detail": v.detail})).collect::<Vec<_>>(),
        }),
        table: format!("overall: {overall}\n{}", table(&["VERDICT", "LEVEL", "DETAIL"], &rows)),
        partial: Vec::new(),
    };
    if overall != "ok" {
        // Health is a verdict, not a transport problem: exit 1 says "not healthy".
        report.partial.clear();
        report.result["exit_hint"] = json!(1);
    }
    Ok(report)
}

fn node_row_json(n: &kv9_client::NodeRow) -> Value {
    json!({
        "node_id": n.node_id,
        "address": n.address,
        "store_incarnation": hex(&n.store_incarnation),
        "endpoint_generation": n.endpoint_generation,
        "active": n.active,
        "last_heartbeat_unix": n.last_heartbeat_unix,
        "raft_role": n.raft_role,
        "matched_index": n.matched_index,
        "recent_active": n.recent_active,
        "admission_state": n.admission_state,
        "admission_expires_unix": n.admission_expires_unix,
    })
}

async fn node_list(cluster: &mut Cluster) -> Outcome {
    let directory = cluster.directory().await?;
    let rows: Vec<Vec<String>> = directory
        .nodes
        .iter()
        .map(|n| {
            vec![
                n.node_id.to_string(),
                n.address.clone(),
                n.raft_role.clone(),
                if n.active { "yes".into() } else { "no".into() },
                n.matched_index.to_string(),
                if n.recent_active { "yes".into() } else { "no".into() },
                n.admission_state.clone(),
            ]
        })
        .collect();
    Ok(Report {
        command: "node list".into(),
        result: json!({
            "leader_node_id": directory.leader_node_id,
            "raft_committed": directory.raft_committed,
            "nodes": directory.nodes.iter().map(node_row_json).collect::<Vec<_>>(),
        }),
        table: format!(
            "leader: node {}  committed: {}\n{}",
            directory.leader_node_id,
            directory.raft_committed,
            table(&["NODE", "ADDRESS", "RAFT", "ACTIVE", "MATCHED", "RECENT", "ADMISSION"], &rows)
        ),
        partial: Vec::new(),
    })
}

async fn node_describe(cluster: &mut Cluster, node_id: u64) -> Outcome {
    let directory = cluster.directory().await?;
    let row = directory
        .nodes
        .iter()
        .find(|n| n.node_id == node_id)
        .ok_or_else(|| Fail::Refused(format!("node {node_id} is not in the directory")))?;
    let statuses = cluster.fan_out(Some(node_id)).await;
    let status = statuses.into_iter().next().map(|(_, _, r)| r);
    let mut partial = Vec::new();
    let mut lines = vec![
        format!("node:        {}", row.node_id),
        format!("address:     {}", row.address),
        format!("raft role:   {}", row.raft_role),
        format!("active:      {}", row.active),
        format!("matched:     {} (leader committed {})", row.matched_index, directory.raft_committed),
        format!("admission:   {}", if row.admission_state.is_empty() { "none" } else { &row.admission_state }),
        format!("incarnation: {}", hex(&row.store_incarnation)),
    ];
    let status_value = match &status {
        Some(Ok(status)) => {
            for key in ["role", "leader_id", "term", "applied_term", "applied_index", "raft_committed", "bootstrap_state", "fatal"] {
                lines.push(format!("{:<13}{}", format!("{key}:"), status.get(key).unwrap_or("")));
            }
            status_json(status)
        }
        Some(Err(error)) => {
            partial.push(format!("node {node_id}: {error}"));
            lines.push(format!("status:      unreachable ({error})"));
            Value::Null
        }
        None => Value::Null,
    };
    Ok(Report {
        command: "node describe".into(),
        result: json!({ "directory": node_row_json(row), "status": status_value }),
        table: lines.join("\n"),
        partial,
    })
}

async fn node_admit(global: &Global, cluster: &mut Cluster, args: &VerbArgs, node_id: u64) -> Outcome {
    let address = args
        .flags
        .get("addr")
        .ok_or_else(|| Fail::Usage("node admit needs --addr <host:port>".into()))?;
    let ttl: u64 = match args.flags.get("ttl") {
        Some(ttl) => ttl.parse().map_err(|_| Fail::Usage("--ttl must be seconds".into()))?,
        None => 600,
    };
    confirm(
        global,
        &format!("plan: AdmitNode node={node_id} addr={address} ttl={ttl}s at leader node {}", cluster.leader_id),
        None,
    )?;
    let change = cluster.leader.admit_node(node_id, address, ttl).await?;
    Ok(membership_report(
        "node admit",
        &change,
        json!({ "node_id": node_id, "address": address, "ttl_seconds": ttl }),
    ))
}

async fn node_membership(
    global: &Global,
    cluster: &mut Cluster,
    verb: &str,
    args: &VerbArgs,
    node_id: u64,
) -> Outcome {
    let wait = args.switches.iter().any(|s| s == "wait");
    let wait_for = match args.flags.get("wait-secs") {
        Some(secs) => Duration::from_secs(secs.parse().map_err(|_| Fail::Usage("--wait-secs must be seconds".into()))?),
        None => DEFAULT_WAIT,
    };
    let leader = cluster.leader_id;
    let change = match verb {
        "revoke" => {
            confirm(global, &format!("plan: RevokeAdmission node={node_id} at leader node {leader}"), None)?;
            cluster.leader.revoke_admission(node_id).await?
        }
        "promote" => {
            confirm(global, &format!("plan: PromoteNode node={node_id} (learner → voter) at leader node {leader}"), None)?;
            cluster.leader.promote_node(node_id).await?
        }
        "remove" => {
            confirm(
                global,
                &format!("plan: RemoveNode node={node_id} (voter → removed) at leader node {leader}; the server refuses below three voters"),
                Some(&node_id.to_string()),
            )?;
            cluster.leader.remove_node(node_id).await?
        }
        _ => unreachable!(),
    };
    let mut report = membership_report(&format!("node {verb}"), &change, json!({ "node_id": node_id }));
    if wait {
        let what = match verb {
            "promote" => format!("node {node_id} among meta_voters"),
            "remove" => format!("node {node_id} gone from meta_voters"),
            _ => format!("node {node_id} gone from pending_admissions"),
        };
        let status = cluster
            .wait_for(&what, wait_for, |status| match verb {
                "promote" => status.ids("meta_voters").contains(&node_id),
                "remove" => !status.ids("meta_voters").contains(&node_id),
                _ => !status.ids("pending_admissions").contains(&node_id),
            })
            .await?;
        report.result["waited"] = json!({
            "meta_voters": status.ids("meta_voters"),
            "meta_learners": status.ids("meta_learners"),
            "conf_index": status.u64("conf_index"),
        });
        report.table.push_str(&format!(
            "\nconfigured: voters={} learners={} (conf_index {})",
            status.get("meta_voters").unwrap_or(""),
            status.get("meta_learners").unwrap_or(""),
            status.get("conf_index").unwrap_or("?")
        ));
    }
    Ok(report)
}

/// `node transfer-leader` (metadata group) and `group transfer-leader` (a data group).
async fn transfer_leader(global: &Global, cluster: &mut Cluster, args: &VerbArgs, region: u64, label: &str) -> Outcome {
    let to = parse_id(
        args.flags.get("to").ok_or_else(|| Fail::Usage(format!("{label} transfer-leader needs --to <node>")))?,
        "--to",
    )?;
    confirm(global, &format!("plan: TransferLeader region={region} to=node {to} (leader-only; nothing is committed)"), None)?;
    let client = if region == kv9_client::META_REGION_ID {
        cluster.leader.clone()
    } else {
        // Ask a replica of the group; the hint leads to the group leader.
        cluster.group_leader_client(region).await?
    };
    let r = client.transfer_leader(region, to).await?;
    let mut report = Report {
        command: format!("{label} transfer-leader"),
        result: json!({
            "region_id": region, "transferee": to, "previous_leader": r.previous_leader,
            "leader": r.leader, "term": r.term, "completed": r.completed,
        }),
        table: if r.completed {
            format!("leadership of region {region} moved from node {} to node {} (term {})", r.previous_leader, r.leader.unwrap_or(0), r.term)
        } else {
            format!("transfer expired: node {} leads region {region} again; retry", r.previous_leader)
        },
        partial: Vec::new(),
    };
    if !r.completed {
        report.result["exit_hint"] = json!(1);
    }
    if args.switches.iter().any(|s| s == "wait") && region == kv9_client::META_REGION_ID {
        let status = cluster
            .wait_for(&format!("node {to} leading the metadata group"), DEFAULT_WAIT, |status| status.u64("leader_id") == Some(to))
            .await?;
        report.result["waited"] = json!({ "leader_id": status.u64("leader_id"), "term": status.u64("term") });
    }
    Ok(report)
}

async fn keyspace_list(cluster: &mut Cluster) -> Outcome {
    let keyspaces = cluster.leader.list_keyspaces().await?;
    let rows: Vec<Vec<String>> = keyspaces
        .iter()
        .map(|k| vec![k.id.to_string(), k.name.clone(), k.tenant_id.to_string()])
        .collect();
    Ok(Report {
        command: "keyspace list".into(),
        result: json!(keyspaces
            .iter()
            .map(|k| json!({"id": k.id, "name": k.name, "tenant_id": k.tenant_id}))
            .collect::<Vec<_>>()),
        table: table(&["ID", "NAME", "TENANT"], &rows),
        partial: Vec::new(),
    })
}

async fn keyspace_describe(cluster: &mut Cluster, which: &str) -> Outcome {
    let keyspaces = cluster.leader.list_keyspaces().await?;
    // Name first; a bare number that names no keyspace is an id.
    let keyspace = keyspaces
        .iter()
        .find(|k| k.name == which)
        .or_else(|| keyspaces.iter().find(|k| k.id.to_string() == which))
        .ok_or_else(|| Fail::Refused(format!("keyspace {which:?} not found")))?;
    let region = cluster.leader.get_region(keyspace.id, b"").await?;
    Ok(Report {
        command: "keyspace describe".into(),
        result: json!({
            "id": keyspace.id, "name": keyspace.name, "tenant_id": keyspace.tenant_id,
            "region": { "region_id": region.region_id, "epoch": { "conf_ver": region.epoch.conf_ver, "version": region.epoch.version }, "leader_node_id": region.leader_node_id },
        }),
        table: format!(
            "id:       {}\nname:     {}\ntenant:   {}\nregion:   {} (epoch {}/{}, leader {})",
            keyspace.id,
            keyspace.name,
            keyspace.tenant_id,
            region.region_id,
            region.epoch.conf_ver,
            region.epoch.version,
            region.leader_node_id.map_or("unknown".to_string(), |l| l.to_string())
        ),
        partial: Vec::new(),
    })
}

async fn keyspace_create(global: &Global, cluster: &mut Cluster, args: &VerbArgs) -> Outcome {
    let name = args
        .flags
        .get("name")
        .ok_or_else(|| Fail::Usage("keyspace create needs --name".into()))?;
    let tenant: u64 = match args.flags.get("tenant") {
        Some(t) => t.parse().map_err(|_| Fail::Usage("--tenant must be a number".into()))?,
        None => 0,
    };
    if let Some(group) = args.flags.get("group") {
        let creation_task = parse_id(group, "--group")?;
        let root = cluster.root_digest().await?;
        confirm(
            global,
            &format!("plan: CreateDataKeyspace name={name} tenant={tenant} on data group {creation_task} (root {})", hex(&root)),
            None,
        )?;
        let created = cluster
            .leader
            .create_data_keyspace(root, creation_task, name, tenant)
            .await?;
        return Ok(Report {
            command: "keyspace create".into(),
            result: json!({
                "id": created.keyspace_id, "name": name, "tenant_id": tenant,
                "creation_task": creation_task, "region_id": created.region_id,
                "outcome": if created.changed { "created" } else { "confirmed" },
                "applied": { "term": created.applied.term, "index": created.applied.index },
            }),
            table: format!(
                "keyspace_id={} region_id={} ({})",
                created.keyspace_id,
                created.region_id,
                if created.changed { "created" } else { "confirmed" }
            ),
            partial: Vec::new(),
        });
    }
    confirm(global, &format!("plan: CreateKeyspace name={name} tenant={tenant} (raw, served by the metadata group)"), None)?;
    let id = cluster.leader.create_keyspace(name, tenant).await?;
    Ok(Report {
        command: "keyspace create".into(),
        result: json!({ "id": id, "name": name, "tenant_id": tenant }),
        table: format!("keyspace_id={id}"),
        partial: Vec::new(),
    })
}

// ------------------------------------------------------------------ data groups

fn parse_hex(text: &str, what: &str, len: usize) -> Result<Vec<u8>, Fail> {
    let text = text.trim();
    if text.len() != len * 2 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Fail::Usage(format!("{what} must be {len} bytes of hex")));
    }
    Ok((0..len)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).expect("hex digits"))
        .collect())
}

/// `sha16(domain ++ root ++ subject)`: the derived retry identity of docs/KV9-ADMIN.md §3.
fn derived_operation(domain: &str, root: &[u8; 32], subject: &[u8]) -> [u8; 16] {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update(root);
    hasher.update(subject);
    let digest = hasher.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

fn group_json(g: &DataGroup) -> Value {
    json!({
        "creation_task": g.creation_task,
        "region_id": g.region_id,
        "operation": hex(&g.operation),
        "activated": g.activated,
        "replicas": g.replicas.iter().map(|r| json!({"node_id": r.node_id, "store_incarnation": hex(&r.store_incarnation)})).collect::<Vec<_>>(),
        "members": g.current_members().iter().map(|r| json!({"node_id": r.node_id, "store_incarnation": hex(&r.store_incarnation)})).collect::<Vec<_>>(),
        "keyspaces": g.keyspaces.iter().map(|k| json!({
            "keyspace_id": k.keyspace_id, "tenant_id": k.tenant_id, "bind_task": k.bind_task,
            "epoch": {"conf_ver": k.conf_ver, "version": k.version},
            "start": hex(&k.start), "end": hex(&k.end), "sealed": k.sealed,
        })).collect::<Vec<_>>(),
        "migrations": g.migrations.iter().map(|m| json!({
            "task": m.task, "operation": hex(&m.operation),
            "destination": {"node_id": m.destination.node_id, "store_incarnation": hex(&m.destination.store_incarnation)},
            "evidence_task": m.evidence_task, "abort_task": m.abort_task,
            "removal_task": m.removal_task, "truncation_task": m.truncation_task,
        })).collect::<Vec<_>>(),
        "compaction_floor": g.compaction_floor.map(|f| json!({"term": f.term, "index": f.index})),
        "plans": g.plans.iter().map(|p| json!({
            "task": p.task, "operation": hex(&p.operation), "migration_task": p.migration_task,
            "mode": p.mode, "stage": p.stage,
            "destination": {"node_id": p.destination.node_id, "store_incarnation": hex(&p.destination.store_incarnation)},
            "source": p.source.as_ref().map(|s| json!({"node_id": s.node_id, "store_incarnation": hex(&s.store_incarnation)})),
        })).collect::<Vec<_>>(),
    })
}

/// The 16-byte operation id of a plan: `--operation`, derived from `--label`, or minted.
fn operation_for(args: &VerbArgs, root: &[u8; 32], domain: &str, subject: &str) -> Result<([u8; 16], &'static str), Fail> {
    Ok(match (args.flags.get("operation"), args.flags.get("label")) {
        (Some(hex_text), _) => {
            let bytes = parse_hex(hex_text, "--operation", 16)?;
            (bytes.try_into().expect("16 bytes"), "given")
        }
        (None, Some(label)) => (
            derived_operation(domain, root, format!("{subject}/{label}").as_bytes()),
            "derived from --label",
        ),
        (None, None) => {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            (
                derived_operation(&format!("{domain}/random"), root, &nanos.to_be_bytes()),
                "random (pass it back as --operation to retry)",
            )
        }
    })
}

/// `group add-replica` / `group move-replica`: one committed plan; the cluster drives
/// the chain (docs/REPLICA-ADDITION.md). Watch it with `group describe`.
async fn group_plan_replica(global: &Global, cluster: &mut Cluster, args: &VerbArgs, creation_task: u64, mode: &str) -> Outcome {
    let to = parse_id(
        args.flags.get("to").ok_or_else(|| Fail::Usage(format!("group {mode}-replica <creation-task> --to <node>")))?,
        "--to",
    )?;
    let from = match (mode, args.flags.get("from")) {
        ("move", Some(from)) => Some(parse_id(from, "--from")?),
        ("move", None) => return Err(Fail::Usage("group move-replica needs --from <node>".into())),
        (_, Some(_)) => return Err(Fail::Usage("group add-replica takes no --from".into())),
        _ => None,
    };
    let root = cluster.root_digest().await?;
    let subject = format!("{creation_task}/{to}/{}", from.unwrap_or(0));
    let (operation, origin) = operation_for(args, &root, "kv9-admin/replica-plan/v1", &subject)?;
    confirm(
        global,
        &format!(
            "plan: AddReplica group={creation_task} mode={mode} to=node {to}{} operation={} ({origin}); the cluster attaches, captures, installs, proves and promotes by itself",
            from.map_or(String::new(), |f| format!(" from=node {f}")),
            hex(&operation)
        ),
        None,
    )?;
    let planned = cluster
        .leader
        .add_replica(root, operation, creation_task, to, mode, from)
        .await?;
    let outcome = if planned.changed { "planned" } else { "confirmed" };
    Ok(Report {
        command: format!("group {mode}-replica"),
        result: json!({
            "plan_task": planned.plan_task, "migration_task": planned.migration_task,
            "region_id": planned.region_id, "operation": hex(&operation), "operation_origin": origin,
            "mode": mode, "destination": to, "source": from, "outcome": outcome,
            "applied": {"term": planned.applied.term, "index": planned.applied.index},
        }),
        table: format!(
            "plan_task={} migration_task={} region_id={} operation={} ({outcome})\nwatch: kv9-admin group describe {creation_task}",
            planned.plan_task, planned.migration_task, planned.region_id, hex(&operation)
        ),
        partial: Vec::new(),
    })
}

async fn group_list(cluster: &mut Cluster) -> Outcome {
    let groups = cluster.leader.list_data_groups().await?;
    let rows: Vec<Vec<String>> = groups
        .iter()
        .map(|g| {
            vec![
                g.creation_task.to_string(),
                g.region_id.to_string(),
                ids(&g.current_members().iter().map(|r| r.node_id).collect::<Vec<_>>()),
                if g.activated { "yes".into() } else { "no".into() },
                ids(&g.keyspaces.iter().map(|k| k.keyspace_id as u64).collect::<Vec<_>>()),
                g.migrations.len().to_string(),
                g.compaction_floor.map_or("-".into(), |f| format!("{}/{}", f.term, f.index)),
                g.plans
                    .iter()
                    .map(|p| format!("{}→{}:{}", p.mode, p.destination.node_id, p.stage))
                    .collect::<Vec<_>>()
                    .join(" "),
            ]
        })
        .collect();
    Ok(Report {
        command: "group list".into(),
        result: json!(groups.iter().map(group_json).collect::<Vec<_>>()),
        table: table(&["TASK", "REGION", "MEMBERS", "ACTIVATED", "KEYSPACES", "MIGRATIONS", "COMPACTED", "PLANS"], &rows),
        partial: Vec::new(),
    })
}

async fn group_describe(cluster: &mut Cluster, creation_task: u64) -> Outcome {
    let groups = cluster.leader.list_data_groups().await?;
    let group = groups
        .iter()
        .find(|g| g.creation_task == creation_task)
        .ok_or_else(|| Fail::Refused(format!("data group {creation_task} is not committed")))?;
    let _ = cluster.directory().await; // replica addresses
    let keyspaces = cluster.leader.list_keyspaces().await.unwrap_or_default();
    let mut lines = vec![
        format!("creation task: {}", group.creation_task),
        format!("region:        {}", group.region_id),
        format!("operation:     {}", hex(&group.operation)),
        format!("activated:     {}", group.activated),
    ];
    for k in &group.keyspaces {
        let name = keyspaces
            .iter()
            .find(|ks| ks.id == k.keyspace_id)
            .map(|ks| ks.name.clone())
            .unwrap_or_default();
        lines.push(format!(
            "keyspace:      {} {} tenant {} epoch {}/{} bound by task {}{}",
            k.keyspace_id, name, k.tenant_id, k.conf_ver, k.version, k.bind_task,
            if k.sealed { " (sealed)" } else { "" }
        ));
    }
    for m in &group.migrations {
        lines.push(format!(
            "migration:     task {} → node {} evidence {} removal {} truncation {} abort {}",
            m.task, m.destination.node_id, m.evidence_task, m.removal_task, m.truncation_task, m.abort_task
        ));
    }
    if let Some(f) = group.compaction_floor {
        lines.push(format!("compacted:     through term {} index {}", f.term, f.index));
    }
    for p in &group.plans {
        lines.push(format!(
            "plan:          task {} {} → node {}{} stage {} (operation {})",
            p.task,
            p.mode,
            p.destination.node_id,
            p.source.as_ref().map_or(String::new(), |s| format!(" retiring node {}", s.node_id)),
            p.stage,
            hex(&p.operation)
        ));
    }
    // Every replica's own view of the group, from its status document.
    let mut partial = Vec::new();
    let mut replicas = Vec::new();
    let mut rows = Vec::new();
    // The live view comes from the committed members of today, not the
    // creation's initial replicas: after a move the leader may be a destination.
    for replica in group.current_members() {
        let address = cluster.peers.get(&replica.node_id).cloned();
        let observed = match &address {
            Some(address) => match cluster.client_at(address) {
                Ok(client) => client.node_status().await.map_err(|e| e.to_string()),
                Err(e) => Err(e.message().to_string()),
            },
            None => Err("address unknown".to_string()),
        };
        let view = observed.as_ref().ok().and_then(|status| {
            status
                .get("data_groups")
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .and_then(|groups| {
                    groups
                        .as_array()?
                        .iter()
                        .find(|g| g["region"].as_u64() == Some(group.region_id))
                        .cloned()
                })
        });
        let cell = |key: &str| view.as_ref().map_or(String::new(), |v| match &v[key] {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        });
        match (&observed, &view) {
            (Ok(_), Some(_)) => rows.push(vec![
                replica.node_id.to_string(),
                address.clone().unwrap_or_default(),
                cell("state"),
                cell("role"),
                cell("term"),
                cell("leader"),
                cell("committed"),
                cell("engine_applied"),
            ]),
            (Ok(_), None) => rows.push(vec![
                replica.node_id.to_string(),
                address.clone().unwrap_or_default(),
                "not local yet".into(),
                String::new(), String::new(), String::new(), String::new(), String::new(),
            ]),
            (Err(error), _) => {
                partial.push(format!("node {}: {error}", replica.node_id));
                rows.push(vec![
                    replica.node_id.to_string(),
                    address.clone().unwrap_or_default(),
                    "unreachable".into(),
                    String::new(), String::new(), String::new(), String::new(), error.clone(),
                ]);
            }
        }
        replicas.push(json!({
            "node_id": replica.node_id,
            "store_incarnation": hex(&replica.store_incarnation),
            "address": address,
            "observed": view,
            "error": observed.as_ref().err(),
        }));
    }
    lines.push(String::new());
    lines.push(table(&["REPLICA", "ADDRESS", "STATE", "ROLE", "TERM", "LEADER", "COMMITTED", "APPLIED"], &rows));
    let mut result = group_json(group);
    result["replica_views"] = json!(replicas);
    Ok(Report {
        command: "group describe".into(),
        result,
        table: lines.join("\n"),
        partial,
    })
}

async fn group_create(global: &Global, cluster: &mut Cluster, args: &VerbArgs) -> Outcome {
    let voters: Vec<u64> = args
        .flags
        .get("voters")
        .ok_or_else(|| Fail::Usage("group create needs --voters <id,id,id>".into()))?
        .split(',')
        .map(|v| parse_id(v.trim(), "voter id"))
        .collect::<Result<_, _>>()?;
    if ![3, 5, 7].contains(&voters.len()) {
        return Err(Fail::Usage("a data group has exactly 3, 5 or 7 voters".into()));
    }
    let root = cluster.root_digest().await?;
    let (operation, origin) = operation_for(args, &root, "kv9-admin/group-create/v1", "")?;
    confirm(
        global,
        &format!(
            "plan: CreateDataGroup voters={} operation={} ({origin}) root={}",
            ids(&voters),
            hex(&operation),
            hex(&root)
        ),
        None,
    )?;
    let created = cluster.leader.create_data_group(root, operation, voters.clone()).await?;
    let outcome = if created.changed { "created" } else { "confirmed" };
    Ok(Report {
        command: "group create".into(),
        result: json!({
            "creation_task": created.creation_task,
            "region_id": created.region_id,
            "operation": hex(&operation),
            "operation_origin": origin,
            "voters": voters,
            "outcome": outcome,
            "applied": { "term": created.applied.term, "index": created.applied.index },
        }),
        table: format!(
            "creation_task={} region_id={} operation={} ({outcome}; applied term={} index={})\nreadiness is not asserted: watch `group describe {}` until every replica is active",
            created.creation_task,
            created.region_id,
            hex(&operation),
            created.applied.term,
            created.applied.index,
            created.creation_task
        ),
        partial: Vec::new(),
    })
}

fn task_row_json(t: &kv9_client::TaskRow) -> Value {
    json!({
        "id": t.id, "kind": t.kind, "kind_name": t.kind_name, "state": t.state,
        "created_unix": t.created_unix, "payload_bytes": t.payload_bytes,
        "operation": hex(&t.operation), "reference_task": t.reference_task,
    })
}

async fn task_list(cluster: &mut Cluster, args: &VerbArgs) -> Outcome {
    let after = match args.flags.get("after") {
        Some(a) => parse_id(a, "--after")?,
        None => 0,
    };
    let limit: u32 = match args.flags.get("limit") {
        Some(l) => l.parse().ok().filter(|l| (1..=256).contains(l)).ok_or_else(|| Fail::Usage("--limit must be 1..=256".into()))?,
        None => 64,
    };
    let kind = match args.flags.get("kind") {
        Some(k) => k.parse().map_err(|_| Fail::Usage("--kind must be a number".into()))?,
        None => 0,
    };
    let page = cluster.leader.list_tasks(after, limit, kind).await?;
    let rows: Vec<Vec<String>> = page
        .tasks
        .iter()
        .map(|t| {
            vec![
                t.id.to_string(),
                format!("{} {}", t.kind, t.kind_name),
                if t.reference_task == 0 { "-".into() } else { t.reference_task.to_string() },
                if t.operation.is_empty() { "-".into() } else { hex(&t.operation) },
                t.created_unix.to_string(),
            ]
        })
        .collect();
    let mut table_text = table(&["TASK", "KIND", "REFERS_TO", "OPERATION", "CREATED"], &rows);
    if page.next_after != 0 {
        table_text.push_str(&format!("\n(more: --after {})", page.next_after));
    }
    Ok(Report {
        command: "task list".into(),
        result: json!({ "tasks": page.tasks.iter().map(task_row_json).collect::<Vec<_>>(), "next_after": page.next_after }),
        table: table_text,
        partial: Vec::new(),
    })
}

async fn task_describe(cluster: &mut Cluster, id: u64) -> Outcome {
    let page = cluster.leader.list_tasks(id - 1, 1, 0).await?;
    let task = page
        .tasks
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| Fail::Refused(format!("task {id} is not in the history")))?;
    Ok(Report {
        command: "task describe".into(),
        table: format!(
            "task:       {}\nkind:       {} ({})\nrefers to:  {}\noperation:  {}\nstate:      {}\ncreated:    {}\npayload:    {} bytes",
            task.id,
            task.kind,
            task.kind_name,
            if task.reference_task == 0 { "-".into() } else { task.reference_task.to_string() },
            if task.operation.is_empty() { "-".into() } else { hex(&task.operation) },
            task.state,
            task.created_unix,
            task.payload_bytes
        ),
        result: task_row_json(&task),
        partial: Vec::new(),
    })
}

async fn dispatch(global: &Global, words: &[String]) -> Outcome {
    let (noun, verb, rest) = match words {
        [noun, verb, rest @ ..] => (noun.as_str(), verb.as_str(), rest),
        [noun] if noun == "version" => {
            return Ok(Report {
                command: "version".into(),
                result: json!({ "kv9_admin": env!("CARGO_PKG_VERSION") }),
                table: format!("kv9-admin {}", env!("CARGO_PKG_VERSION")),
                partial: Vec::new(),
            })
        }
        _ => return Err(Fail::Usage(usage())),
    };
    let args = verb_args(rest, &["wait"])?;
    let mut cluster = Cluster::connect(global).await?;
    let mut report = match (noun, verb) {
        ("cluster", "info") => cluster_info(&mut cluster).await?,
        ("cluster", "endpoints") => cluster_endpoints(&mut cluster).await?,
        ("cluster", "status") => {
            let only = args.flags.get("node").map(|n| parse_id(n, "--node")).transpose()?;
            cluster_status(&mut cluster, only).await?
        }
        ("cluster", "health") => cluster_health(&mut cluster).await?,
        ("node", "list") => node_list(&mut cluster).await?,
        ("node", "describe") => {
            let id = parse_id(args.positional.first().ok_or_else(|| Fail::Usage("node describe <id>".into()))?, "node id")?;
            node_describe(&mut cluster, id).await?
        }
        ("node", "admit") => {
            let id = parse_id(args.positional.first().ok_or_else(|| Fail::Usage("node admit <id> --addr <host:port>".into()))?, "node id")?;
            node_admit(global, &mut cluster, &args, id).await?
        }
        ("node", "transfer-leader") => transfer_leader(global, &mut cluster, &args, kv9_client::META_REGION_ID, "node").await?,
        ("group", "transfer-leader") => {
            let id = parse_id(args.positional.first().ok_or_else(|| Fail::Usage("group transfer-leader <creation-task> --to <node>".into()))?, "creation task")?;
            let groups = cluster.leader.list_data_groups().await?;
            let region = groups
                .iter()
                .find(|g| g.creation_task == id)
                .map(|g| g.region_id)
                .ok_or_else(|| Fail::Refused(format!("data group {id} is not committed")))?;
            transfer_leader(global, &mut cluster, &args, region, "group").await?
        }
        ("node", verb @ ("revoke" | "promote" | "remove")) => {
            let id = parse_id(args.positional.first().ok_or_else(|| Fail::Usage(format!("node {verb} <id>")))?, "node id")?;
            node_membership(global, &mut cluster, verb, &args, id).await?
        }
        ("keyspace", "list") => keyspace_list(&mut cluster).await?,
        ("keyspace", "describe") => {
            let which = args.positional.first().ok_or_else(|| Fail::Usage("keyspace describe <id|name>".into()))?;
            keyspace_describe(&mut cluster, which).await?
        }
        ("keyspace", "create") => keyspace_create(global, &mut cluster, &args).await?,
        ("group", "list") => group_list(&mut cluster).await?,
        ("group", "describe") => {
            let id = parse_id(args.positional.first().ok_or_else(|| Fail::Usage("group describe <creation-task>".into()))?, "creation task")?;
            group_describe(&mut cluster, id).await?
        }
        ("group", "create") => group_create(global, &mut cluster, &args).await?,
        ("group", verb @ ("add-replica" | "move-replica")) => {
            let id = parse_id(args.positional.first().ok_or_else(|| Fail::Usage(format!("group {verb} <creation-task> --to <node>")))?, "creation task")?;
            group_plan_replica(global, &mut cluster, &args, id, verb.trim_end_matches("-replica")).await?
        }
        ("task", "list") => task_list(&mut cluster, &args).await?,
        ("task", "describe") => {
            let id = parse_id(args.positional.first().ok_or_else(|| Fail::Usage("task describe <id>".into()))?, "task id")?;
            task_describe(&mut cluster, id).await?
        }
        _ => return Err(Fail::Usage(format!("unknown command {noun} {verb}\n\n{}", usage()))),
    };
    report.result = json!({
        "schema_version": SCHEMA_VERSION,
        "command": report.command,
        "resolved_leader": cluster.resolved(),
        "result": report.result,
        "partial": report.partial,
    });
    if cluster.verbose && !cluster.unreachable.is_empty() {
        eprintln!("unreachable while resolving: {}", cluster.unreachable.join("; "));
    }
    Ok(report)
}

#[tokio::main]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut args = match parse_args(argv) {
        Ok(args) => args,
        Err(fail) => return finish(Output::Table, Err(fail)),
    };
    if let Err(fail) = resolve_context(&mut args.global) {
        return finish(args.global.output, Err(fail));
    }
    let output = args.global.output;
    let outcome = dispatch(&args.global, &args.words).await;
    finish(output, outcome)
}

fn finish(output: Output, outcome: Outcome) -> ExitCode {
    match outcome {
        Ok(report) => {
            match output {
                Output::Json => println!("{}", serde_json::to_string_pretty(&report.result).expect("json")),
                Output::Table => {
                    println!("{}", report.table);
                    for problem in &report.partial {
                        eprintln!("warning: {problem}");
                    }
                }
            }
            if !report.partial.is_empty() {
                return ExitCode::from(4);
            }
            if report.result["result"]["exit_hint"] == json!(1) {
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Err(fail) => {
            match output {
                Output::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "schema_version": SCHEMA_VERSION,
                        "error": { "kind": fail.kind(), "message": fail.message() },
                    }))
                    .expect("json")
                ),
                Output::Table => eprintln!("{}", fail.message()),
            }
            ExitCode::from(fail.code())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_accept_bare_and_id_prefixed_addresses_and_refuse_the_rest() {
        let parsed = parse_endpoints("1=127.0.0.1:20161, 127.0.0.1:20162").unwrap();
        assert_eq!(
            parsed,
            vec![
                (Some(1), "127.0.0.1:20161".to_string()),
                (None, "127.0.0.1:20162".to_string())
            ]
        );
        assert!(matches!(parse_endpoints("x=127.0.0.1:1"), Err(Fail::Usage(_))));
        assert!(matches!(parse_endpoints("nohostport"), Err(Fail::Usage(_))));
        assert!(matches!(parse_endpoints(""), Err(Fail::Usage(_))));
    }

    #[test]
    fn the_contexts_file_subset_parses_and_rejects_what_it_does_not_understand() {
        let dir = std::env::temp_dir().join(format!("kv9-admin-ctx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("admin.toml");
        std::fs::write(
            &path,
            "default = \"local\"\n# a comment\n[context.local]\nendpoints = [\"1=127.0.0.1:20161\", \"127.0.0.1:20162\"]\ntoken_env = \"KV9_ADMIN_TOKEN\"\ntimeout_secs = 7\n[context.prod]\nendpoints = [\"10.0.0.1:20161\"]\n",
        )
        .unwrap();
        let (default, contexts) = load_contexts(&path).unwrap();
        assert_eq!(default.as_deref(), Some("local"));
        assert_eq!(contexts.len(), 2);
        let local = &contexts["local"];
        assert_eq!(local.endpoints.len(), 2);
        assert_eq!(local.token_env.as_deref(), Some("KV9_ADMIN_TOKEN"));
        assert_eq!(local.timeout, Some(Duration::from_secs(7)));
        assert!(contexts["prod"].token_env.is_none());

        std::fs::write(&path, "[other.x]\nendpoints = [\"a:1\"]\n").unwrap();
        assert!(matches!(load_contexts(&path), Err(Fail::Usage(_))));
        std::fs::write(&path, "[context.x]\nunknown = 1\n").unwrap();
        assert!(matches!(load_contexts(&path), Err(Fail::Usage(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sdk_errors_map_to_the_exit_code_table() {
        let refused = Fail::from(Error::NotLeader { leader: Some(2) });
        assert_eq!((refused.code(), refused.kind()), (1, "refused"));
        let denied = Fail::from(Error::Rpc {
            code: kv9_client::Code::PermissionDenied,
            message: "no".into(),
        });
        assert_eq!(denied.code(), 1);
        let unavailable = Fail::from(Error::Rpc {
            code: kv9_client::Code::Unavailable,
            message: "down".into(),
        });
        assert_eq!((unavailable.code(), unavailable.kind()), (3, "transport"));
        assert_eq!(Fail::from(Error::Transport("dial".into())).code(), 3);
        assert_eq!(Fail::from(Error::Bounds("key")).code(), 2);
        assert_eq!(Fail::WaitDeadline("x".into()).code(), 6);
    }

    #[test]
    fn tables_pad_columns_and_status_documents_keep_json_lines_structured() {
        let rendered = table(
            &["A", "BB"],
            &[vec!["1".into(), "x".into()], vec!["22".into(), "y".into()]],
        );
        assert_eq!(rendered, "A   BB\n1   x\n22  y");
        let status = NodeStatus {
            fields: vec![
                ("role".into(), "leader".into()),
                ("meta_voters".into(), "1,2,3".into()),
                ("voter_replacement".into(), "{\"enabled\":true,\"voters\":[]}".into()),
            ],
            peers: Vec::new(),
        };
        assert_eq!(status.ids("meta_voters"), vec![1, 2, 3]);
        let json = status_json(&status);
        assert_eq!(json["role"], "leader");
        assert_eq!(json["voter_replacement"]["enabled"], true);
    }
}
