//! Raw (direct-KV) executor for `raw` keyspaces (DESIGN §9.2).
//!
//! `RawPut/RawGet/RawDelete/RawScan/RawBatchGet`, optional TTL, optional causal
//! timestamps for ordering without full transactions. No locks, no 2PC.
//!
//! # Why this type holds nothing
//!
//! [`RawExecutor`] owns no engine handle, no raft handle, and no read view. That is
//! deliberate, and it is the whole safety argument of this module:
//!
//! * It **cannot write storage directly.** A raw write is replicated state; if this type
//!   could reach an `Engine` it could apply a mutation on one node only, and the replicas
//!   would diverge *silently* — no error, no log line, just nodes that disagree. Instead
//!   it returns a [`WriteBatch`] *plan*, and only a caller holding the raft driver can
//!   turn that plan into a committed entry.
//! * It **cannot read a follower's state by accident.** Reads take a [`LeaderRead`],
//!   which cannot be constructed without supplying the answer to "am I the leader?".
//!
//! Both properties are enforced by types rather than by comment, because the natural way
//! to "just fill in the stubs" — hand the executor an engine and write — produces a
//! system that looks healthy and is wrong.
//!
//! # Physical keys
//!
//! Every key crossing this module is encoded through [`kv9_common::codec`] as
//! `mode_byte + keyspace_id + user_key` (DESIGN §3.4). That prefix is the *only* thing
//! keeping one tenant's keys from colliding with another's, so no path here may take a
//! caller-supplied key and use it unprefixed.

use kv9_common::codec::{decode_key, encode_key, keyspace_range, KeyMode};
use kv9_common::{Error, KeyspaceId, Result, UserKey, Value};
use kv9_engine::{ColumnFamily, ReadView, WriteBatch};

/// Raw values live in the default column family; `lock`/`write` belong to Percolator.
const RAW_CF: ColumnFamily = ColumnFamily::Default;

/// Optional per-key metadata for raw writes (DESIGN §9.2).
#[derive(Debug, Clone, Copy, Default)]
pub struct RawWriteOptions {
    /// TTL in seconds (`None` = no expiry).
    pub ttl_secs: Option<u64>,
    /// Optional causal timestamp (monotonic per key) for ordering (DESIGN §9.2).
    pub causal_ts: Option<u64>,
}

impl RawWriteOptions {
    /// Reject options this executor does not yet honour.
    ///
    /// Accepting a TTL and never expiring the key is worse than refusing it: the caller
    /// would believe the data disappears, and it never would.
    fn reject_unsupported(&self) -> Result<()> {
        if self.ttl_secs.is_some() {
            return Err(Error::NotImplemented("raw TTL"));
        }
        if self.causal_ts.is_some() {
            return Err(Error::NotImplemented("raw causal timestamp"));
        }
        Ok(())
    }
}

/// A read view obtainable only after confirming this node currently leads.
///
/// The constructor takes the leadership answer as an argument, so a caller cannot reach a
/// read without having asked the question. This does **not** make raw reads linearizable:
/// `check_quorum` bounds how long a deposed leader keeps believing it leads (roughly one
/// election timeout), but inside that window it still answers from stale state. Only a
/// ReadIndex round-trip closes that window, and it is not wired yet — so what we document
/// to users must say "fresh in normal operation", never "strongly consistent".
pub struct LeaderRead<'a> {
    view: &'a dyn ReadView,
}

impl<'a> LeaderRead<'a> {
    /// Wrap `view` for reading, given the caller's leadership check.
    ///
    /// `leader_hint` is carried into the error so a client can be redirected.
    pub fn new(view: &'a dyn ReadView, is_leader: bool, leader_hint: Option<u64>) -> Result<Self> {
        if !is_leader {
            return Err(Error::NotLeader { leader: leader_hint });
        }
        Ok(LeaderRead { view })
    }
}

/// The raw executor (DESIGN §9.2).
///
/// Stateless by construction — see the module docs for why that is the point.
#[derive(Debug, Default, Clone, Copy)]
pub struct RawExecutor;

impl RawExecutor {
    pub fn new() -> Self {
        RawExecutor
    }

