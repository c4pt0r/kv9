//! kv9 — the single binary (DESIGN §1 goal 2, §11).
//!
//! One `kv9` executable is *every* role: storage node, metadata member, request router.
//! This entry point parses a minimal CLI (`--join`, `--data-dir`, `--addr`) and wires
//! a [`kv9_server::Node`]. Transaction groups belong to individual keyspaces in the
//! metadata catalog and are deliberately not node startup configuration.

use std::process::ExitCode;

use kv9_common::{Config, NodeId};
use kv9_server::Node;

/// Parsed CLI arguments (DESIGN §11).
#[derive(Debug, Default)]
struct Cli {
    addr: Option<String>,
    data_dir: Option<String>,
    join: Vec<String>,
}

fn print_usage() {
    eprintln!(
        "kv9 — single-binary distributed KV\n\
         \n\
         USAGE:\n\
           kv9 [--addr <host:port>] [--data-dir <path>] [--join <peer>[,<peer>...]]\n\
         \n\
         FLAGS (DESIGN §11):\n\
           --addr        serving address to bind (default 127.0.0.1:20160)\n\
           --data-dir    local data directory (default ./kv9-data)\n\
           --join        comma-separated seed peers to join an existing cluster\n\
           -h, --help    print this help"
    );
}

fn parse_cli(args: impl Iterator<Item = String>) -> std::result::Result<Cli, String> {
    let mut cli = Cli::default();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--addr" => cli.addr = Some(args.next().ok_or("--addr needs a value")?),
            "--data-dir" => cli.data_dir = Some(args.next().ok_or("--data-dir needs a value")?),
            "--join" => {
                let v = args.next().ok_or("--join needs a value")?;
                cli.join = v
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "-h" | "--help" => return Err("help".to_string()),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(cli)
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

    let config = config_from_cli(cli);
    // v0 skeleton: a fixed node id; a real cluster derives it during bootstrap.
    let node = match Node::new(NodeId(1), config) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("failed to assemble node: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "kv9 node {:?} assembled (addr={}, data_dir={}, join={:?})",
        node.id, node.config.addr, node.config.data_dir, node.config.join
    );

    if let Err(e) = node.bootstrap() {
        eprintln!("bootstrap failed: {e}");
        return ExitCode::FAILURE;
    }
    println!("kv9 node bootstrapped; metadata initialized (M0 skeleton — serving not yet wired).");

    ExitCode::SUCCESS
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
            "--addr",
            "127.0.0.1:30160",
            "--data-dir",
            "/tmp/kv9-test",
            "--join",
            "n1:20160,n2:20160",
        ]))
        .unwrap();
        let config = config_from_cli(cli);

        assert_eq!(config.addr, "127.0.0.1:30160");
        assert_eq!(config.data_dir, "/tmp/kv9-test");
        assert_eq!(config.join, ["n1:20160", "n2:20160"]);
    }

    #[test]
    fn txn_groups_is_not_a_node_cli_option() {
        let err = parse_cli(args(&["--txn-groups", "4"])).unwrap_err();
        assert_eq!(err, "unknown argument: --txn-groups");
    }
}
