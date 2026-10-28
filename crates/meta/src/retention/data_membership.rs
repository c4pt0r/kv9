//! Durable observations of one exact source store's local configuration
//! history. Placement witnesses are not historical node/incarnation mappings;
//! a completed round is never an all-store gate or release capability.
use super::*;
use crate::data_groups::store_history::Origin;
use crate::endpoint::NodeEndpoint;
use kv9_common::{NodeId, RegionId, StoreIdentity, StoreIncarnation};
use kv9_raft::storage::membership_history::{MembershipCursor, MembershipPage, MAX_KNOWN_MEMBERS};

const PROGRESS_MAGIC: &[u8; 8] = b"KV9DMP01";
const MEMBER_MAGIC: &[u8; 8] = b"KV9DMM01";
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub region: RegionId,
    pub creation: RootDigest,
    pub node: NodeId,
    pub store: StoreIncarnation,
}
impl Source {
    pub fn from_origin(o: &Origin) -> Self {
        Self {
            region: o.region,
            creation: o.creation,
            node: o.replica.node,
            store: o.replica.incarnation,
        }
    }
    pub fn identity(self, root: &RootDescriptor) -> StoreIdentity {
        StoreIdentity {
            root_digest: root.digest(),
            cluster_id: root.cluster_id,
            node_id: self.node,
            store_incarnation: self.store,
        }
    }
    pub(super) fn encode(self) -> Result<Vec<u8>> {
        if self.region.0 < 100
            || self.creation.as_bytes() == &[0; 32]
            || self.node.0 == 0
            || self.store.as_bytes() == &[0; 16]
        {
            return Err(invalid("DATA membership source identity"));
        }
        let mut b = self.region.0.to_be_bytes().to_vec();
        b.extend_from_slice(self.creation.as_bytes());
        uint(&mut b, self.node.0);
        b.extend_from_slice(self.store.as_bytes());
        Ok(b)
    }
    pub(super) fn decode(r: &mut Reader<'_>) -> Result<Self> {
        let s = Self {
            region: RegionId(r.uint()?),
            creation: RootDigest::from_bytes(r.array()?),
            node: NodeId(r.uint()?),
            store: StoreIncarnation::from_bytes(r.array()?),
        };
        s.encode()?;
        Ok(s)
    }
    pub(super) fn key(self, namespace: u8) -> Result<Vec<u8>> {
        Ok(key(namespace, &self.encode()?))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub source: Source,
    pub task: u64,
    pub expected: u64,
    pub revision: u64,
}
impl Request {
    pub(super) fn encode(self) -> Result<Vec<u8>> {
        if self.task < 100 || self.revision == 0 || self.expected == u64::MAX {
            return Err(invalid("DATA membership request position"));
        }
        let mut b = self.source.encode()?;
        uint(&mut b, self.task);
        uint(&mut b, self.expected);
        uint(&mut b, self.revision);
        Ok(b)
    }
    pub(super) fn decode(r: &mut Reader<'_>) -> Result<Self> {
        let q = Self {
            source: Source::decode(r)?,
            task: r.uint()?,
            expected: r.uint()?,
            revision: r.uint()?,
        };
        q.encode()?;
        Ok(q)
    }
    pub(super) fn digest(self) -> Result<RootDigest> {
        Ok(RootDigest::sha256(&self.encode()?))
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub source: Source,
    pub sequence: u64,
    pub source_revision: u64,
    pub endpoint_generation: u64,
    pub recorded_at: u64,
    pub round: u64,
    pub seen: u64,
    pub unmatched_nodes: u64,
    pub retained_nodes: u64,
    pub retention_overflow: bool,
    /// Sticky missing/ambiguous/snapshot-source history, until a separate
    /// ancestry qualification protocol exists. Later clean pages cannot erase it.
    pub source_history_unknown: bool,
    pub page: MembershipPage,
    last_request: RootDigest,
}
impl Progress {
    pub fn round_complete(&self) -> bool {
        self.page.next.is_none()
    }
    pub fn request(&self, task: u64, revision: u64) -> LedgerRequest {
        LedgerRequest::RecordDataMembership(Request {
            source: self.source,
            task,
            expected: self.sequence,
            revision,
        })
    }
    /// Descriptive blockers only: their absence does not certify historical
    /// store identity coverage, snapshot ancestry, future closure or freshness.
    pub fn has_known_gaps(&self) -> bool {
        self.source_history_unknown
            || self.page.gaps.any()
            || self.unmatched_nodes != 0
            || self.retention_overflow
            || self.retained_nodes > self.page.known_nodes as u64
    }
    fn encode(&self, root: RootDigest) -> Result<Vec<u8>> {
        let page = self.page.encode()?;
        let mut b = PROGRESS_MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&self.source.encode()?);
        for n in [
            self.sequence,
            self.source_revision,
            self.endpoint_generation,
            self.recorded_at,
            self.round,
            self.seen,
            self.unmatched_nodes,
            self.retained_nodes,
        ] {
            uint(&mut b, n);
        }
        b.push(u8::from(self.retention_overflow));
        b.push(u8::from(self.source_history_unknown));
        b.extend_from_slice(self.last_request.as_bytes());
        b.push(page.len() as u8);
        b.extend_from_slice(&page);
        Ok(seal(b))
    }
}
pub(super) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(52, &[]), &key(54, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid(
            "DATA membership records under an older ledger header",
        ));
    }
    Ok(())
}
pub(super) fn planner<'a>(view: &'a dyn ReadView, root: &RootDescriptor) -> Result<Planner<'a>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let h = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("DATA membership ledger absent"))?;
    let (revision, rows_enabled) = read_header(&h, root.digest())?;
    if !h.starts_with(HEADER_MAGIC) {
        check_upgrade(view, &h)?;
        return Err(conflict(
            "DATA history observation requires the current ledger header",
        ));
    }
    Ok(Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    })
}
fn read(p: &Planner<'_>, source: Source) -> Result<Option<Progress>> {
    let Some(bytes) = p.view.get(ColumnFamily::Default, &source.key(52)?)? else {
        return Ok(None);
    };
    let mut r = Reader(checked(&bytes, PROGRESS_MAGIC, 347)?);
    if r.array::<32>()? != *p.root.as_bytes() || Source::decode(&mut r)? != source {
        return Err(invalid("DATA membership progress scope"));
    }
    let sequence = r.uint()?;
    let source_revision = r.uint()?;
    let endpoint_generation = r.uint()?;
    let recorded_at = r.uint()?;
    let round = r.uint()?;
    let seen = r.uint()?;
    let unmatched_nodes = r.uint()?;
    let retained_nodes = r.uint()?;
    let retention_overflow = r.boolean()?;
    let source_history_unknown = r.boolean()?;
    let last_request = RootDigest::from_bytes(r.array()?);
    let len = r.byte()? as usize;
    let page = MembershipPage::decode(r.take(len)?)?;
    let out = Progress {
        source,
        sequence,
        source_revision,
        endpoint_generation,
        recorded_at,
        round,
        seen,
        unmatched_nodes,
        retained_nodes,
        retention_overflow,
        source_history_unknown,
        last_request,
        page,
    };
    if !r.0.is_empty()
        || sequence == 0
        || sequence > recorded_at
        || recorded_at > p.revision
        || source_revision == 0
        || round == 0
        || round > sequence
        || seen > out.page.known_nodes as u64
        || seen < out.page.members.len() as u64
        || (out.page.gaps.any() && !source_history_unknown)
        || unmatched_nodes > seen
        || retained_nodes > MAX_KNOWN_MEMBERS as u64
        || (!retention_overflow && retained_nodes < seen)
        || (out.round_complete() && seen != out.page.known_nodes as u64)
        || (!out.round_complete()
            && (seen == 0 || !seen.is_multiple_of(4) || seen >= out.page.known_nodes as u64))
        || last_request.as_bytes() == &[0; 32]
        || out.encode(p.root)? != bytes
    {
        return Err(invalid("DATA membership progress encoding or count"));
    }
    Ok(Some(out))
}
pub fn observation(
    view: &dyn ReadView,
    root: &RootDescriptor,
    source: Source,
) -> Result<Option<Progress>> {
    read(&planner(view, root)?, source)
}
pub(super) fn check_source(p: &Planner<'_>, root: &RootDescriptor, q: Request) -> Result<()> {
    q.encode()?;
    let data_store_history::Work::Complete { revision } =
        data_store_history::observe(p.view, root)?
    else {
        return Err(conflict("DATA membership placement census incomplete"));
    };
    if q.revision != revision {
        return Err(conflict("DATA membership placement revision changed"));
    }
    data_store_history::exact_origin(
        p.view,
        p.root,
        q.source.region,
        q.source.creation,
        q.source.node,
        q.source.store,
        q.task,
    )?;
    Ok(())
}
/// Exact historical receipt replay may succeed after source unavailability or
/// catalog changes. It never refreshes source_revision or establishes freshness.
pub fn is_retry(view: &dyn ReadView, root: &RootDescriptor, q: Request) -> Result<bool> {
    q.encode()?;
    Ok(read(&planner(view, root)?, q.source)?.is_some_and(|p| {
        p.sequence == q.expected + 1 && p.last_request == q.digest().expect("validated request")
    }))
}
pub fn plan_retry(view: &dyn ReadView, root: &RootDescriptor, q: Request) -> Result<LedgerPlan> {
    if !is_retry(view, root, q)? {
        return Err(conflict("DATA membership receipt does not match"));
    }
    planner(view, root)?.finish()
}
pub fn continuation(
    view: &dyn ReadView,
    root: &RootDescriptor,
    q: Request,
) -> Result<(Option<MembershipCursor>, usize)> {
    let p = planner(view, root)?;
    check_source(&p, root, q)?;
    match read(&p, q.source)? {
        None if q.expected == 0 => Ok((None, 0)),
        Some(old) if old.sequence == q.expected => {
            if old.source_revision > q.revision {
                return Err(conflict("DATA membership placement revision regressed"));
            }
            if old.source_revision == q.revision && !old.round_complete() {
                Ok((old.page.next, old.seen as usize))
            } else {
                Ok((None, 0))
            }
        }
        _ => Err(conflict("DATA membership sequence changed")),
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub node: NodeId,
    pub roles: u8,
    pub first_observed: u64,
    pub last_observed: u64,
    pub round: u64,
    pub placement: Option<Origin>,
}
fn member_key(source: Source, node: NodeId) -> Result<Vec<u8>> {
    let mut k = source.key(53)?;
    uint(&mut k, node.0);
    Ok(k)
}
impl Member {
    fn encode(&self, root: RootDigest, source: Source) -> Result<Vec<u8>> {
        let mut b = MEMBER_MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&source.encode()?);
        uint(&mut b, self.node.0);
        b.push(self.roles);
        for n in [self.first_observed, self.last_observed, self.round] {
            uint(&mut b, n);
        }
        b.push(u8::from(self.placement.is_some()));
        if let Some(o) = &self.placement {
            b.extend_from_slice(&data_store_history::encode_origin(root, o));
        }
        Ok(seal(b))
    }
    fn decode(p: &Planner<'_>, source: Source, k: &[u8], bytes: &[u8]) -> Result<Self> {
        let mut r = Reader(checked(bytes, MEMBER_MAGIC, 354)?);
        if r.array::<32>()? != *p.root.as_bytes() || Source::decode(&mut r)? != source {
            return Err(invalid("DATA membership row scope"));
        }
        let node = NodeId(r.uint()?);
        let roles = r.byte()?;
        let first_observed = r.uint()?;
        let last_observed = r.uint()?;
        let round = r.uint()?;
        let placement = if r.boolean()? {
            let raw = r.take(184)?;
            // Origin key fields are decoded from its bounded canonical payload.
            let mut o = Reader(checked(raw, b"KV9DSH01", 184)?);
            o.array::<32>()?;
            let region = RegionId(o.uint()?);
            let creation = RootDigest::from_bytes(o.array()?);
            let replica = crate::data_groups::InitialReplica {
                node: NodeId(o.uint()?),
                incarnation: StoreIncarnation::from_bytes(o.array()?),
            };
            let task = o.uint()?;
            let kind = o.uint()?;
            let source_digest = RootDigest::from_bytes(o.array()?);
            let candidate = Origin {
                region,
                creation,
                replica,
                task,
                kind,
                source_digest,
            };
            Some(data_store_history::decode_origin(
                p.root,
                &data_store_history::origin_key(&candidate),
                raw,
            )?)
        } else {
            None
        };
        let out = Self {
            node,
            roles,
            first_observed,
            last_observed,
            round,
            placement,
        };
        if !r.0.is_empty()
            || node.0 == 0
            || roles == 0
            || roles & !15 != 0
            || first_observed == 0
            || first_observed > last_observed
            || last_observed > p.revision
            || round == 0
            || round > last_observed
            || k != member_key(source, node)?
            || out.encode(p.root, source)? != bytes
            || out.placement.is_some_and(|o| {
                o.region != source.region || o.creation != source.creation || o.replica.node != node
            })
        {
            return Err(invalid("DATA membership row encoding"));
        }
        Ok(out)
    }
}
/// Trusted runtime seam: the page must come from independent exact-store RPC,
/// including its cursor/seen correlation; recheck endpoint under catalog lock.
pub fn plan_verified(
    view: &dyn ReadView,
    root: &RootDescriptor,
    q: Request,
    endpoint: NodeEndpoint,
    cursor: Option<MembershipCursor>,
    seen: usize,
    page: &MembershipPage,
) -> Result<LedgerPlan> {
    let mut p = planner(view, root)?;
    check_source(&p, root, q)?;
    // Endpoint equality is independently checked by the runtime immediately
    // before this planner; this module validates the source descriptor itself.
    if !endpoint.active
        || endpoint.node != q.source.node
        || endpoint.incarnation != q.source.store
        || endpoint.address.port() == 0
        || endpoint.address.ip().is_unspecified()
        || root
            .voter(endpoint.node)
            .is_some_and(|v| v.store_incarnation != endpoint.incarnation)
    {
        return Err(conflict("DATA membership endpoint differs"));
    }
    let current = read(&p, q.source)?;
    if current
        .as_ref()
        .is_some_and(|old| page.last_configuration_index < old.page.last_configuration_index)
    {
        return Err(conflict("DATA membership configuration position regressed"));
    }
    if current.as_ref().map_or(0, |p| p.sequence) != q.expected {
        return Err(conflict("DATA membership sequence changed"));
    }
    page.validate_progress(cursor, seen)?;
    let continuation = continuation(view, root, q)?;
    if cursor.is_some()
        && (continuation != (cursor, seen)
            || current
                .as_ref()
                .is_none_or(|v| v.endpoint_generation != endpoint.generation))
    {
        return Err(conflict("DATA membership continuation differs"));
    }
    if cursor.is_none() && seen != 0 {
        return Err(invalid("DATA membership reset skips prefix"));
    }
    // Fresh first-page confirmation can retain a fully recorded same-source
    // round. It creates no new durable assertion or refreshed observation time.
    if let Some(old) = &current {
        if old.round_complete()
            && old.source_revision == q.revision
            && old.endpoint_generation == endpoint.generation
            && old.page.fingerprint == page.fingerprint
            && cursor.is_none()
        {
            return p.finish();
        }
    }
    let sequence = q.expected + 1;
    let at = p
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("ledger revision exhausted"))?;
    let round = if cursor.is_none() {
        sequence
    } else {
        current.as_ref().unwrap().round
    };
    let mut retained = current.as_ref().map_or(0, |v| v.retained_nodes);
    let mut overflow = current.as_ref().is_some_and(|v| v.retention_overflow);
    let mut unmatched = if cursor.is_none() {
        0
    } else {
        current.as_ref().unwrap().unmatched_nodes
    };
    for entry in &page.members {
        let k = member_key(q.source, entry.node)?;
        let old = view
            .get(ColumnFamily::Default, &k)?
            .map(|b| Member::decode(&p, q.source, &k, &b))
            .transpose()?;
        let placement = data_store_history::node_origin(
            view,
            p.root,
            q.source.region,
            q.source.creation,
            entry.node,
        )?;
        unmatched += u64::from(placement.is_none());
        if old.is_none() && retained == MAX_KNOWN_MEMBERS as u64 {
            overflow = true;
            continue;
        }
        let member = Member {
            node: entry.node,
            roles: entry.roles | old.as_ref().map_or(0, |v| v.roles),
            first_observed: old.as_ref().map_or(at, |v| v.first_observed),
            last_observed: at,
            round,
            placement,
        };
        retained += u64::from(old.is_none());
        p.writes.insert(k, member.encode(p.root, q.source)?);
    }
    let progress = Progress {
        source: q.source,
        sequence,
        source_revision: q.revision,
        endpoint_generation: endpoint.generation,
        recorded_at: at,
        round,
        seen: seen as u64 + page.members.len() as u64,
        unmatched_nodes: unmatched,
        retained_nodes: retained,
        retention_overflow: overflow,
        source_history_unknown: page.gaps.any()
            || current.as_ref().is_some_and(|p| p.source_history_unknown),
        page: page.clone(),
        last_request: q.digest()?,
    };
    p.writes.insert(q.source.key(52)?, progress.encode(p.root)?);
    p.finish()
}
#[derive(Debug, Clone, Copy)]
pub struct MemberCursor {
    source: Source,
    sequence: u64,
    after: NodeId,
    seen: u64,
}
pub struct MemberPage {
    pub progress: Progress,
    /// The retained placement census is complete at this observation's task
    /// revision. This still does not establish live membership freshness.
    pub placement_current: bool,
    pub members: Vec<Member>,
    pub next: Option<MemberCursor>,
}
pub fn members(
    view: &dyn ReadView,
    root: &RootDescriptor,
    source: Source,
    after: Option<MemberCursor>,
) -> Result<MemberPage> {
    let p = planner(view, root)?;
    let progress = read(&p, source)?.ok_or_else(|| conflict("DATA membership not observed"))?;
    if after.is_some_and(|c| {
        c.source != source
            || c.sequence != progress.sequence
            || c.after.0 == 0
            || c.seen == 0
            || !c.seen.is_multiple_of(4)
            || c.seen >= progress.retained_nodes
    }) {
        return Err(conflict("DATA membership query cursor differs"));
    }
    let mut start = source.key(53)?;
    let mut end = start.clone();
    while end.last() == Some(&255) {
        end.pop();
    }
    *end.last_mut().unwrap() += 1;
    if let Some(c) = after {
        uint(&mut start, c.after.0);
        start.push(0);
    }
    let mut rows = Vec::with_capacity(4);
    let mut more = false;
    for row in view.iter(ColumnFamily::Default, &start, &end)?.take(5) {
        let (k, b) = row?;
        let m = Member::decode(&p, source, &k, &b)?;
        if m.round > progress.round || m.last_observed > progress.recorded_at {
            return Err(invalid("DATA membership row ahead of progress"));
        }
        if rows.len() == 4 {
            more = true;
            break;
        }
        rows.push(m);
    }
    let seen = after.map_or(0, |c| c.seen) + rows.len() as u64;
    if (more && seen >= progress.retained_nodes) || (!more && seen != progress.retained_nodes) {
        return Err(invalid("DATA membership index count differs"));
    }
    let next = if more {
        Some(MemberCursor {
            source,
            sequence: progress.sequence,
            after: rows.last().unwrap().node,
            seen,
        })
    } else {
        None
    };
    let placement_current = matches!(data_store_history::observe(view, root)?,
        data_store_history::Work::Complete { revision } if revision == progress.source_revision);
    Ok(MemberPage {
        progress,
        placement_current,
        members: rows,
        next,
    })
}
#[cfg(test)]
mod tests;
