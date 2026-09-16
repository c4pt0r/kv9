//! Fixed-corpus index comparison. No server, WAL or Raft workload is launched.
use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::PathBuf;
#[cfg(not(feature = "allocation-counting"))]
use std::time::Instant;

mod probes;
use rpds::RedBlackTreeMapSync;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[cfg(feature = "allocation-counting")]
mod counting;
#[cfg(feature = "allocation-counting")]
#[global_allocator]
static ALLOCATOR: counting::Allocator = counting::Allocator;
#[cfg(not(feature = "allocation-counting"))]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

type Pair = (Vec<u8>, Vec<u8>);
type Model = BTreeMap<Vec<u8>, Vec<u8>>;
trait Index: Clone + Default {
    fn put(&mut self, key: Vec<u8>, value: Vec<u8>);
    fn get(&self, key: &[u8]) -> Option<&Vec<u8>>;
    fn predecessor(&self, key: &[u8]) -> Option<(&Vec<u8>, &Vec<u8>)>;
    fn scan(&self, start: &[u8], end: &[u8], limit: usize) -> Vec<Pair>;
    fn rows(&self) -> Vec<Pair>;
    fn validate(&self);
}

impl Index for RedBlackTreeMapSync<Vec<u8>, Vec<u8>> {
    fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.insert_mut(key, value);
    }
    fn get(&self, key: &[u8]) -> Option<&Vec<u8>> {
        self.get(key)
    }
    fn predecessor(&self, key: &[u8]) -> Option<(&Vec<u8>, &Vec<u8>)> {
        self.range(..=key.to_vec()).next_back()
    }
    fn scan(&self, start: &[u8], end: &[u8], limit: usize) -> Vec<Pair> {
        self.range(start.to_vec()..end.to_vec())
            .take(limit)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
    fn rows(&self) -> Vec<Pair> {
        self.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
    fn validate(&self) {}
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest(rows: &[Pair]) -> String {
    let mut h = Sha256::new();
    h.update((rows.len() as u64).to_le_bytes());
    for (key, value) in rows {
        h.update((key.len() as u64).to_le_bytes());
        h.update(key);
        h.update((value.len() as u64).to_le_bytes());
        h.update(value);
    }
    format!("{:x}", h.finalize())
}
fn corpus(data: &[u8]) -> Vec<Vec<Pair>> {
    assert_eq!(
        hash(data),
        "d978ba9e49131f6277f975ecd333c33854845cc03bd14ee57a22fff915a08350"
    );
    assert_eq!(&data[..8], b"KV9GRP01");
    let mut offset = 8;
    let mut previous_end = 14221;
    let mut groups = Vec::new();
    fn u32_at(data: &[u8], offset: &mut usize) -> usize {
        let n = u32::from_le_bytes(data[*offset..*offset + 4].try_into().unwrap()) as usize;
        *offset += 4;
        n
    }
    while offset < data.len() {
        let previous = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        let last = u64::from_le_bytes(data[offset + 8..offset + 16].try_into().unwrap());
        let term = u64::from_le_bytes(data[offset + 16..offset + 24].try_into().unwrap());
        offset += 24;
        assert_eq!(previous, previous_end);
        assert!(last > previous);
        assert_eq!(term, 1);
        previous_end = last;
        let length = u32_at(data, &mut offset);
        let end = offset + length;
        assert!(end <= data.len());
        let count = u32_at(data, &mut offset);
        let mut group = Vec::with_capacity(count);
        for _ in 0..count {
            assert_eq!(&data[offset..offset + 2], &[0, 0]);
            offset += 2;
            let keylen = u32_at(data, &mut offset);
            let key = data[offset..offset + keylen].to_vec();
            offset += keylen;
            let vallen = u32_at(data, &mut offset);
            let value = data[offset..offset + vallen].to_vec();
            offset += vallen;
            assert!(key.len() >= 4 && key[0] == b'r' && offset <= end);
            assert!(!value.is_empty());
            group.push((key, value));
        }
        assert_eq!(offset, end);
        groups.push(group);
    }
    assert_eq!(groups.len(), 106);
    assert_eq!(previous_end, 15785);
    assert_eq!(groups.iter().map(Vec::len).sum::<usize>(), 100096);
    groups
}
struct Workload {
    dataset: &'static str,
    name: &'static str,
    groups: Vec<Vec<Pair>>,
    initial: Vec<Pair>,
    final_rows: Vec<Pair>,
}
fn workload(dataset: &'static str, name: &'static str, source: &[Vec<Pair>]) -> Workload {
    let mut groups = source.to_vec();
    if name == "unique_insert" {
        for (i, (key, _)) in groups.iter_mut().flatten().enumerate() {
            key.extend_from_slice(&(i as u64 + 1).to_be_bytes());
        }
    }
    let mut model = Model::new();
    if name == "overwrite" {
        for (key, value) in groups.iter().flatten() {
            let mut value = value.clone();
            value[0] ^= 255;
            model.insert(key.clone(), value);
        }
    }
    let initial = model.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (key, value) in groups.iter().flatten() {
        model.insert(key.clone(), value.clone());
    }
    if name == "unique_insert" {
        assert_eq!(model.len(), 100096);
    }
    Workload {
        dataset,
        name,
        groups,
        initial,
        final_rows: model.into_iter().collect(),
    }
}
fn populate<M: Index>(rows: &[Pair]) -> M {
    let mut map = M::default();
    for (key, value) in rows {
        map.put(key.clone(), value.clone());
    }
    map
}
fn check_workload<M: Index>(work: &Workload, pinned: bool) {
    let mut map = populate::<M>(&work.initial);
    let mut expected: Model = work.initial.iter().cloned().collect();
    for group in &work.groups {
        let old = pinned.then(|| map.clone());
        for (key, value) in group {
            map.put(key.clone(), value.clone());
        }
        if let Some(old) = old {
            assert_eq!(
                old.rows(),
                expected
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<Vec<_>>()
            );
        }
        for (key, value) in group {
            expected.insert(key.clone(), value.clone());
        }
        assert_eq!(
            map.rows(),
            expected
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Vec<_>>()
        );
        map.validate();
    }
    assert_eq!(map.rows(), work.final_rows);
}

#[cfg(feature = "allocation-counting")]
struct Stamp;
#[cfg(not(feature = "allocation-counting"))]
type Stamp = Instant;
fn begin() -> Stamp {
    #[cfg(feature = "allocation-counting")]
    {
        counting::start();
        Stamp
    }
    #[cfg(not(feature = "allocation-counting"))]
    {
        Instant::now()
    }
}
#[derive(Default)]
struct Metrics {
    samples: Vec<u64>,
    counts: [u64; 6],
    windows: usize,
}
impl Metrics {
    fn new(windows: usize) -> Self {
        Self {
            samples: if cfg!(feature = "allocation-counting") {
                vec![]
            } else {
                Vec::with_capacity(windows)
            },
            ..Self::default()
        }
    }
    fn end(&mut self, stamp: Stamp) {
        #[cfg(feature = "allocation-counting")]
        {
            let _ = stamp;
            for (sum, n) in self.counts.iter_mut().zip(counting::stop()) {
                *sum += n;
            }
        }
        #[cfg(not(feature = "allocation-counting"))]
        {
            self.samples.push(stamp.elapsed().as_nanos() as u64);
        }
        self.windows += 1;
    }
    fn report(self) -> Value {
        if cfg!(feature = "allocation-counting") {
            json!({"windows":self.windows,"alloc_calls":self.counts[0],"alloc_requested_bytes":self.counts[1],
                "realloc_calls":self.counts[2],"realloc_requested_bytes":self.counts[3],
                "dealloc_calls":self.counts[4],"dealloc_layout_bytes":self.counts[5]})
        } else {
            assert_eq!(self.windows, self.samples.len());
            let mut sorted = self.samples.clone();
            sorted.sort_unstable();
            let n = sorted.len();
            json!({"windows":n,"sum_ns":self.samples.iter().sum::<u64>(),"mean_ns":self.samples.iter().sum::<u64>() as f64/n as f64,
                "p50_ns":sorted[(n*50).div_ceil(100)-1],"p95_ns":sorted[(n*95).div_ceil(100)-1],"p99_ns":sorted[(n*99).div_ceil(100)-1],"samples_ns":self.samples})
        }
    }
}
fn write_case<M: Index>(work: &Workload, pinned: bool, passes: usize) -> Value {
    let mut metrics = Metrics::new(passes * work.groups.len());
    #[cfg(feature = "allocation-counting")]
    let mut memory = (0i64, 0i64, 0i64);
    for _ in 0..passes {
        #[cfg(feature = "allocation-counting")]
        let baseline = counting::live();
        let mut map = populate::<M>(&work.initial);
        #[cfg(feature = "allocation-counting")]
        {
            memory.0 = memory.0.max(counting::live() - baseline);
        }
        for group in &work.groups {
            let old = pinned.then(|| map.clone());
            let stamp = begin();
            for (key, value) in black_box(group) {
                map.put(key.clone(), value.clone());
            }
            metrics.end(stamp);
            #[cfg(feature = "allocation-counting")]
            {
                memory.1 = memory.1.max(counting::peak() - baseline);
            }
            black_box(&old);
            drop(old);
        }
        #[cfg(feature = "allocation-counting")]
        {
            memory.2 = memory.2.max(counting::live() - baseline);
        }
        assert_eq!(map.rows(), work.final_rows);
        map.validate();
        drop(map);
        #[cfg(feature = "allocation-counting")]
        assert_eq!(
            counting::live(),
            baseline,
            "all map/snapshot requested bytes must be released"
        );
    }
    let mut row = json!({"operation":"write","workload":work.name,"dataset":work.dataset,"pinned":pinned,"passes":passes,
        "mutations":100096*passes,"final_state_sha256":digest(&work.final_rows),"metrics":metrics.report()});
    #[cfg(feature = "allocation-counting")]
    {
        row["requested_live_bytes"] = json!({"initial_map":memory.0,"peak_during_group_with_optional_old_view":memory.1,"final_map":memory.2,"released_after_drop":true});
    }
    row["final_keys"] = json!(work.final_rows.len());
    row
}

enum Answer<'a> {
    Point(Option<&'a Vec<u8>>),
    Pair(Option<(&'a Vec<u8>, &'a Vec<u8>)>),
    Rows(Vec<Pair>),
}
impl Answer<'_> {
    fn observe(&self) {
        match self {
            Self::Point(p) => {
                black_box(p);
            }
            Self::Pair(p) => {
                black_box(p);
            }
            Self::Rows(p) => {
                black_box(p);
            }
        }
    }
    fn owned(&self) -> Vec<Pair> {
        match self {
            Self::Point(Some(value)) => vec![(vec![], (*value).clone())],
            Self::Point(None) => vec![],
            Self::Pair(Some((key, value))) => vec![((*key).clone(), (*value).clone())],
            Self::Pair(None) => vec![],
            Self::Rows(rows) => rows.clone(),
        }
    }
}
fn answer<'a, M: Index>(map: &'a M, op: &str, key: &[u8]) -> Answer<'a> {
    match op {
        "get_hit" | "get_miss" => Answer::Point(map.get(key)),
        "predecessor" => Answer::Pair(map.predecessor(key)),
        "scan16" => Answer::Rows(map.scan(key, &[255], 16)),
        _ => unreachable!(),
    }
}
fn queries(work: &Workload, op: &str) -> Vec<Vec<u8>> {
    let model: Model = work.final_rows.iter().cloned().collect();
    probes::indices(work.final_rows.len(), 512)
        .into_iter()
        .enumerate()
        .map(|(i, index)| {
            let mut key = work.final_rows[index].0.clone();
            if op == "get_miss" || (op == "predecessor" && i % 2 == 1) {
                key.push(255);
                while model.contains_key(&key) {
                    key.push(255);
                }
            }
            key
        })
        .collect()
}
fn read_panels<M: Index>(
    work: &Workload,
    op: &str,
    probes: &[Vec<u8>],
    epochs: usize,
    warm_passes: usize,
    warmup_passes: usize,
) -> Vec<Value> {
    let model: Model = work.final_rows.iter().cloned().collect();
    let expected: Vec<Vec<Pair>> = probes
        .iter()
        .map(|key| match op {
            "get_hit" | "get_miss" => model
                .get(key)
                .map(|value| vec![(vec![], value.clone())])
                .unwrap_or_default(),
            "predecessor" => model
                .range(..=key.clone())
                .next_back()
                .map(|(k, v)| vec![(k.clone(), v.clone())])
                .unwrap_or_default(),
            "scan16" => model
                .range(key.clone()..vec![255])
                .take(16)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            _ => unreachable!(),
        })
        .collect();
    let mut first = Metrics::new(epochs * probes.len());
    let mut warm = Metrics::new(epochs * warm_passes * probes.len());
    for _ in 0..epochs {
        // Construction touches nodes, but no probe is issued on this map before
        // its first measured pass. This is not an OS/hardware cache flush.
        let map = populate::<M>(&work.final_rows);
        for key in probes {
            let stamp = begin();
            let output = answer(black_box(&map), op, black_box(key));
            output.observe();
            first.end(stamp);
            drop(output);
        }
        for (key, expected) in probes.iter().zip(&expected) {
            assert_eq!(answer(&map, op, key).owned(), *expected);
        }
        // Fixed untimed passes and warm measurements use this same live map.
        for _ in 0..warmup_passes {
            for key in probes {
                let output = answer(black_box(&map), op, black_box(key));
                output.observe();
                drop(output);
            }
        }
        for _ in 0..warm_passes {
            for key in probes {
                let stamp = begin();
                let output = answer(black_box(&map), op, black_box(key));
                output.observe();
                warm.end(stamp);
                drop(output);
            }
        }
        assert_eq!(map.rows(), work.final_rows);
    }
    [("first_touch", 1, first), ("warm", warm_passes, warm)].into_iter().map(|(phase, passes_per_epoch, metrics)| {
        json!({"operation":op,"workload":work.name,"dataset":work.dataset,"pinned":false,
            "phase":phase,"epochs":epochs,"passes_per_epoch":passes_per_epoch,"passes":epochs*passes_per_epoch,
            "warmup_passes":warmup_passes,"queries":probes.len(),"keys":work.final_rows.len(),
            "query_sha256":digest(&probes.iter().map(|k|(k.clone(),vec![])).collect::<Vec<_>>()),
            "metrics":metrics.report(),"all_query_outputs_checked":true})
    }).collect()
}

