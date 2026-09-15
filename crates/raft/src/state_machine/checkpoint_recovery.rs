//! Match a recovered checkpoint's actual WAL publication to committed Raft.
//! This validates local retained history, not a portable snapshot/install token.
use kv9_common::{AppliedPosition, Error, Result};
use kv9_engine::checkpoint::CheckpointManifest;
use kv9_engine::{
    ColumnFamily, DurableAppliedPosition, Engine, Mutation, ReplicatedEngine, WalEngine, WriteBatch,
};
use raft::{GetEntriesContext, Storage};

use super::{manifest_gen_key, manifest_pair_key};
use crate::storage::{CommittedConfiguration, ConfigurationLookup, DiskRaftStorage};
use crate::{Command, ManifestPair};

/// Open a local engine and validate the exact selected checkpoint before the
/// caller can publish it. Observations cannot be supplied by the caller, and a
/// partial/failed open cannot mint a recovered publication.
pub fn open_checkpoint_engine(
    storage: &mut DiskRaftStorage,
    path: impl AsRef<std::path::Path>,
    uploader: Option<&kv9_engine::checkpoint::RemoteUploader>,
    cluster: String,
    region: u64,
) -> Result<(
    WalEngine,
    kv9_engine::EngineReplay,
    Option<RecoveredCheckpointPublication>,
)> {
    let recovery = std::cell::RefCell::new(CheckpointRecovery::new(storage, cluster, region));
    let (engine, report) = WalEngine::open_with_replay_observer(
        path,
        uploader,
        |base| recovery.borrow_mut().checkpoint(base),
        |batch, at| recovery.borrow_mut().batch(batch, at),
    )?;
    let publication = recovery.into_inner().finish(&engine)?;
    Ok((engine, report, publication))
}

/// Provisional startup observations. Never reuse this object after any engine
/// open error. `finish` is called only on that successful open's returned engine.
struct CheckpointRecovery<'a> {
    storage: &'a mut DiskRaftStorage,
    cluster: String,
    region: u64,
    started: bool,
    selected: Option<SelectedCheckpoint>,
    publication: Option<(AppliedPosition, u64)>,
}

struct SelectedCheckpoint {
    manifest: CheckpointManifest,
    configuration: CommittedConfiguration,
    installed: Vec<u8>,
    history_prefix: Vec<u8>,
    pair_key: Vec<u8>,
}

