//! Initial native configuration observation matched to the immutable creation.
//! This describes historical native voters, not all later stores or ancestry.
use super::*;
use crate::data_groups::{store_history::initial_creation, InitialReplica};
use kv9_common::{NodeId, StoreIncarnation};
use kv9_raft::storage::membership_history::InitialConfiguration;

const MAGIC: &[u8; 8] = b"KV9DOI01";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialOrigin {
    pub source: Source,
    pub recorded_at: u64,
    pub creation_task: u64,
    pub source_digest: RootDigest,
    pub replicas: Vec<InitialReplica>,
}
impl InitialOrigin {
    fn encode(&self, root: RootDigest) -> Result<Vec<u8>> {
        if self.recorded_at == 0
            || self.creation_task < 100
            || self.source_digest.as_bytes() == &[0; 32]
            || ![3, 5, 7].contains(&self.replicas.len())
            || self
                .replicas
                .iter()
                .any(|r| r.node.0 == 0 || r.incarnation.as_bytes() == &[0; 16])
            || self.replicas.windows(2).any(|w| w[0].node >= w[1].node)
            || self.replicas.iter().enumerate().any(|(i, r)| {
                self.replicas[..i]
                    .iter()
                    .any(|p| p.incarnation == r.incarnation)
            })
            || !self
                .replicas
                .iter()
                .any(|r| r.node == self.source.node && r.incarnation == self.source.store)
        {
            return Err(invalid("native initial origin identity"));
        }
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(&self.source.encode()?);
        uint(&mut b, self.recorded_at);
        uint(&mut b, self.creation_task);
        b.extend_from_slice(self.source_digest.as_bytes());
        b.push(self.replicas.len() as u8);
        for r in &self.replicas {
            uint(&mut b, r.node.0);
            b.extend_from_slice(r.incarnation.as_bytes());
        }
        Ok(seal(b))
    }
}
pub(in crate::retention) fn check_old_format(view: &dyn ReadView) -> Result<()> {
    if view
        .iter(ColumnFamily::Default, &key(56, &[]), &key(57, &[]))?
        .next()
        .transpose()?
        .is_some()
    {
        return Err(invalid("native initial origins under older ledger header"));
    }
    Ok(())
}
fn read(p: &Planner<'_>, source: Source) -> Result<Option<InitialOrigin>> {
    let Some(b) = p.view.get(ColumnFamily::Default, &source.key(56)?)? else {
        return Ok(None);
    };
    let mut r = Reader(checked(&b, MAGIC, 353)?);
    if r.array::<32>()? != *p.root.as_bytes() || Source::decode(&mut r)? != source {
        return Err(invalid("native initial origin scope"));
    }
    let recorded_at = r.uint()?;
    let creation_task = r.uint()?;
    let source_digest = RootDigest::from_bytes(r.array()?);
    let n = r.byte()?;
    if ![3, 5, 7].contains(&n) {
        return Err(invalid("native initial origin count"));
    }
    let mut replicas = Vec::with_capacity(n as usize);
    for _ in 0..n {
        replicas.push(InitialReplica {
            node: NodeId(r.uint()?),
            incarnation: StoreIncarnation::from_bytes(r.array()?),
        });
    }
    let out = InitialOrigin {
        source,
        recorded_at,
        creation_task,
        source_digest,
        replicas,
    };
    if !r.0.is_empty() || out.recorded_at > p.revision || out.encode(p.root)? != b {
        return Err(invalid("native initial origin encoding"));
    }
    Ok(Some(out))
}
pub fn observation(
    view: &dyn ReadView,
    root: &RootDescriptor,
    source: Source,
) -> Result<Option<InitialOrigin>> {
    let p = planner(view, root)?;
    let progress = super::read(&p, source)?;
    let out = read(&p, source)?;
    if let Some(o) = &out {
        let progress = progress.ok_or_else(|| invalid("native initial origin has no progress"))?;
        if o.recorded_at > progress.recorded_at {
            return Err(invalid("native initial origin ahead of progress"));
        }
    }
    Ok(out)
}
pub(super) fn validate_progress(p: &Planner<'_>, progress: &Progress) -> Result<()> {
    let stored = read(p, progress.source)?;
    if stored
        .as_ref()
        .is_some_and(|s| s.recorded_at > progress.recorded_at)
    {
        return Err(invalid("native initial origin ahead of progress"));
    }
    if let Some(InitialConfiguration::Voters(voters)) = &progress.page.initial {
        if stored
            .as_ref()
            .is_none_or(|s| !s.replicas.iter().map(|r| r.node).eq(voters.iter().copied()))
        {
            return Err(invalid(
                "native initial origin missing or different from progress",
            ));
        }
    }
    Ok(())
}
pub(super) fn plan(p: &mut Planner<'_>, q: Request, page: &OriginPage) -> Result<()> {
    let Some(InitialConfiguration::Voters(voters)) = &page.initial else {
        return Ok(());
    };
    let placement = data_store_history::exact_origin(
        p.view,
        p.root,
        q.source.region,
        q.source.creation,
        q.source.node,
        q.source.store,
        q.task,
    )?;
    let (creation, source_digest) = initial_creation(p.view, p.root, &placement)?;
    if !creation
        .replicas()
        .iter()
        .map(|r| r.node)
        .eq(voters.iter().copied())
        || !creation
            .replicas()
            .iter()
            .any(|r| r.node == q.source.node && r.incarnation == q.source.store)
    {
        return Err(conflict(
            "observed native initial configuration differs from original creation",
        ));
    }
    let old = read(p, q.source)?;
    let previous = super::read(p, q.source)?;
    if old.as_ref().is_some_and(|o| {
        previous
            .as_ref()
            .is_none_or(|v| o.recorded_at > v.recorded_at)
    }) || (old.is_none()
        && previous
            .as_ref()
            .is_some_and(|v| matches!(v.page.initial, Some(InitialConfiguration::Voters(_)))))
    {
        return Err(invalid(
            "native initial origin missing or ahead of progress",
        ));
    }
    let out = InitialOrigin {
        source: q.source,
        recorded_at: match &old {
            Some(o) => o.recorded_at,
            None => p
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("ledger revision exhausted"))?,
        },
        creation_task: creation.task(),
        source_digest,
        replicas: creation.replicas().to_vec(),
    };
    match old {
        Some(old) if old != out => return Err(conflict("native initial origin changed")),
        None => {
            p.writes.insert(q.source.key(56)?, out.encode(p.root)?);
        }
        _ => (),
    }
    Ok(())
}
