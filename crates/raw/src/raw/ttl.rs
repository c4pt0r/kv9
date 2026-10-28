//! Snapshot-bound TTL reads. Public runtime activation is a separate gate.
//!
//! Default CF holds opaque values; Write CF at the same physical Raw key holds
//! optional expiry metadata. The replicated default-CF clock record belongs to
//! this exact snapshot. A caller must first commit a suitable time observation
//! and establish its normal linearizable read barrier before constructing this
//! view. Merely sampling a wall clock does not authorize a read.

use super::{LeaderRead, RawExecutor, RAW_CF};
use kv9_common::codec::{decode_key, encode_key, KeyMode};
use kv9_common::raw_ttl::{ClockWatermark, ExpiryRecord, CLOCK_KEY, EXPIRY_RECORD_BYTES};
use kv9_common::{Error, KeyspaceId, Result, UserKey, Value};
use kv9_engine::{ColumnFamily, ReadView};

pub struct TtlRead<'a> {
    view: &'a dyn ReadView,
    watermark: ClockWatermark,
}

/// A bounded physical scan can produce no visible rows and still have a cursor.
/// Continue until `next` is None, never merely until `rows` is empty.
#[derive(Debug, PartialEq, Eq)]
pub struct TtlPage {
    pub rows: Vec<(UserKey, Value)>,
    pub next: Option<UserKey>,
    pub examined: usize,
}

/// Response rows, physical work, and a conservative protobuf row byte budget.
/// Each returned pair is charged key + value + 16 bytes of framing overhead.
pub struct ScanBudget {
    pub rows: usize,
    pub examined: usize,
    pub result_bytes: usize,
}

/// Candidate collection has no delete authority. Ordered apply must compare
/// every exact record again against the current version and committed time.
#[derive(Debug, PartialEq, Eq)]
pub struct CleanupPage {
    pub candidates: Vec<(UserKey, ExpiryRecord)>,
    pub next: Option<UserKey>,
    pub examined: usize,
    /// Encoded candidate bodies: key length, user key, expiry record. The
    /// caller must reserve the separate command envelope from its wire budget.
    pub candidate_bytes: usize,
}

/// Snapshot-selected explicit deletes, including expired keys. Apply deletes
/// current values and expiry records at these keys; this is not TTL cleanup.
#[derive(Debug, PartialEq, Eq)]
pub struct DeletePage {
    pub keys: Vec<UserKey>,
    pub next: Option<UserKey>,
}

impl<'a> TtlRead<'a> {
    pub fn delete_page(
        &self,
        keyspace: KeyspaceId,
        start: &[u8],
        end: &[u8],
        max_keys: usize,
        max_mutation_bytes: usize,
    ) -> Result<DeletePage> {
        raw_key(keyspace, start)?;
        if max_keys == 0 || max_mutation_bytes == 0 {
            return Err(Error::Config(
                "TTL delete page budgets must be positive".into(),
            ));
        }
        let (lo, hi) = RawExecutor::bounds(keyspace, start, end)?;
        let mut page = DeletePage {
            keys: Vec::new(),
            next: None,
        };
        if lo >= hi {
            return Ok(page);
        }
        let mut iter = self.view.iter(RAW_CF, &lo, &hi)?;
        let mut charged = 0;
        while page.keys.len() < max_keys {
            let Some(entry) = iter.next() else {
                page.next = None;
                return Ok(page);
            };
            let (physical, _) = entry?;
            let key = decode_key(&physical)?.user_key.to_vec();
            let bytes = key
                .len()
                .checked_add(5)
                .ok_or_else(|| Error::Config("TTL delete key size overflows".into()))?;
            if key.len() > kv9_common::data_range::MAX_BOUND || bytes > max_mutation_bytes {
                return Err(Error::Config(
                    "one TTL delete key exceeds its budget".into(),
                ));
            }
            if bytes > max_mutation_bytes - charged {
                page.next = Some(key); // Unselected row: preserve the inclusive position.
                return Ok(page);
            }
            charged += bytes;
            let mut next = key.clone();
            next.push(0);
            page.next = Some(next);
            page.keys.push(key);
        }
        if page
            .next
            .as_deref()
            .is_some_and(|next| !end.is_empty() && next >= end)
        {
            page.next = None;
        }
        Ok(page)
    }

