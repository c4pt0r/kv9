//! Bounded projection of retained DATA placement tasks. An origin records a
//! possible holder, not a proof of live membership, read fencing or drain.
use super::*;
use crate::codec::encode_row_key;
use kv9_common::data_range::DataRange;
use kv9_engine::{ColumnFamily, ReadView};

pub const MAX_ROW_BYTES: usize = kv9_common::data_range::MAX_ENCODED_BYTES + 4096;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    pub region: RegionId,
    pub creation: RootDigest,
    pub replica: InitialReplica,
    pub task: u64,
    pub kind: u64,
    pub source_digest: RootDigest,
}
pub fn task_key(task: u64) -> Result<Vec<u8>> {
    encode_row_key(TASKS_DESC.id, &[memcmp_uint(task)])
}
struct Task {
    id: u64,
    kind: u64,
    payload: Vec<u8>,
    digest: RootDigest,
}
impl Task {
    fn decode(key: &[u8], bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_ROW_BYTES {
            return Err(invalid("store-history task row too large"));
        }
        let row = RowValue::decode(bytes)?;
        let number = |column| match row.get(ColumnId(column)) {
            Some(ColumnValue::Uint(v)) => Ok(*v),
            _ => Err(invalid("store-history task field missing or mistyped")),
        };
        let id = number(1)?;
        let kind = number(2)?;
        let Some(ColumnValue::Bytes(payload)) = row.get(ColumnId(3)) else {
            return Err(invalid("store-history task payload absent"));
        };
        if id < FIRST_DYNAMIC_ID
            || key != task_key(id)?
            || number(4)? != 0
            || number(5)? != 0
            || row.encode() != bytes
        {
            return Err(invalid("store-history task key or phase differs"));
        }
        Ok(Self {
            id,
            kind,
            payload: payload.clone(),
            digest: RootDigest::sha256(bytes),
        })
    }
    fn load(view: &dyn ReadView, task: u64, kind: u64) -> Result<Self> {
        let key = task_key(task)?;
        let bytes = view
            .get(ColumnFamily::Default, &key)?
            .ok_or_else(|| invalid("store-history referenced task absent"))?;
        let row = Self::decode(&key, &bytes)?;
        if row.kind != kind {
            return Err(invalid("store-history referenced task kind differs"));
        }
        Ok(row)
    }
    fn creation(&self, root: RootDigest) -> Result<CreationIntent> {
        let c = CreationIntent::decode(&self.payload)?;
        if self.kind != 100 || c.root() != root || c.task() != self.id || c.encode() != self.payload
        {
            return Err(invalid("store-history creation differs"));
        }
        Ok(c)
    }
    fn migration(
        &self,
        view: &dyn ReadView,
        root: RootDigest,
    ) -> Result<(migration::MigrationIntent, CreationIntent)> {
        let m = migration::MigrationIntent::decode(&self.payload)?;
        if self.kind != 103 || m.root() != root || m.task() != self.id || m.encode() != self.payload
        {
            return Err(invalid("store-history migration differs"));
        }
        let c = Task::load(view, m.creation_task(), 100)?.creation(root)?;
        if c.region() != m.region()
            || c.replicas().iter().any(|r| {
                r.node == m.destination().node || r.incarnation == m.destination().incarnation
            })
        {
            return Err(invalid(
                "store-history migration creation or destination differs",
            ));
        }
        Ok((m, c))
    }
    fn evidence(
        &self,
        view: &dyn ReadView,
        root: RootDigest,
    ) -> Result<(evidence::InstallEvidence, CreationIntent)> {
        let e = evidence::InstallEvidence::decode(&self.payload)?;
        if self.kind != 104 || e.root() != root || e.task() != self.id || e.encode() != self.payload
        {
            return Err(invalid("store-history evidence differs"));
        }
        let (m, c) = Task::load(view, e.migration_task(), 103)?.migration(view, root)?;
        if e.operation() != m.operation()
            || e.region() != m.region()
            || e.destination() != m.destination()
        {
            return Err(invalid("store-history evidence migration differs"));
        }
        Ok((e, c))
    }
    fn range(&self, view: &dyn ReadView, root: RootDigest) -> Result<CreationIntent> {
        if self.kind != 102 || self.payload.len() < 8 {
            return Err(invalid("store-history binding payload"));
        }
        let task = u64::from_be_bytes(self.payload[..8].try_into().unwrap());
        let c = Task::load(view, task, 100)?.creation(root)?;
        let r = kv9_common::data_range::DataRange::decode(&self.payload[8..])?;
        if r.root != root
            || r.creation != c.digest()
            || r.region != c.region()
            || r.encode() != self.payload[8..]
        {
            return Err(invalid("store-history binding creation differs"));
        }
        Ok(c)
    }
    fn origin(&self, c: &CreationIntent, replica: InitialReplica) -> Origin {
        Origin {
            region: c.region(),
            creation: c.digest(),
            replica,
            task: self.id,
            kind: self.kind,
            source_digest: self.digest,
        }
    }
}
/// No global task scans or membership vectors: at most seven origins and a
/// constant-depth chain of exact referenced task point reads per input row.
pub fn project(
    view: &dyn ReadView,
    root: RootDigest,
    key: &[u8],
    bytes: &[u8],
) -> Result<Vec<Origin>> {
    let t = Task::decode(key, bytes)?;
    let mut out = Vec::new();
    match t.kind {
        100 => {
            let c = t.creation(root)?;
            out.extend(c.replicas().iter().map(|r| t.origin(&c, *r)));
        }
        101 => {
            let c = CreationIntent::decode(&t.payload)?;
            if c.root() != root
                || c.task() == t.id
                || Task::load(view, c.task(), 100)?.creation(root)? != c
                || c.encode() != t.payload
            {
                return Err(invalid("store-history activation differs"));
            }
        }
        102 => {
            t.range(view, root)?;
        }
        103 => {
            let (m, c) = t.migration(view, root)?;
            out.push(t.origin(&c, m.destination()));
        }
        104 => {
            let (e, c) = t.evidence(view, root)?;
            out.push(t.origin(&c, e.destination()));
        }
        105 => {
            let d = truncation::TruncationDecision::decode(&t.payload)?;
            if d.root() != root || d.task() != t.id || d.encode() != t.payload {
                return Err(invalid("store-history truncation differs"));
            }
            let (e, _) = Task::load(view, d.evidence_task(), 104)?.evidence(view, root)?;
            if e.operation() != d.operation() || e.region() != d.region() {
                return Err(invalid("store-history truncation evidence differs"));
            }
        }
        106 => {
            let d = removal::RemovalDecision::decode(&t.payload)?;
            if d.root() != root || d.task() != t.id || d.encode() != t.payload {
                return Err(invalid("store-history removal differs"));
            }
            let (e, c) = Task::load(view, d.evidence_task(), 104)?.evidence(view, root)?;
            if e.operation() != d.operation() || e.region() != d.region() {
                return Err(invalid("store-history removal evidence or source differs"));
            }
            // V1 retires an initial replica; V2 one introduced by the evidence row
            // it points at (one more exact point read, no scan).
            if d.origin_task() == 0 {
                if !c.replicas().contains(&d.source()) {
                    return Err(invalid("store-history removal source is not a creation replica"));
                }
            } else {
                let (origin, _) = Task::load(view, d.origin_task(), 104)?.evidence(view, root)?;
                if origin.region() != d.region()
                    || origin.destination() != d.source()
                    || origin.operation() == d.operation()
                {
                    return Err(invalid("store-history removal origin does not introduce the source"));
                }
            }
            out.push(t.origin(&c, d.source()));
        }
        // Kind 107 (range-split intent) is retired with the split experiment
        // (branch `experimental/range-split`, 2026-10-28). A catalog that still
        // carries one was written by that branch; this binary refuses it rather
        // than walking history it cannot interpret.
        107 => {
            return Err(invalid(
                "store-history contains a retired range-split intent",
            ))
        }
        108 => {
            let a = abort::MigrationAbort::decode(&t.payload)?;
            if a.root() != root || a.task() != t.id || a.encode() != t.payload {
                return Err(invalid("store-history abort differs"));
            }
            let (m, c) = Task::load(view, a.migration_task(), 103)?.migration(view, root)?;
            if a.operation() != m.operation()
                || a.region() != m.region()
                || a.destination() != m.destination()
            {
                return Err(invalid("store-history abort migration differs"));
            }
            out.push(t.origin(&c, a.destination()));
        }
        110 => {
            let c = compaction::GroupCompaction::decode(&t.payload)?;
            if c.root() != root
                || c.task() != t.id
                || c.encode() != t.payload
                || Task::load(view, c.creation_task(), 100)?
                    .creation(root)?
                    .region()
                    != c.region()
            {
                return Err(invalid("store-history compaction differs"));
            }
        }
        // A replica plan (REPLICA-ADDITION) introduces no store of its own: the
        // kind-103 migration it names does. Verify the pair and move on.
        111 => {
            let p = addition::ReplicaPlan::decode(&t.payload)?;
            if p.root() != root || p.task() != t.id || p.encode() != t.payload {
                return Err(invalid("store-history replica plan differs"));
            }
            let (m, _) = Task::load(view, p.migration_task(), 103)?.migration(view, root)?;
            if m.operation() != p.operation() || m.region() != p.region() {
                return Err(invalid("store-history replica plan migration differs"));
            }
        }
        _ => {
            return Err(invalid(
                "unsupported task kind leaves DATA store history unknown",
            ))
        }
    }
    Ok(out)
}
pub fn task_id(key: &[u8], bytes: &[u8]) -> Result<u64> {
    Ok(Task::decode(key, bytes)?.id)
}

