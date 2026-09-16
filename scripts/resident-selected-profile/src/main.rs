//! Bounded offline CPU diagnosis of the unchanged production MemEngine.
//! Recorded apply spans exclude input cloning, map setup and correctness checks.
use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kv9_common::AppliedPosition;
use kv9_engine::{ColumnFamily, Engine, MemEngine, ReadView, ReplicatedEngine, WriteBatch};
use serde_json::json;
use sha2::{Digest, Sha256};

#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
type Pair = (Vec<u8>, Vec<u8>);
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest(rows: &[Pair]) -> String {
    let mut h = Sha256::new();
    h.update((rows.len() as u64).to_le_bytes());
    for (k, v) in rows {
        h.update((k.len() as u64).to_le_bytes());
        h.update(k);
        h.update((v.len() as u64).to_le_bytes());
        h.update(v);
    }
    format!("{:x}", h.finalize())
}
fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: ts is a valid writable timespec for the entire synchronous call.
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) },
        0
    );
    u64::try_from(ts.tv_sec).unwrap() * 1_000_000_000 + u64::try_from(ts.tv_nsec).unwrap()
}
fn save(path: &Path, value: &serde_json::Value) {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    serde_json::to_writer(file, value).unwrap();
}
fn corpus(bytes: &[u8], unique: bool) -> Vec<Vec<Pair>> {
    assert_eq!(
        sha(bytes),
        "d978ba9e49131f6277f975ecd333c33854845cc03bd14ee57a22fff915a08350"
    );
    assert_eq!(&bytes[..8], b"KV9GRP01");
    let mut offset = 8;
    let mut previous_end = 14221;
    let mut ordinal = 0u64;
    let mut groups = Vec::new();
    fn u32le(bytes: &[u8], offset: &mut usize) -> usize {
        let n = u32::from_le_bytes(bytes[*offset..*offset + 4].try_into().unwrap()) as usize;
        *offset += 4;
        n
    }
    while offset < bytes.len() {
        let previous = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        let last = u64::from_le_bytes(bytes[offset + 8..offset + 16].try_into().unwrap());
        let term = u64::from_le_bytes(bytes[offset + 16..offset + 24].try_into().unwrap());
        offset += 24;
        assert_eq!(previous, previous_end);
        assert!(last > previous);
        assert_eq!(term, 1);
        previous_end = last;
        let size = u32le(bytes, &mut offset);
        let end = offset + size;
        assert!(end <= bytes.len());
        let count = u32le(bytes, &mut offset);
        let mut group = Vec::with_capacity(count);
        for _ in 0..count {
            assert_eq!(&bytes[offset..offset + 2], &[0, 0]);
            offset += 2;
            let len = u32le(bytes, &mut offset);
            let mut key = bytes[offset..offset + len].to_vec();
            offset += len;
            let len = u32le(bytes, &mut offset);
            let value = bytes[offset..offset + len].to_vec();
            offset += len;
            assert!(offset <= end && key.first() == Some(&b'r') && !value.is_empty());
            ordinal += 1;
            if unique {
                key.extend_from_slice(&ordinal.to_be_bytes());
            }
            group.push((key, value));
        }
        assert_eq!(offset, end);
        groups.push(group);
    }
    assert_eq!(groups.len(), 106);
    assert_eq!(ordinal, 100096);
    assert_eq!(previous_end, 15785);
    groups
}
fn batch(rows: &[Pair]) -> WriteBatch {
    let mut batch = WriteBatch::new();
    for (k, v) in rows {
        batch.put(ColumnFamily::Default, k.clone(), v.clone());
    }
    batch
}
fn rows(view: &dyn ReadView) -> Vec<Pair> {
    view.scan(ColumnFamily::Default, b"", &[255], 100097)
        .unwrap()
}
#[inline(never)]
fn selected_apply(engine: &MemEngine, batch: WriteBatch, at: AppliedPosition) {
    engine.write_applied(batch, at).unwrap();
    // Keep an actual outer boundary after the call rather than a tail call.
    black_box(engine);
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        6,
        "root fresh-output overwrite|unique_insert pinned|unpinned passes"
    );
    let root = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    assert!(output.is_dir());
    let unique = match args[3].as_str() {
        "overwrite" => false,
        "unique_insert" => true,
        _ => panic!("invalid workload"),
    };
    let pinned = match args[4].as_str() {
        "pinned" => true,
        "unpinned" => false,
        _ => panic!("invalid snapshot mode"),
    };
    let passes: usize = args[5].parse().unwrap();
    assert!(passes > 0 && passes <= 300);
    let bytes = std::fs::read(root.join("groups.bin")).unwrap();
    let groups = corpus(&bytes, unique);
    let mut final_model = BTreeMap::new();
    for (k, v) in groups.iter().flatten() {
        final_model.insert(k.clone(), v.clone());
    }
    assert_eq!(final_model.len(), if unique { 100096 } else { 4096 });
    let initial: Vec<Pair> = if unique {
        vec![]
    } else {
        final_model
            .iter()
            .map(|(k, v)| {
                let mut v = v.clone();
                v[0] ^= 255;
                (k.clone(), v)
            })
            .collect()
    };
    let final_rows: Vec<Pair> = final_model.into_iter().collect();
    // Every prefix and old view is checked once before recording. No production code changes.
    {
        let engine = MemEngine::new();
        engine.write(batch(&initial)).unwrap();
        let mut model: BTreeMap<_, _> = initial.iter().cloned().collect();
        for (i, group) in groups.iter().enumerate() {
            let old = pinned.then(|| engine.snapshot().unwrap());
            selected_apply(
                &engine,
                batch(group),
                AppliedPosition {
                    term: 1,
                    index: i as u64 + 1,
                },
            );
            if let Some(old) = old {
                assert_eq!(
                    rows(old.as_ref()),
                    model
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<Vec<_>>()
                );
            }
            for (k, v) in group {
                model.insert(k.clone(), v.clone());
            }
            assert_eq!(
                rows(engine.snapshot().unwrap().as_ref()),
                model
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(engine.data_revision(), 106);
    }
    save(
        &output.join("ready.json"),
        &json!({"pid":std::process::id(),"workload":args[3],"pinned":pinned,"passes":passes,"input_sha256":sha(&bytes),"final_state_sha256":digest(&final_rows),"final_keys":final_rows.len(),"prefix_states_checked":106,"old_views_checked":if pinned{106}else{0},"monotonic_ns":monotonic_ns()}),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while !output.join("go").exists() {
        assert!(Instant::now() < deadline, "profiler gate timeout");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut spans = Vec::with_capacity(passes * groups.len());
    for pass in 0..passes {
        let engine = MemEngine::new();
        engine.write(batch(&initial)).unwrap();
        for (i, group) in groups.iter().enumerate() {
            let old = pinned.then(|| engine.snapshot().unwrap());
            let pending = batch(group);
            let start = monotonic_ns();
            selected_apply(
                black_box(&engine),
                pending,
                AppliedPosition {
                    term: 1,
                    index: i as u64 + 1,
                },
            );
            let end = monotonic_ns();
            assert!(start < end);
            spans.push([pass as u64, i as u64, start, end]);
            black_box(&old);
            drop(old);
        }
        assert_eq!(engine.data_revision(), 106);
        assert_eq!(
            engine.volatile_applied_position(),
            Some(AppliedPosition {
                term: 1,
                index: 106
            })
        );
        assert_eq!(rows(engine.snapshot().unwrap().as_ref()), final_rows);
    }
    save(
        &output.join("result.json"),
        &json!({"complete":true,"pid":std::process::id(),"workload":args[3],"pinned":pinned,"passes":passes,"spans":spans,"final_state_sha256":digest(&final_rows),"final_keys":final_rows.len(),"mutations":100096*passes,"scope":"Offline unchanged MemEngine::write_applied calls, not a server or QPS benchmark. Inputs cloned, snapshots created/dropped, maps initialized and final states checked outside recorded apply spans. Each pass uses a fresh map."}),
    );
}