    pub fn from_leader(read: &LeaderRead<'a>) -> Result<Self> {
        let bytes = read.view.get(RAW_CF, CLOCK_KEY)?.ok_or_else(|| {
            Error::MetaNotReady("Raw TTL clock is not initialized in this snapshot".into())
        })?;
        Ok(Self {
            view: read.view,
            watermark: ClockWatermark::decode(&bytes)?,
        })
    }

    fn visible(&self, physical: &[u8]) -> Result<bool> {
        self.view
            .get(ColumnFamily::Write, physical)?
            .map(|bytes| {
                ExpiryRecord::decode(&bytes).map(|record| !record.expired_at(self.watermark))
            })
            .unwrap_or(Ok(true))
    }

    pub fn get(&self, keyspace: KeyspaceId, key: &[u8]) -> Result<Option<Value>> {
        let physical = raw_key(keyspace, key)?;
        if !self.visible(&physical)? {
            return Ok(None);
        }
        self.view.get(RAW_CF, &physical)
    }

    /// Input order, duplicates, empty values and misses use one time/data view.
    pub fn batch_get(&self, keyspace: KeyspaceId, keys: &[UserKey]) -> Result<Vec<Option<Value>>> {
        keys.iter().map(|key| self.get(keyspace, key)).collect()
    }

    /// Scan only expiry metadata, without materializing user values. Limits
    /// bound rows fetched, candidates returned, and their encoded bodies;
    /// engine snapshot/iterator construction has its own storage budget.
    /// An empty page with `next` is progress, not EOF. Dropping a page is safe.
    pub fn cleanup_page(
        &self,
        keyspace: KeyspaceId,
        start: &[u8],
        end: &[u8],
        max_candidates: usize,
        max_examined: usize,
        max_candidate_bytes: usize,
    ) -> Result<CleanupPage> {
        raw_key(keyspace, start)?;
        if max_candidates == 0 || max_examined == 0 || max_candidate_bytes == 0 {
            return Err(Error::Config(
                "Raw TTL cleanup budgets must be positive".into(),
            ));
        }
        let (lo, hi) = RawExecutor::bounds(keyspace, start, end)?;
        let mut page = CleanupPage {
            candidates: Vec::new(),
            next: None,
            examined: 0,
            candidate_bytes: 0,
        };
        if lo >= hi {
            return Ok(page);
        }
        let mut iter = self.view.iter(ColumnFamily::Write, &lo, &hi)?;
        while page.examined < max_examined && page.candidates.len() < max_candidates {
            let Some(entry) = iter.next() else {
                page.next = None;
                return Ok(page);
            };
            let (physical, bytes) = entry?;
            page.examined += 1;
            let decoded = decode_key(&physical)?;
            let record = ExpiryRecord::decode(&bytes)?;
            if record.expired_at(self.watermark) {
                let size = decoded
                    .user_key
                    .len()
                    .checked_add(4 + EXPIRY_RECORD_BYTES)
                    .ok_or_else(|| {
                        Error::Config("Raw TTL cleanup candidate size overflows".into())
                    })?;
                if size > max_candidate_bytes {
                    return Err(Error::Config(
                        "Raw TTL cleanup budget cannot hold one candidate".into(),
                    ));
                }
                if size > max_candidate_bytes - page.candidate_bytes {
                    // This row was examined but not emitted. Resume AT it,
                    // never after it, or the byte boundary would lose a key.
                    page.next = Some(decoded.user_key.to_vec());
                    return Ok(page);
                }
                page.candidate_bytes += size;
                page.candidates.push((decoded.user_key.to_vec(), record));
            }
            let mut next = decoded.user_key.to_vec();
            next.push(0);
            page.next = Some(next);
        }
        if page
            .next
            .as_deref()
            .is_some_and(|next| !end.is_empty() && next >= end)
        {
            page.next = None;
        }
        Ok(page)
    }

