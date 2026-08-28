//! `MetaStore` — the small SQL/catalog engine over the system keyspace (METADATA-CATALOG §4, §8).
//!
//! `membership / catalog / routing / placement / tso` stop being five hand-rolled
//! structs and become **one `MetaStore`** (METADATA-CATALOG §8): a fixed, versioned
//! relational schema with auto-maintained indexes and transactions, so control-plane
//! consistency is structural, not by convention.
//!
//! A metadata mutation is one **`system`-group transaction** (§5): `begin()` → typed
//! reads/writes across tables → `commit()`. Multi-table changes are atomic; a failed
//! step rolls back everything *including index rows*.
//!
//! Phase-1 transaction model: the txn keeps a **read-your-writes overlay** (physical
//! key → put/delete) consulted before the engine on every read, so PK/UNIQUE/FK checks
//! see buffered writes; `commit`/`into_batch` lower the overlay into one atomic
//! [`WriteBatch`]. Concurrency control is the raft log (committed commands apply
//! serially); Percolator 2PC with real conflict detection arrives in Phase-3 and
//! replaces the overlay's optimism, not this API.

use std::collections::BTreeMap;
use std::sync::Arc;

use kv9_engine::{ColumnFamily, Engine, ReadView, WriteBatch};

use kv9_common::{Error, Result};

use crate::codec::{
    self, encode_row_key, index_prefix_range, row_range, ColumnValue, PkComponent, RowValue,
};
use crate::schema::{ColumnType, IndexDesc, IndexId, TableDesc};

/// A decoded catalog row: the primary key components (in encoded form, symmetric with
/// what `get` accepts) plus the tagged column set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub pk: Vec<PkComponent>,
    pub value: RowValue,
}

/// A change-set for [`MetaTxn::update`]: the columns to overwrite (METADATA-CATALOG §4).
pub type Changes = Vec<(crate::schema::ColumnId, ColumnValue)>;

/// A merged `(physical key, value)` pair yielded by a range read.
pub type KvPair = (Vec<u8>, Vec<u8>);
/// A streaming merged range: engine view + txn overlay, either direction.
type MergedRange<'t> = Box<dyn Iterator<Item = Result<KvPair>> + 't>;

/// Stable id-sequence kinds served by [`MetaTxn::allocate_id`] (rows of the
/// `id_sequences` table). Codes are persisted — never reuse or renumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceKind {
    Tenant = 1,
    Keyspace = 2,
    TxnGroup = 3,
    Timeline = 4,
    Region = 5,
    Task = 6,
    SstFile = 7,
    Node = 8,
}

/// Ids below this are reserved for bootstrap-fixed entities (the `default` tenant, the
/// system keyspace's own records, …); dynamic allocation starts here.
pub const FIRST_DYNAMIC_ID: u64 = 100;

/// The catalog engine (METADATA-CATALOG §8). Generic over the storage [`Engine`] so the
/// mocked `MemEngine` (Phase-1) and the real disaggregated engine (Phase-2) plug in
/// behind the same trait.
#[derive(Clone)]
pub struct MetaStore<E: Engine> {
    engine: Arc<E>,
}

impl<E: Engine> MetaStore<E> {
    /// Open a `MetaStore` over an engine holding the system keyspace (METADATA-CATALOG §6).
    pub fn new(engine: Arc<E>) -> Self {
        MetaStore { engine }
    }

    /// The backing engine (system keyspace KV).
    pub fn engine(&self) -> &Arc<E> {
        &self.engine
    }

    /// Begin a metadata transaction (METADATA-CATALOG §4, §5).
    ///
    /// Captures one engine [`ReadView`] for the whole transaction: every read —
    /// point get, scan, index scan — comes from the same snapshot (overlaid with the
    /// txn's own writes), so multi-step lookups like `index_scan → get(pk)` and
    /// `region_for_key`'s candidate + `end_key` check can never chase state that
    /// changed between steps (acceptance items 8/14).
    pub fn begin(&self) -> Result<MetaTxn<'_, E>> {
        Ok(MetaTxn {
            view: self.engine.snapshot()?,
            store: self,
            overlay: BTreeMap::new(),
        })
    }
}

