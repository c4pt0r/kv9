//! kv9 — the single binary (DESIGN §1 goal 2, §11).
//!
//! One `kv9` executable is *every* role: storage node, metadata member, request router.
//! This entry point parses the node CLI (`--node-id`, `--join`, `--data-dir`, `--addr`)
//! and wires a [`kv9_server::NodeRuntime`]. Txn groups are deliberately NOT a CLI flag:
//! a txn group is a TSO shard *inside a keyspace* (DESIGN §3.6 "Txn group (a TSO
//! shard inside a keyspace)"), declared per keyspace at
//! `CREATE KEYSPACE ... [, txn_group = <g>]` (§3.2) and stored in the `txn_groups`
//! catalog table keyed by `keyspace_id`.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::process::ExitCode;

use kv9_common::{ApiType, ClusterId, Config, NodeId, SeedPeer};
use kv9_server::{
    admit_node_blocking, create_keyspace_blocking, promote_node_blocking, NodeRuntime, RuntimeAuth,
};

/// Parsed CLI arguments (DESIGN §11).
#[derive(Debug, Default)]
struct Cli {
    node_id: Option<NodeId>,
    addr: Option<String>,
    data_dir: Option<String>,
    join: Vec<SeedPeer>,
    cluster_id: Option<ClusterId>,
}

fn print_usage() {
    eprintln!(
        "kv9 — single-binary distributed KV\n\
         \n\
         USAGE:\n\
           KV9_CLUSTER_TOKEN=<token> KV9_CLIENT_TOKENS=<principal=token,...> kv9 --node-id <id> [--addr <ip:port>] [--data-dir <path>] [--join <id@ip:port>[,<id@ip:port>...]] [--cluster-id <32-hex>]\n\
           KV9_CLIENT_TOKEN=<token> kv9 client create-keyspace --addr <ip:port> --name <name> --api-type <txn|raw> [--tenant-id <id>]\n\
           KV9_CLIENT_TOKEN=<token> kv9 client admit-node --addr <leader-ip:port> --node-id <id> --node-addr <ip:port> [--ttl-seconds <seconds>]\n\
           KV9_CLIENT_TOKEN=<token> kv9 client promote-node --addr <leader-ip:port> --node-id <id>\n\
         \n\
         FLAGS (DESIGN §11):\n\
           --node-id     non-zero stable identity of this node (required)\n\
           --addr        serving address to bind (default 127.0.0.1:20160)\n\
           --data-dir    local data directory (default ./kv9-data)\n\
           --join        fixed seed voters as comma-separated node-id@ip:port pairs\n\
           --cluster-id  required only when this node is not one of the seed voters\n\
           -h, --help    print this help"
    );
}

fn parse_cli(args: impl Iterator<Item = String>) -> std::result::Result<Cli, String> {
    let mut cli = Cli::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--node-id" => {
                let value = args.next().ok_or("--node-id needs a value")?;
                let id = value
                    .parse::<u64>()
                    .map_err(|_| "--node-id must be a non-zero integer".to_string())?;
                if id == 0 {
                    return Err("--node-id must be a non-zero integer".to_string());
                }
                cli.node_id = Some(NodeId(id));
            }
            "--addr" => cli.addr = Some(args.next().ok_or("--addr needs a value")?),
            "--data-dir" => cli.data_dir = Some(args.next().ok_or("--data-dir needs a value")?),
            "--join" => {
                let v = args.next().ok_or("--join needs a value")?;
                cli.join = v
                    .split(',')
                    .filter(|s| !s.trim().is_empty())
                    .map(parse_seed_peer)
                    .collect::<std::result::Result<Vec<_>, _>>()?;
            }
            "--cluster-id" => {
                let value = args.next().ok_or("--cluster-id needs a value")?;
                cli.cluster_id = Some(
                    value
                        .parse::<ClusterId>()
                        .map_err(|error| error.to_string())?,
                );
            }
            "-h" | "--help" => return Err("help".to_string()),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if cli.node_id.is_none() {
        return Err("--node-id is required".to_string());
    }
    validate_declared_seed_set(&cli)?;
    Ok(cli)
}

fn parse_seed_peer(raw: &str) -> std::result::Result<SeedPeer, String> {
    let (id, addr) = raw
        .trim()
        .split_once('@')
        .ok_or_else(|| format!("invalid --join peer '{raw}': expected node-id@ip:port"))?;
    let node_id = id
        .parse::<u64>()
        .map_err(|_| format!("invalid --join peer '{raw}': node id must be non-zero"))?;
    if node_id == 0 {
        return Err(format!(
            "invalid --join peer '{raw}': node id must be non-zero"
        ));
    }
    let addr = addr
        .parse::<SocketAddr>()
        .map_err(|_| format!("invalid --join peer '{raw}': address must be ip:port"))?;
    Ok(SeedPeer {
        node_id: NodeId(node_id),
        addr,
    })
}

