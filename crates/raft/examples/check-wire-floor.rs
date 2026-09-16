//! Exercise both actual internal service generations during a supported-writer
//! upgrade fixture. Known paths must reach authentication; cross-generation
//! paths must be absent. The probe sends no credential or mutation.
use kv9_raft::grpc::pb::{DiscoverRequest, DiscoverResponse};
use tonic::{client::Grpc, Code, Request};
use tonic_prost::ProstCodec;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 2 || args.len() > 3 {
        return Err(
            "usage: check-wire-floor <old-address> <new-address> [previous-generation=1|2]".into(),
        );
    }
    tokio::runtime::Runtime::new()?.block_on(async {
        let old_path = match args.get(2).map(String::as_str).unwrap_or("1") {
            "1" => "/kv9.raft.Kv9Raft/Discover",
            "2" => "/kv9.raft.v2.Kv9Raft/Discover",
            _ => return Err("invalid previous generation".into()),
        };
        for (server, address) in args[..2].iter().enumerate() {
            let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{address}"))?
                .connect_timeout(std::time::Duration::from_secs(3))
                .timeout(std::time::Duration::from_secs(3));
            let channel = endpoint.connect().await?;
            for (generation, path) in [old_path, "/kv9.raft.v3.Kv9Raft/Discover"].iter().enumerate() {
                let mut client = Grpc::new(channel.clone());
                client.ready().await?;
                let result = client.unary(Request::new(DiscoverRequest::default()),
                    tonic::codegen::http::uri::PathAndQuery::from_static(path),
                    ProstCodec::<DiscoverRequest, DiscoverResponse>::default()).await;
                let status = result.expect_err("unauthenticated discovery unexpectedly succeeded");
                let expected = if server == generation { Code::Unauthenticated } else { Code::Unimplemented };
                if status.code() != expected {
                    return Err(format!("wire floor mismatch: server={server} path={path} expected={expected:?} actual={status}").into());
                }
                println!("server_generation={} request_generation={} code={:?}", server+1, generation+1, status.code());
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