/// Follow one retained placement's original task chain to the native creation.
/// No global scan and no node endpoint/incarnation substitution.
pub fn initial_creation(
    view: &dyn ReadView,
    root: RootDigest,
    origin: &Origin,
) -> Result<(CreationIntent, RootDigest)> {
    let k = task_key(origin.task)?;
    let bytes = view
        .get(ColumnFamily::Default, &k)?
        .ok_or_else(|| invalid("initial source task absent"))?;
    if !project(view, root, &k, &bytes)?.contains(origin) {
        return Err(invalid("initial source placement differs"));
    }
    let t = Task::decode(&k, &bytes)?;
    let c = match t.kind {
        100 => t.creation(root)?,
        103 => t.migration(view, root)?.1,
        104 => t.evidence(view, root)?.1,
        106 => {
            let removal = removal::RemovalDecision::decode(&t.payload)?;
            Task::load(view, removal.evidence_task(), 104)?
                .evidence(view, root)?
                .1
        }
        108 => {
            let abort = abort::MigrationAbort::decode(&t.payload)?;
            Task::load(view, abort.migration_task(), 103)?
                .migration(view, root)?
                .1
        }
        _ => return Err(invalid("initial source task kind")),
    };
    if c.region() != origin.region || c.digest() != origin.creation {
        return Err(invalid("initial source creation differs"));
    }
    let task = Task::load(view, c.task(), 100)?;
    if task.creation(root)? != c {
        return Err(invalid("initial creation task changed"));
    }
    Ok((c, task.digest))
}