fn validate_declared_seed_set(cli: &Cli) -> std::result::Result<(), String> {
    if cli.join.is_empty() {
        return Ok(());
    }
    let mut ids = HashSet::new();
    let mut addrs = HashSet::new();
    for seed in &cli.join {
        if !ids.insert(seed.node_id) {
            return Err(format!("duplicate --join node id {}", seed.node_id.0));
        }
        if !addrs.insert(seed.addr) {
            return Err(format!("duplicate --join address {}", seed.addr));
        }
    }
    let node_id = cli.node_id.expect("node id checked before seed set");
    let own_addr = cli
        .addr
        .as_deref()
        .unwrap_or("127.0.0.1:20160")
        .parse::<SocketAddr>()
        .map_err(|_| "--addr must be ip:port".to_string())?;
    match cli.join.iter().find(|seed| seed.node_id == node_id) {
        Some(seed) if seed.addr == own_addr && cli.cluster_id.is_none() => Ok(()),
        Some(_) if cli.cluster_id.is_some() => Err(
            "--cluster-id is only valid when this node is absent from --join (join-existing mode)"
                .to_string(),
        ),
        Some(seed) => Err(format!(
            "--join declares this node {} at {}, but --addr is {}",
            node_id.0, seed.addr, own_addr
        )),
        None if cli.cluster_id.is_some() => Ok(()),
        None => Err(format!(
            "node {} is absent from --join; join-existing mode requires --cluster-id",
            node_id.0
        )),
    }
}

