//! Allocation-only companion; no elapsed-time observations are emitted.
use kv9_common::AppliedPosition;
use kv9_engine::{ColumnFamily, Engine, MemEngine, ReadView, ReplicatedEngine, WriteBatch};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::PathBuf;
mod counting;
mod probes;
#[global_allocator]
static ALLOCATOR: counting::Allocator = counting::Allocator;
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

const SENTINEL_KEY: &[u8] = b"engine-interface-sentinel";
const SENTINEL_VALUE: &[u8] = b"unchanged-other-column-family";

struct Workload {
    name: &'static str,
    groups: Vec<Vec<Pair>>,
    initial: Vec<Pair>,
    final_rows: Vec<Pair>,
}
impl Workload {
    fn new(bytes: &[u8], unique: bool) -> Self {
        let groups = corpus(bytes, unique);
        let final_model: BTreeMap<_, _> = groups.iter().flatten().cloned().collect();
        let final_rows: Vec<Pair> = final_model.into_iter().collect();
        assert_eq!(final_rows.len(), if unique { 100096 } else { 4096 });
        let initial = if unique {
            vec![]
        } else {
            final_rows
                .iter()
                .map(|(k, v)| {
                    let mut v = v.clone();
                    v[0] ^= 255;
                    (k.clone(), v)
                })
                .collect()
        };
        Self {
            name: if unique { "unique_insert" } else { "overwrite" },
            groups,
            initial,
            final_rows,
        }
    }
    fn metadata(&self) -> Value {
        json!({"dataset":"original24","workload":self.name,"groups":self.groups.len(),
            "initial_keys":self.initial.len(),"final_keys":self.final_rows.len(),
            "final_state_sha256":digest(&self.final_rows)})
    }
    fn queries(&self, miss: bool) -> Vec<Vec<u8>> {
        let model: BTreeMap<_, _> = self.final_rows.iter().cloned().collect();
        probes::indices(self.final_rows.len(), 512)
            .into_iter()
            .map(|i| {
                let mut key = self.final_rows[i].0.clone();
                if miss {
                    key.push(255);
                    while model.contains_key(&key) {
                        key.push(255);
                    }
                }
                key
            })
            .collect()
    }
}

fn initial_engine(work: &Workload) -> MemEngine {
    let engine = MemEngine::new();
    let mut initial = batch(&work.initial);
    initial.put(
        ColumnFamily::Lock,
        SENTINEL_KEY.to_vec(),
        SENTINEL_VALUE.to_vec(),
    );
    initial.put(
        ColumnFamily::Write,
        SENTINEL_KEY.to_vec(),
        SENTINEL_VALUE.to_vec(),
    );
    engine.write(initial).unwrap();
    assert_eq!(engine.data_revision(), 0);
    assert_eq!(engine.volatile_applied_position(), None);
    engine
}
fn position(index: usize) -> AppliedPosition {
    AppliedPosition {
        term: 1,
        index: index as u64,
    }
}
fn verify_position(engine: &MemEngine, index: usize) {
    assert_eq!(engine.data_revision(), index as u64);
    assert_eq!(engine.volatile_applied_position(), Some(position(index)));
}
fn verify_other_cfs(view: &dyn ReadView) {
    for cf in [ColumnFamily::Lock, ColumnFamily::Write] {
        assert_eq!(
            view.get(cf, SENTINEL_KEY).unwrap().as_deref(),
            Some(SENTINEL_VALUE)
        );
        assert_eq!(
            view.get_resident(cf, SENTINEL_KEY),
            Some(Some(SENTINEL_VALUE))
        );
        assert_eq!(
            view.scan(cf, b"", &[255], 2).unwrap(),
            vec![(SENTINEL_KEY.to_vec(), SENTINEL_VALUE.to_vec())]
        );
    }
}
fn populated_engine(work: &Workload) -> MemEngine {
    let engine = initial_engine(work);
    for (i, group) in work.groups.iter().enumerate() {
        engine.write_applied(batch(group), position(i + 1)).unwrap();
    }
    verify_position(&engine, 106);
    engine
}
fn check_workload(work: &Workload, pinned: bool) -> Value {
    let engine = initial_engine(work);
    let mut model: BTreeMap<_, _> = work.initial.iter().cloned().collect();
    for (i, group) in work.groups.iter().enumerate() {
        let old = pinned.then(|| engine.snapshot().unwrap());
        engine.write_applied(batch(group), position(i + 1)).unwrap();
        verify_position(&engine, i + 1);
        if let Some(old) = old {
            assert_eq!(
                rows(old.as_ref()),
                model
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<Vec<_>>()
            );
            verify_other_cfs(old.as_ref());
        }
        for (k, v) in group {
            model.insert(k.clone(), v.clone());
        }
        let current = engine.snapshot().unwrap();
        assert_eq!(
            rows(current.as_ref()),
            model
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Vec<_>>()
        );
        verify_other_cfs(current.as_ref());
    }
    assert_eq!(rows(engine.snapshot().unwrap().as_ref()), work.final_rows);
    json!({"workload":work.name,"pinned":pinned,"live_prefixes":106,
        "old_views":if pinned {106}else{0},"position_checks":106,"other_cfs_checked":true})
}