fn rekey(dataset: &str, source: &[Vec<Pair>]) -> Vec<Vec<Pair>> {
    if dataset == "original24" {
        return source.to_vec();
    }
    let keys: std::collections::BTreeSet<_> =
        source.iter().flatten().map(|(k, _)| k.clone()).collect();
    let translated: BTreeMap<_, _> = keys
        .into_iter()
        .enumerate()
        .map(|(rank, key)| {
            let digest = Sha256::digest(&key);
            let mut new = if dataset == "short4" {
                b"raw:".to_vec()
            } else {
                assert_eq!(dataset, "dispersed0");
                Vec::new()
            };
            new.extend_from_slice(&digest[..19 - new.len()]);
            new[0] &= 127; // Keep all keys below the common exclusive scan bound [255].
            new.extend_from_slice(&(rank as u64).to_be_bytes());
            assert_eq!(new.len(), 27);
            (key, new)
        })
        .collect();
    assert_eq!(
        translated
            .values()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        translated.len()
    );
    source
        .iter()
        .map(|g| {
            g.iter()
                .map(|(k, v)| (translated[k].clone(), v.clone()))
                .collect()
        })
        .collect()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn emit(output: &PathBuf, value: &Value) {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    serde_json::to_writer(file, value).unwrap();
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        6,
        "root fresh-output prepare|measure case-index order"
    );
    let root = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    assert!(!output.exists());
    let plan_bytes = std::fs::read(root.join("timing-plan.json")).unwrap();
    let plan: Value = serde_json::from_slice(&plan_bytes).unwrap();
    let orders = ["baseline", "candidate", "candidate", "baseline"];
    assert_eq!(plan["orders"], json!(orders));
    assert_eq!(plan["passes"], 12);
    assert_eq!(plan["count_passes"], 1);
    let order: usize = args[5].parse().unwrap();
    assert!(order < orders.len());
    let group_bytes = std::fs::read(root.join("groups.bin")).unwrap();
    let groups = corpus(&group_bytes);
    let mut works = Vec::new();
    for dataset in ["original24", "short4", "dispersed0"] {
        let source = rekey(dataset, &groups);
        for name in ["overwrite", "initial_fill", "unique_insert"] {
            works.push(workload(dataset, name, &source));
        }
    }
    let metadata: Vec<_> = works.iter().map(|w| json!({"workload":w.name,"dataset":w.dataset,"groups":w.groups.len(),"initial_keys":w.initial.len(),"final_keys":w.final_rows.len(),"final_state_sha256":digest(&w.final_rows)})).collect();
    if args[3] == "prepare" {
        assert_eq!(args[4], "0");
        for work in &works {
            for pinned in [false, true] {
                check_workload::<RedBlackTreeMapSync<Vec<u8>, Vec<u8>>>(work, pinned);
            }
            println!(
                "{}",
                json!({"event":"correspondence_passed","dataset":work.dataset,"workload":work.name})
            );
        }
        let inputs: Vec<_> = works.iter().filter(|w| w.name != "initial_fill").map(|w| {
            let queries: Vec<_> = ["get_hit", "get_miss", "predecessor", "scan16"].into_iter().map(|op| {
                let probes = queries(w, op);
                json!({"operation":op,"queries_hex":probes.iter().map(|k| hex(k)).collect::<Vec<_>>(),"query_sha256":digest(&probes.iter().map(|k| (k.clone(), vec![])).collect::<Vec<_>>())})
            }).collect();
            json!({"dataset":w.dataset,"workload":w.name,"keys_hex":w.final_rows.iter().map(|(k,_)|hex(k)).collect::<Vec<_>>(),"queries":queries})
        }).collect();
        emit(
            &output,
            &json!({"complete":true,"mode":"prepare","input_plan_sha256":hash(&plan_bytes),"group_file_sha256":hash(&group_bytes),"workloads":metadata,"read_inputs":inputs,"correctness_live_prefixes":1908,"correctness_old_views":954}),
        );
        return;
    }
    assert_eq!(args[3], "measure");
    let case: usize = args[4].parse().unwrap();
    let mut cases = Vec::new();
    for i in 0..works.len() {
        for pinned in [false, true] {
            cases.push((i, "write", pinned));
        }
    }
    for (i, _) in works
        .iter()
        .enumerate()
        .filter(|(_, w)| w.name != "initial_fill")
    {
        for operation in ["get_hit", "get_miss", "predecessor", "scan16"] {
            cases.push((i, operation, false));
        }
    }
    assert_eq!(cases.len(), 42);
    assert!(case < cases.len());
    let (work_index, operation, pinned) = cases[case];
    let work = &works[work_index];
    let passes = if cfg!(feature = "allocation-counting") {
        1
    } else {
        12
    };
    let mut rows = if operation == "write" {
        if !cfg!(feature = "allocation-counting") {
            black_box(write_case::<RedBlackTreeMapSync<Vec<u8>, Vec<u8>>>(
                work, pinned, 1,
            ));
        }
        let mut row = write_case::<RedBlackTreeMapSync<Vec<u8>, Vec<u8>>>(work, pinned, passes);
        row["phase"] = json!("group");
        vec![row]
    } else {
        assert_eq!(plan["read_epochs"], 12);
        assert_eq!(plan["read_warm_passes"], 12);
        assert_eq!(plan["read_warmup_passes"], 8);
        let probes = queries(work, operation);
        read_panels::<RedBlackTreeMapSync<Vec<u8>, Vec<u8>>>(
            work, operation, &probes, passes, passes, 8,
        )
    };
    for row in &mut rows {
        row["backend"] = json!(orders[order]);
        row["order"] = json!(order);
        row["case"] = json!(case);
    }
    emit(
        &output,
        &json!({"complete":true,"counting_build":cfg!(feature="allocation-counting"),"input_plan_sha256":hash(&plan_bytes),"group_file_sha256":hash(&group_bytes),"workloads":metadata,"rows":rows,
        "scope":"Standalone rpds calls only; baseline versus outlined shared-mutation dependency. First-touch and same-map warm read panels are separate. Owned input clones in timed writes, borrowed point/predecessor returns and owned scan16. Per-call timer overhead remains. No engine/WAL/Raft/RPC/database-QPS claim. Allocation counts are requested bytes, not RSS/usable sizes."}),
    );
}