fn config_from_cli(cli: Cli) -> Config {
    let mut cfg = Config::default();
    if let Some(a) = cli.addr {
        cfg.addr = a;
    }
    if let Some(d) = cli.data_dir {
        cfg.data_dir = d;
    }
    cfg.join = cli.join;
    cfg
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "client")
    {
        return run_client(arguments.into_iter().skip(1));
    }
    let cli = match parse_cli(arguments.into_iter()) {
        Ok(cli) => cli,
        Err(e) => {
            if e == "help" {
                print_usage();
                return ExitCode::SUCCESS;
            }
            eprintln!("error: {e}\n");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let node_id = cli.node_id.expect("parse_cli enforces node id");
    let expected_cluster_id = cli.cluster_id;
    let auth = RuntimeAuth {
        cluster_token: std::env::var("KV9_CLUSTER_TOKEN").unwrap_or_default(),
        client_tokens: parse_client_tokens(&std::env::var("KV9_CLIENT_TOKENS").unwrap_or_default()),
    };
    let config = config_from_cli(cli);
    let runtime = match NodeRuntime::start_with_cluster(node_id, config, auth, expected_cluster_id)
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("failed to start node: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "kv9 node {} running; machine-readable status: {}",
        node_id.0,
        runtime.status_path().display()
    );
    if let Err(e) = runtime.run() {
        eprintln!("node runtime failed: {e}");
        return ExitCode::FAILURE;
    }
    unreachable!("node runtime returns only on failure")
}

fn run_client(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next().as_deref() {
        Some("create-keyspace") => run_create_keyspace(args),
        Some("admit-node") => run_admit_node(args),
        Some("promote-node") => run_promote_node(args),
        _ => {
            eprintln!("error: client command must be create-keyspace, admit-node, or promote-node");
            ExitCode::FAILURE
        }
    }
}

fn run_create_keyspace(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut addr = None;
    let mut name = None;
    let mut api_type = None;
    let mut tenant_id = 0u64;
    while let Some(flag) = args.next() {
        let value = match args.next() {
            Some(value) => value,
            None => {
                eprintln!("error: {flag} needs a value");
                return ExitCode::FAILURE;
            }
        };
        match flag.as_str() {
            "--addr" => addr = Some(value),
            "--name" => name = Some(value),
            "--api-type" => {
                api_type = match value.as_str() {
                    "txn" => Some(ApiType::Txn),
                    "raw" => Some(ApiType::Raw),
                    _ => {
                        eprintln!("error: --api-type must be txn or raw");
                        return ExitCode::FAILURE;
                    }
                };
            }
            "--tenant-id" => match value.parse() {
                Ok(value) => tenant_id = value,
                Err(_) => {
                    eprintln!("error: --tenant-id must be an integer");
                    return ExitCode::FAILURE;
                }
            },
            _ => {
                eprintln!("error: unknown client flag {flag}");
                return ExitCode::FAILURE;
            }
        }
    }
    let Some((addr, name, api_type)) = addr
        .zip(name)
        .zip(api_type)
        .map(|((addr, name), api_type)| (addr, name, api_type))
    else {
        eprintln!("error: --addr, --name, and --api-type are required");
        return ExitCode::FAILURE;
    };
    let Some(token) = client_token() else {
        return ExitCode::FAILURE;
    };
    match create_keyspace_blocking(&addr, &token, name, tenant_id, api_type) {
        Ok(response) => {
            println!("keyspace_id={}", response.keyspace_id);
            println!("proposed_term={}", response.proposed_term.unwrap_or(0));
            println!("proposed_index={}", response.proposed_index.unwrap_or(0));
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("client request failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_admit_node(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut addr = None;
    let mut node_id = None;
    let mut node_addr = None;
    let mut ttl_seconds = 600u64;
    while let Some(flag) = args.next() {
        let Some(value) = args.next() else {
            eprintln!("error: {flag} needs a value");
            return ExitCode::FAILURE;
        };
        match flag.as_str() {
            "--addr" => addr = Some(value),
            "--node-id" => node_id = parse_client_node_id(&value),
            "--node-addr" => match value.parse::<SocketAddr>() {
                Ok(value) => node_addr = Some(value),
                Err(_) => {
                    eprintln!("error: --node-addr must be ip:port");
                    return ExitCode::FAILURE;
                }
            },
            "--ttl-seconds" => match value.parse::<u64>() {
                Ok(value) if value > 0 => ttl_seconds = value,
                _ => {
                    eprintln!("error: --ttl-seconds must be a non-zero integer");
                    return ExitCode::FAILURE;
                }
            },
            _ => {
                eprintln!("error: unknown client flag {flag}");
                return ExitCode::FAILURE;
            }
        }
        if flag == "--node-id" && node_id.is_none() {
            return ExitCode::FAILURE;
        }
    }
    let Some((addr, node_id, node_addr)) = addr
        .zip(node_id)
        .zip(node_addr)
        .map(|((addr, node_id), node_addr)| (addr, node_id, node_addr))
    else {
        eprintln!("error: --addr, --node-id, and --node-addr are required");
        return ExitCode::FAILURE;
    };
    let Some(token) = client_token() else {
        return ExitCode::FAILURE;
    };
    match admit_node_blocking(&addr, &token, node_id, node_addr.to_string(), ttl_seconds) {
        Ok(response) => print_membership_response(response),
        Err(error) => {
            eprintln!("client request failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_promote_node(mut args: impl Iterator<Item = String>) -> ExitCode {
    let mut addr = None;
    let mut node_id = None;
    while let Some(flag) = args.next() {
        let Some(value) = args.next() else {
            eprintln!("error: {flag} needs a value");
            return ExitCode::FAILURE;
        };
        match flag.as_str() {
            "--addr" => addr = Some(value),
            "--node-id" => node_id = parse_client_node_id(&value),
            _ => {
                eprintln!("error: unknown client flag {flag}");
                return ExitCode::FAILURE;
            }
        }
        if flag == "--node-id" && node_id.is_none() {
            return ExitCode::FAILURE;
        }
    }
    let Some((addr, node_id)) = addr.zip(node_id) else {
        eprintln!("error: --addr and --node-id are required");
        return ExitCode::FAILURE;
    };
    let Some(token) = client_token() else {
        return ExitCode::FAILURE;
    };
    match promote_node_blocking(&addr, &token, node_id) {
        Ok(response) => print_membership_response(response),
        Err(error) => {
            eprintln!("client request failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_client_node_id(value: &str) -> Option<NodeId> {
    match value.parse::<u64>() {
        Ok(value) if value > 0 => Some(NodeId(value)),
        _ => {
            eprintln!("error: --node-id must be a non-zero integer");
            None
        }
    }
}

fn client_token() -> Option<String> {
    let token = std::env::var("KV9_CLIENT_TOKEN").unwrap_or_default();
    if token.is_empty() {
        eprintln!("error: KV9_CLIENT_TOKEN must be non-empty");
        None
    } else {
        Some(token)
    }
}

fn print_membership_response(
    response: kv9_server::grpc::proto::MembershipChangeResponse,
) -> ExitCode {
    println!("applied_term={}", response.applied_term);
    println!("applied_index={}", response.applied_index);
    println!(
        "meta_voters={}",
        response
            .voters
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    println!(
        "meta_learners={}",
        response
            .learners
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    ExitCode::SUCCESS
}

fn parse_client_tokens(value: &str) -> Vec<(String, String)> {
    value
        .split(',')
        .filter_map(|credential| {
            let (principal, token) = credential.split_once('=')?;
            (!principal.is_empty() && !token.is_empty())
                .then(|| (token.to_string(), principal.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> impl Iterator<Item = String> {
        items
            .iter()
            .map(|item| (*item).to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn cli_maps_node_configuration() {
        let cli = parse_cli(args(&[
            "--node-id",
            "7",
            "--addr",
            "127.0.0.1:30160",
            "--data-dir",
            "/tmp/kv9-test",
            "--join",
            "7@127.0.0.1:30160,8@127.0.0.1:30161",
        ]))
        .unwrap();
        let config = config_from_cli(cli);

        assert_eq!(config.addr, "127.0.0.1:30160");
        assert_eq!(config.data_dir, "/tmp/kv9-test");
        assert_eq!(
            config.join,
            [
                SeedPeer {
                    node_id: NodeId(7),
                    addr: "127.0.0.1:30160".parse().unwrap(),
                },
                SeedPeer {
                    node_id: NodeId(8),
                    addr: "127.0.0.1:30161".parse().unwrap(),
                },
            ]
        );
    }

    #[test]
    fn cli_requires_a_non_zero_node_id() {
        assert_eq!(parse_cli(args(&[])).unwrap_err(), "--node-id is required");
        assert_eq!(
            parse_cli(args(&["--node-id", "0"])).unwrap_err(),
            "--node-id must be a non-zero integer"
        );
    }

    #[test]
    fn txn_groups_is_not_a_node_cli_option() {
        let err = parse_cli(args(&["--node-id", "1", "--txn-groups", "4"])).unwrap_err();
        assert_eq!(err, "unknown argument: --txn-groups");
    }

    #[test]
    fn cli_rejects_ambiguous_or_mismatched_seed_identity() {
        for bad in [
            "127.0.0.1:20160",
            "0@127.0.0.1:20160",
            "x@127.0.0.1:20160",
            "1@localhost:20160",
        ] {
            assert!(parse_cli(args(&["--node-id", "1", "--join", bad])).is_err());
        }

        let duplicate_id = parse_cli(args(&[
            "--node-id",
            "1",
            "--join",
            "1@127.0.0.1:20160,1@127.0.0.1:20161",
        ]));
        assert_eq!(duplicate_id.unwrap_err(), "duplicate --join node id 1");

        let duplicate_addr = parse_cli(args(&[
            "--node-id",
            "1",
            "--join",
            "1@127.0.0.1:20160,2@127.0.0.1:20160",
        ]));
        assert_eq!(
            duplicate_addr.unwrap_err(),
            "duplicate --join address 127.0.0.1:20160"
        );

        let missing_self = parse_cli(args(&[
            "--node-id",
            "1",
            "--join",
            "2@127.0.0.1:20161,3@127.0.0.1:20162",
        ]));
        assert_eq!(
            missing_self.unwrap_err(),
            "node 1 is absent from --join; join-existing mode requires --cluster-id"
        );

        let joining = parse_cli(args(&[
            "--node-id",
            "4",
            "--addr",
            "127.0.0.1:20163",
            "--join",
            "1@127.0.0.1:20160,2@127.0.0.1:20161,3@127.0.0.1:20162",
            "--cluster-id",
            "00112233445566778899aabbccddeeff",
        ]))
        .unwrap();
        assert_eq!(
            joining.cluster_id.unwrap().to_string(),
            "00112233445566778899aabbccddeeff"
        );

        let initial_with_cluster = parse_cli(args(&[
            "--node-id",
            "1",
            "--join",
            "1@127.0.0.1:20160,2@127.0.0.1:20161",
            "--cluster-id",
            "00112233445566778899aabbccddeeff",
        ]));
        assert_eq!(
            initial_with_cluster.unwrap_err(),
            "--cluster-id is only valid when this node is absent from --join (join-existing mode)"
        );

        let mismatched_self = parse_cli(args(&[
            "--node-id",
            "1",
            "--addr",
            "127.0.0.1:30160",
            "--join",
            "1@127.0.0.1:20160,2@127.0.0.1:20161",
        ]));
        assert_eq!(
            mismatched_self.unwrap_err(),
            "--join declares this node 1 at 127.0.0.1:20160, but --addr is 127.0.0.1:30160"
        );
    }
}