/// A metadata transaction exposing the typed op API (METADATA-CATALOG §4).
///
/// All catalog rows live in a single column family ([`ColumnFamily::Default`]) of the
/// system keyspace; the MVCC/lock/write CFs of a real `txn` keyspace are used once the
/// store runs real 2PC (Phase-3). Dropping an uncommitted txn discards the overlay —
/// nothing was written, so there is nothing to roll back.
pub struct MetaTxn<'a, E: Engine> {
    store: &'a MetaStore<E>,
    /// The transaction's single consistent snapshot; all reads go through it.
    view: Box<dyn ReadView + 'a>,
    /// Read-your-writes buffer: physical key → `Some(value)` (put) / `None` (delete).
    /// Consulted before the snapshot on every read; lowered into the commit batch.
    overlay: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
}

impl<'a, E: Engine> MetaTxn<'a, E> {
    // -- raw overlay-aware KV --------------------------------------------------

    fn read_kv(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if let Some(buffered) = self.overlay.get(key) {
            return Ok(buffered.clone());
        }
        self.view.get(ColumnFamily::Default, key)
    }

    fn write_kv(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.overlay.insert(key, Some(value));
    }

    fn delete_kv(&mut self, key: Vec<u8>) {
        self.overlay.insert(key, None);
    }