    /// Plan a single put. The batch is *not* applied here; the caller proposes it.
    pub fn plan_put(
        &self,
        keyspace: KeyspaceId,
        key: &[u8],
        value: Value,
        opts: RawWriteOptions,
    ) -> Result<WriteBatch> {
        opts.reject_unsupported()?;
        let mut batch = WriteBatch::new();
        batch.put(RAW_CF, encode_key(KeyMode::Raw, keyspace, key)?, value);
        Ok(batch)
    }

    /// Plan a batch put. One batch ⇒ one raft entry ⇒ applied atomically.
    pub fn plan_batch_put(
        &self,
        keyspace: KeyspaceId,
        pairs: &[(UserKey, Value)],
        opts: RawWriteOptions,
    ) -> Result<WriteBatch> {
        opts.reject_unsupported()?;
        let mut batch = WriteBatch::new();
        for (key, value) in pairs {
            batch.put(
                RAW_CF,
                encode_key(KeyMode::Raw, keyspace, key)?,
                value.clone(),
            );
        }
        Ok(batch)
    }

    /// Plan a single delete.
    pub fn plan_delete(&self, keyspace: KeyspaceId, key: &[u8]) -> Result<WriteBatch> {
        let mut batch = WriteBatch::new();
        batch.delete(RAW_CF, encode_key(KeyMode::Raw, keyspace, key)?);
        Ok(batch)
    }

    /// Plan **one bounded chunk** of a range delete, resuming from `from`.
    ///
    /// Returns the batch plus the last physical key it covered, so the caller can commit
    /// that chunk and resume strictly after it. Returns `None` when the range is
    /// exhausted.
    ///
    /// An earlier version read the whole range with `usize::MAX` and then sliced it into
    /// chunks. That bounded the raft *entry* while leaving the planner itself unbounded —
    /// a range of a hundred million keys was materialised in memory first. The comment
    /// even cited DESIGN §13 principle 13 ("no unquota'd in-memory path") while the code
    /// broke it, which is worse than a silent bug: the citation tells the next reader the
    /// question has already been settled. Nothing here may read more than `chunk` rows.
    ///
    /// **Atomicity is per chunk, not per range**, and that is now observable rather than
    /// merely documented: the caller learns how many chunks committed and where it
    /// stopped, so a half-finished delete cannot masquerade as one that never began.
    pub fn plan_delete_range_chunk(
        &self,
        read: &LeaderRead<'_>,
        keyspace: KeyspaceId,
        from: Option<&[u8]>,
        start: &[u8],
        end: &[u8],
        chunk: usize,
    ) -> Result<Option<(WriteBatch, UserKey)>> {
        if chunk == 0 {
            return Err(Error::Config(
                "raw delete_range chunk size must be non-zero".into(),
            ));
        }
        let (range_lo, hi) = Self::bounds(keyspace, start, end)?;
        // Resume strictly after the last key already deleted.
        let lo = match from {
            Some(previous) => {
                let mut next = previous.to_vec();
                next.push(0);
                next.max(range_lo)
            }
            None => range_lo,
        };
        if lo >= hi {
            return Ok(None);
        }

        let doomed = read.view.scan(RAW_CF, &lo, &hi, chunk)?;
        let Some((last_key, _)) = doomed.last() else {
            return Ok(None);
        };
        let last_key = last_key.clone();

        let mut batch = WriteBatch::new();
        for (physical_key, _) in doomed {
            batch.delete(RAW_CF, physical_key);
        }
        Ok(Some((batch, last_key)))
    }

    /// Point read from this keyspace.
    pub fn get(
        &self,
        read: &LeaderRead<'_>,
        keyspace: KeyspaceId,
        key: &[u8],
    ) -> Result<Option<Value>> {
        read.view
            .get(RAW_CF, &encode_key(KeyMode::Raw, keyspace, key)?)
    }

    /// Multi-key point read, preserving request order.
    pub fn batch_get(
        &self,
        read: &LeaderRead<'_>,
        keyspace: KeyspaceId,
        keys: &[UserKey],
    ) -> Result<Vec<Option<Value>>> {
        keys.iter()
            .map(|key| self.get(read, keyspace, key))
            .collect()
    }