    /// `start` (or the preceding `next`) is inclusive; `end` is exclusive.
    /// Both budgets must be positive. Value byte/admission budgets belong to the
    /// caller, as they do for existing Raw reads; this bounds rows examined even
    /// when every row is expired. It does not scan ahead merely to detect EOF.
    pub fn scan_page(
        &self,
        keyspace: KeyspaceId,
        start: &[u8],
        end: &[u8],
        limit: usize,
        max_examined: usize,
    ) -> Result<TtlPage> {
        self.scan_page_with_budget(
            keyspace,
            start,
            end,
            ScanBudget {
                rows: limit,
                examined: max_examined,
                result_bytes: usize::MAX,
            },
        )
    }

    pub fn scan_page_with_budget(
        &self,
        keyspace: KeyspaceId,
        start: &[u8],
        end: &[u8],
        budget: ScanBudget,
    ) -> Result<TtlPage> {
        raw_key(keyspace, start)?;
        if budget.rows == 0 || budget.examined == 0 || budget.result_bytes == 0 {
            return Err(Error::Config(
                "Raw TTL scan budgets must be positive".into(),
            ));
        }
        let (lo, hi) = RawExecutor::bounds(keyspace, start, end)?;
        let mut page = TtlPage {
            rows: Vec::new(),
            next: None,
            examined: 0,
        };
        if lo >= hi {
            return Ok(page);
        }
        let mut iter = self.view.iter(RAW_CF, &lo, &hi)?;
        let mut result_bytes = 0usize;
        while page.examined < budget.examined && page.rows.len() < budget.rows {
            let Some(entry) = iter.next() else {
                page.next = None;
                return Ok(page);
            };
            let (physical, value) = entry?;
            let decoded = decode_key(&physical)?;
            let mut next = decoded.user_key.to_vec();
            next.push(0);
            page.next = Some(next);
            page.examined += 1;
            if self.visible(&physical)? {
                let bytes = decoded
                    .user_key
                    .len()
                    .saturating_add(value.len())
                    .saturating_add(16);
                if bytes > budget.result_bytes {
                    return Err(Error::Config(
                        "one Raw TTL scan row exceeds its byte budget".into(),
                    ));
                }
                if bytes > budget.result_bytes - result_bytes {
                    // This row was inspected, but not returned. Keep an inclusive
                    // cursor so continuation cannot silently skip the value.
                    page.next = Some(decoded.user_key.to_vec());
                    return Ok(page);
                }
                result_bytes += bytes;
                page.rows.push((decoded.user_key.to_vec(), value));
            }
        }
        if page
            .next
            .as_deref()
            .is_some_and(|next| !end.is_empty() && next >= end)
        {
            page.next = None;
        }
        Ok(page)
    }
}

