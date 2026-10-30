//! Row classes for the scoped metadata mode (docs/META-REPLICA-MODES.md §3b).
//!
//! Every key a `CatalogTxn` can write is declared here: the catalog tables and
//! their indexes, the retention ledger, the task-history stamp and the capture
//! catalog. [`classify`] says what one op is about; the full node's router decides
//! from that and a subscriber's closure whether the op is delivered. A key not
//! declared here returns `None`, and the stream stops on it rather than guess
//! (M3: nothing outside the declared catalog is routed by accident).

use kv9_common::codec::{encode_key, KeyMode};
use kv9_common::{KeyspaceId, NodeId, RegionId, Result};

use crate::codec::{decode_uint_component, split_components, RowValue};
use crate::data_groups::subject::{task_subject, TaskSubject};
use crate::schema;
use crate::ColumnValue;

const RETENTION_PREFIX: &[u8] = b"\0kv9\0retention_v1\0";
/// The metadata group's own manifest state (ManifestChange apply).
const MANIFEST_PAIR_PREFIX: &[u8] = b"\0kv9\0manifest_pair\0";
const MANIFEST_GEN_PREFIX: &[u8] = b"\0kv9\0manifest_gen\0";
/// Engine-owned singletons under the system prefix (`kv9.raw-ttl-clock.v1`,
/// `kv9.data-range.v1`, `kv9.compaction-confirmed.v1`).
const ENGINE_SINGLETON: &[u8] = b"kv9.";
const CAPTURE_PREFIX: &[u8] = b"\0kv9\0migration-capture-v1\0";

/// What one catalog op is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowClass {
    /// Every scope holds it: the root certificate, cluster identity, schema version.
    Global,
    /// Only full nodes hold it: id sequences, file GC rows, quotas, the retention
    /// ledger, the task-history stamp.
    FullOnly,
    Task(TaskSubject),
    Region(RegionId),
    RegionPeer(RegionId, NodeId),
    Keyspace(u64),
    Tenant(u64),
    /// A NODES or NODE_ADMISSIONS row.
    Node(NodeId),
    /// A committed capture descriptor of a migration on this region.
    Capture(RegionId),
    /// A delete whose subject lived in the value it removes: delivered iff the
    /// subscriber holds the key.
    Held,
}

fn system_prefix() -> Vec<u8> {
    encode_key(KeyMode::System, KeyspaceId::SYSTEM, &[]).expect("the system keyspace is valid")
}

fn uint(bytes: &[u8]) -> Result<u64> {
    decode_uint_component(bytes)
}

fn malformed(what: &str) -> kv9_common::Error {
    kv9_common::Error::MalformedKey(format!("catalog scope: {what}"))
}

/// Classify one op of default-CF `key` (`value` is `None` for a delete).
/// `Ok(None)`: the key is not declared.
pub fn classify(key: &[u8], value: Option<&[u8]>) -> Result<Option<RowClass>> {
    if key.starts_with(RETENTION_PREFIX)
        || key == crate::task_history::KEY
        || key.starts_with(MANIFEST_PAIR_PREFIX)
        || key.starts_with(MANIFEST_GEN_PREFIX)
    {
        return Ok(Some(RowClass::FullOnly));
    }
    // Data of a keyspace the metadata group serves itself (mode 'r'): full nodes only.
    if key.first() == Some(&b'r') {
        return Ok(Some(RowClass::FullOnly));
    }
    if key.starts_with(CAPTURE_PREFIX) {
        let Some(bytes) = value else {
            return Ok(Some(RowClass::Held));
        };
        if bytes.len() <= 48 {
            return Err(malformed("capture record too short"));
        }
        let descriptor =
            kv9_raft::snapshot_install::capture::CaptureDescriptor::decode(&bytes[16..bytes.len() - 32])?;
        return Ok(Some(RowClass::Capture(descriptor.request().range.region)));
    }
    let prefix = system_prefix();
    let Some(rest) = key.strip_prefix(prefix.as_slice()) else {
        return Ok(None);
    };
    if rest.starts_with(ENGINE_SINGLETON) {
        return Ok(Some(RowClass::FullOnly));
    }
    if rest.len() < 5 {
        return Ok(None);
    }
    let table = schema::TableId(u32::from_be_bytes(rest[..4].try_into().expect("4 bytes")));
    let marker = rest[4];
    let body = &rest[5..];
    match marker {
        b'r' => classify_row(table, body, value),
        b'i' => classify_index(table, body, value),
        _ => Ok(None),
    }
}