    /// Range scan, returning **user** keys (the physical prefix is stripped back off).
    pub fn scan(
        &self,
        read: &LeaderRead<'_>,
        keyspace: KeyspaceId,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(UserKey, Value)>> {
        let (lo, hi) = Self::bounds(keyspace, start, end)?;
        let rows = read.view.scan(RAW_CF, &lo, &hi, limit)?;
        rows.into_iter()
            .map(|(physical_key, value)| {
                let decoded = decode_key(&physical_key)?;
                Ok((decoded.user_key.to_vec(), value))
            })
            .collect()
    }

    /// The physical half-open range to scan for `keyspace`.
    ///
    /// An empty `end` means "to the end of *my* keyspace", never "unbounded". A non-empty
    /// `end` is simply encoded: because a physical key is `mode + keyspace_id + user_key`
    /// and the keyspace's upper bound is `mode + (keyspace_id + 1)`, **any** user bytes
    /// encoded under this keyspace already land inside `[lo, hi)`. A caller therefore
    /// *cannot express* an `end` in someone else's keyspace.
    ///
    /// So there is deliberately **no clamping here**. Clamping would only ever do
    /// something if `encode_key` had violated that invariant — and silently repairing a
    /// broken invariant is how a real bug gets hidden behind a range that merely looks
    /// sane. The debug assertion below fails loudly instead.
    fn bounds(keyspace: KeyspaceId, start: &[u8], end: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let (ks_lo, ks_hi) = keyspace_range(KeyMode::Raw, keyspace)?;

        let lo = if start.is_empty() {
            ks_lo.clone()
        } else {
            encode_key(KeyMode::Raw, keyspace, start)?
        };
        let hi = if end.is_empty() {
            ks_hi.clone()
        } else {
            encode_key(KeyMode::Raw, keyspace, end)?
        };

        debug_assert!(
            lo >= ks_lo && lo <= ks_hi && hi >= ks_lo && hi <= ks_hi,
            "encoded raw bounds escaped keyspace {keyspace:?}: this means the physical key \
             codec's prefix invariant is broken, not that the caller passed a bad range"
        );

        // An inverted or empty range yields nothing rather than erroring:
        // `scan(b"z", b"a")` is empty, not invalid.
        if lo >= hi {
            return Ok((lo.clone(), lo));
        }
        Ok((lo, hi))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_engine::{Engine, MemEngine, Mutation};

    const KS_A: KeyspaceId = KeyspaceId(7);
    const KS_B: KeyspaceId = KeyspaceId(8);

    /// Apply plans the way the real apply loop would, so tests exercise the same batches
    /// that raft would replicate — not a separate write path invented for testing.
    fn apply(engine: &MemEngine, batches: Vec<WriteBatch>) {
        for batch in batches {
            engine.write(batch).unwrap();
        }
    }

    fn put(engine: &MemEngine, ks: KeyspaceId, key: &[u8], value: &[u8]) {
        let plan = RawExecutor
            .plan_put(ks, key, value.to_vec(), RawWriteOptions::default())
            .unwrap();
        apply(engine, vec![plan]);
    }

    /// Reads in tests still have to go through the leader gate, exactly like production.
    fn leader<'a>(view: &'a dyn ReadView) -> LeaderRead<'a> {
        LeaderRead::new(view, true, Some(1)).unwrap()
    }

    #[test]
    fn put_encodes_the_tenant_prefix_and_never_the_bare_key() {
        let plan = RawExecutor
            .plan_put(KS_A, b"k", b"v".to_vec(), RawWriteOptions::default())
            .unwrap();

        let Mutation::Put { key, .. } = &plan.mutations()[0] else {
            panic!("expected a put");
        };
        // The whole point: what gets replicated is prefixed, not b"k".
        assert_ne!(key.as_slice(), b"k", "the bare user key must never be stored");
        assert_eq!(key, &encode_key(KeyMode::Raw, KS_A, b"k").unwrap());
        let decoded = decode_key(key).unwrap();
        assert_eq!(decoded.mode, KeyMode::Raw);
        assert_eq!(decoded.keyspace, KS_A);
        assert_eq!(decoded.user_key, b"k");
    }

