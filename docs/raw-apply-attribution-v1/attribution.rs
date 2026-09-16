//! Offline component attribution. Never serves requests or opens storage for writing.
use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use kv9_common::{AppliedPosition, Config, NodeId, RegionId};
use kv9_engine::{ColumnFamily, MemEngine, Mutation, WriteBatch};
use kv9_raft::{
    Command, FenceAdjudicator, FencedInner, MemStateMachine, SingleNodeRaft, StateMachine,
};
use kv9_server::{fence::CatalogFenceAdjudicator, Node};
use protobuf::Message;
use raft::prelude::{ConfState, Entry, EntryType, HardState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[cfg(not(feature = "allocation-counting"))]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(feature = "allocation-counting")]
mod counting {
    use std::alloc::{GlobalAlloc, Layout};
    use std::cell::Cell;
    thread_local! {
        static COUNTS: Cell<Option<[u64; 6]>> = const { Cell::new(None) };
    }
    pub struct Allocator;
    fn record(kind: usize, bytes: usize) {
        COUNTS.with(|slot| {
            if let Some(mut counters) = slot.get() {
                counters[kind] += 1;
                counters[kind + 1] += bytes as u64;
                slot.set(Some(counters));
            }
        });
    }
    // This wrapper exists only in the separately built counting executable.
    // Every allocation and deallocation uses the same jemalloc/layout contract.
    unsafe impl GlobalAlloc for Allocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let p = unsafe { tikv_jemallocator::Jemalloc.alloc(layout) };
            if !p.is_null() {
                record(0, layout.size());
            }
            p
        }
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let p = unsafe { tikv_jemallocator::Jemalloc.alloc_zeroed(layout) };
            if !p.is_null() {
                record(0, layout.size());
            }
            p
        }
        unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
            record(4, layout.size());
            unsafe { tikv_jemallocator::Jemalloc.dealloc(p, layout) }
        }
        unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            let next = unsafe { tikv_jemallocator::Jemalloc.realloc(p, layout, size) };
            if !next.is_null() {
                record(2, size);
            }
            next
        }
    }
    pub fn start() {
        COUNTS.with(|s| {
            assert!(s.get().is_none());
            s.set(Some([0; 6]));
        });
    }
    pub fn stop() -> [u64; 6] {
        COUNTS.with(|s| s.replace(None).unwrap())
    }
}
#[cfg(feature = "allocation-counting")]
#[global_allocator]
static ALLOCATOR: counting::Allocator = counting::Allocator;
fn start_counts() {
    #[cfg(feature = "allocation-counting")]
    counting::start();
}
fn stop_counts() -> [u64; 6] {
    #[cfg(feature = "allocation-counting")]
    {
        counting::stop()
    }
    #[cfg(not(feature = "allocation-counting"))]
    {
        [0; 6]
    }
}

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
fn measure(
    component: &str,
    groups: &[Group],
    fence: &impl FenceAdjudicator,
    passes: usize,
) -> Value {
    let mut samples = Vec::with_capacity(groups.len() * passes);
    let mut counts = [0u64; 6];
    for _ in 0..passes {
        for group in groups {
            // Output container capacity is setup; payload construction is timed.
            let mut decoded = Vec::with_capacity(group.commands.len());
            start_counts();
            let start = Instant::now();
            let output = match component {
                "decode" => {
                    for bytes in &group.wire {
                        decoded.push(Command::decode_for_harness(black_box(bytes)).unwrap());
                    }
                    black_box(&decoded);
                    None
                }
                "lower" => Some(black_box(lower(black_box(&group.commands)))),
                "fence" => {
                    for command in &group.commands {
                        let Command::Fenced {
                            fence: proposed, ..
                        } = command
                        else {
                            unreachable!()
                        };
                        assert!(black_box(fence.is_fresh(black_box(proposed)).unwrap()));
                    }
                    None
                }
                _ => unreachable!(),
            };
            let elapsed = start.elapsed().as_nanos() as u64;
            let observed = stop_counts();
            for (sum, value) in counts.iter_mut().zip(observed) {
                *sum += value;
            }
            samples.push(elapsed);
            // Result destruction is outside the interval and counting window.
            drop(output);
            drop(decoded);
        }
    }
    let mut ordered = samples.clone();
    ordered.sort_unstable();
    let quantile = |p: usize| ordered[(ordered.len() * p).div_ceil(100) - 1];
    let mut result = json!({"component": component, "passes": passes, "groups": samples.len(), "commands": groups.iter().map(|g|g.commands.len()).sum::<usize>() * passes, "mutations": 100096 * passes, "counting_build": cfg!(feature="allocation-counting")});
    if cfg!(feature = "allocation-counting") {
        result["allocation_counts"] = json!({"alloc_calls":counts[0],"alloc_requested_bytes":counts[1],"realloc_calls":counts[2],"realloc_requested_bytes":counts[3],"dealloc_calls_inside_interval":counts[4],"dealloc_layout_bytes_inside_interval":counts[5]});
    } else {
        result["timing"] = json!({"sum_ns":samples.iter().sum::<u64>(),"mean_group_ns":samples.iter().sum::<u64>() as f64/samples.len() as f64,"p50_group_ns":quantile(50),"p95_group_ns":quantile(95),"p99_group_ns":quantile(99),"samples_ns":samples});
    }
    result
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
    // These measurements are components on a fixed catalog image, not complete
    // apply or database latency. Group eligibility/position checks, WAL, index,
    // Raft, RPC, queues and final output destruction are not included.
    for component in ["decode", "lower", "fence"] {
        black_box(measure(component, &groups, fence.as_ref(), 1));
    }
    let rows: Vec<_> = plan["orders"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(order, component)| {
            let mut row = measure(
                component.as_str().unwrap(),
                &groups,
                fence.as_ref(),
                plan["passes"].as_u64().unwrap() as usize,
            );
            row["order"] = json!(order);
            row
        })
        .collect();
    let result = json!({"complete":true,"counting_build":cfg!(feature="allocation-counting"),"groups":groups.len(),"commands":joins.len(),"mutations":100096,"prefix_entries_replayed":prefix,"all_selected_fences_fresh":true,"all_group_payloads_byte_identical":true,"raft":raft_summary,"input_plan_sha256":hash(&std::fs::read(root.join("plan.json")).unwrap()),"group_file_sha256":hash(&group_bytes),"joins":joins,"rows":rows,"scope":"Offline production command decode, accepted lowering/append and catalog-fence components on a fixed original catalog prefix. No complete apply/engine/WAL/RPC/queue timing or database QPS. Final output destruction is outside timing; allocations in production callees remain included. Counting executable supplies no latency result. Protocol entries supply no user-data mutation and do not reconstruct membership authority."});
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    serde_json::to_writer_pretty(&mut file, &result).unwrap();
    println!(
        "{}",
        json!({"complete":true,"groups":groups.len(),"commands":joins.len(),"mutations":100096,"counting_build":cfg!(feature="allocation-counting")})
    );
}
