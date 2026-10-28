//! Durable progress, not reclamation authority. One constant-size cursor per
//! closed attempt lets different leaders fairly revisit arbitrarily many
//! attempts without keeping a per-attempt map or relying on local journals.
use super::*;

const MAGIC: &[u8; 8] = b"KV9CSW01";
pub const SWEEP_PAGE: usize = 4;
fn cursor_key(pending: OwnerId) -> Vec<u8> {
    key(12, pending.as_bytes())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepCursor {
    pub sequence: u64,
    pub after: Option<OwnerId>,
    pub done: bool,
    previous: Option<OwnerId>,
}
impl SweepCursor {
    pub fn advance_request(&self, pending: OwnerId, next: Option<OwnerId>) -> LedgerRequest {
        LedgerRequest::AdvanceCheckpointSweep {
            pending: OwnerToken {
                owner: pending,
                generation: NonZeroU64::new(1).unwrap(),
            },
            expected: self.sequence,
            after: self.after,
            next,
        }
    }
    fn encode(&self, root: RootDigest, gate: &Admission) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        b.extend_from_slice(root.as_bytes());
        b.extend_from_slice(gate.pending.as_bytes());
        uint(&mut b, self.sequence);
        uint(&mut b, gate.closed_at);
        b.push(u8::from(self.done));
        for id in [self.after, self.previous] {
            b.push(u8::from(id.is_some()));
            if let Some(id) = id {
                b.extend_from_slice(id.as_bytes());
            }
        }
        seal(b)
    }
}
fn cursor(p: &Planner<'_>, gate: &Admission) -> Result<SweepCursor> {
    let Some(bytes) = p
        .view
        .get(ColumnFamily::Default, &cursor_key(gate.pending))?
    else {
        // Restarting a lost progress hint rescans; it cannot skip work or grant
        // any release authority. The gate/owner/evidence records are mandatory.
        return Ok(SweepCursor::default());
    };
    let mut r = Reader(checked(&bytes, MAGIC, 139)?);
    if r.array::<32>()? != *p.root.as_bytes() || r.array::<16>()? != *gate.pending.as_bytes() {
        return Err(invalid("checkpoint sweep identity differs"));
    }
    let sequence = r.uint()?;
    if r.uint()? != gate.closed_at || !gate.is_closed() {
        return Err(invalid("checkpoint sweep closure differs"));
    }
    let done = r.boolean()?;
    let mut id = || -> Result<Option<OwnerId>> {
        if r.boolean()? {
            Ok(Some(OwnerId::new(r.array()?).map_err(pin_error)?))
        } else {
            Ok(None)
        }
    };
    let sweep = SweepCursor {
        sequence,
        done,
        after: id()?,
        previous: id()?,
    };
    if !r.0.is_empty()
        || sequence == 0
        || sequence > p.revision
        || (sequence == 1) != sweep.previous.is_none()
        || done != sweep.after.is_none()
        || sweep
            .after
            .zip(sweep.previous)
            .is_some_and(|(next, old)| next <= old)
        || sweep.encode(p.root, gate) != bytes
    {
        return Err(invalid("checkpoint sweep cursor is invalid"));
    }
    Ok(sweep)
}

/// At most one gate key. Caller must validate its gate/intent before acting;
/// keeping discovery separate lets one damaged value report an error without
/// forever hiding later valid keys. EOF is only a scan observation.
pub fn next_attempt(
    view: &dyn ReadView,
    root: &RootDescriptor,
    after: Option<OwnerId>,
) -> Result<Option<OwnerId>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let prefix = key(9, &[]);
    let end = key(10, &[]);
    let header = view.get(ColumnFamily::Default, &key(1, &[]))?;
    match header {
        Some(bytes) => {
            read_header(&bytes, root.digest())?;
        }
        None => {
            let mut namespace_end = PREFIX.to_vec();
            *namespace_end.last_mut().unwrap() += 1;
            if view
                .iter(ColumnFamily::Default, PREFIX, &namespace_end)?
                .next()
                .transpose()?
                .is_some()
            {
                return Err(invalid("checkpoint discovery lost ledger header"));
            }
            return Ok(None);
        }
    }
    let mut start = after.map_or_else(|| prefix.clone(), gate_key);
    if after.is_some() {
        start.push(0);
    }
    let Some((key, _)) = view
        .iter(ColumnFamily::Default, &start, &end)?
        .next()
        .transpose()?
    else {
        return Ok(None);
    };
    if key.len() != prefix.len() + 16 {
        return Err(invalid("checkpoint discovery key length"));
    }
    Ok(Some(
        OwnerId::new(key[prefix.len()..].try_into().unwrap()).map_err(pin_error)?,
    ))
}

pub enum SweepObservation {
    Open,
    Done,
    Page {
        cursor: SweepCursor,
        page: ExecutorPage,
    },
}
/// One established snapshot; at most five executor index rows (four plus
/// lookahead). Done only means this indexed Held-only sweep finished.
pub fn observe(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
) -> Result<SweepObservation> {
    let intent = checkpoint_intent::checkpoint_intent(view, root, pending)?
        .ok_or_else(|| invalid("checkpoint sweep intent missing"))?;
    let header = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("checkpoint sweep header missing"))?;
    let (revision, rows_enabled) = read_header(&header, root.digest())?;
    let p = Planner {
        view,
        root: root.digest(),
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    let gate = load(&p, &intent)?;
    let progress = cursor(&p, &gate)?;
    if !gate.is_closed() {
        return Ok(SweepObservation::Open);
    }
    if progress.done {
        return Ok(SweepObservation::Done);
    }
    let page = executors(view, root, pending, progress.after, SWEEP_PAGE)?;
    Ok(SweepObservation::Page {
        cursor: progress,
        page,
    })
}

pub(in crate::retention) fn advance(
    p: &mut Planner<'_>,
    root: &RootDescriptor,
    pending: OwnerToken,
    expected: u64,
    after: Option<OwnerId>,
    next: Option<OwnerId>,
) -> Result<()> {
    let intent = checkpoint_intent::load(p, pending.owner)?
        .ok_or_else(|| conflict("checkpoint sweep intent absent"))?;
    if intent.binding(OwnerKind::Pending)?.token() != pending {
        return Err(conflict("checkpoint sweep pending differs"));
    }
    let gate = load(p, &intent)?;
    if !gate.is_closed() {
        return Err(conflict("checkpoint sweep requires closed admission"));
    }
    let current = cursor(p, &gate)?;
    let sequence = expected
        .checked_add(1)
        .ok_or_else(|| invalid("checkpoint sweep sequence exhausted"))?;
    if current.sequence == sequence
        && current.previous == after
        && current.after == next
        && current.done == next.is_none()
    {
        return Ok(());
    }
    if current.sequence != expected || current.after != after || current.done {
        return Err(conflict("checkpoint sweep cursor changed"));
    }
    let page = executors(p.view, root, pending.owner, after, SWEEP_PAGE)?;
    if page.next != next || page.owners.iter().any(|o| o.phase == PinPhase::Held) {
        return Err(conflict(
            "checkpoint sweep page still has Held work or cursor differs",
        ));
    }
    let advanced = SweepCursor {
        sequence,
        after: next,
        done: next.is_none(),
        previous: after,
    };
    p.writes
        .insert(cursor_key(pending.owner), advanced.encode(p.root, &gate));
    Ok(())
}

#[cfg(test)]
mod tests;
