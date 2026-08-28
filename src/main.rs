//! kv9 — the single binary (DESIGN §1 goal 2, §11).
//!
//! One `kv9` executable is *every* role: storage node, metadata member, request router.
//! This entry point parses a minimal CLI (`--node-id`, `--join`, `--data-dir`,
//! `--addr`) and wires a [`kv9_server::Node`]. Transaction groups belong to individual
//! keyspaces in the metadata catalog and are deliberately not node startup
//! configuration.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::process::ExitCode;

use kv9_common::{Config, NodeId, SeedPeer};
use kv9_server::NodeRuntime;

/// Parsed CLI arguments (DESIGN §11).
#[derive(Debug, Default)]
struct Cli {
    node_id: Option<NodeId>,
    addr: Option<String>,
    data_dir: Option<String>,
    join: Vec<SeedPeer>,
}

fn print_usage() {
    eprintln!(
        "kv9 — single-binary distributed KV\n\
         \n\
         USAGE:\n\
           kv9 --node-id <id> [--addr <ip:port>] [--data-dir <path>] [--join <id@ip:port>[,<id@ip:port>...]]\n\
         \n\
         FLAGS (DESIGN §11):\n\
           --node-id     non-zero stable identity of this node (required)\n\
           --addr        serving address to bind (default 127.0.0.1:20160)\n\
           --data-dir    local data directory (default ./kv9-data)\n\
           --join        fixed seed voters as comma-separated node-id@ip:port pairs\n\
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
        Some(seed) if seed.addr == own_addr => Ok(()),
        Some(seed) => Err(format!(
            "--join declares this node {} at {}, but --addr is {}",
            node_id.0, seed.addr, own_addr
        )),
        None => Err(format!(
            "--join fixed voter set must include this node id {}",
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
    let cli = match parse_cli(std::env::args().skip(1)) {
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
    let config = config_from_cli(cli);
    let runtime = match NodeRuntime::start(node_id, config) {
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
            "--join fixed voter set must include this node id 1"
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