/// Select an exact historical store from immutable applied task bytes. Never
/// resolve a node through its current endpoint or substitute a replacement disk.
pub fn configuration_origin(
    view: &dyn ReadView,
    range: &DataRange,
    task: u64,
    operation: [u8; 16],
    replica: InitialReplica,
    action: kv9_raft::configuration_origin::Action,
) -> Result<kv9_raft::configuration_origin::ConfigurationOrigin> {
    let source = Task::load(view, task, action.task_kind())?;
    let origin = kv9_raft::configuration_origin::ConfigurationOrigin {
        root: range.root,
        creation: range.creation,
        region: range.region,
        operation,
        task,
        node: replica.node,
        store: replica.incarnation,
        source_digest: source.digest,
        action,
    };
    validate_configuration_origin(view, range.root, &origin)?;
    Ok(origin)
}

/// Independently reconcile a Raft observation with the retained catalog.
/// This proves this one origin only, not coverage of an entire node's history.
pub fn validate_configuration_origin(
    view: &dyn ReadView,
    root: RootDigest,
    origin: &kv9_raft::configuration_origin::ConfigurationOrigin,
) -> Result<()> {
    origin.encode()?;
    if origin.root != root {
        return Err(invalid("configuration origin root differs"));
    }
    let k = task_key(origin.task)?;
    let bytes = view
        .get(ColumnFamily::Default, &k)?
        .ok_or_else(|| invalid("configuration origin source absent"))?;
    let source = Task::decode(&k, &bytes)?;
    if source.kind != origin.action.task_kind()
        || source.payload.get(40..56) != Some(origin.operation.as_slice())
        || !project(view, root, &k, &bytes)?.contains(&Origin {
            region: origin.region,
            creation: origin.creation,
            replica: InitialReplica {
                node: origin.node,
                incarnation: origin.store,
            },
            task: origin.task,
            kind: origin.action.task_kind(),
            source_digest: origin.source_digest,
        })
    {
        return Err(invalid(
            "configuration origin does not match its retained task",
        ));
    }
    Ok(())
}