// All buffers are reserved before the counted operation windows. Counter
// bookkeeping and JSON construction occur after stop(), so their allocations
// cannot be misreported as engine work.
#[derive(Default)]
struct Metrics {
    windows: Vec<([u64; 6], i64, i64)>,
}
fn begin() -> i64 {
    counting::start();
    counting::live()
}
impl Metrics {
    fn new(capacity: usize) -> Self {
        Self {
            windows: Vec::with_capacity(capacity),
        }
    }
    fn finish(&mut self, initial_live: i64) {
        let counts = counting::stop();
        let live_delta = counting::live() - initial_live;
        let extra_peak = counting::peak() - initial_live;
        assert!(extra_peak >= 0);
        self.windows.push((counts, live_delta, extra_peak));
    }
    fn report(&self) -> Value {
        assert!(!self.windows.is_empty());
        let mut counts = [0u64; 6];
        let mut peak = 0i64;
        let mut live_delta = 0i64;
        for (window, delta, extra_peak) in &self.windows {
            for (sum, n) in counts.iter_mut().zip(window) {
                *sum += n;
            }
            peak = peak.max(*extra_peak);
            live_delta += delta;
        }
        json!({"windows":self.windows.len(),"allocation_counts":counts,
            "maximum_window_extra_live_bytes":peak,"sum_window_live_delta_bytes":live_delta,
            "window_records":self.windows,"elapsed_time_recorded":false})
    }
}

fn apply_pass(work: &Workload, pinned: bool, mut metrics: Option<&mut Metrics>) {
    let engine = initial_engine(work);
    for (i, group) in work.groups.iter().enumerate() {
        let old = pinned.then(|| engine.snapshot().unwrap());
        let pending = batch(group);
        if let Some(ref mut metrics) = metrics {
            let started = begin();
            black_box(&engine)
                .write_applied(pending, position(i + 1))
                .unwrap();
            metrics.finish(started);
        } else {
            engine.write_applied(pending, position(i + 1)).unwrap();
        }
        black_box(&old);
        drop(old);
    }
    verify_position(&engine, 106);
    let snapshot = engine.snapshot().unwrap();
    assert_eq!(rows(snapshot.as_ref()), work.final_rows);
    verify_other_cfs(snapshot.as_ref());
}
fn measure_apply(work: &Workload, pinned: bool) -> Vec<Value> {
    apply_pass(work, pinned, None);
    let mut metrics = Metrics::new(12 * 106);
    for _ in 0..12 {
        apply_pass(work, pinned, Some(&mut metrics));
    }
    vec![
        json!({"phase":"apply_group","unit":"ns_per_group","passes":12,"mutations":12*100096,
        "final_keys":work.final_rows.len(),"final_state_sha256":digest(&work.final_rows),
        "applied_index":106,"data_revision":106,"other_cfs_checked":true,"metrics":metrics.report()}),
    ]
}

// Selection of the interface is static. The timed loops have no operation-string
// switch or multi-operation Answer enum. ReadView dispatch is the actual API.
trait Reader {
    type Output<'a>;
    fn query<'a>(view: &'a dyn ReadView, key: &[u8]) -> Self::Output<'a>;
    fn bytes<'a>(value: &'a Self::Output<'_>) -> Option<&'a [u8]>;
}
struct Owned;
impl Reader for Owned {
    type Output<'a> = Option<Vec<u8>>;
    #[inline(always)]
    fn query<'a>(view: &'a dyn ReadView, key: &[u8]) -> Self::Output<'a> {
        view.get(ColumnFamily::Default, key).unwrap()
    }
    fn bytes<'a>(value: &'a Self::Output<'_>) -> Option<&'a [u8]> {
        value.as_deref()
    }
}
struct Resident;
impl Reader for Resident {
    type Output<'a> = Option<&'a [u8]>;
    #[inline(always)]
    fn query<'a>(view: &'a dyn ReadView, key: &[u8]) -> Self::Output<'a> {
        view.get_resident(ColumnFamily::Default, key)
            .expect("resident API declined")
    }
    fn bytes<'a>(value: &'a Self::Output<'_>) -> Option<&'a [u8]> {
        *value
    }
}