    /// Streaming merge of the snapshot view with the txn overlay over `[start, end)`
    /// (overlay wins; tombstones skipped), ascending or descending. Nothing beyond
    /// what the caller consumes is materialized (principle 13 — no unmetered
    /// whole-range Vec; the former `usize::MAX` scan sites all route through here).
    fn merged_range<'t>(
        &'t self,
        start: &[u8],
        end: &[u8],
        rev: bool,
    ) -> Result<MergedRange<'t>> {
        let view_iter = if rev {
            self.view.iter_rev(ColumnFamily::Default, start, end)?
        } else {
            self.view.iter(ColumnFamily::Default, start, end)?
        };
        // The overlay is txn-local and small; snapshot the in-range slice so the
        // two cursors advance independently.
        let mut ov: Vec<(Vec<u8>, Option<Vec<u8>>)> = self
            .overlay
            .range(start.to_vec()..end.to_vec())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if rev {
            ov.reverse();
        }
        let mut view = view_iter.peekable();
        let mut ov = ov.into_iter().peekable();
        Ok(Box::new(std::iter::from_fn(move || loop {
            // Errors from the view iterator propagate immediately.
            if matches!(view.peek(), Some(Err(_))) {
                return view.next().map(|r| r.map(|_| unreachable!()));
            }
            let ord = match (view.peek(), ov.peek()) {
                (None, None) => return None,
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(Ok((vk, _))), Some((ok, _))) => {
                    let fwd = vk.cmp(ok);
                    if rev {
                        fwd.reverse()
                    } else {
                        fwd
                    }
                }
                (Some(Err(_)), _) => unreachable!("handled above"),
            };
            match ord {
                // View entry comes first and is not shadowed.
                std::cmp::Ordering::Less => {
                    return view.next();
                }
                // Same key: the overlay wins — a put replaces, a tombstone hides.
                std::cmp::Ordering::Equal => {
                    let _ = view.next();
                    match ov.next() {
                        Some((k, Some(v))) => return Some(Ok((k, v))),
                        _ => continue,
                    }
                }
                // Overlay-only key (insert of a key absent from the view), or a
                // tombstone for a key the view doesn't have.
                std::cmp::Ordering::Greater => match ov.next() {
                    Some((k, Some(v))) => return Some(Ok((k, v))),
                    _ => continue,
                },
            }
        })))
    }

    /// First `limit` merged entries of `[start, end)` in ascending key order.
    fn merged_scan(
        &self,
        start: &[u8],
        end: &[u8],
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.merged_range(start, end, false)?.take(limit).collect()
    }

    // -- typed op API (METADATA-CATALOG §4) ------------------------------------

    /// Point read of a row by primary key. Sees the txn's own buffered writes.
    pub fn get(&self, table: &TableDesc, pk: &[PkComponent]) -> Result<Option<Row>> {
        let key = encode_row_key(table.id, pk)?;
        match self.read_kv(&key)? {
            None => Ok(None),
            Some(bytes) => Ok(Some(Row {
                pk: pk.to_vec(),
                value: RowValue::decode(&bytes)?,
            })),
        }
    }

    /// Insert a row, maintaining all secondary indexes; PK/UNIQUE/FK are checked
    /// against the merged (engine + buffered) view (METADATA-CATALOG §4).
    pub fn insert(&mut self, table: &TableDesc, pk: &[PkComponent], value: RowValue) -> Result<()> {
        let key = encode_row_key(table.id, pk)?;
        if self.read_kv(&key)?.is_some() {
            return Err(Error::WriteConflict(format!(
                "duplicate primary key in `{}`",
                table.name
            )));
        }
        self.check_foreign_keys(table, &value)?;
        for index in table.indexes {
            let cols = self.index_columns(table, index, &value)?;
            if index.unique {
                // Unique index: the key is the columns alone; presence = duplicate.
                let idx_key = codec::encode_index_key(table.id, index.id, &cols, &[])?;
                if self.read_kv(&idx_key)?.is_some() {
                    return Err(Error::WriteConflict(format!(
                        "unique index `{}.{}` violated",
                        table.name, index.name
                    )));
                }
                self.write_kv(idx_key, codec::index_value(pk));
            } else {
                let idx_key = codec::encode_index_key(table.id, index.id, &cols, pk)?;
                self.write_kv(idx_key, Vec::new());
            }
        }
        self.write_kv(key, value.encode());
        Ok(())
    }

    /// Update a row by primary key, re-maintaining affected indexes (METADATA-CATALOG §4).
    pub fn update(&mut self, table: &TableDesc, pk: &[PkComponent], changes: Changes) -> Result<()> {
        let key = encode_row_key(table.id, pk)?;
        let old_bytes = self.read_kv(&key)?.ok_or_else(|| {
            Error::WriteConflict(format!("update of missing row in `{}`", table.name))
        })?;
        let old_value = RowValue::decode(&old_bytes)?;
        let mut new_value = old_value.clone();
        for (col, v) in changes {
            new_value.set(col, v);
        }
        self.check_foreign_keys(table, &new_value)?;
        for index in table.indexes {
            let old_cols = self.index_columns(table, index, &old_value)?;
            let new_cols = self.index_columns(table, index, &new_value)?;
            if old_cols == new_cols {
                continue;
            }
            if index.unique {
                let old_key = codec::encode_index_key(table.id, index.id, &old_cols, &[])?;
                self.delete_kv(old_key);
                let new_key = codec::encode_index_key(table.id, index.id, &new_cols, &[])?;
                if self.read_kv(&new_key)?.is_some() {
                    return Err(Error::WriteConflict(format!(
                        "unique index `{}.{}` violated",
                        table.name, index.name
                    )));
                }
                self.write_kv(new_key, codec::index_value(pk));
            } else {
                let old_key = codec::encode_index_key(table.id, index.id, &old_cols, pk)?;
                self.delete_kv(old_key);
                let new_key = codec::encode_index_key(table.id, index.id, &new_cols, pk)?;
                self.write_kv(new_key, Vec::new());
            }
        }
        self.write_kv(key, new_value.encode());
        Ok(())
    }

    /// Delete a row by primary key, removing its index rows too (METADATA-CATALOG §4).
    /// Deleting a missing row is a no-op (idempotent retries, principle 15).
    pub fn delete(&mut self, table: &TableDesc, pk: &[PkComponent]) -> Result<()> {
        let key = encode_row_key(table.id, pk)?;
        let Some(bytes) = self.read_kv(&key)? else {
            return Ok(());
        };
        let value = RowValue::decode(&bytes)?;
        for index in table.indexes {
            let cols = self.index_columns(table, index, &value)?;
            let idx_key = if index.unique {
                codec::encode_index_key(table.id, index.id, &cols, &[])?
            } else {
                codec::encode_index_key(table.id, index.id, &cols, pk)?
            };
            self.delete_kv(idx_key);
        }
        self.delete_kv(key);
        Ok(())
    }

    /// Full table scan over the row space, reconstructing each row's pk from its
    /// physical key (METADATA-CATALOG §4).
    pub fn scan(&self, table: &TableDesc, limit: usize) -> Result<Vec<Row>> {
        let (start, end) = row_range(table.id)?;
        let entries = self.merged_scan(&start, &end, limit)?;
        let pk_ts = pk_types(table);
        let mut out = Vec::with_capacity(entries.len());
        for (k, v) in entries {
            let suffix = row_key_suffix(&k)?;
            let pk = codec::split_components(&pk_ts, suffix, true)?;
            out.push(Row {
                pk,
                value: RowValue::decode(&v)?,
            });
        }
        Ok(out)
    }

    /// Index scan: resolve an index prefix to the primary keys it points at
    /// (METADATA-CATALOG §4). This is the driver for the known joins in [`crate::tables`].
    pub fn index_scan(
        &self,
        table: &TableDesc,
        index: IndexId,
        prefix: &[PkComponent],
        limit: usize,
    ) -> Result<Vec<Vec<PkComponent>>> {
        let idx = table
            .index(index)
            .ok_or(Error::NotImplemented("unknown index id"))?;
        let pk_ts = pk_types(table);
        let idx_ts = index_col_types(table, idx);
        let entries = self.index_entries(table, index, prefix, limit)?;
        let mut out = Vec::with_capacity(entries.len());
        for (suffix, value) in entries {
            let pk = if idx.unique {
                // Unique index: the stored value is the encoded pk.
                codec::split_components(&pk_ts, &value, true)?
            } else {
                // Non-unique index: the pk follows the index columns in the key.
                let idx_comps = codec::split_components(&idx_ts, &suffix, false)?;
                let consumed: usize = idx_comps.iter().map(|c| c.len()).sum();
                codec::split_components(&pk_ts, &suffix[consumed..], true)?
            };
            out.push(pk);
        }
        Ok(out)
    }

    /// Range read of raw index entries under a prefix: `(index-cols+pk key suffix,
    /// stored value)` pairs in key order, at most `limit` (streamed, not
    /// materialized beyond the result).
    pub fn index_entries(
        &self,
        table: &TableDesc,
        index: IndexId,
        prefix: &[PkComponent],
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let (start, end) = index_prefix_range(table.id, index, prefix)?;
        let entries = self.merged_scan(&start, &end, limit)?;
        entries
            .into_iter()
            .map(|(k, v)| Ok((index_key_suffix(&k)?.to_vec(), v)))
            .collect()
    }

    /// The **greatest live** index entry whose columns are ≤ `(prefix, bound_col)`,
    /// within the prefix — one reverse-merged step, O(result) memory, overlay-aware
    /// (a tombstoned best candidate falls back to the previous live one; an
    /// overlay-inserted candidate wins if greatest). Drives `region_for_key`'s
    /// "last `start_key ≤ K`" lookup (contract item 8 / later-gate 17 closure).
    pub fn index_rev_first_le(
        &self,
        table: &TableDesc,
        index: IndexId,
        prefix: &[PkComponent],
        bound_col: &PkComponent,
    ) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        let (start, _) = index_prefix_range(table.id, index, prefix)?;
        let mut bound_cols: Vec<PkComponent> = prefix.to_vec();
        bound_cols.push(bound_col.clone());
        let bound_key = codec::encode_index_key(table.id, index, &bound_cols, &[])?;
        // Everything with bound-col ≤ target, including its pk-suffixed extensions.
        let end = codec::prefix_upper_bound(bound_key);
        match self.merged_range(&start, &end, true)?.next() {
            None => Ok(None),
            Some(entry) => {
                let (k, v) = entry?;
                Ok(Some((index_key_suffix(&k)?.to_vec(), v)))
            }
        }
    }

    /// Allocate the next id from the named system sequence (the "system id sequence"
    /// of METADATA-CATALOG §4). Read-modify-write through this txn's overlay, so the
    /// bump commits atomically with the rows that consume the id.
    ///
    /// Concurrency: Phase-1 serializes catalog txns through the raft propose path, so
    /// two in-flight txns never interleave on the sequence row. Real optimistic
    /// conflict detection (two concurrent proposers bumping the same sequence) arrives
    /// with Percolator 2PC in Phase-3.
    pub fn allocate_id(&mut self, kind: SequenceKind) -> Result<u64> {
        let table = &crate::schema::ID_SEQUENCES_DESC;
        let pk = vec![codec::memcmp_uint(kind as u64)];
        let next = match self.get(table, &pk)? {
            Some(row) => match row.value.get(crate::schema::ColumnId(2)) {
                Some(ColumnValue::Uint(v)) => *v,
                _ => {
                    return Err(Error::MalformedKey(
                        "id_sequences.next missing or mistyped".into(),
                    ))
                }
            },
            None => FIRST_DYNAMIC_ID,
        };
        let mut row = RowValue::new();
        row.set(crate::schema::ColumnId(1), ColumnValue::Uint(kind as u64));
        row.set(crate::schema::ColumnId(2), ColumnValue::Uint(next + 1));
        let key = encode_row_key(table.id, &pk)?;
        self.write_kv(key, row.encode());
        Ok(next)
    }

    // -- commit ----------------------------------------------------------------

    /// Commit the buffered mutations atomically, writing directly to the engine.
    ///
    /// Standalone-store path (tests, tools). The replicated path is [`Self::into_batch`]
    /// → `Command::CatalogTxn` → raft commit → state-machine apply, which applies the
    /// same batch on every replica (METADATA-CATALOG §5).
    pub fn commit(self) -> Result<()> {
        let (store, batch) = self.into_parts();
        store.engine.write(batch)
    }

    /// Take the buffered write batch without committing, e.g. to hand it to raft as the
    /// payload of a `Command::CatalogTxn` (METADATA-CATALOG §5).
    pub fn into_batch(self) -> WriteBatch {
        self.into_parts().1
    }

    fn into_parts(self) -> (&'a MetaStore<E>, WriteBatch) {
        let mut batch = WriteBatch::new();
        for (k, v) in self.overlay {
            match v {
                Some(v) => batch.put(ColumnFamily::Default, k, v),
                None => batch.delete(ColumnFamily::Default, k),
            };
        }
        (self.store, batch)
    }

    // -- constraint helpers ----------------------------------------------------

    /// Enforce declared FKs: each *present* FK column must reference an existing row,
    /// checked against the merged view so parents inserted earlier in this txn count
    /// (bootstrap writes parents and children in one txn, in order).
    fn check_foreign_keys(&self, table: &TableDesc, value: &RowValue) -> Result<()> {
        for fk in table.fks {
            let Some(v) = value.get(fk.column) else {
                continue;
            };
            let ColumnValue::Uint(id) = v else {
                return Err(Error::MalformedKey(format!(
                    "FK column {} in `{}` must be a uint",
                    fk.column.0, table.name
                )));
            };
            let parent = crate::schema::table_desc(fk.references)
                .ok_or(Error::NotImplemented("FK references unknown table"))?;
            let parent_key = encode_row_key(parent.id, &[codec::memcmp_uint(*id)])?;
            if self.read_kv(&parent_key)?.is_none() {
                return Err(Error::WriteConflict(format!(
                    "FK violation: `{}`.col{} = {} has no row in `{}`",
                    table.name, fk.column.0, id, parent.name
                )));
            }
        }
        Ok(())
    }

    /// Compute the memcomparable index-key columns for `index` from a row's values.
    fn index_columns(
        &self,
        table: &TableDesc,
        index: &IndexDesc,
        value: &RowValue,
    ) -> Result<Vec<PkComponent>> {
        let mut cols = Vec::with_capacity(index.columns.len());
        for col_id in index.columns {
            let comp = match value.get(*col_id) {
                Some(ColumnValue::Uint(v)) => codec::memcmp_uint(*v),
                Some(ColumnValue::Text(s)) => codec::memcmp_text(s),
                Some(ColumnValue::Bytes(b)) => codec::memcmp_bytes(b),
                Some(ColumnValue::Unknown { .. }) => {
                    return Err(Error::MalformedKey(format!(
                        "indexed column {} in `{}` has unknown type",
                        col_id.0, table.name
                    )))
                }
                // A missing indexed column encodes as an empty bytes component so the
                // row is still indexed (and re-locatable on delete).
                None => codec::memcmp_bytes(&[]),
            };
            cols.push(comp);
        }
        Ok(cols)
    }
}