fn row_value(value: &[u8]) -> Result<RowValue> {
    RowValue::decode(value)
}

fn classify_row(table: schema::TableId, pk: &[u8], value: Option<&[u8]>) -> Result<Option<RowClass>> {
    use schema::*;
    let first = || -> Result<u64> {
        if pk.len() < 8 {
            return Err(malformed("short primary key"));
        }
        uint(&pk[..8])
    };
    Ok(Some(match table {
        t if t == TENANTS => RowClass::Tenant(first()?),
        t if t == KEYSPACES => RowClass::Keyspace(first()?),
        t if t == NODES || t == NODE_ADMISSIONS => RowClass::Node(NodeId(first()?)),
        t if t == REGIONS => RowClass::Region(RegionId(first()?)),
        t if t == REGION_PEERS => {
            if pk.len() != 16 {
                return Err(malformed("region peer key"));
            }
            RowClass::RegionPeer(RegionId(uint(&pk[..8])?), NodeId(uint(&pk[8..])?))
        }
        t if t == SCHEMA_VERSION_TABLE || t == CLUSTER_META || t == ROOT_META => RowClass::Global,
        t if t == SST_FILES || t == ID_SEQUENCES || t == GAC_ALLOTMENTS => RowClass::FullOnly,
        t if t == PLACEMENT_RULES => match value {
            None => RowClass::Held,
            Some(bytes) => match row_value(bytes)?.get(ColumnId(2)) {
                Some(ColumnValue::Uint(keyspace)) => RowClass::Keyspace(*keyspace),
                _ => return Err(malformed("placement rule without a keyspace")),
            },
        },
        t if t == TASKS => match value {
            None => RowClass::Held,
            Some(bytes) => match task_subject(&row_value(bytes)?)? {
                Some(subject) => RowClass::Task(subject),
                None => return Ok(None),
            },
        },
        _ => return Ok(None),
    }))
}

fn classify_index(table: schema::TableId, body: &[u8], value: Option<&[u8]>) -> Result<Option<RowClass>> {
    use schema::*;
    let Some((&index, cols)) = body.split_first() else {
        return Ok(None);
    };
    let unique_pk = || -> Result<RowClass> {
        match value {
            None => Ok(RowClass::Held),
            Some(pk) if pk.len() == 8 => Ok(RowClass::Keyspace(uint(pk)?)),
            Some(_) => Err(malformed("unique index value")),
        }
    };
    Ok(Some(match (table, index) {
        (t, 1) if t == TENANTS => match unique_pk()? {
            RowClass::Keyspace(tenant) => RowClass::Tenant(tenant),
            other => other,
        },
        (t, 1) if t == KEYSPACES => {
            let parts = split_components(&[ColumnType::Uint, ColumnType::Uint], cols, true)?;
            RowClass::Keyspace(uint(&parts[1])?)
        }
        (t, 2) if t == KEYSPACES => unique_pk()?,
        (t, 1) if t == REGIONS => {
            let parts = split_components(&[ColumnType::Uint, ColumnType::Bytes, ColumnType::Uint], cols, true)?;
            RowClass::Region(RegionId(uint(&parts[2])?))
        }
        (t, 1) if t == REGION_PEERS => {
            let parts = split_components(&[ColumnType::Uint, ColumnType::Uint, ColumnType::Uint], cols, true)?;
            RowClass::RegionPeer(RegionId(uint(&parts[1])?), NodeId(uint(&parts[2])?))
        }
        _ => return Ok(None),
    }))
}

// ---------------------------------------------------------------------------
// Closures and routing.
// ---------------------------------------------------------------------------

use std::collections::{BTreeMap, BTreeSet, HashMap};

use kv9_engine::{ColumnFamily, ReadView};

/// A task subject with its references followed: everything the row is about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    pub regions: BTreeSet<u64>,
    pub keyspaces: BTreeSet<u64>,
    pub tenants: BTreeSet<u64>,
    pub nodes: BTreeSet<u64>,
    pub drain: Option<u64>,
}

const MAX_REF_DEPTH: usize = 4;