/// A local recovery observation. Root/store admission and full range/retention
/// binding remain the enclosing runtime/anchor layer's responsibility.
#[derive(Debug)]
pub struct RecoveredCheckpointPublication {
    manifest: CheckpointManifest,
    configuration: CommittedConfiguration,
    publication: AppliedPosition,
    generation: u64,
}
impl RecoveredCheckpointPublication {
    pub fn manifest(&self) -> &CheckpointManifest {
        &self.manifest
    }
    pub fn configuration(&self) -> &CommittedConfiguration {
        &self.configuration
    }
    pub fn publication(&self) -> AppliedPosition {
        self.publication
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl<'a> CheckpointRecovery<'a> {
    fn new(storage: &'a mut DiskRaftStorage, cluster: String, region: u64) -> Self {
        Self {
            storage,
            cluster,
            region,
            started: false,
            selected: None,
            publication: None,
        }
    }

    fn checkpoint(&mut self, selected: Option<&CheckpointManifest>) -> Result<()> {
        if self.started {
            return Err(bad("recovery base observed twice"));
        }
        self.started = true;
        let Some(manifest) = selected else {
            return Ok(());
        };
        let manifest = CheckpointManifest::decode(&manifest.encode()?)?;
        if manifest.scope.cluster != self.cluster || manifest.scope.region != self.region {
            return Err(bad("checkpoint scope disagrees with this store"));
        }
        let ConfigurationLookup::Found(configuration) = self
            .storage
            .configuration_at_committed(manifest.position())?
        else {
            return Err(bad("checkpoint configuration authority is unavailable"));
        };
        let mut installed = manifest.term.to_be_bytes().to_vec();
        installed.extend_from_slice(&manifest.index.to_be_bytes());
        installed.extend_from_slice(&manifest.encode()?);
        let mut history_prefix = manifest_gen_key(self.region, 0);
        history_prefix.truncate(history_prefix.len() - 8);
        self.selected = Some(SelectedCheckpoint {
            manifest,
            configuration,
            installed,
            history_prefix,
            pair_key: manifest_pair_key(self.region),
        });
        Ok(())
    }

    fn batch(&mut self, batch: &WriteBatch, at: Option<AppliedPosition>) -> Result<()> {
        if !self.started {
            return Err(bad("WAL observation precedes the recovery base"));
        }
        let Some(selected) = &self.selected else {
            return Ok(());
        };
        let manifest = &selected.manifest;
        let at = at.ok_or_else(|| bad("unpositioned WAL beside a checkpoint"))?;
        if at.index <= manifest.index || at.term < manifest.term {
            return Err(bad("uncovered WAL position contradicts checkpoint cut"));
        }
        let bytes = &selected.installed[16..];
        let prefix = &selected.history_prefix;
        for mutation in batch.mutations() {
            let Mutation::Put {
                cf: ColumnFamily::Default,
                key,
                value,
            } = mutation
            else {
                continue;
            };
            if key.len() != prefix.len() + 8
                || !key.starts_with(prefix)
                || value != &selected.installed
            {
                continue;
            }
            let generation =
                u64::from_be_bytes(key[key.len() - 8..].try_into().expect("length checked"));
            let predecessor = generation
                .checked_sub(1)
                .ok_or_else(|| bad("zero manifest generation"))?;
            let change_id = manifest.change_id(predecessor)?;
            if self
                .publication
                .is_some_and(|(_, earlier)| generation <= earlier)
            {
                return Err(bad("checkpoint publication was rewritten"));
            }
            let pair_key = &selected.pair_key;
            let pair = ManifestPair {
                generation,
                last_change_id: change_id.clone(),
            }
            .encode();
            let pair_writes: Vec<_> = batch
                .mutations()
                .iter()
                .filter(|m| match m {
                    Mutation::Put { cf, key, .. } | Mutation::Delete { cf, key } => {
                        *cf == ColumnFamily::Default && key == pair_key
                    }
                })
                .collect();
            if pair_writes.len() != 1
                || !matches!(pair_writes[0], Mutation::Put { value, .. } if value == &pair)
            {
                return Err(bad("manifest descriptor lacks its atomic winning pair"));
            }
            if self.storage.committed_term(at.index)? != at.term {
                return Err(bad(
                    "manifest publication term disagrees with committed Raft",
                ));
            }
            let entries = self
                .storage
                .entries(
                    at.index,
                    at.index
                        .checked_add(1)
                        .ok_or_else(|| bad("publication index overflow"))?,
                    Some(1),
                    GetEntriesContext::empty(false),
                )
                .map_err(|e| bad(&e.to_string()))?;
            let entry = entries
                .first()
                .filter(|entry| {
                    entries.len() == 1
                        && entry.index == at.index
                        && entry.term == at.term
                        && entry.entry_type == raft::eraftpb::EntryType::EntryNormal
                })
                .ok_or_else(|| bad("manifest publication has no exact normal Raft entry"))?;
            let Command::ManifestChange(payload) = Command::decode(&entry.data)? else {
                return Err(bad("winning WAL batch does not name a manifest command"));
            };
            if payload.region() != self.region
                || payload.expected_generation() != predecessor
                || payload.change_id() != change_id
                || payload.changeset() != bytes
                || payload.watermark() != (manifest.term, manifest.index)
            {
                return Err(bad(
                    "manifest publication command disagrees with the winning WAL batch",
                ));
            }
            self.publication.get_or_insert((at, generation));
        }
        Ok(())
    }

    /// Only call after the observed engine open returned successfully. The
    /// runtime keeps the store guard and has not constructed a Raft peer yet.
    fn finish(self, engine: &WalEngine) -> Result<Option<RecoveredCheckpointPublication>> {
        if !self.started {
            return Err(bad("recovery never selected a base"));
        }
        let Some(SelectedCheckpoint {
            manifest,
            configuration,
            installed,
            ..
        }) = self.selected
        else {
            return Ok(None);
        };
        let (publication, generation) = self
            .publication
            .ok_or_else(|| bad("checkpoint lacks a retained successful publication batch"))?;
        let DurableAppliedPosition::AppliedThrough(applied) = engine.applied_position()? else {
            return Err(bad("checkpoint recovery has no durable applied position"));
        };
        if applied.index < publication.index
            || applied.term < publication.term
            || engine
                .get(
                    ColumnFamily::Default,
                    &manifest_gen_key(self.region, generation),
                )?
                .as_deref()
                != Some(installed.as_slice())
        {
            return Err(bad(
                "recovered engine does not retain its observed publication",
            ));
        }
        let pair = engine
            .get(ColumnFamily::Default, &manifest_pair_key(self.region))?
            .ok_or_else(|| bad("recovered manifest pair is missing"))?;
        let pair = ManifestPair::decode(&pair)?;
        if pair.generation < generation
            || (pair.generation == generation
                && pair.last_change_id != manifest.change_id(generation - 1)?)
        {
            return Err(bad("recovered manifest generation precedes publication"));
        }
        Ok(Some(RecoveredCheckpointPublication {
            manifest,
            configuration,
            publication,
            generation,
        }))
    }
}
fn bad(message: &str) -> Error {
    Error::Engine(format!("checkpoint recovery: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rawnode::PersistentRaftStorage;
    use crate::{ApplyOutcome, FenceAdjudicator, ManifestVerdict, MemStateMachine, RegionFence};
    use raft::prelude::{Entry, HardState};
    use std::path::PathBuf;
    use std::sync::Arc;

    struct Fresh;
    impl FenceAdjudicator for Fresh {
        fn is_fresh(&self, _: &RegionFence) -> Result<bool> {
            Ok(true)
        }
    }
    fn directory(name: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "kv9-publication-{name}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }
    fn at(index: u64) -> AppliedPosition {
        AppliedPosition {
            term: if index == 1 { 1 } else { 2 },
            index,
        }
    }
    fn manifest() -> CheckpointManifest {
        use kv9_engine::checkpoint::{FlushScope, SstReference};
        let sha256 = "0".repeat(64);
        CheckpointManifest {
            scope: FlushScope {
                cluster: "test".into(),
                region: 9,
                conf_ver: 1,
                version: 1,
            },
            term: 1,
            index: 1,
            files: vec![SstReference {
                key: format!("clusters/test/regions/9/sst/{sha256}"),
                sha256,
                cf: 0,
                smallest: b"a".to_vec(),
                largest: b"z".to_vec(),
                size: 100,
                count: 2,
            }],
        }
    }
    fn command(manifest: &CheckpointManifest, generation: u64) -> Command {
        Command::ManifestChange(crate::ManifestChangePayload::for_harness(
            9,
            manifest.change_id(generation).unwrap(),
            generation,
            manifest.encode().unwrap(),
            manifest.term,
            manifest.index,
        ))
    }
    fn commands(second: Command, third: Command) -> Vec<Command> {
        vec![
            Command::Put {
                cf: 0,
                key: b"base".to_vec(),
                value: b"value".to_vec(),
            },
            second,
            third,
        ]
    }
    fn raft_log(path: &std::path::Path, commands: &[Command]) -> DiskRaftStorage {
        let store = DiskRaftStorage::open(&path.join("raft"), &[1, 2, 3])
            .unwrap()
            .0;
        let entries: Vec<_> = commands
            .iter()
            .enumerate()
            .map(|(i, c)| Entry {
                index: i as u64 + 1,
                term: at(i as u64 + 1).term,
                data: c.encode().into(),
                ..Default::default()
            })
            .collect();
        store
            .persist_ready(
                &entries,
                Some(&HardState {
                    term: 2,
                    vote: 1,
                    commit: commands.len() as u64,
                    ..Default::default()
                }),
            )
            .unwrap();
        store
    }
    fn apply_log(path: &std::path::Path, commands: &[Command]) -> Vec<ApplyOutcome> {
        let engine = Arc::new(WalEngine::open(path.join("catalog.wal")).unwrap().0);
        let mut sm = MemStateMachine::with_engine(engine).unwrap();
        sm.set_fence_adjudicator(Arc::new(Fresh));
        commands
            .iter()
            .enumerate()
            .map(|(i, c)| sm.apply_at(at(i as u64 + 1), c).unwrap().outcome)
            .collect()
    }
    // Component fixture: use actual durable apply batches while supplying the
    // base separately. The real MinIO test below exercises selected-base open.
    fn inspect(
        path: &std::path::Path,
        storage: &mut DiskRaftStorage,
        selected: &CheckpointManifest,
    ) -> Result<RecoveredCheckpointPublication> {
        let mut recovery = CheckpointRecovery::new(storage, "test".into(), 9);
        recovery.checkpoint(Some(selected))?;
        let (engine, _) = WalEngine::open_with_replay_observer(
            path.join("catalog.wal"),
            None,
            |_| Ok(()),
            |batch, pos| {
                if pos.is_some_and(|p| p.index > selected.index) {
                    recovery.batch(batch, pos)
                } else {
                    Ok(())
                }
            },
        )?;
        Ok(recovery.finish(&engine)?.unwrap())
    }
    #[test]
    fn committed_losing_proposal_is_not_checkpoint_publication() {
        let path = directory("loser");
        let winner = manifest();
        let mut loser = winner.clone();
        loser.files[0].count += 1;
        let cmds = commands(command(&winner, 0), command(&loser, 0));
        let mut storage = raft_log(&path, &cmds);
        let outcomes = apply_log(&path, &cmds);
        assert!(matches!(
            outcomes[1],
            ApplyOutcome::Manifest(ManifestVerdict::Applied { generation: 1, .. })
        ));
        assert!(!matches!(
            outcomes[2],
            ApplyOutcome::Manifest(ManifestVerdict::Applied { .. })
        ));
        assert!(
            storage
                .has_committed_checkpoint(&loser.encode().unwrap())
                .unwrap(),
            "old presence check accepts this loser"
        );
        assert!(inspect(&path, &mut storage, &loser)
            .unwrap_err()
            .to_string()
            .contains("lacks a retained successful publication"));
        let actual = inspect(&path, &mut storage, &winner).unwrap();
        assert_eq!(actual.publication(), at(2));
        assert_eq!(actual.configuration().cut(), at(1));
        assert_eq!(actual.generation(), 1);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn retry_or_later_republication_keeps_the_first_actual_winner_position() {
        for next_generation in [0, 1] {
            let path = directory("retry");
            let selected = manifest();
            let cmds = commands(command(&selected, 0), command(&selected, next_generation));
            let mut storage = raft_log(&path, &cmds);
            apply_log(&path, &cmds);
            let actual = inspect(&path, &mut storage, &selected).unwrap();
            assert_eq!(actual.publication(), at(2));
            assert_eq!(actual.generation(), 1);
            std::fs::remove_dir_all(path).unwrap();
        }
    }
    #[test]
    fn winning_batch_requires_its_atomic_pair_and_exact_publication_term() {
        let path = directory("witness");
        let selected = manifest();
        let cmds = commands(command(&selected, 0), Command::Noop);
        let mut storage = raft_log(&path, &cmds);
        apply_log(&path, &cmds);
        let mut observed = None;
        WalEngine::open_with_replay_observer(
            path.join("catalog.wal"),
            None,
            |_| Ok(()),
            |batch, pos| {
                if pos == Some(at(2)) {
                    observed = Some(batch.clone());
                }
                Ok(())
            },
        )
        .unwrap();
        let batch = &observed.unwrap();
        let mut missing_pair = WriteBatch::new();
        for m in batch.mutations() {
            if let Mutation::Put { cf, key, value } = m {
                if key != &manifest_pair_key(9) {
                    missing_pair.put(*cf, key.clone(), value.clone());
                }
            }
        }
        let mut recovery = CheckpointRecovery::new(&mut storage, "test".into(), 9);
        recovery.checkpoint(Some(&selected)).unwrap();
        assert!(recovery
            .batch(&missing_pair, Some(at(2)))
            .unwrap_err()
            .to_string()
            .contains("atomic winning pair"));
        let mut recovery = CheckpointRecovery::new(&mut storage, "test".into(), 9);
        recovery.checkpoint(Some(&selected)).unwrap();
        assert!(recovery
            .batch(batch, Some(AppliedPosition { term: 3, index: 2 }))
            .is_err());
        let mut recovery = CheckpointRecovery::new(&mut storage, "test".into(), 9);
        recovery.checkpoint(Some(&selected)).unwrap();
        assert!(recovery
            .batch(batch, Some(at(3)))
            .unwrap_err()
            .to_string()
            .contains("does not name a manifest command"));
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn wrong_scope_or_cut_is_refused_before_object_restore() {
        let path = directory("scope");
        let selected = manifest();
        let cmds = commands(command(&selected, 0), Command::Noop);
        let mut storage = raft_log(&path, &cmds);
        for (cluster, region, term) in [("foreign", 9, 1), ("test", 10, 1), ("test", 9, 2)] {
            let mut other = selected.clone();
            other.term = term;
            let mut recovery = CheckpointRecovery::new(&mut storage, cluster.into(), region);
            assert!(recovery.checkpoint(Some(&other)).is_err());
        }
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn normal_open_without_checkpoint_has_no_publication_claim() {
        let path = directory("plain");
        let cmds = commands(Command::Noop, Command::Noop);
        let mut storage = raft_log(&path, &cmds);
        apply_log(&path, &cmds);
        let (engine, report, publication) = open_checkpoint_engine(
            &mut storage,
            path.join("catalog.wal"),
            None,
            "test".into(),
            9,
        )
        .unwrap();
        assert!(publication.is_none());
        assert_eq!(report.replayed_records, 3);
        assert_eq!(
            engine.get(ColumnFamily::Default, b"base").unwrap(),
            Some(b"value".to_vec())
        );
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    #[ignore = "requires a real MinIO server"]
    fn real_minio_recovery_accepts_winner_and_refuses_committed_loser_in_both_layouts() {
        use kv9_engine::checkpoint::{FlushScope, RemoteUploader};
        use kv9_engine::{MinioConfig, MinioObjectStore};
        let uploader = RemoteUploader::new(Arc::new(
            MinioObjectStore::connect(MinioConfig::from_env().unwrap()).unwrap(),
        ));
        for segmented in [false, true] {
            let path = directory(if segmented {
                "minio-segmented"
            } else {
                "minio-legacy"
            });
            let cluster = path.file_name().unwrap().to_str().unwrap().to_string();
            let engine = Arc::new(WalEngine::open(path.join("catalog.wal")).unwrap().0);
            if segmented {
                engine.enable_segmentation().unwrap();
            }
            let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
            sm.set_fence_adjudicator(Arc::new(Fresh));
            let first = Command::Put {
                cf: 0,
                key: b"base".to_vec(),
                value: b"first".to_vec(),
            };
            sm.apply_at(at(1), &first).unwrap();
            let scope = FlushScope {
                cluster: cluster.clone(),
                region: 9,
                conf_ver: 1,
                version: 1,
            };
            let winner = uploader
                .upload(engine.freeze(scope.clone()).unwrap())
                .unwrap()
                .into_manifest();
            let second = Command::Put {
                cf: 0,
                key: b"base".to_vec(),
                value: b"later".to_vec(),
            };
            sm.apply_at(at(2), &second).unwrap();
            let loser = uploader
                .upload(engine.freeze(scope).unwrap())
                .unwrap()
                .into_manifest();
            let win = command(&winner, 0);
            let lose = command(&loser, 0);
            assert!(matches!(
                sm.apply_at(at(3), &win).unwrap().outcome,
                ApplyOutcome::Manifest(ManifestVerdict::Applied { generation: 1, .. })
            ));
            assert!(!matches!(
                sm.apply_at(at(4), &lose).unwrap().outcome,
                ApplyOutcome::Manifest(ManifestVerdict::Applied { .. })
            ));
            let mut storage = raft_log(&path, &[first, second, win, lose]);
            assert!(storage
                .has_committed_checkpoint(&loser.encode().unwrap())
                .unwrap());
            assert!(engine.checkpoint_applied(&winner).unwrap());
            drop(sm);
            drop(engine);
            let (engine, _, proof) = open_checkpoint_engine(
                &mut storage,
                path.join("catalog.wal"),
                Some(&uploader),
                cluster.clone(),
                9,
            )
            .unwrap();
            let proof = proof.unwrap();
            assert_eq!(proof.publication(), at(3));
            assert_eq!(proof.manifest().position(), at(1));
            assert_eq!(
                engine.get(ColumnFamily::Default, b"base").unwrap(),
                Some(b"later".to_vec())
            );
            // Deliberately violate adoption's caller contract to test startup:
            // all objects and the proposal exist, but this transition lost.
            assert!(engine.checkpoint_applied(&loser).unwrap());
            drop(engine);
            let error = open_checkpoint_engine(
                &mut storage,
                path.join("catalog.wal"),
                Some(&uploader),
                cluster,
                9,
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("lacks a retained successful publication"),
                "{error}"
            );
            std::fs::remove_dir_all(path).unwrap();
        }
    }
}