/// The declared types of a table's pk columns, in declaration order.
fn pk_types(table: &TableDesc) -> Vec<ColumnType> {
    table.pk_columns().map(|c| c.ty).collect()
}

/// The declared types of an index's columns, in index order.
fn index_col_types(table: &TableDesc, index: &IndexDesc) -> Vec<ColumnType> {
    index
        .columns
        .iter()
        .filter_map(|id| table.column(*id).map(|c| c.ty))
        .collect()
}

/// Strip a physical row key down to its pk-components suffix:
/// `<sys-prefix> <table_id:u32> 'r' <pk…>` → `<pk…>`.
fn row_key_suffix(physical: &[u8]) -> Result<&[u8]> {
    let decoded = kv9_common::codec::decode_key(physical)?;
    let user = decoded.user_key;
    if user.len() < 5 {
        return Err(Error::MalformedKey(
            "row key shorter than table header".into(),
        ));
    }
    Ok(&user[5..])
}

/// Strip a physical index key down to its `<idx-cols…><pk…>` suffix:
/// `<sys-prefix> <table_id:u32> 'i' <index_id:u8> <suffix>` → `<suffix>`.
fn index_key_suffix(physical: &[u8]) -> Result<&[u8]> {
    let decoded = kv9_common::codec::decode_key(physical)?;
    let user = decoded.user_key;
    if user.len() < 6 {
        return Err(Error::MalformedKey(
            "index key shorter than index header".into(),
        ));
    }
    Ok(&user[6..])
}