#[cfg(test)]
mod protocol_tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static MAPS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    }

    #[derive(Clone)]
    struct TracedMap {
        id: usize,
        map: Model,
    }
    impl Default for TracedMap {
        fn default() -> Self {
            let id = MAPS.with(|maps| {
                let mut maps = maps.borrow_mut();
                let id = maps.len();
                maps.push(0);
                id
            });
            Self {
                id,
                map: Model::new(),
            }
        }
    }
    impl Index for TracedMap {
        fn put(&mut self, key: Vec<u8>, value: Vec<u8>) {
            self.map.insert(key, value);
        }
        fn get(&self, key: &[u8]) -> Option<&Vec<u8>> {
            MAPS.with(|maps| maps.borrow_mut()[self.id] += 1);
            self.map.get(key)
        }
        fn predecessor(&self, key: &[u8]) -> Option<(&Vec<u8>, &Vec<u8>)> {
            self.map.range(..=key.to_vec()).next_back()
        }
        fn scan(&self, start: &[u8], end: &[u8], limit: usize) -> Vec<Pair> {
            self.map
                .range(start.to_vec()..end.to_vec())
                .take(limit)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        }
        fn rows(&self) -> Vec<Pair> {
            self.map
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        }
        fn validate(&self) {}
    }

    #[test]
    fn first_touch_validation_warmup_and_warm_reads_share_each_map() {
        MAPS.with(|maps| maps.borrow_mut().clear());
        let work = Workload {
            dataset: "original24",
            name: "overwrite",
            groups: vec![],
            initial: vec![],
            final_rows: vec![
                (b"a".to_vec(), b"x".to_vec()),
                (b"b".to_vec(), b"y".to_vec()),
            ],
        };
        let probes = vec![b"a".to_vec(), b"b".to_vec()];
        let rows = read_panels::<TracedMap>(&work, "get_hit", &probes, 2, 2, 8);
        // Exactly two map lifetimes. Each serves first pass + validation +
        // eight warmup passes + two measured warm passes, two probes each.
        MAPS.with(|maps| assert_eq!(*maps.borrow(), vec![24, 24]));
        assert_eq!(rows[0]["phase"], "first_touch");
        assert_eq!(rows[0]["metrics"]["windows"], 4);
        assert_eq!(rows[1]["phase"], "warm");
        assert_eq!(rows[1]["metrics"]["windows"], 8);
    }
}
