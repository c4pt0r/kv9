use super::*;
use crate::{FenceAdjudicator, FencedInner, KvOp, RegionFence};
use kv9_common::{
    codec::{decode_key, KeyMode},
    data_range::{DataRange, RANGE_KEY},
    Error, RegionId, RootDigest,
};

struct RangeFence<E: ApplyStore> {
    engine: Arc<E>,
    identity: (RootDigest, RegionId, RootDigest),
}

impl<E: ApplyStore> RangeFence<E> {
    fn current(&self) -> Result<Option<DataRange>> {
        let range = self
            .engine
            .get(ColumnFamily::Default, RANGE_KEY)?
            .map(|b| DataRange::decode(&b))
            .transpose()?;
        if range
            .as_ref()
            .is_some_and(|r| (r.root, r.region, r.creation) != self.identity)
        {
            return Err(Error::Raft(
                "local data range belongs to another group".into(),
            ));
        }
        Ok(range)
    }
    fn matches(range: &DataRange, fence: &RegionFence) -> bool {
        !range.sealed
            && range.region.0 == fence.region_id
            && range.conf_ver == fence.conf_ver
            && range.version == fence.version
    }
}
impl<E: ApplyStore> FenceAdjudicator for RangeFence<E> {
    fn is_fresh(&self, fence: &RegionFence) -> Result<bool> {
        Ok(self.current()?.is_some_and(|r| Self::matches(&r, fence)))
    }
    fn is_fresh_write(&self, fence: &RegionFence, inner: &FencedInner) -> Result<bool> {
        let Some(range) = self.current()? else {
            return Ok(false);
        };
        if !Self::matches(&range, fence) {
            return Ok(false);
        }
        let FencedInner::Write { ops } = inner;
        Ok(ops.iter().all(|op| {
            let (cf, key) = match op {
                KvOp::Put { cf, key, .. } | KvOp::Delete { cf, key } => (*cf, key),
            };
            cf == crate::cf_code(ColumnFamily::Default)
                && decode_key(key).is_ok_and(|k| {
                    k.mode == KeyMode::Raw
                        && k.keyspace == range.keyspace
                        && range.contains(k.user_key)
                })
        }))
    }
    fn independent_of_raw_writes(&self) -> bool {
        true
    }
}

impl<E: ApplyStore> MemStateMachine<E> {
    /// Immutable group identity comes from the validated durable creation record.
    /// Dynamic ownership comes ONLY from this group's own ordered log.
    pub fn set_data_group(
        &mut self,
        root: RootDigest,
        region: RegionId,
        creation: RootDigest,
    ) -> Result<()>
    where
        E: 'static,
    {
        if self.data_identity.is_some()
            || root.as_bytes() == &[0; 32]
            || region.0 < 100
            || creation.as_bytes() == &[0; 32]
        {
            return Err(Error::Raft(
                "invalid or duplicate data group configuration".into(),
            ));
        }
        let fence = RangeFence {
            engine: self.engine.clone(),
            identity: (root, region, creation),
        };
        fence.current()?;
        self.data_identity = Some(fence.identity);
        self.adjudicator = Some(Arc::new(fence));
        Ok(())
    }