/// Follows task references against one consistent view. Task rows are
/// immutable once written, so resolutions are cached by task id.
pub struct Resolver<'v> {
    view: &'v dyn ReadView,
    tasks: HashMap<u64, Option<Resolved>>,
    backups: Option<HashMap<[u8; 16], u64>>,
}

fn task_key(task: u64) -> Result<Vec<u8>> {
    crate::codec::encode_row_key(schema::TASKS, &[crate::codec::memcmp_uint(task)])
}

impl<'v> Resolver<'v> {
    pub fn new(view: &'v dyn ReadView) -> Self {
        Resolver { view, tasks: HashMap::new(), backups: None }
    }

    fn subject_of(&self, task: u64) -> Result<Option<TaskSubject>> {
        match self.view.get(ColumnFamily::Default, &task_key(task)?)? {
            None => Ok(None),
            Some(bytes) => task_subject(&row_value(&bytes)?),
        }
    }

    fn backup_task(&mut self, operation: [u8; 16]) -> Result<Option<u64>> {
        if self.backups.is_none() {
            let mut map = HashMap::new();
            let (start, end) = crate::codec::row_range(schema::TASKS)?;
            for_each(self.view, &start, &end, |key, value| {
                if let Some(op) = crate::data_groups::subject::backup_intent_operation(&row_value(value)?)? {
                    map.insert(op, uint(&key[key.len() - 8..])?);
                }
                Ok(())
            })?;
            self.backups = Some(map);
        }
        Ok(self.backups.as_ref().expect("filled above").get(&operation).copied())
    }

    /// Resolve a subject: its own names plus those of every task it references.
    /// A reference that is not in the view (or not declared) contributes nothing.
    pub fn resolve(&mut self, subject: &TaskSubject) -> Result<Resolved> {
        self.resolve_depth(subject, 0)
    }

    fn resolve_depth(&mut self, subject: &TaskSubject, depth: usize) -> Result<Resolved> {
        let mut out = Resolved {
            regions: subject.regions.iter().map(|r| r.0).collect(),
            keyspaces: subject.keyspaces.iter().copied().collect(),
            tenants: subject.tenants.iter().copied().collect(),
            nodes: subject.nodes.iter().map(|n| n.0).collect(),
            drain: subject.drain.map(|n| n.0),
        };
        if depth >= MAX_REF_DEPTH {
            return Ok(out);
        }
        let mut refs = subject.refs.clone();
        if let Some(operation) = subject.backup_operation {
            refs.extend(self.backup_task(operation)?);
        }
        for task in refs {
            let Some(inner) = self.task_depth(task, depth + 1)? else { continue };
            out.regions.extend(inner.regions);
            out.keyspaces.extend(inner.keyspaces);
            out.tenants.extend(inner.tenants);
            // Nodes named by a referenced task are that task's business (a creation's
            // replicas are not a split evidence's subject); regions and keyspaces carry.
        }
        Ok(out)
    }

    fn task_depth(&mut self, task: u64, depth: usize) -> Result<Option<Resolved>> {
        if let Some(cached) = self.tasks.get(&task) {
            return Ok(cached.clone());
        }
        let resolved = match self.subject_of(task)? {
            None => None,
            Some(subject) => Some(self.resolve_depth(&subject, depth)?),
        };
        if depth == 0 || resolved.is_some() {
            self.tasks.insert(task, resolved.clone());
        }
        Ok(resolved)
    }

    /// The resolution of a task row already in the view.
    pub fn task(&mut self, task: u64) -> Result<Option<Resolved>> {
        self.task_depth(task, 0)
    }
}

/// Visit every default-CF entry of `[start, end)` in key order.
pub fn for_each(
    view: &dyn ReadView,
    start: &[u8],
    end: &[u8],
    mut visit: impl FnMut(&[u8], &[u8]) -> Result<()>,
) -> Result<()> {
    const PAGE: usize = 1024;
    let mut from = start.to_vec();
    loop {
        let page = view.scan(ColumnFamily::Default, &from, end, PAGE)?;
        let full = page.len() == PAGE;
        for (key, value) in &page {
            visit(key, value)?;
        }
        match page.last() {
            Some((key, _)) if full => {
                from = key.clone();
                from.push(0);
            }
            _ => return Ok(()),
        }
    }
}

/// The whole default-CF key space of a metadata engine.
pub const ALL_START: &[u8] = &[];
pub const ALL_END: &[u8] = &[0xff; 16];

