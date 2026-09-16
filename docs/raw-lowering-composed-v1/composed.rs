//! Offline lowering plus actual MemEngine updates. Never serves requests.
use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use kv9_common::{AppliedPosition, Config, NodeId, RegionId};
use kv9_engine::{
    ColumnFamily, Engine, MemEngine, Mutation, ReadView, ReplicatedEngine, WriteBatch,
};
use kv9_raft::{Command, FenceAdjudicator, FencedInner, MemStateMachine, SingleNodeRaft};
use kv9_server::{fence::CatalogFenceAdjudicator, Node};
use protobuf::Message;
use raft::prelude::{ConfState, Entry, EntryType, HardState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_pinned(path: &Path, bytes: u64, expected: &str, cap: u64) -> Vec<u8> {
    assert!(bytes <= cap && !path.is_symlink());
    let before = std::fs::metadata(path).unwrap();
    assert_eq!(before.len(), bytes);
    let data = std::fs::read(path).unwrap();
    assert_eq!(data.len() as u64, bytes);
    assert_eq!(hash(&data), expected);
    let after = std::fs::metadata(path).unwrap();
    assert_eq!(before.len(), after.len());
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    data
}
fn u32le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().unwrap())
}
fn u64le(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().unwrap())
}
fn fnv(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c9dc5u32, |h, b| {
        (h ^ u32::from(*b)).wrapping_mul(0x01000193)
    })
}
struct Group {
    previous: u64,
    last: u64,
    term: u64,
    payload: Vec<u8>,
    commands: Vec<Command>,
    wire: Vec<Vec<u8>>,
}
fn groups(data: &[u8]) -> Vec<Group> {
    assert_eq!(&data[..8], b"KV9GRP01");
    let mut pos = 8;
    let mut groups = Vec::new();
    while pos < data.len() {
        assert!(pos + 28 <= data.len());
        let previous = u64le(&data[pos..pos + 8]);
        let last = u64le(&data[pos + 8..pos + 16]);
        let term = u64le(&data[pos + 16..pos + 24]);
        let len = u32le(&data[pos + 24..pos + 28]) as usize;
        pos += 28;
        assert!(last > previous && len <= 64 * 1024 * 1024 && pos + len <= data.len());
        groups.push(Group {
            previous,
            last,
            term,
            payload: data[pos..pos + len].to_vec(),
            commands: vec![],
            wire: vec![],
        });
        pos += len;
    }
    assert_eq!(groups.len(), 106);
    assert!(groups.windows(2).all(|w| w[0].last == w[1].previous));
    groups
}
fn read_raft(bytes: &[u8]) -> (BTreeMap<u64, Entry>, Value) {
    let mut entries = BTreeMap::new();
    let mut position = 0;
    let mut kinds = BTreeMap::<u8, u64>::new();
    let mut hard = HardState::default();
    let mut replaced = 0;
    while position < bytes.len() {
        assert!(position + 8 <= bytes.len(), "truncated retained frame");
        let size = u32::from_be_bytes(bytes[position..position + 4].try_into().unwrap()) as usize;
        let checksum = u32::from_be_bytes(bytes[position + 4..position + 8].try_into().unwrap());
        position += 8;
        assert!(size > 0 && size <= 64 * 1024 * 1024 && position + size <= bytes.len());
        let body = &bytes[position..position + size];
        assert_eq!(fnv(body), checksum, "retained record checksum");
        *kinds.entry(body[0]).or_default() += 1;
        match body[0] {
            1 => {
                ConfState::parse_from_bytes(&body[1..]).unwrap();
            }
            2 => {
                let next = HardState::parse_from_bytes(&body[1..]).unwrap();
                assert!(next.term >= hard.term && next.commit >= hard.commit);
                hard = next;
            }
            3 => {
                let entry = Entry::parse_from_bytes(&body[1..]).unwrap();
                assert!(entry.index > 0 && entry.term > 0);
                let suffix = entries.split_off(&entry.index);
                replaced += suffix.len();
                entries.insert(entry.index, entry);
            }
            4 => {
                assert!(body.len() >= 9);
                ConfState::parse_from_bytes(&body[9..]).unwrap();
            }
            _ => panic!("unsupported retained record kind {}", body[0]),
        }
        position += size;
    }
    let report = json!({"frames_by_kind": kinds, "entries": entries.len(), "replaced_suffix_entries": replaced, "final_term": hard.term, "final_commit": hard.commit, "checksum_checked_bytes": bytes.len()});
    (entries, report)
}
fn normal_command(entry: &Entry) -> Command {
    if entry.get_entry_type() != EntryType::EntryNormal || entry.data.is_empty() {
        // Protocol-only entries have no user-data/catalog mutation here. This
        // harness does not reconstruct membership or claim recovery authority.
        Command::Noop
    } else {
        Command::decode_for_harness(&entry.data).unwrap()
    }
}
fn encode_batch(batch: &WriteBatch) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(u32::try_from(batch.len()).unwrap().to_le_bytes());
    for mutation in batch.mutations() {
        let (tag, cf, key, value) = match mutation {
            Mutation::Put { cf, key, value } => (0, cf, key, Some(value)),
            Mutation::Delete { cf, key } => (1, cf, key, None),
        };
        out.push(tag);
        out.push(match cf {
            ColumnFamily::Default => 0,
            ColumnFamily::Lock => 1,
            ColumnFamily::Write => 2,
        });
        out.extend(u32::try_from(key.len()).unwrap().to_le_bytes());
        out.extend(key);
        if let Some(value) = value {
            out.extend(u32::try_from(value.len()).unwrap().to_le_bytes());
            out.extend(value);
        }
    }
    out
}
fn lower(commands: &[Command]) -> WriteBatch {
    let mut batch = WriteBatch::new();
    for cmd in commands {
        // All accepted fence outcomes are established against original prefix
        // state before any component is measured. No generic fence bypass API.
        let next = match cmd {
            Command::Fenced { inner, .. } => inner.to_write_batch(),
            _ => cmd.to_write_batch().unwrap(),
        };
        batch.append(next);
    }
    batch
}
fn flat_lower(commands: &[Command]) -> WriteBatch {
    let mut batch = WriteBatch::new();
    for command in commands {
        let ops = match command {
            Command::Fenced {
                inner: FencedInner::Write { ops },
                ..
            }
            | Command::Write { ops } => ops,
            Command::Put { cf, key, value } => {
                batch.put(kv9_raft::cf_from_code(*cf), key.clone(), value.clone());
                continue;
            }
            _ => panic!("direct prototype accepts only already-adjudicated Raw commands"),
        };
        for op in ops {
            match op {
                kv9_raft::KvOp::Put { cf, key, value } => {
                    batch.put(kv9_raft::cf_from_code(*cf), key.clone(), value.clone());
                }
                kv9_raft::KvOp::Delete { cf, key } => {
                    batch.delete(kv9_raft::cf_from_code(*cf), key.clone());
                }
            }
        }
    }
    batch
}