    /// The cross-tenant isolation this module exists to preserve.
    #[test]
    fn identical_user_keys_in_two_keyspaces_do_not_collide() {
        let engine = MemEngine::new();
        put(&engine, KS_A, b"same", b"from-a");
        put(&engine, KS_B, b"same", b"from-b");

        let view = engine.snapshot().unwrap();
        let read = leader(view.as_ref());

        // Control: each keyspace sees its own value...
        assert_eq!(
            RawExecutor.get(&read, KS_A, b"same").unwrap(),
            Some(b"from-a".to_vec())
        );
        assert_eq!(
            RawExecutor.get(&read, KS_B, b"same").unwrap(),
            Some(b"from-b".to_vec())
        );
        // ...and a full scan of one keyspace never yields the other's row.
        let a_rows = RawExecutor.scan(&read, KS_A, b"", b"", usize::MAX).unwrap();
        assert_eq!(a_rows, vec![(b"same".to_vec(), b"from-a".to_vec())]);
    }

    /// An empty `end` must stop at this keyspace's upper bound, not run on into the next.
    #[test]
    fn unbounded_scan_stops_at_the_keyspace_boundary() {
        let engine = MemEngine::new();
        put(&engine, KS_A, b"a", b"1");
        put(&engine, KS_A, b"z", b"2");
        put(&engine, KS_B, b"a", b"neighbour");

        let view = engine.snapshot().unwrap();
        let read = leader(view.as_ref());

        let rows = RawExecutor.scan(&read, KS_A, b"", b"", usize::MAX).unwrap();
        assert_eq!(
            rows,
            vec![
                (b"a".to_vec(), b"1".to_vec()),
                (b"z".to_vec(), b"2".to_vec())
            ],
            "an empty end must mean 'end of my keyspace', never 'unbounded'"
        );
        // Control: the neighbour row does exist, so the assertion above isn't vacuous.
        assert_eq!(
            RawExecutor.get(&read, KS_B, b"a").unwrap(),
            Some(b"neighbour".to_vec())
        );
    }

    #[test]
    fn scan_returns_user_keys_not_physical_keys() {
        let engine = MemEngine::new();
        put(&engine, KS_A, b"key", b"v");
        let view = engine.snapshot().unwrap();
        let rows = RawExecutor
            .scan(&leader(view.as_ref()), KS_A, b"", b"", usize::MAX)
            .unwrap();
        assert_eq!(rows[0].0, b"key".to_vec(), "the prefix must be stripped back off");
    }

    #[test]
    fn inverted_and_empty_ranges_yield_nothing_rather_than_erroring() {
        let engine = MemEngine::new();
        put(&engine, KS_A, b"m", b"v");
        let view = engine.snapshot().unwrap();
        let read = leader(view.as_ref());

        assert!(RawExecutor.scan(&read, KS_A, b"z", b"a", 10).unwrap().is_empty());
        assert!(RawExecutor.scan(&read, KS_A, b"m", b"m", 10).unwrap().is_empty());
        // Control: the row is findable with a range that does contain it.
        assert_eq!(RawExecutor.scan(&read, KS_A, b"a", b"z", 10).unwrap().len(), 1);
    }

    /// Silently ignoring a TTL would let a caller believe data expires when it never will.
    #[test]
    fn unsupported_write_options_are_refused_not_ignored() {
        let ttl = RawWriteOptions {
            ttl_secs: Some(60),
            causal_ts: None,
        };
        assert!(matches!(
            RawExecutor.plan_put(KS_A, b"k", b"v".to_vec(), ttl),
            Err(Error::NotImplemented(_))
        ));
        let causal = RawWriteOptions {
            ttl_secs: None,
            causal_ts: Some(1),
        };
        assert!(matches!(
            RawExecutor.plan_put(KS_A, b"k", b"v".to_vec(), causal),
            Err(Error::NotImplemented(_))
        ));
        // Control: without those options the same call succeeds.
        assert!(RawExecutor
            .plan_put(KS_A, b"k", b"v".to_vec(), RawWriteOptions::default())
            .is_ok());
    }