// A single borrowed view is supplied for the complete epoch, so the warmup
// cannot silently construct and discard another map.
fn read_epoch<P: Reader>(
    view: &dyn ReadView,
    keys: &[Vec<u8>],
    expected: &[Option<Vec<u8>>],
    per_call: bool,
    warm_passes: usize,
    first: &mut Metrics,
    warm: &mut Metrics,
) {
    measured_read_pass::<P>(view, keys, expected, per_call, first);
    for _ in 0..8 {
        for key in keys {
            let output = P::query(black_box(view), black_box(key));
            black_box(&output);
            drop(output);
        }
    }
    for _ in 0..warm_passes {
        measured_read_pass::<P>(view, keys, expected, per_call, warm);
    }
}
#[inline(never)]
fn measured_read_pass<P: Reader>(
    view: &dyn ReadView,
    keys: &[Vec<u8>],
    expected: &[Option<Vec<u8>>],
    per_call: bool,
    metrics: &mut Metrics,
) {
    let mut outputs = Vec::with_capacity(keys.len());
    if per_call {
        for key in keys {
            let started = begin();
            let value = P::query(black_box(view), black_box(key));
            black_box(&value);
            metrics.finish(started);
            outputs.push(value);
        }
    } else {
        let started = begin();
        for key in keys {
            let value = P::query(black_box(view), black_box(key));
            black_box(&value);
            outputs.push(value);
        }
        metrics.finish(started);
    }
    for (value, expected) in outputs.iter().zip(expected) {
        assert_eq!(P::bytes(value), expected.as_deref());
    }
    assert_eq!(outputs.len(), expected.len());
    // Owned and borrowed outputs are retained for the pass. Destruction and
    // output validation occur outside all measured windows in both modes.
    drop(outputs);
}
fn verify_after_read(engine: &MemEngine, view: &dyn ReadView, work: &Workload) {
    assert_eq!(rows(view), work.final_rows);
    verify_other_cfs(view);
    verify_position(engine, 106);
    let (key, value) = &work.final_rows[0];
    let mut replacement = value.clone();
    replacement[0] ^= 255;
    let mut next = WriteBatch::new();
    next.put(ColumnFamily::Default, key.clone(), replacement.clone());
    engine.write_applied(next, position(107)).unwrap();
    verify_position(engine, 107);
    assert_eq!(
        engine.get(ColumnFamily::Default, key).unwrap(),
        Some(replacement)
    );
    assert_eq!(rows(view), work.final_rows);
    verify_other_cfs(engine.snapshot().unwrap().as_ref());
}
fn measure_reads<P: Reader>(work: &Workload, miss: bool, per_call: bool) -> Vec<Value> {
    let keys = work.queries(miss);
    let model: BTreeMap<_, _> = work.final_rows.iter().cloned().collect();
    let expected: Vec<_> = keys.iter().map(|key| model.get(key).cloned()).collect();
    let windows_per_pass = if per_call { keys.len() } else { 1 };
    let mut first = Metrics::new(12 * windows_per_pass);
    let mut warm = Metrics::new(12 * 12 * windows_per_pass);
    for _ in 0..12 {
        let engine = populated_engine(work);
        let view = engine.snapshot().unwrap();
        read_epoch::<P>(
            view.as_ref(),
            &keys,
            &expected,
            per_call,
            12,
            &mut first,
            &mut warm,
        );
        verify_after_read(&engine, view.as_ref(), work);
    }
    let query_sha = digest(&keys.iter().map(|k| (k.clone(), vec![])).collect::<Vec<_>>());
    [("first_probe", 1, first), ("warm", 12, warm)]
        .into_iter()
        .map(|(phase, passes_per_epoch, metrics)| {
            json!({"phase":phase,"unit":if per_call{"ns_per_read"}else{"ns_per_512_read_pass"},
            "epochs":12,"passes_per_epoch":passes_per_epoch,"passes":12*passes_per_epoch,
            "queries_per_pass":512,"queries_per_window":if per_call{1}else{512},"warmup_passes":8,
            "query_sha256":query_sha,"all_outputs_checked":true,"stable_old_view_checks":12,
            "initial_applied_index":106,"later_applied_index":107,"other_cfs_checked":true,
            "metrics":metrics.report()})
        })
        .collect()
}
fn measure_snapshots(work: &Workload) -> Vec<Value> {
    let engine = populated_engine(work);
    let snapshot_bytes = std::mem::size_of_val(engine.snapshot().unwrap().as_ref());
    for _ in 0..8 * 512 {
        black_box(engine.snapshot().unwrap());
    }
    let mut metrics = Metrics::new(12 * 512);
    for _ in 0..12 {
        for _ in 0..512 {
            let started = begin();
            let view = black_box(&engine).snapshot().unwrap();
            black_box(&view);
            metrics.finish(started);
            drop(view);
        }
        let view = engine.snapshot().unwrap();
        assert_eq!(rows(view.as_ref()), work.final_rows);
        verify_other_cfs(view.as_ref());
        verify_position(&engine, 106);
    }
    vec![
        json!({"phase":"steady_snapshot","snapshot_bytes":snapshot_bytes,"unit":"ns_per_snapshot","passes":12,"queries_per_pass":512,
        "warmup_passes":8,"applied_index":106,"data_revision":106,"other_cfs_checked":true,
        "final_state_sha256":digest(&work.final_rows),"metrics":metrics.report()}),
    ]
}
fn hex(value: &[u8]) -> String {
    value.iter().map(|b| format!("{b:02x}")).collect()
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        6,
        "root fresh-output prepare|measure case order"
    );
    let root = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    assert!(!output.exists());
    let bytes = std::fs::read(root.join("groups.bin")).unwrap();
    let plan_bytes = std::fs::read(root.join("plan.json")).unwrap();
    let plan: Value = serde_json::from_slice(&plan_bytes).unwrap();
    assert_eq!(
        plan["orders"],
        json!(["baseline", "candidate", "candidate", "baseline"])
    );
    assert_eq!(plan["write_passes"], 12);
    assert_eq!(plan["write_excluded_passes"], 1);
    for name in ["read_epochs", "read_warm_passes", "snapshot_passes"] {
        assert_eq!(plan[name], 12);
    }
    for name in ["read_warmup_passes", "snapshot_warmup_passes"] {
        assert_eq!(plan[name], 8);
    }
    assert_eq!(plan["query_count"], 512);
    assert_eq!(plan["query_seed"], 71);
    assert_eq!(plan["snapshot_queries_per_pass"], 512);
    let works = [Workload::new(&bytes, false), Workload::new(&bytes, true)];
    let metadata: Vec<_> = works.iter().map(Workload::metadata).collect();
    let mut result = json!({"complete":true,"input_plan_sha256":sha(&plan_bytes),"corpus_sha256":sha(&bytes),"workloads":metadata});
    result["counting_build"] = json!(true);
    if args[3] == "prepare" {
        let mut checks = vec![];
        let mut inputs = vec![];
        for work in &works {
            for pinned in [false, true] {
                checks.push(check_workload(work, pinned));
            }
            let queries: Vec<_> = [false,true].into_iter().map(|miss| {
                let keys = work.queries(miss);
                json!({"operation":if miss{"get_miss"}else{"get_hit"},
                    "queries_hex":keys.iter().map(|k|hex(k)).collect::<Vec<_>>(),
                    "query_sha256":digest(&keys.iter().map(|k|(k.clone(),vec![])).collect::<Vec<_>>())})
            }).collect();
            inputs.push(json!({"dataset":"original24","workload":work.name,
                "keys_hex":work.final_rows.iter().map(|(k,_)|hex(k)).collect::<Vec<_>>(),"queries":queries}));
        }
        result["mode"] = json!("prepare");
        result["checks"] = json!(checks);
        result["read_inputs"] = json!(inputs);
    } else {
        assert_eq!(args[3], "measure");
        let case: usize = args[4].parse().unwrap();
        let order: usize = args[5].parse().unwrap();
        assert!(order < 4);
        let spec = &plan["cases"][case];
        let work = works.iter().find(|w| spec["workload"] == w.name).unwrap();
        let mut measured = match spec["operation"].as_str().unwrap() {
            "write_applied" => measure_apply(work, spec["pinned"].as_bool().unwrap()),
            "snapshot" => measure_snapshots(work),
            "read" => {
                let miss = spec["queries"] == "get_miss";
                assert!(miss || spec["queries"] == "get_hit");
                let per_call = spec["timer"] == "per_call";
                assert!(per_call || spec["timer"] == "whole_pass");
                match spec["api"].as_str().unwrap() {
                    "owned" => measure_reads::<Owned>(work, miss, per_call),
                    "resident" => measure_reads::<Resident>(work, miss, per_call),
                    _ => panic!("unexpected interface"),
                }
            }
            _ => panic!("unexpected operation"),
        };
        for row in &mut measured {
            let scope = std::mem::replace(&mut row["unit"], json!("allocator_counts_per_window"));
            row["operation_window"] = scope;
        }
        result["mode"] = json!("measure");
        result["case"] = json!(case);
        result["order"] = json!(order);
        result["backend"] = plan["orders"][order].clone();
        result["spec"] = spec.clone();
        result["rows"] = json!(measured);
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    serde_json::to_writer(file, &result).unwrap();
}