/// One node's scope (docs/META-REPLICA-MODES.md S1/S2): every keyspace it hosts a
/// replica of, with all of those keyspaces' regions, their tenants, and the nodes
/// it has to know (itself, the full nodes, every node its scope names).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Closure {
    pub node: u64,
    pub regions: BTreeSet<u64>,
    pub keyspaces: BTreeSet<u64>,
    pub tenants: BTreeSet<u64>,
    pub nodes: BTreeSet<u64>,
}

/// How one op reaches one subscriber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Deliver,
    Skip,
    /// A delete whose subject went with its value: deliver iff held.
    IfHeld,
    /// The op widens the closure: send a fresh snapshot instead.
    Resnapshot,
    /// The key is not declared: stop the stream.
    Undeclared,
}

impl Closure {
    fn names_task(&self, r: &Resolved) -> bool {
        r.drain == Some(self.node)
            || r.nodes.contains(&self.node)
            || r.regions.iter().any(|x| self.regions.contains(x))
            || r.keyspaces.iter().any(|x| self.keyspaces.contains(x))
    }
    fn covers_task(&self, r: &Resolved) -> bool {
        r.regions.is_subset(&self.regions)
            && r.keyspaces.is_subset(&self.keyspaces)
            && r.tenants.is_subset(&self.tenants)
            && r.nodes.is_subset(&self.nodes)
    }
}