    /// A follower must not be able to answer a read at all.
    #[test]
    fn a_non_leader_cannot_obtain_a_read_view() {
        let engine = MemEngine::new();
        put(&engine, KS_A, b"k", b"v");
        let view = engine.snapshot().unwrap();

        match LeaderRead::new(view.as_ref(), false, Some(3)) {
            Err(Error::NotLeader { leader }) => assert_eq!(leader, Some(3), "hint must survive"),
            other => panic!("a follower must be refused, got {other:?}", other = other.is_ok()),
        }
        // Control: the same view is readable once leadership is asserted, so the refusal
        // above is about leadership and not about the view being empty or broken.
        assert_eq!(
            RawExecutor.get(&leader(view.as_ref()), KS_A, b"k").unwrap(),
            Some(b"v".to_vec())
        );
    }

    /// Drives the chunk loop the way the runtime does, and checks the *bound* as well as
    /// the result: no single call may read more than `chunk` rows, which is the property
    /// the previous `usize::MAX` version silently lacked.
    #[test]
    fn delete_range_streams_in_bounded_chunks_and_resumes_after_each() {
        let engine = MemEngine::new();
        for i in 0..5u8 {
            put(&engine, KS_A, &[b'k', i], b"v");
        }
        put(&engine, KS_B, b"survivor", b"v");

        let mut cursor: Option<Vec<u8>> = None;
        let mut chunks = 0;
        loop {
            let view = engine.snapshot().unwrap();
            let read = leader(view.as_ref());
            let Some((batch, last)) = RawExecutor
                .plan_delete_range_chunk(&read, KS_A, cursor.as_deref(), b"", b"", 2)
                .unwrap()
            else {
                break;
            };
            assert!(
                batch.mutations().len() <= 2,
                "a chunk must never plan more rows than its bound"
            );
            apply(&engine, vec![batch]);
            cursor = Some(last);
            chunks += 1;
            assert!(chunks <= 5, "loop must terminate");
        }
        assert_eq!(chunks, 3, "5 keys at chunk 2 ⇒ 3 chunks (2+2+1)");

        let view = engine.snapshot().unwrap();
        let read = leader(view.as_ref());
        assert!(RawExecutor
            .scan(&read, KS_A, b"", b"", usize::MAX)
            .unwrap()
            .is_empty());
        assert_eq!(
            RawExecutor.get(&read, KS_B, b"survivor").unwrap(),
            Some(b"v".to_vec()),
            "delete_range must not reach across the keyspace boundary"
        );
    }

    /// Stopping after the first chunk must leave exactly that chunk deleted — the caller
    /// can then report where it stopped instead of implying nothing happened.
    #[test]
    fn a_partial_delete_range_leaves_exactly_the_committed_chunks_gone() {
        let engine = MemEngine::new();
        for i in 0..5u8 {
            put(&engine, KS_A, &[b'k', i], b"v");
        }
        let view = engine.snapshot().unwrap();
        let read = leader(view.as_ref());
        let (batch, _last) = RawExecutor
            .plan_delete_range_chunk(&read, KS_A, None, b"", b"", 2)
            .unwrap()
            .expect("first chunk exists");
        apply(&engine, vec![batch]);

        let view = engine.snapshot().unwrap();
        let remaining = RawExecutor
            .scan(&leader(view.as_ref()), KS_A, b"", b"", usize::MAX)
            .unwrap();
        assert_eq!(remaining.len(), 3, "only the committed chunk is gone");
    }

    #[test]
    fn delete_range_rejects_a_zero_chunk_instead_of_looping_forever() {
        let engine = MemEngine::new();
        let view = engine.snapshot().unwrap();
        assert!(RawExecutor
            .plan_delete_range_chunk(&leader(view.as_ref()), KS_A, None, b"", b"", 0)
            .is_err());
    }

    #[test]
    fn batch_put_is_one_batch_so_raft_applies_it_atomically() {
        let pairs = vec![
            (b"a".to_vec(), b"1".to_vec()),
            (b"b".to_vec(), b"2".to_vec()),
        ];
        let plan = RawExecutor
            .plan_batch_put(KS_A, &pairs, RawWriteOptions::default())
            .unwrap();
        assert_eq!(
            plan.mutations().len(),
            2,
            "both writes must ride one batch — two batches could half-apply"
        );
    }
}
