//! Test fixture only. Never launches a peer or publishes migration authority.
use kv9_common::codec::{encode_key, KeyMode};
use kv9_common::data_range::{DataRange, RANGE_KEY};
use kv9_common::root::{load_root_bundle, persist_root_bundle};
use kv9_common::store_lifecycle::StoreGuard;
use kv9_common::{
    AppliedPosition, BootstrapGeneration, ClusterId, KeyspaceId, NodeId, RegionId, RootDescriptor,
    RootDigest, RootVoter, StoreIdentity, TenantId,
};
use kv9_engine::checkpoint::{FlushScope, RemoteUploader};
use kv9_engine::minio::{MinioConfig, MinioObjectStore};
use kv9_engine::{ColumnFamily, Engine, ReplicatedEngine, WalEngine, WriteBatch};
use kv9_raft::snapshot_install::{data_image, InstallStep, JointInstaller};
use protobuf::Message;
use raft::prelude::{ConfState, HardState, Snapshot};
use serde_json::json;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn put(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if !(4..=5).contains(&args.len()) {
        return Err("usage: joint-install-probe seed|install|recover STORE BUNDLE [PAUSE]".into());
    }
    let directory = PathBuf::from(&args[2]);
    let bundle = PathBuf::from(&args[3]);
    let mut guard = StoreGuard::lock(&directory)?;
    let uploader = RemoteUploader::new(Arc::new(MinioObjectStore::connect(
        MinioConfig::from_env()?,
    )?));
    if args[1] == "seed" {
        // Two source images share one exact range/root. A has 4 records at 10;
        // B has 5 records and an overwritten first value at 20.
        if guard.record().is_some() || bundle.exists() {
            return Err("seed requires a fresh destination and bundle".into());
        }
        let prepared = guard.prepare(NodeId(4))?;
        let root = RootDescriptor::new(
            ClusterId::mint()?,
            BootstrapGeneration::mint()?,
            vec![RootVoter {
                node_id: NodeId(4),
                addr: "127.0.0.1:19444".parse()?,
                store_incarnation: prepared.incarnation,
            }],
            b"non-serving-joint-install-test-fixture",
        )?;
        let identity = StoreIdentity::for_voter(&root, NodeId(4))?;
        persist_root_bundle(&directory, &root, &identity)?;
        guard.bind(&root, &identity)?;
        let range = DataRange {
            root: root.digest(),
            creation: RootDigest::sha256(b"joint-install-fixture"),
            region: RegionId(100),
            keyspace: KeyspaceId(7),
            tenant: TenantId(1),
            conf_ver: 1,
            version: 1,
            start: b"a".to_vec(),
            end: b"z".to_vec(),
            sealed: false,
        };
        put(&directory.join("fixture-range"), &range.encode())?;
        fs::create_dir_all(directory.join("observations"))?;
        File::open(&directory)?.sync_all()?;
        let source = bundle.with_extension("source");
        fs::create_dir(&source)?;
        let (engine, _) = WalEngine::open(source.join("data.wal"))?;
        for (index, count, destination) in [
            (10, 4, bundle.with_extension("old")),
            (20, 5, bundle.clone()),
        ] {
            let mut batch = WriteBatch::new();
            batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), range.encode());
            for n in 0..count {
                batch.put(
                    ColumnFamily::Default,
                    encode_key(KeyMode::Raw, range.keyspace, format!("key-{n}").as_bytes())?,
                    format!("value-{index}-{n}").into_bytes(),
                );
            }
            engine.write_applied(batch, AppliedPosition { term: 3, index })?;
            let manifest = uploader
                .upload(engine.freeze(FlushScope {
                    cluster: identity.cluster_id.to_string(),
                    region: 100,
                    conf_ver: 1,
                    version: 1,
                })?)?
                .into_manifest();
            let mut snapshot = Snapshot {
                data: data_image(&range, &manifest)?.into(),
                ..Default::default()
            };
            snapshot.mut_metadata().index = index;
            snapshot.mut_metadata().term = 3;
            snapshot
                .mut_metadata()
                .set_conf_state(ConfState::from((vec![1, 2, 3], vec![4])));
            put(&destination, &snapshot.write_to_bytes()?)?;
        }
        File::open(bundle.parent().ok_or("bundle parent missing")?)?.sync_all()?;
        println!(
            "{}",
            json!({"status":"seeded", "old_cut":10,"new_cut":20,"root":root.digest().to_string(),"target_node":4,"target_incarnation":identity.store_incarnation.to_string(),"source_authority":"test fixture only"})
        );
        return Ok(());
    }
    let (root, identity) = load_root_bundle(&directory)?;
    identity.verify(&root, NodeId(4))?;
    let range = DataRange::decode(&fs::read(directory.join("fixture-range"))?)?;
    let mut installer = JointInstaller::open(&guard, identity, range)?;
    let result = if args[1] == "recover" {
        installer.recover(&uploader)?
    } else if args[1] == "install" {
        let bytes = fs::read(&bundle)?;
        let snapshot = Snapshot::parse_from_bytes(&bytes)?;
        let digest = RootDigest::sha256(&bytes).to_string();
        let hs = HardState {
            term: 5,
            vote: 2,
            commit: snapshot.get_metadata().index,
            ..Default::default()
        };
        Some(installer.install_observed(&snapshot, &hs, &uploader, &mut |step, generation| {
            let name = match step { InstallStep::AfterEngine => "after-engine", InstallStep::AfterProtocol => "after-protocol",
                InstallStep::BeforeSelector => "before-selector", InstallStep::AfterSelectorRename => "after-selector-rename",
                InstallStep::AfterSelectorSync => "after-selector-sync" };
            if args.get(4).is_some_and(|pause| pause == name) {
                let event = json!({"step":name,"generation":generation.to_string(),"pid":std::process::id(),"bundle_sha256":digest,"target_incarnation":identity.store_incarnation.to_string(),"term":3,"index":snapshot.get_metadata().index});
                // Only sync this pre-existing observation directory: syncing the
                // selector's parent here would erase the rename/sync fault cut.
                let observations = directory.join("observations");
                put(&observations.join("pause.json"), event.to_string().as_bytes()).map_err(|e| kv9_common::Error::Config(e.to_string()))?;
                File::open(&observations).and_then(|f| f.sync_all()).map_err(|e| kv9_common::Error::Config(e.to_string()))?;
                println!("{event}");
                loop { std::thread::sleep(std::time::Duration::from_secs(1)); }
            }
            Ok(())
        })?)
    } else {
        return Err("unknown command".into());
    };
    if let Some(value) = &result {
        let path = directory.join(format!(
            "data-groups/100/install-{}/data.wal",
            value.generation
        ));
        let (engine, _) = WalEngine::open_with_uploader(path, Some(&uploader))?;
        let count = match value.position.index {
            10 => 4,
            20 => 5,
            _ => return Err("unexpected fixture cut".into()),
        };
        if value.records != count {
            return Err("fixture count mismatch".into());
        }
        for n in 0..count {
            let key = encode_key(KeyMode::Raw, KeyspaceId(7), format!("key-{n}").as_bytes())?;
            if engine.get(ColumnFamily::Default, &key)?
                != Some(format!("value-{}-{n}", value.position.index).into_bytes())
            {
                return Err("fixture value mismatch".into());
            }
        }
    }
    match result {
        Some(value) => println!(
            "{}",
            json!({"status":"selected","generation":value.generation.to_string(),"image_digest":value.image_digest.to_string(),"term":value.position.term,"index":value.position.index,"target_node":value.target.node_id.0,"target_incarnation":value.target.store_incarnation.to_string(),"records":value.records,"object_bytes":value.object_bytes,"values_verified":true,"serving":false})
        ),
        None => println!("{}", json!({"status":"pending","serving":false})),
    }
    Ok(())
}