/// Compute `node`'s closure from one consistent view. `full` are the full nodes
/// (metadata voters and full learners), which every scope can reach.
pub fn closure(view: &dyn ReadView, node: u64, full: &[u64]) -> Result<Closure> {
    let mut resolver = Resolver::new(view);
    let mut tasks: Vec<Resolved> = Vec::new();
    let mut region_keyspace: BTreeMap<u64, u64> = BTreeMap::new();
    let mut peers: Vec<(u64, u64)> = Vec::new();
    let mut keyspace_tenant: BTreeMap<u64, u64> = BTreeMap::new();
    let mut task_ids = Vec::new();
    for_each(view, ALL_START, ALL_END, |key, value| {
        match classify(key, Some(value))? {
            Some(RowClass::Task(_)) => task_ids.push(uint(&key[key.len() - 8..])?),
            Some(RowClass::Region(r)) if is_row(key) => {
                if let Some(ColumnValue::Uint(k)) = row_value(value)?.get(schema::ColumnId(2)) {
                    region_keyspace.insert(r.0, *k);
                }
            }
            Some(RowClass::RegionPeer(r, n)) if is_row(key) => peers.push((r.0, n.0)),
            Some(RowClass::Keyspace(k)) if is_row(key) && is_table(key, schema::KEYSPACES) => {
                if let Some(ColumnValue::Uint(t)) = row_value(value)?.get(schema::ColumnId(3)) {
                    keyspace_tenant.insert(k, *t);
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    for task in task_ids {
        if let Some(r) = resolver.task(task)? {
            tasks.push(r);
        }
    }
    let mut c = Closure { node, ..Closure::default() };
    // S0: what names this node.
    for t in &tasks {
        if t.nodes.contains(&node) {
            c.regions.extend(t.regions.iter().copied());
            c.keyspaces.extend(t.keyspaces.iter().copied());
        }
    }
    for (r, n) in &peers {
        if *n == node {
            c.regions.insert(*r);
        }
    }
    // Fixpoint: a hosted region brings its keyspace, a keyspace brings all its regions.
    loop {
        let before = (c.regions.len(), c.keyspaces.len());
        for (r, k) in &region_keyspace {
            if c.regions.contains(r) {
                c.keyspaces.insert(*k);
            }
            if c.keyspaces.contains(k) {
                c.regions.insert(*r);
            }
        }
        for t in &tasks {
            if t.regions.iter().any(|r| c.regions.contains(r)) || t.keyspaces.iter().any(|k| c.keyspaces.contains(k)) {
                c.regions.extend(t.regions.iter().copied());
                c.keyspaces.extend(t.keyspaces.iter().copied());
            }
        }
        if before == (c.regions.len(), c.keyspaces.len()) {
            break;
        }
    }
    c.nodes.insert(node);
    c.nodes.extend(full.iter().copied());
    for t in &tasks {
        if c.names_task(t) {
            c.tenants.extend(t.tenants.iter().copied());
            c.nodes.extend(t.nodes.iter().copied());
        }
    }
    for (r, n) in &peers {
        if c.regions.contains(r) {
            c.nodes.insert(*n);
        }
    }
    for k in &c.keyspaces {
        if let Some(t) = keyspace_tenant.get(k) {
            c.tenants.insert(*t);
        }
    }
    Ok(c)
}

fn is_row(key: &[u8]) -> bool {
    let prefix = system_prefix();
    key.len() > prefix.len() + 4 && key[prefix.len() + 4] == b'r'
}

fn is_table(key: &[u8], table: schema::TableId) -> bool {
    let prefix = system_prefix();
    key.len() > prefix.len() + 4 && key[prefix.len()..prefix.len() + 4] == table.0.to_be_bytes()
}

/// Route one op (`value` `None` for a delete) to the subscriber with closure `c`.
pub fn route(c: &Closure, resolver: &mut Resolver<'_>, key: &[u8], value: Option<&[u8]>) -> Result<Route> {
    let Some(class) = classify(key, value)? else {
        return Ok(Route::Undeclared);
    };
    Ok(match class {
        RowClass::Global => Route::Deliver,
        RowClass::FullOnly => Route::Skip,
        RowClass::Held => Route::IfHeld,
        RowClass::Task(subject) => {
            let r = resolver.resolve(&subject)?;
            if !c.names_task(&r) {
                Route::Skip
            } else if r.drain.is_some() {
                // A drain is about its node alone; it widens nothing.
                if r.drain == Some(c.node) { Route::Deliver } else { Route::Skip }
            } else if c.covers_task(&r) {
                Route::Deliver
            } else {
                Route::Resnapshot
            }
        }
        RowClass::Region(r) => {
            if c.regions.contains(&r.0) {
                Route::Deliver
            } else if let Some(bytes) = value.filter(|_| is_row(key)) {
                match row_value(bytes)?.get(schema::ColumnId(2)) {
                    Some(ColumnValue::Uint(k)) if c.keyspaces.contains(k) => Route::Resnapshot,
                    _ => Route::Skip,
                }
            } else {
                Route::Skip
            }
        }
        RowClass::RegionPeer(r, n) => {
            if c.regions.contains(&r.0) {
                if c.nodes.contains(&n.0) || value.is_none() { Route::Deliver } else { Route::Resnapshot }
            } else if n.0 == c.node && value.is_some() {
                Route::Resnapshot
            } else {
                Route::Skip
            }
        }
        RowClass::Keyspace(k) => {
            if !c.keyspaces.contains(&k) {
                Route::Skip
            } else if let Some(bytes) = value.filter(|_| is_row(key) && is_table(key, schema::KEYSPACES)) {
                match row_value(bytes)?.get(schema::ColumnId(3)) {
                    Some(ColumnValue::Uint(t)) if !c.tenants.contains(t) => Route::Resnapshot,
                    _ => Route::Deliver,
                }
            } else {
                Route::Deliver
            }
        }
        RowClass::Tenant(t) => if c.tenants.contains(&t) { Route::Deliver } else { Route::Skip },
        RowClass::Node(n) => if c.nodes.contains(&n.0) { Route::Deliver } else { Route::Skip },
        RowClass::Capture(r) => if c.regions.contains(&r.0) { Route::Deliver } else { Route::Skip },
    })
}

/// Every row of `view` in `c`'s scope, in key order: the snapshot a subscriber
/// starts from. Fails on an undeclared key (M3).
pub fn snapshot_rows(view: &dyn ReadView, c: &Closure) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let mut resolver = Resolver::new(view);
    let mut rows = Vec::new();
    for_each(view, ALL_START, ALL_END, |key, value| {
        match route(c, &mut resolver, key, Some(value))? {
            Route::Deliver => rows.push((key.to_vec(), value.to_vec())),
            Route::Undeclared => {
                return Err(kv9_common::Error::MalformedKey(format!(
                    "catalog scope: undeclared metadata key {}",
                    hex(key)
                )))
            }
            // A snapshot is computed with the closure of the same view: a row that
            // would widen it is one the closure already covers or skips.
            _ => {}
        }
        Ok(())
    })?;
    Ok(rows)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().take(48).map(|b| format!("{b:02x}")).collect()
}
