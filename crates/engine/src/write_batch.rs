//! Atomic multi-CF write batch (DESIGN §6.2, §8.1).
//!
//! A committed raft entry is applied into the engine as one `WriteBatch`, so all of a
//! transaction's CF mutations land atomically.

use kv9_common::{UserKey, Value};

use crate::cf::ColumnFamily;

/// A single mutation within a batch.
#[derive(Debug, Clone)]
pub enum Mutation {
    /// Put `value` at `key` in `cf`.
    Put {
        cf: ColumnFamily,
        key: UserKey,
        value: Value,
    },
    /// Delete `key` from `cf`.
    Delete { cf: ColumnFamily, key: UserKey },
}

/// An ordered set of mutations applied atomically (DESIGN §6.2).
#[derive(Debug, Clone, Default)]
pub struct WriteBatch {
    pub(crate) mutations: Vec<Mutation>,
}

impl WriteBatch {
    pub fn new() -> Self {
        WriteBatch::default()
    }

    pub fn put(&mut self, cf: ColumnFamily, key: UserKey, value: Value) -> &mut Self {
        self.mutations.push(Mutation::Put { cf, key, value });
        self
    }

    pub fn delete(&mut self, cf: ColumnFamily, key: UserKey) -> &mut Self {
        self.mutations.push(Mutation::Delete { cf, key });
        self
    }

    pub fn is_empty(&self) -> bool {
        self.mutations.is_empty()
    }

    pub fn len(&self) -> usize {
        self.mutations.len()
    }

    pub fn mutations(&self) -> &[Mutation] {
        &self.mutations
    }
}
