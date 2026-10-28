//! End-to-end smoke used by scripts/sdk-e2e.sh. Reads KV9_ADDR, KV9_TOKEN,
//! KV9_KEYSPACE and optionally KV9_PEERS (`1=host:port,2=…`).

use kv9_client::{Client, Error, Mutation};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set"))
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let peers: Vec<(u64, String)> = std::env::var("KV9_PEERS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|item| {
            let (node, address) = item.split_once('=').expect("node=address");
            (node.parse().expect("node id"), address.to_owned())
        })
        .collect();
    let client = Client::new(env("KV9_ADDR"), &env("KV9_TOKEN"))?.with_peers(peers);
    let info = client.cluster_info().await?;
    assert!(info.node_count >= 1);
    let ks = client.keyspace(env("KV9_KEYSPACE").parse().expect("keyspace id"));
    let p = |suffix: &str| format!("rs-smoke/{suffix}").into_bytes();

    let first = ks.put(p("k1"), b"v1".to_vec()).await?;
    assert!(first.term > 0 && first.index > 0);
    assert_eq!(ks.get(p("k1")).await?, Some(b"v1".to_vec()));
    assert_eq!(ks.get(p("absent")).await?, None);

    let second = ks
        .batch(vec![
            Mutation::Put {
                key: p("k2"),
                value: b"v2".to_vec(),
            },
            Mutation::Put {
                key: p("k3"),
                value: b"v3".to_vec(),
            },
            Mutation::Delete { key: p("k1") },
        ])
        .await?;
    assert!((second.term, second.index) > (first.term, first.index));
    assert_eq!(
        ks.batch_get(vec![p("k1"), p("k2"), p("k3"), p("k2")])
            .await?,
        vec![
            None,
            Some(b"v2".to_vec()),
            Some(b"v3".to_vec()),
            Some(b"v2".to_vec())
        ]
    );
    let keys: Vec<_> = ks
        .scan_all(p(""), p("\u{7f}"), 1024)
        .await?
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert_eq!(keys, vec![p("k2"), p("k3")]);

    ks.batch_put(vec![(p("k4"), b"v4".to_vec()), (p("k5"), b"v5".to_vec())])
        .await?;
    let page = ks.scan(p(""), p("\u{7f}"), 2).await?;
    assert_eq!(page.pairs.len(), 2);

    let deleted = ks.delete_range(p(""), p("\u{7f}")).await?;
    assert!(deleted.committed_chunks >= 1);
    assert!(ks.scan_all(p(""), p("\u{7f}"), 1024).await?.is_empty());

    assert_eq!(
        ks.put(vec![0; kv9_client::MAX_KEY_BYTES + 1], vec![]).await,
        Err(Error::Bounds("key exceeds 4096 bytes"))
    );
    println!("rust sdk smoke: OK (leader {})", client.address());
    Ok(())
}