fn raw_key(keyspace: KeyspaceId, key: &[u8]) -> Result<Vec<u8>> {
    if keyspace == KeyspaceId::SYSTEM {
        return Err(Error::Config(
            "Raw TTL cannot address the system keyspace".into(),
        ));
    }
    encode_key(KeyMode::Raw, keyspace, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::raw_ttl::{ClockSample, ExpiryPlan};
    use kv9_common::{AppliedPosition, RootDigest, StoreIncarnation};
    use kv9_engine::{Engine, MemEngine, ReplicatedEngine, WalEngine, WriteBatch};

    const KEYSPACE: KeyspaceId = KeyspaceId(100);

    fn watermark(now: u64) -> ClockWatermark {
        ClockSample::with_uncertainty(now, 0)
            .unwrap()
            .advance(ClockWatermark::default())
            .unwrap()
    }

    fn value_batch(key: &[u8], value: &[u8], deadline: Option<u64>, index: u64) -> WriteBatch {
        let mut batch = WriteBatch::new();
        let physical = raw_key(KEYSPACE, key).unwrap();
        batch.put(RAW_CF, physical.clone(), value.to_vec());
        match deadline {
            Some(deadline) => {
                let expiry = ExpiryRecord::new(
                    RootDigest::from_bytes([1; 32]),
                    AppliedPosition { term: 1, index },
                    deadline,
                )
                .unwrap();
                batch.put(ColumnFamily::Write, physical, expiry.encode().to_vec());
            }
            None => {
                batch.delete(ColumnFamily::Write, physical);
            }
        }
        batch.put(
            RAW_CF,
            CLOCK_KEY.to_vec(),
            watermark(2000).encode().to_vec(),
        );
        batch
    }

    #[test]
    fn explicit_delete_pages_include_expired_keys_and_preserve_prefixes_and_byte_overflow() {
        let engine = MemEngine::new();
        for (i, key) in [b"a".as_slice(), b"a\0", b"a\0\0", b"b"]
            .into_iter()
            .enumerate()
        {
            engine
                .write(value_batch(
                    key,
                    b"v",
                    if i % 2 == 0 { Some(1000) } else { None },
                    i as u64 + 1,
                ))
                .unwrap();
        }
        let view = engine.snapshot().unwrap();
        let read = LeaderRead::new(view.as_ref(), true, None).unwrap();
        let ttl = TtlRead::from_leader(&read).unwrap();
        let first = ttl.delete_page(KEYSPACE, b"", b"", 10, 13).unwrap();
        assert_eq!(first.keys, vec![b"a".to_vec(), b"a\0".to_vec()]);
        assert_eq!(first.next.as_deref(), Some(b"a\0\0".as_slice()));
        let second = ttl
            .delete_page(KEYSPACE, first.next.as_ref().unwrap(), b"", 1, 1024)
            .unwrap();
        assert_eq!(second.keys, vec![b"a\0\0".to_vec()]);
        let last = ttl
            .delete_page(KEYSPACE, second.next.as_ref().unwrap(), b"", 10, 1024)
            .unwrap();
        assert_eq!(last.keys, vec![b"b".to_vec()]);
        assert!(last.next.is_none());
        assert!(ttl.delete_page(KEYSPACE, b"", b"", 10, 5).is_err());
        assert!(ttl.delete_page(KEYSPACE, b"", b"", 0, 1024).is_err());
        assert_eq!(
            view.scan(
                RAW_CF,
                &raw_key(KEYSPACE, b"").unwrap(),
                &raw_key(KEYSPACE, b"z").unwrap(),
                10
            )
            .unwrap()
            .len(),
            4
        );
    }

    #[test]
    fn explicit_delete_budget_handles_maximum_internal_keys_without_unbounded_commands() {
        let engine = MemEngine::new();
        for i in 1..=3 {
            engine
                .write(value_batch(
                    &vec![i; kv9_common::data_range::MAX_BOUND],
                    b"v",
                    Some(1000),
                    i as u64,
                ))
                .unwrap();
        }
        let view = engine.snapshot().unwrap();
        let read = LeaderRead::new(view.as_ref(), true, None).unwrap();
        let ttl = TtlRead::from_leader(&read).unwrap();
        let first = ttl
            .delete_page(
                KEYSPACE,
                b"",
                b"",
                256,
                kv9_common::data_range::MAX_BOUND + 5,
            )
            .unwrap();
        assert_eq!(first.keys.len(), 1);
        assert_eq!(first.next, Some(vec![2; kv9_common::data_range::MAX_BOUND]));
        let end = ttl
            .delete_page(
                KEYSPACE,
                &vec![3; kv9_common::data_range::MAX_BOUND],
                b"",
                1,
                100_000,
            )
            .unwrap();
        assert_eq!(
            end.next.as_ref().unwrap().len(),
            kv9_common::data_range::MAX_BOUND + 1
        );
        assert!(ttl
            .delete_page(KEYSPACE, end.next.as_ref().unwrap(), b"", 1, 100_000)
            .unwrap()
            .keys
            .is_empty());
    }

    #[test]
    fn cleanup_scan_bounds_empty_pages_candidates_and_bytes_without_skipping_prefixes() {
        let engine = MemEngine::new();
        let keys: &[&[u8]] = &[b"a", b"a\0", b"a\0\0", b"b", b"c"];
        for (n, key) in keys.iter().enumerate() {
            engine
                .write(value_batch(
                    key,
                    b"value",
                    Some(if n < 2 { 3000 } else { 1000 }),
                    n as u64 + 1,
                ))
                .unwrap();
        }
        // Nonexpiring data has no row in the metadata scan.
        engine
            .write(value_batch(b"aa", b"permanent", None, 6))
            .unwrap();
        let view = engine.snapshot().unwrap();
        let read = LeaderRead::new(view.as_ref(), true, None).unwrap();
        let ttl = TtlRead::from_leader(&read).unwrap();
        let first = ttl.cleanup_page(KEYSPACE, b"", b"", 10, 2, 1024).unwrap();
        assert!(first.candidates.is_empty());
        assert_eq!(first.examined, 2);
        assert_eq!(first.next.as_deref(), Some(b"a\0\0".as_slice()));
        let size = 4 + 3 + EXPIRY_RECORD_BYTES;
        let second = ttl
            .cleanup_page(KEYSPACE, first.next.as_ref().unwrap(), b"", 10, 10, size)
            .unwrap();
        assert_eq!(second.candidate_bytes, size);
        assert_eq!(second.candidates.len(), 1);
        assert_eq!(second.candidates[0].0, b"a\0\0");
        assert_eq!(second.examined, 2);
        assert_eq!(
            second.next.as_deref(),
            Some(b"b".as_slice()),
            "byte budget must not skip the examined overflow row"
        );
        let third = ttl
            .cleanup_page(KEYSPACE, second.next.as_ref().unwrap(), b"", 1, 10, 1024)
            .unwrap();
        assert_eq!(third.examined, 1);
        assert_eq!(third.candidates[0].0, b"b");
        let fourth = ttl
            .cleanup_page(KEYSPACE, third.next.as_ref().unwrap(), b"", 10, 10, 1024)
            .unwrap();
        assert_eq!(fourth.candidates[0].0, b"c");
        assert_eq!(fourth.next, None);
        let bounded = ttl
            .cleanup_page(KEYSPACE, b"b", b"c", 10, 10, 1024)
            .unwrap();
        assert_eq!(bounded.candidates.len(), 1);
        assert_eq!(bounded.next, None);
        assert!(ttl
            .cleanup_page(KEYSPACE, b"c", b"c", 10, 10, 1024)
            .unwrap()
            .candidates
            .is_empty());
    }

    #[test]
    fn cleanup_scan_rejects_unusable_budgets_corruption_and_returns_no_delete_authority() {
        let engine = MemEngine::new();
        engine
            .write(value_batch(b"b", b"old", Some(1000), 1))
            .unwrap();
        let view = engine.snapshot().unwrap();
        let read = LeaderRead::new(view.as_ref(), true, None).unwrap();
        let ttl = TtlRead::from_leader(&read).unwrap();
        for (candidates, examined, bytes) in [(0, 1, 1024), (1, 0, 1024), (1, 1, 0), (1, 1, 68)] {
            assert!(ttl
                .cleanup_page(KEYSPACE, b"", b"", candidates, examined, bytes)
                .is_err());
        }
        assert!(ttl
            .cleanup_page(KeyspaceId::SYSTEM, b"", b"", 1, 1, 1024)
            .is_err());
        let old = ttl
            .cleanup_page(KEYSPACE, b"", b"", 1, 1, 1024)
            .unwrap()
            .candidates
            .remove(0)
            .1;
        engine
            .write(value_batch(b"b", b"permanent", None, 2))
            .unwrap();
        // This snapshot may be retried or discarded after an overwrite. The
        // old candidate retains its identity; it is not a blind delete batch.
        assert_eq!(
            ttl.cleanup_page(KEYSPACE, b"", b"", 1, 1, 1024)
                .unwrap()
                .candidates[0]
                .1,
            old
        );
        assert!(!old.permits_cleanup(None, watermark(2000)));
        assert_eq!(
            engine
                .get(RAW_CF, &raw_key(KEYSPACE, b"b").unwrap())
                .unwrap(),
            Some(b"permanent".to_vec())
        );
        let mut corrupt = WriteBatch::new();
        corrupt.put(
            ColumnFamily::Write,
            raw_key(KEYSPACE, b"b").unwrap(),
            vec![0],
        );
        engine.write(corrupt).unwrap();
        let after = engine.snapshot().unwrap();
        let read = LeaderRead::new(after.as_ref(), true, None).unwrap();
        assert!(TtlRead::from_leader(&read)
            .unwrap()
            .cleanup_page(KEYSPACE, b"", b"", 1, 1, 1024)
            .is_err());
    }

    #[test]
    fn point_and_batch_reads_bind_metadata_and_time_to_the_same_snapshot() {
        let engine = MemEngine::new();
        engine
            .write(value_batch(b"expired", b"old", Some(1999), 1))
            .unwrap();
        engine
            .write(value_batch(b"live", b"", Some(2001), 2))
            .unwrap();
        // Arbitrary legacy bytes resembling a new record stay opaque values.
        engine
            .write(value_batch(b"legacy", b"KV9TTL01\0arbitrary", None, 3))
            .unwrap();
        let view = engine.snapshot().unwrap();
        let leader = LeaderRead::new(view.as_ref(), true, None).unwrap();
        let ttl = TtlRead::from_leader(&leader).unwrap();
        engine
            .write(value_batch(b"expired", b"renewed", None, 4))
            .unwrap();
        assert_eq!(
            ttl.batch_get(
                KEYSPACE,
                &[
                    b"expired".to_vec(),
                    b"live".to_vec(),
                    b"missing".to_vec(),
                    b"live".to_vec(),
                    b"legacy".to_vec()
                ]
            )
            .unwrap(),
            vec![
                None,
                Some(vec![]),
                None,
                Some(vec![]),
                Some(b"KV9TTL01\0arbitrary".to_vec())
            ]
        );
        let current = engine.snapshot().unwrap();
        let leader = LeaderRead::new(current.as_ref(), true, None).unwrap();
        assert_eq!(
            TtlRead::from_leader(&leader)
                .unwrap()
                .get(KEYSPACE, b"expired")
                .unwrap(),
            Some(b"renewed".to_vec())
        );
    }

    #[test]
    fn expired_pages_advance_past_prefix_keys_without_stopping_the_scan() {
        let engine = MemEngine::new();
        let keys: &[&[u8]] = &[b"a", b"a\0", b"a\0\0", b"b", b"c", b"d"];
        for (n, key) in keys.iter().enumerate() {
            engine
                .write(value_batch(
                    key,
                    key,
                    Some(if n < 4 { 2000 } else { 3000 }),
                    n as u64 + 1,
                ))
                .unwrap();
        }
        let view = engine.snapshot().unwrap();
        let leader = LeaderRead::new(view.as_ref(), true, None).unwrap();
        let ttl = TtlRead::from_leader(&leader).unwrap();
        let first = ttl.scan_page(KEYSPACE, b"", b"", 2, 2).unwrap();
        assert!(first.rows.is_empty());
        assert_eq!(first.next, Some(b"a\0\0".to_vec()));
        assert_eq!(first.examined, 2);
        let second = ttl
            .scan_page(KEYSPACE, first.next.as_deref().unwrap(), b"", 2, 2)
            .unwrap();
        assert!(second.rows.is_empty());
        assert!(second.next.is_some());
        let third = ttl
            .scan_page(KEYSPACE, second.next.as_deref().unwrap(), b"", 2, 2)
            .unwrap();
        assert_eq!(
            third.rows,
            vec![
                (b"c".to_vec(), b"c".to_vec()),
                (b"d".to_vec(), b"d".to_vec())
            ]
        );
        let last = ttl
            .scan_page(KEYSPACE, third.next.as_deref().unwrap(), b"", 2, 2)
            .unwrap();
        assert!(last.rows.is_empty());
        assert_eq!(last.next, None);
        assert_eq!(
            ttl.scan_page(KEYSPACE, b"c", b"d", 10, 10)
                .unwrap()
                .rows
                .len(),
            1
        );
        assert!(ttl.scan_page(KEYSPACE, b"", b"", 0, 1).is_err());
        assert!(ttl.scan_page(KEYSPACE, b"", b"", 1, 0).is_err());
        assert!(ttl.get(KeyspaceId::SYSTEM, b"k").is_err());
    }

    #[test]
    fn absent_clock_and_corrupt_expiry_are_errors_not_live_values() {
        let engine = MemEngine::new();
        let view = engine.snapshot().unwrap();
        let leader = LeaderRead::new(view.as_ref(), true, None).unwrap();
        assert!(TtlRead::from_leader(&leader).is_err());
        let mut batch = value_batch(b"key", b"value", Some(1000), 1);
        batch.put(
            ColumnFamily::Write,
            raw_key(KEYSPACE, b"key").unwrap(),
            vec![255],
        );
        engine.write(batch).unwrap();
        let view = engine.snapshot().unwrap();
        let leader = LeaderRead::new(view.as_ref(), true, None).unwrap();
        let ttl = TtlRead::from_leader(&leader).unwrap();
        assert!(ttl.get(KEYSPACE, b"key").is_err());
        assert!(ttl.scan_page(KEYSPACE, b"", b"", 1, 1).is_err());
    }

    #[test]
    fn wal_reopen_preserves_expiry_and_clock_without_restarting_the_ttl() {
        let dir = std::env::temp_dir().join(format!(
            "kv9-ttl-recovery-{}",
            StoreIncarnation::mint().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("data.wal");
        let (engine, _) = WalEngine::open(&path).unwrap();
        let plan = ExpiryPlan::resolve(
            Some(1),
            ClockSample::with_uncertainty(1000, 0).unwrap(),
            ClockWatermark::default(),
        )
        .unwrap();
        let batch = value_batch(b"key", b"old", plan.expires_at_ms(), 1);
        engine
            .write_applied(batch, AppliedPosition { term: 1, index: 1 })
            .unwrap();
        drop(engine);
        let (engine, _) = WalEngine::open(&path).unwrap();
        {
            let view = engine.snapshot().unwrap();
            let leader = LeaderRead::new(view.as_ref(), true, None).unwrap();
            assert_eq!(
                TtlRead::from_leader(&leader)
                    .unwrap()
                    .get(KEYSPACE, b"key")
                    .unwrap(),
                None
            );
            let expired = ExpiryRecord::decode(
                &view
                    .get(ColumnFamily::Write, &raw_key(KEYSPACE, b"key").unwrap())
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            assert!(expired.expired_at(watermark(2000)));
        }
        engine
            .write_applied(
                value_batch(b"key", b"new", None, 2),
                AppliedPosition { term: 1, index: 2 },
            )
            .unwrap();
        drop(engine);
        let (engine, _) = WalEngine::open(&path).unwrap();
        let view = engine.snapshot().unwrap();
        let leader = LeaderRead::new(view.as_ref(), true, None).unwrap();
        assert_eq!(
            TtlRead::from_leader(&leader)
                .unwrap()
                .get(KEYSPACE, b"key")
                .unwrap(),
            Some(b"new".to_vec())
        );
        assert!(view
            .get(ColumnFamily::Write, &raw_key(KEYSPACE, b"key").unwrap())
            .unwrap()
            .is_none());
        drop(view);
        drop(engine);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
