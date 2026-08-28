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
//! In Phase-1 the store is backed by the mocked [`kv9_engine::Engine`] (a `MemEngine`);
//! the same code applies committed raft entries into that engine via the raft state
//! machine (`kv9_raft::MemStateMachine`). The engine swap to the real disaggregated LSM
//! is Phase-2 and does not touch this API.

use std::sync::Arc;

use kv9_engine::{ColumnFamily, Engine, WriteBatch};

use kv9_common::Result;

use crate::codec::{
    self, encode_row_key, index_prefix_range, row_range, ColumnValue, PkComponent, RowValue,
};
use crate::schema::{IndexId, TableDesc};

/// A decoded catalog row: the primary key components plus the tagged column set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub pk: Vec<PkComponent>,
    pub value: RowValue,
}

/// A change-set for [`MetaTxn::update`]: the columns to overwrite (METADATA-CATALOG §4).
pub type Changes = Vec<(crate::schema::ColumnId, ColumnValue)>;

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
    /// Phase-1: the txn buffers writes and applies them as one atomic [`WriteBatch`] on
    /// [`MetaTxn::commit`]. A real impl acquires a `start_ts` from the system TSO and
    /// runs Percolator 2PC within the `system` group (§5) — that lands in Phase-3.
    pub fn begin(&self) -> MetaTxn<'_, E> {
        MetaTxn {
            store: self,
            batch: WriteBatch::new(),
            committed: false,
        }
    }
}

/// A metadata transaction exposing the typed op API (METADATA-CATALOG §4).
///
/// All catalog rows live in a single column family ([`ColumnFamily::Default`]) of the
/// system keyspace; MVCC/lock/write CFs of a real `txn` keyspace are used once the store
/// runs real 2PC (Phase-3).
pub struct MetaTxn<'a, E: Engine> {
    store: &'a MetaStore<E>,
    batch: WriteBatch,
    committed: bool,
}

impl<'a, E: Engine> MetaTxn<'a, E> {
    /// Point read of a row by primary key (METADATA-CATALOG §4).
    pub fn get(&self, table: &TableDesc, pk: &[PkComponent]) -> Result<Option<Row>> {
        let key = encode_row_key(table.id, pk)?;
        match self.store.engine.get(ColumnFamily::Default, &key)? {
            None => Ok(None),
            Some(bytes) => {
                let value = RowValue::decode(&bytes)?;
                Ok(Some(Row {
                    pk: pk.to_vec(),
                    value,
                }))
            }
        }
    }

    /// Insert a row, maintaining all secondary indexes; UNIQUE/FK are checked
    /// (METADATA-CATALOG §4). Buffers into the txn's write batch.
    ///
    /// Phase-1 stub: buffers the row put and the index puts. UNIQUE/FK enforcement and
    /// read-your-writes over the buffer are `unimplemented!()`; the signature is real.
    pub fn insert(&mut self, table: &TableDesc, pk: &[PkComponent], value: RowValue) -> Result<()> {
        let key = encode_row_key(table.id, pk)?;
        self.batch.put(ColumnFamily::Default, key, value.encode());
        // Maintain indexes: for each index, encode its key from the row's columns.
        for index in table.indexes {
            let cols = self.index_columns(table, index.id, &value)?;
            let idx_key = codec::encode_index_key(table.id, index.id, &cols, pk)?;
            let idx_val = if index.unique {
                codec::index_value(pk)
            } else {
                Vec::new()
            };
            self.batch.put(ColumnFamily::Default, idx_key, idx_val);
        }
        Ok(())
    }

    /// Update a row by primary key, re-maintaining affected indexes (METADATA-CATALOG §4).
    pub fn update(&mut self, _table: &TableDesc, _pk: &[PkComponent], _changes: Changes) -> Result<()> {
        // TODO(phase1): read current row, diff changed index columns, delete stale index
        // entries and write new ones, then put the merged row value.
        unimplemented!("MetaTxn::update — index re-maintenance (METADATA-CATALOG §4)")
    }

    /// Delete a row by primary key, removing its index rows too (METADATA-CATALOG §4).
    pub fn delete(&mut self, _table: &TableDesc, _pk: &[PkComponent]) -> Result<()> {
        // TODO(phase1): read the row to recompute index keys, then delete row + indexes.
        unimplemented!("MetaTxn::delete — index cleanup (METADATA-CATALOG §4)")
    }

    /// Full/prefix table scan over the row space (METADATA-CATALOG §4).
    pub fn scan(&self, table: &TableDesc, limit: usize) -> Result<Vec<Row>> {
        let (start, end) = row_range(table.id)?;
        let entries = self.store.engine.scan(ColumnFamily::Default, &start, &end, limit)?;
        let mut out = Vec::with_capacity(entries.len());
        for (_k, v) in entries {
            // Phase-1: pk reconstruction from the physical key is not yet wired; callers
            // that need pks use index_scan + get. Row.pk left empty here.
            out.push(Row {
                pk: Vec::new(),
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
        let (start, end) = index_prefix_range(table.id, index, prefix)?;
        let entries = self.store.engine.scan(ColumnFamily::Default, &start, &end, limit)?;
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        // TODO(phase1): decode the pk suffix from each index key (non-unique) or its
        // value (unique). Requires the memcomparable component splitter in codec.
        Err(kv9_common::Error::NotImplemented(
            "MetaTxn::index_scan pk decode (METADATA-CATALOG §3/§4)",
        ))
    }

    /// Commit the buffered mutations atomically (METADATA-CATALOG §4, §5).
    ///
    /// Phase-1: applies the write batch to the engine in one atomic call. The real path
    /// routes through the raft `Command::CatalogTxn` so the commit is replicated before
    /// it is applied (see `kv9_raft::Command` / `MemStateMachine`).
    pub fn commit(mut self) -> Result<()> {
        self.store.engine.write(std::mem::take(&mut self.batch))?;
        self.committed = true;
        Ok(())
    }

    /// Take the buffered write batch without committing, e.g. to hand it to raft as the
    /// payload of a `Command::CatalogTxn` (METADATA-CATALOG §5).
    pub fn into_batch(mut self) -> WriteBatch {
        self.committed = true;
        std::mem::take(&mut self.batch)
    }

    /// Compute the memcomparable index-key columns for `index` from a row's values.
    fn index_columns(
        &self,
        table: &TableDesc,
        index: IndexId,
        value: &RowValue,
    ) -> Result<Vec<PkComponent>> {
        let idx = table
            .index(index)
            .ok_or(kv9_common::Error::NotImplemented("unknown index id"))?;
        let mut cols = Vec::with_capacity(idx.columns.len());
        for col_id in idx.columns {
            let comp = match value.get(*col_id) {
                Some(ColumnValue::Uint(v)) => codec::memcmp_uint(*v),
                Some(ColumnValue::Text(s)) => codec::memcmp_text(s),
                Some(ColumnValue::Bytes(b)) => codec::memcmp_bytes(b),
                None => Vec::new(),
            };
            cols.push(comp);
        }
        Ok(cols)
    }
}

impl<'a, E: Engine> Drop for MetaTxn<'a, E> {
    fn drop(&mut self) {
        // A dropped, uncommitted txn discards its buffer — nothing was written, so
        // there is nothing to roll back (Phase-1 buffer-then-apply model).
        let _ = self.committed;
    }
}