    pub(super) fn apply_data_range(
        &mut self,
        at: AppliedPosition,
        expected: Option<RootDigest>,
        next: &DataRange,
    ) -> Result<ApplyResult> {
        next.validate()?;
        if self.data_identity != Some((next.root, next.region, next.creation)) {
            return Err(Error::Raft(
                "data range command lacks exact local group identity".into(),
            ));
        }
        let current = self
            .engine
            .get(ColumnFamily::Default, RANGE_KEY)?
            .map(|b| DataRange::decode(&b))
            .transpose()?;
        if current
            .as_ref()
            .is_some_and(|r| self.data_identity != Some((r.root, r.region, r.creation)))
        {
            return Err(Error::Raft(
                "stored range has a foreign group identity".into(),
            ));
        }
        let accepted = if current.as_ref() == Some(next) {
            true // New confirmation of identical state, not an old mutation receipt.
        } else {
            match (&current, expected) {
                (None, None) => !next.sealed && next.conf_ver == 1 && next.version == 1,
                (Some(prior), Some(digest)) => prior.digest() == digest && next.may_follow(prior),
                _ => false,
            }
        };
        let mut batch = WriteBatch::new();
        if accepted && current.as_ref() != Some(next) {
            batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), next.encode());
        }
        self.engine.write_applied(batch, at)?;
        self.applied = LogIndex(at.index);
        Ok(if accepted {
            ApplyResult::write_ok(self.applied)
        } else {
            ApplyResult::fence_rejected(self.applied, next.region)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::{KeyspaceId, TenantId};
    use kv9_engine::{Engine, MemEngine};
    fn binding() -> DataRange {
        DataRange {
            root: RootDigest::from_bytes([1; 32]),
            creation: RootDigest::from_bytes([2; 32]),
            region: RegionId(100),
            keyspace: KeyspaceId(100),
            tenant: TenantId(1),
            conf_ver: 1,
            version: 1,
            start: b"b".to_vec(),
            end: b"m".to_vec(),
            sealed: false,
        }
    }
    fn write(r: &DataRange, keyspace: KeyspaceId, key: &[u8]) -> Command {
        Command::Fenced {
            fence: RegionFence {
                region_id: r.region.0,
                conf_ver: r.conf_ver,
                version: r.version,
            },
            inner: FencedInner::Write {
                ops: vec![KvOp::Put {
                    cf: 0,
                    key: kv9_common::codec::encode_key(KeyMode::Raw, keyspace, key).unwrap(),
                    value: b"value".to_vec(),
                }],
            },
        }
    }
    #[test]
    fn data_range_codec_and_wire_reject_truncation_and_unknown_forms() {
        let r = binding();
        let bytes = r.encode();
        assert_eq!(DataRange::decode(&bytes).unwrap(), r);
        for cut in 0..bytes.len() {
            assert!(DataRange::decode(&bytes[..cut]).is_err());
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(DataRange::decode(&extra).is_err());
        let command = Command::DataRange {
            expected: None,
            next: r.clone(),
        };
        assert_eq!(Command::decode(&command.encode()).unwrap(), command);
        assert!(command.to_write_batch().is_err());
        for cut in 0..command.encode().len() {
            assert!(Command::decode(&command.encode()[..cut]).is_err());
        }
        let mut malformed = r;
        malformed.end = malformed.start.clone();
        assert!(DataRange::decode(&malformed.encode()).is_err());
    }
    #[test]
    fn data_range_authority_corruption_cannot_become_a_stale_verdict_or_advance_apply() {
        let range = binding();
        for corrupt in [vec![0; 117], {
            let mut foreign = range.clone();
            foreign.creation = RootDigest::from_bytes([3; 32]);
            foreign.encode()
        }] {
            let engine = Arc::new(MemEngine::new());
            let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
            sm.set_data_group(range.root, range.region, range.creation)
                .unwrap();
            let mut batch = WriteBatch::new();
            batch.put(ColumnFamily::Default, RANGE_KEY.to_vec(), corrupt);
            Engine::write(engine.as_ref(), batch).unwrap();
            let at = AppliedPosition { term: 1, index: 1 };
            assert!(sm
                .apply_at(at, &write(&range, range.keyspace, b"c"))
                .is_err());
            assert!(sm
                .apply_raw_group(&[(at, write(&range, range.keyspace, b"c"))])
                .is_err());
            assert!(sm
                .apply_at(
                    at,
                    &Command::DataRange {
                        expected: None,
                        next: range.clone()
                    }
                )
                .is_err());
            assert_eq!(sm.applied_index(), LogIndex(0));
            assert!(MemStateMachine::with_engine(engine)
                .unwrap()
                .set_data_group(range.root, range.region, range.creation)
                .is_err());
        }
    }
    #[test]
    fn data_range_apply_checks_namespace_bounds_epoch_and_terminal_seal() {
        let r = binding();
        let engine = Arc::new(MemEngine::new());
        let mut sm = MemStateMachine::with_engine(engine.clone()).unwrap();
        sm.set_data_group(r.root, r.region, r.creation).unwrap();
        let mut index = 0;
        let mut apply = |sm: &mut MemStateMachine<MemEngine>, cmd: &Command| {
            index += 1;
            sm.apply_at(AppliedPosition { term: 1, index }, cmd)
                .unwrap()
        };
        assert!(matches!(
            apply(&mut sm, &write(&r, r.keyspace, b"c")).outcome,
            super::super::ApplyOutcome::FenceRejected(_)
        ));
        let init = Command::DataRange {
            expected: None,
            next: r.clone(),
        };
        apply(&mut sm, &init);
        apply(&mut sm, &write(&r, r.keyspace, b"c"));
        let mut future = r.clone();
        future.version += 1;
        let mut old = r.clone();
        old.conf_ver = 0;
        for bad in [
            write(&future, r.keyspace, b"d"),
            write(&old, r.keyspace, b"d"),
            write(&r, KeyspaceId(101), b"d"),
            write(&r, r.keyspace, b"a"),
            write(&r, r.keyspace, b"m"),
        ] {
            assert!(matches!(
                apply(&mut sm, &bad).outcome,
                super::super::ApplyOutcome::FenceRejected(_)
            ));
        }
        // A boundary-crossing batch must have no accepted prefix, even when
        // coalesced with neighboring writes by the production Raw apply grouper.
        let mut bad = write(&r, r.keyspace, b"d");
        let Command::Fenced {
            inner: FencedInner::Write { ops },
            ..
        } = &mut bad
        else {
            unreachable!()
        };
        ops.push(KvOp::Put {
            cf: 0,
            key: kv9_common::codec::encode_key(KeyMode::Raw, r.keyspace, b"z").unwrap(),
            value: b"outside".to_vec(),
        });
        let at = AppliedPosition {
            term: 1,
            index: index + 1,
        };
        let results = sm.apply_raw_group(&[(at, bad)]).unwrap();
        index += 1;
        assert!(matches!(
            results[0].outcome,
            super::super::ApplyOutcome::FenceRejected(_)
        ));
        assert!(Engine::get(
            engine.as_ref(),
            ColumnFamily::Default,
            &kv9_common::codec::encode_key(KeyMode::Raw, r.keyspace, b"d").unwrap()
        )
        .unwrap()
        .is_none());
        let mut sealed = r.clone();
        sealed.version += 1;
        sealed.sealed = true;
        sm.apply_at(
            AppliedPosition {
                term: 1,
                index: index + 1,
            },
            &Command::DataRange {
                expected: Some(r.digest()),
                next: sealed.clone(),
            },
        )
        .unwrap();
        index += 1;
        for cmd in [
            write(&r, r.keyspace, b"e"),
            write(&sealed, r.keyspace, b"e"),
            init,
        ] {
            index += 1;
            assert!(matches!(
                sm.apply_at(AppliedPosition { term: 1, index }, &cmd)
                    .unwrap()
                    .outcome,
                super::super::ApplyOutcome::FenceRejected(_)
            ));
        }
        assert_eq!(
            DataRange::decode(
                &Engine::get(engine.as_ref(), ColumnFamily::Default, RANGE_KEY)
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            sealed
        );
        assert_eq!(sm.applied_index(), LogIndex(index));
    }
}