fn raw_command(cmd: &Command) -> bool {
    let Command::Fenced {
        inner: FencedInner::Write { ops },
        ..
    } = cmd
    else {
        return false;
    };
    ops.iter().all(|op| {
        let (cf, key) = match op {
            kv9_raft::KvOp::Put { cf, key, .. } | kv9_raft::KvOp::Delete { cf, key } => (cf, key),
        };
        *cf == 0
            && kv9_common::codec::decode_key(key).is_ok_and(|key| {
                key.mode == kv9_common::codec::KeyMode::Raw
                    && key.keyspace != kv9_common::KeyspaceId::SYSTEM
            })
    })
}
type Model = [BTreeMap<Vec<u8>, Vec<u8>>; 3];

fn cf_index(cf: ColumnFamily) -> usize {
    match cf {
        ColumnFamily::Default => 0,
        ColumnFamily::Lock => 1,
        ColumnFamily::Write => 2,
    }
}
fn model_apply(model: &mut Model, batch: &WriteBatch) {
    for mutation in batch.mutations() {
        match mutation {
            Mutation::Put { cf, key, value } => {
                // All selected Raw keys and semantic fixture keys fit the scan.
                assert!(key.as_slice() < [255u8].as_slice());
                model[cf_index(*cf)].insert(key.clone(), value.clone());
            }
            Mutation::Delete { cf, key } => {
                assert!(key.as_slice() < [255u8].as_slice());
                model[cf_index(*cf)].remove(key);
            }
        }
    }
}
fn check_view(view: &dyn ReadView, expected: &Model) {
    for cf in [
        ColumnFamily::Default,
        ColumnFamily::Lock,
        ColumnFamily::Write,
    ] {
        let rows = view
            .scan(cf, b"", &[255], expected[cf_index(cf)].len() + 1)
            .unwrap();
        assert_eq!(rows.len(), expected[cf_index(cf)].len());
        for ((key, value), (expected_key, expected_value)) in
            rows.iter().zip(expected[cf_index(cf)].iter())
        {
            assert_eq!(key, expected_key);
            assert_eq!(value, expected_value);
        }
    }
}
fn model_digest(model: &Model) -> String {
    let mut h = Sha256::new();
    for (cf, map) in model.iter().enumerate() {
        h.update([cf as u8]);
        h.update((map.len() as u64).to_le_bytes());
        for (key, value) in map {
            h.update((key.len() as u64).to_le_bytes());
            h.update(key);
            h.update((value.len() as u64).to_le_bytes());
            h.update(value);
        }
    }
    format!("{:x}", h.finalize())
}
struct Workload {
    name: String,
    commands: Vec<Vec<Command>>,
    initial: Model,
    expected: Model,
    group_payload_sha256: Vec<String>,
    mutations: usize,
}
fn workload(name: &str, groups: &[Group]) -> Workload {
    let mut commands: Vec<Vec<Command>> = groups.iter().map(|g| g.commands.clone()).collect();
    if name == "unique_insert" {
        let mut ordinal = 0u64;
        for group in &mut commands {
            for command in group {
                let Command::Fenced {
                    inner: FencedInner::Write { ops },
                    ..
                } = command
                else {
                    panic!("unexpected original command")
                };
                for op in ops {
                    let kv9_raft::KvOp::Put { cf, key, .. } = op else {
                        panic!("unique insertion requires the original all-Put corpus")
                    };
                    assert_eq!(*cf, 0);
                    ordinal += 1;
                    key.extend_from_slice(&ordinal.to_be_bytes());
                }
            }
        }
        assert_eq!(ordinal, 100096);
    } else {
        assert!(matches!(name, "overwrite" | "initial_fill"));
    }
    let mut initial: Model = Default::default();
    if name == "overwrite" {
        for group in &commands {
            for mutation in lower(group).mutations() {
                let Mutation::Put { cf, key, value } = mutation else {
                    panic!("original corpus must contain only Put")
                };
                let mut initial_value = value.clone();
                assert!(!initial_value.is_empty());
                initial_value[0] ^= 255;
                initial[cf_index(*cf)].insert(key.clone(), initial_value);
            }
        }
    }
    let mut expected = initial.clone();
    let mut payload_hashes = Vec::new();
    let mut mutations = 0;
    for group in &commands {
        assert!(group.iter().all(raw_command));
        let batch = lower(group);
        let payload = encode_batch(&batch);
        assert_eq!(payload, encode_batch(&flat_lower(group)));
        payload_hashes.push(hash(&payload));
        mutations += batch.len();
        model_apply(&mut expected, &batch);
    }
    if name == "unique_insert" {
        assert_eq!(expected[0].len(), mutations);
    }
    Workload {
        name: name.to_owned(),
        commands,
        initial,
        expected,
        group_payload_sha256: payload_hashes,
        mutations,
    }
}
fn fresh_engine(initial: &Model) -> MemEngine {
    let engine = MemEngine::new();
    let mut batch = WriteBatch::new();
    for cf in [
        ColumnFamily::Default,
        ColumnFamily::Lock,
        ColumnFamily::Write,
    ] {
        for (key, value) in &initial[cf_index(cf)] {
            batch.put(cf, key.clone(), value.clone());
        }
    }
    engine.write(batch).unwrap();
    engine
}
fn composed(engine: &MemEngine, commands: &[Command], at: AppliedPosition, direct: bool) {
    let batch = if direct {
        flat_lower(commands)
    } else {
        lower(commands)
    };
    engine.write_applied(batch, at).unwrap();
}
fn validate_workload(work: &Workload, groups: &[Group], pinned: bool) -> Value {
    let engines = [fresh_engine(&work.initial), fresh_engine(&work.initial)];
    let mut expected = work.initial.clone();
    for (i, (commands, group)) in work.commands.iter().zip(groups).enumerate() {
        let views: Vec<_> = engines
            .iter()
            .map(|engine| pinned.then(|| engine.try_resident_snapshot().unwrap()))
            .collect();
        let at = AppliedPosition {
            term: group.term,
            index: group.last,
        };
        for (direct, engine) in engines.iter().enumerate() {
            composed(engine, commands, at, direct == 1);
            assert_eq!(engine.volatile_applied_position(), Some(at));
            assert_eq!(engine.data_revision(), i as u64 + 1);
        }
        // The model still describes the state before this group.
        for view in views.iter().flatten() {
            check_view(view.as_ref(), &expected);
        }
        model_apply(&mut expected, &lower(commands));
        for engine in &engines {
            check_view(engine.try_resident_snapshot().unwrap().as_ref(), &expected);
        }
    }
    assert_eq!(expected, work.expected);
    json!({"complete":true,"workload":work.name,"pinned":pinned,
        "checked_group_prefixes":groups.len(),"checked_old_snapshots":if pinned {2*groups.len()} else {0},
        "checked_live_states":2*groups.len(),"final_keys":expected.iter().map(BTreeMap::len).sum::<usize>(),
        "final_state_sha256":model_digest(&expected)})
}
fn measure(work: &Workload, groups: &[Group], pinned: bool, direct: bool, passes: usize) -> Value {
    let mut samples = Vec::with_capacity(groups.len() * passes);
    for _ in 0..passes {
        let engine = fresh_engine(&work.initial);
        for (commands, group) in work.commands.iter().zip(groups) {
            // One owned pre-write view forces actual copy-on-write paths when enabled.
            let old = pinned.then(|| engine.try_resident_snapshot().unwrap());
            let at = AppliedPosition {
                term: group.term,
                index: group.last,
            };
            let start = Instant::now();
            composed(
                black_box(&engine),
                black_box(commands),
                at,
                black_box(direct),
            );
            let elapsed = start.elapsed().as_nanos() as u64;
            // write_applied consumed and destroyed the final WriteBatch inside timing.
            black_box(&old);
            samples.push(elapsed);
            drop(old);
        }
        check_view(
            engine.try_resident_snapshot().unwrap().as_ref(),
            &work.expected,
        );
        assert_eq!(
            engine.volatile_applied_position(),
            Some(AppliedPosition {
                term: groups.last().unwrap().term,
                index: groups.last().unwrap().last,
            })
        );
        assert_eq!(engine.data_revision(), groups.len() as u64);
    }
    let mut ordered = samples.clone();
    ordered.sort_unstable();
    let quantile = |p: usize| ordered[(ordered.len() * p).div_ceil(100) - 1];
    json!({"workload":work.name,"pinned":pinned,"component":if direct {"flat_lower"} else {"lower"},
        "passes":passes,"groups":samples.len(),"mutations":work.mutations*passes,
        "validated_final_states":passes,"final_state_sha256":model_digest(&work.expected),
        "timing":{"sum_ns":samples.iter().sum::<u64>(),"mean_group_ns":samples.iter().sum::<u64>() as f64/samples.len() as f64,
        "p50_group_ns":quantile(50),"p95_group_ns":quantile(95),"p99_group_ns":quantile(99),"samples_ns":samples}})
}
fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("artifact root"));
    let output = PathBuf::from(std::env::args().nth(2).expect("fresh result file"));
    assert!(!output.exists());
    let plan: Value =
        serde_json::from_slice(&std::fs::read(root.join("plan.json")).unwrap()).unwrap();
    let engine: Value =
        serde_json::from_slice(&std::fs::read(root.join("groups.json")).unwrap()).unwrap();
    let group_bytes = read_pinned(
        &root.join("groups.bin"),
        engine["group_file_bytes"].as_u64().unwrap(),
        engine["group_file_sha256"].as_str().unwrap(),
        32 * 1024 * 1024,
    );
    let mut groups = groups(&group_bytes);
    let source = &plan["raft_log"];
    let raft_bytes = read_pinned(
        Path::new(source["path"].as_str().unwrap()),
        source["bytes"].as_u64().unwrap(),
        source["sha256"].as_str().unwrap(),
        512 * 1024 * 1024,
    );
    let (entries, raft_summary) = read_raft(&raft_bytes);
    assert!(raft_summary["final_commit"].as_u64().unwrap() >= groups.last().unwrap().last);
    let store = Arc::new(MemEngine::new());
    let peer = Arc::new(SingleNodeRaft::new(NodeId(2), RegionId(0)));
    let node = Arc::new(
        Node::with_raft_and_engine(NodeId(2), Config::default(), peer, store.clone()).unwrap(),
    );
    let fence = Arc::new(CatalogFenceAdjudicator::new(node));
    assert!(fence.independent_of_raw_writes());
    let mut state = MemStateMachine::with_engine(store).unwrap();
    state.set_fence_adjudicator(fence.clone());
    let mut prefix = 0;
    for (_, entry) in entries.range(..=groups[0].previous) {
        let command = normal_command(entry);
        state
            .apply_at(
                AppliedPosition {
                    term: entry.term,
                    index: entry.index,
                },
                &command,
            )
            .unwrap();
        prefix += 1;
    }
    let mut joins = Vec::new();
    for group in &mut groups {
        for index in group.previous + 1..=group.last {
            let entry = &entries[&index];
            assert_eq!(entry.term, group.term);
            assert_eq!(entry.get_entry_type(), EntryType::EntryNormal);
            let command = Command::decode_for_harness(&entry.data).unwrap();
            assert!(raw_command(&command));
            let Command::Fenced {
                fence: proposed, ..
            } = &command
            else {
                unreachable!()
            };
            assert!(fence.is_fresh(proposed).unwrap());
            assert_eq!(command.encode(), entry.data);
            joins.push(json!({"index":index,"term":entry.term,"command_bytes":entry.data.len(),"command_sha256":hash(&entry.data),"region_id":proposed.region_id,"conf_ver":proposed.conf_ver,"version":proposed.version}));
            group.wire.push(entry.data.to_vec());
            group.commands.push(command);
        }
        assert_eq!(
            encode_batch(&lower(&group.commands)),
            group.payload,
            "actual command/group payload join"
        );
    }
    assert_eq!(joins.len(), 1564);
    for group in &groups {
        assert_eq!(
            encode_batch(&flat_lower(&group.commands)),
            group.payload,
            "direct append changed original group bytes"
        );
    }
    let workloads: Vec<_> = plan["workloads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| workload(name.as_str().unwrap(), &groups))
        .collect();
    let mut checks = Vec::new();
    let mut workload_metadata = Vec::new();
    for work in &workloads {
        workload_metadata.push(json!({"name":work.name,"groups":work.commands.len(),
            "mutations":work.mutations,"initial_keys":work.initial.iter().map(BTreeMap::len).sum::<usize>(),
            "final_keys":work.expected.iter().map(BTreeMap::len).sum::<usize>(),
            "initial_state_sha256":model_digest(&work.initial),"final_state_sha256":model_digest(&work.expected),
            "group_payload_sha256":work.group_payload_sha256}));
        for pinned in [false, true] {
            checks.push(validate_workload(work, &groups, pinned));
        }
    }
    println!(
        "{}",
        json!({"event":"all_composed_correctness_checks_passed","cases":checks.len()})
    );
    let mut rows = Vec::new();
    for work in &workloads {
        for pinned in [false, true] {
            for direct in [false, true] {
                black_box(measure(work, &groups, pinned, direct, 1));
            }
            for (order, component) in plan["orders"].as_array().unwrap().iter().enumerate() {
                let direct = match component.as_str().unwrap() {
                    "lower" => false,
                    "flat_lower" => true,
                    _ => panic!("unknown planned component"),
                };
                let mut row = measure(
                    work,
                    &groups,
                    pinned,
                    direct,
                    plan["passes"].as_u64().unwrap() as usize,
                );
                row["order"] = json!(order);
                println!(
                    "{}",
                    json!({"event":"timed_row_complete","workload":work.name,"pinned":pinned,
                    "order":order,"component":component,"mean_group_ns":row["timing"]["mean_group_ns"]})
                );
                rows.push(row);
            }
        }
    }
    let result = json!({"complete":true,"groups":groups.len(),"commands":joins.len(),"mutations":100096,
        "prefix_entries_replayed":prefix,"all_selected_original_fences_fresh":true,
        "all_original_group_payloads_byte_identical":true,"raft":raft_summary,
        "input_plan_sha256":hash(&std::fs::read(root.join("plan.json")).unwrap()),
        "group_file_sha256":hash(&group_bytes),"joins":joins,"workloads":workload_metadata,
        "correctness":checks,"warmup_rows":12,"rows":rows,"scope":plan["scope"],"timed_scope":plan["timed_scope"]});
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    serde_json::to_writer_pretty(&mut file, &result).unwrap();
    println!(
        "{}",
        json!({"complete":true,"rows":24,"correctness_cases":6})
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composed_preserves_old_views_and_refuses_repeated_position_before_mutation() {
        use kv9_raft::KvOp;
        let mut first = Vec::new();
        let mut second = Vec::new();
        for cf in 0..=2 {
            first.push(KvOp::Put {
                cf,
                key: vec![],
                value: vec![cf],
            });
            first.push(KvOp::Put {
                cf,
                key: vec![0, 255],
                value: vec![1, 0, 2],
            });
            second.push(KvOp::Delete { cf, key: vec![] });
            second.push(KvOp::Put {
                cf,
                key: vec![0, 255],
                value: vec![],
            });
        }
        let commands = [
            vec![Command::Write { ops: first }],
            vec![Command::Write { ops: second }],
        ];
        for direct in [false, true] {
            let engine = MemEngine::new();
            let mut expected: Model = Default::default();
            for (i, group) in commands.iter().enumerate() {
                let old = engine.try_resident_snapshot().unwrap();
                let at = AppliedPosition {
                    term: 1,
                    index: i as u64 + 1,
                };
                composed(&engine, group, at, direct);
                check_view(old.as_ref(), &expected);
                model_apply(&mut expected, &lower(group));
                check_view(engine.try_resident_snapshot().unwrap().as_ref(), &expected);
                let refused = [Command::Put {
                    cf: 0,
                    key: b"forbidden".to_vec(),
                    value: b"partial".to_vec(),
                }];
                let batch = if direct {
                    flat_lower(&refused)
                } else {
                    lower(&refused)
                };
                assert!(engine.write_applied(batch, at).is_err());
                check_view(engine.try_resident_snapshot().unwrap().as_ref(), &expected);
                check_view(old.as_ref(), &{
                    let mut before: Model = Default::default();
                    for prior in &commands[..i] {
                        model_apply(&mut before, &lower(prior));
                    }
                    before
                });
                assert_eq!(engine.volatile_applied_position(), Some(at));
                assert_eq!(engine.data_revision(), i as u64 + 1);
            }
        }
    }
    #[test]
    fn direct_append_preserves_order_deletes_empty_inputs_and_all_column_families() {
        use kv9_raft::{KvOp, RegionFence};
        let empty = Command::Write { ops: vec![] };
        let mut commands = vec![empty.clone()];
        for cf in 0..=2 {
            commands.push(Command::Put {
                cf,
                key: vec![],
                value: vec![0],
            });
            commands.push(Command::Write {
                ops: vec![
                    KvOp::Put {
                        cf,
                        key: vec![0, 255],
                        value: vec![1, 0, 2],
                    },
                    KvOp::Delete { cf, key: vec![] },
                    KvOp::Put {
                        cf,
                        key: vec![0, 255],
                        value: vec![],
                    },
                ],
            });
            commands.push(Command::Fenced {
                fence: RegionFence {
                    region_id: 100,
                    conf_ver: 1,
                    version: 1,
                },
                inner: FencedInner::Write {
                    ops: vec![KvOp::Delete {
                        cf,
                        key: vec![0, 255],
                    }],
                },
            });
        }
        commands.push(empty);
        for length in 0..=commands.len() {
            assert_eq!(
                encode_batch(&flat_lower(&commands[..length])),
                encode_batch(&lower(&commands[..length]))
            );
        }
    }
}
