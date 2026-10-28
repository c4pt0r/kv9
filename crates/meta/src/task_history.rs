//! Atomic mutation revision for retained task-catalog censuses. This stamp is
//! not evidence of a complete historical membership or recovery inventory.
use kv9_common::{Error, Result, RootDigest};
use kv9_engine::{ColumnFamily, ReadView};

pub(crate) const KEY: &[u8] = b"\0kv9\0task_history_revision\0";
const MAGIC: &[u8; 8] = b"KV9THR01";
const BYTES: usize = 8 + 8 + 1 + 32;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Revision {
    pub sequence: u64,
    /// A delete or rewrite can hide a participant before it is inventoried.
    /// This flag is sticky; a later clean scan cannot repair lost history.
    pub uncertain: bool,
}
impl Revision {
    fn encode(self) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        bytes.push(u8::from(self.uncertain));
        bytes.extend_from_slice(RootDigest::sha256(&bytes).as_bytes());
        bytes
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != BYTES
            || !bytes.starts_with(MAGIC)
            || RootDigest::sha256(&bytes[..BYTES - 32]).as_bytes() != &bytes[BYTES - 32..]
        {
            return Err(invalid("task revision checksum or length"));
        }
        let value = Self {
            sequence: u64::from_be_bytes(bytes[8..16].try_into().unwrap()),
            uncertain: match bytes[16] {
                0 => false,
                1 => true,
                _ => return Err(invalid("task revision flag")),
            },
        };
        if value.sequence == 0 || value.encode() != bytes {
            return Err(invalid("noncanonical task revision"));
        }
        Ok(value)
    }
}
pub fn revision(view: &dyn ReadView) -> Result<Revision> {
    view.get(ColumnFamily::Default, KEY)?
        .as_deref()
        .map(Revision::decode)
        .transpose()
        .map(|v| v.unwrap_or_default())
}
pub(crate) fn next(previous: Option<&[u8]>, uncertain: bool) -> Result<Vec<u8>> {
    let old = previous
        .map(Revision::decode)
        .transpose()?
        .unwrap_or_default();
    Ok(Revision {
        sequence: old
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("task revision exhausted"))?,
        uncertain: old.uncertain || uncertain,
    }
    .encode())
}
fn invalid(message: &str) -> Error {
    Error::Config(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{memcmp_uint, ColumnValue, RowValue};
    use crate::schema::{ColumnId, TASKS_DESC};
    use crate::store::MetaStore;
    use kv9_engine::{Engine, MemEngine, WriteBatch};
    use std::sync::Arc;
    fn row(id: u64, kind: u64) -> RowValue {
        let mut r = RowValue::new();
        for (column, value) in [(1, id), (2, kind), (4, 0), (5, 0)] {
            r.set(ColumnId(column), ColumnValue::Uint(value));
        }
        r.set(ColumnId(3), ColumnValue::Bytes(vec![1]));
        r
    }
    #[test]
    fn task_revision_is_atomic_overlay_aware_and_uncertainty_never_clears() {
        let engine = Arc::new(MemEngine::new());
        let store = MetaStore::new(engine.clone());
        let mut t = store.begin().unwrap();
        t.insert(&TASKS_DESC, &[memcmp_uint(100)], row(100, 100))
            .unwrap();
        t.insert(&TASKS_DESC, &[memcmp_uint(101)], row(101, 103))
            .unwrap();
        assert_eq!(
            revision(engine.snapshot().unwrap().as_ref()).unwrap(),
            Revision::default()
        );
        drop(t);
        assert_eq!(
            revision(engine.snapshot().unwrap().as_ref()).unwrap(),
            Revision::default()
        );
        let mut t = store.begin().unwrap();
        t.insert(&TASKS_DESC, &[memcmp_uint(100)], row(100, 100))
            .unwrap();
        t.insert(&TASKS_DESC, &[memcmp_uint(101)], row(101, 103))
            .unwrap();
        t.commit().unwrap();
        assert_eq!(
            revision(engine.snapshot().unwrap().as_ref()).unwrap(),
            Revision {
                sequence: 2,
                uncertain: false
            }
        );
        let mut t = store.begin().unwrap();
        t.update(
            &TASKS_DESC,
            &[memcmp_uint(100)],
            vec![(ColumnId(3), ColumnValue::Bytes(vec![2]))],
        )
        .unwrap();
        t.commit().unwrap();
        let mut t = store.begin().unwrap();
        t.insert(&TASKS_DESC, &[memcmp_uint(102)], row(102, 101))
            .unwrap();
        t.commit().unwrap();
        assert_eq!(
            revision(engine.snapshot().unwrap().as_ref()).unwrap(),
            Revision {
                sequence: 4,
                uncertain: true
            }
        );
        let mut t = store.begin().unwrap();
        t.delete(&TASKS_DESC, &[memcmp_uint(101)]).unwrap();
        t.commit().unwrap();
        assert_eq!(
            revision(engine.snapshot().unwrap().as_ref()).unwrap(),
            Revision {
                sequence: 5,
                uncertain: true
            }
        );
        let mut t = store.begin().unwrap();
        t.delete(&TASKS_DESC, &[memcmp_uint(101)]).unwrap();
        t.commit().unwrap();
        assert_eq!(
            revision(engine.snapshot().unwrap().as_ref())
                .unwrap()
                .sequence,
            5
        );
    }
    #[test]
    fn bad_or_exhausted_task_stamp_refuses_mutation_without_partial_overlay() {
        for bytes in [
            vec![0; BYTES],
            Revision {
                sequence: u64::MAX,
                uncertain: false,
            }
            .encode(),
        ] {
            let engine = Arc::new(MemEngine::new());
            let store = MetaStore::new(engine.clone());
            let mut batch = WriteBatch::new();
            batch.put(ColumnFamily::Default, KEY.to_vec(), bytes.clone());
            engine.write(batch).unwrap();
            let mut t = store.begin().unwrap();
            assert!(t
                .insert(&TASKS_DESC, &[memcmp_uint(100)], row(100, 100))
                .is_err());
            assert!(t.get(&TASKS_DESC, &[memcmp_uint(100)]).unwrap().is_none());
            t.commit().unwrap();
            assert_eq!(engine.get(ColumnFamily::Default, KEY).unwrap(), Some(bytes));
        }
    }
    #[test]
    fn task_revision_codec_rejects_truncation_corruption_and_false_boolean() {
        let bytes = Revision {
            sequence: 7,
            uncertain: true,
        }
        .encode();
        for cut in 0..bytes.len() {
            assert!(Revision::decode(&bytes[..cut]).is_err());
        }
        for index in 0..bytes.len() {
            let mut bad = bytes.clone();
            bad[index] ^= 1;
            assert!(Revision::decode(&bad).is_err());
        }
        let mut bad = bytes[..BYTES - 32].to_vec();
        bad[16] = 2;
        bad.extend_from_slice(RootDigest::sha256(&bad).as_bytes());
        assert!(Revision::decode(&bad).is_err());
        assert!(Revision::decode(
            &Revision {
                sequence: 0,
                uncertain: false
            }
            .encode()
        )
        .is_err());
    }
}

#[cfg(test)]
mod mutation_tests {
    use super::*;
    use crate::codec::{memcmp_uint, ColumnValue, RowValue};
    use crate::schema::{ColumnId, TASKS_DESC};
    use crate::store::MetaStore;
    use kv9_engine::{Engine, MemEngine};
    use std::sync::Arc;
    #[test]
    fn binding_changes_invalidate_coverage_but_noop_updates_do_not_and_transient_deletes_stick() {
        let e = Arc::new(MemEngine::new());
        let s = MetaStore::new(e.clone());
        let mut t = s.begin().unwrap();
        let mut row = RowValue::new();
        for (c, v) in [(1, 100), (2, 102), (4, 0), (5, 0)] {
            row.set(ColumnId(c), ColumnValue::Uint(v));
        }
        row.set(ColumnId(3), ColumnValue::Bytes(vec![1]));
        t.insert(&TASKS_DESC, &[memcmp_uint(100)], row).unwrap();
        t.commit().unwrap();
        let mut t = s.begin().unwrap();
        t.update(
            &TASKS_DESC,
            &[memcmp_uint(100)],
            vec![(ColumnId(3), ColumnValue::Bytes(vec![1]))],
        )
        .unwrap();
        t.commit().unwrap();
        assert_eq!(
            revision(e.snapshot().unwrap().as_ref()).unwrap(),
            Revision {
                sequence: 1,
                uncertain: false
            }
        );
        let mut t = s.begin().unwrap();
        t.update(
            &TASKS_DESC,
            &[memcmp_uint(100)],
            vec![(ColumnId(3), ColumnValue::Bytes(vec![2]))],
        )
        .unwrap();
        t.commit().unwrap();
        assert_eq!(
            revision(e.snapshot().unwrap().as_ref()).unwrap(),
            Revision {
                sequence: 2,
                uncertain: false
            }
        );
        let mut t = s.begin().unwrap();
        t.update(
            &TASKS_DESC,
            &[memcmp_uint(100)],
            vec![(ColumnId(2), ColumnValue::Uint(101))],
        )
        .unwrap();
        t.delete(&TASKS_DESC, &[memcmp_uint(100)]).unwrap();
        t.commit().unwrap();
        assert_eq!(
            revision(e.snapshot().unwrap().as_ref()).unwrap(),
            Revision {
                sequence: 4,
                uncertain: true
            }
        );
        assert!(s
            .begin()
            .unwrap()
            .get(&TASKS_DESC, &[memcmp_uint(100)])
            .unwrap()
            .is_none());
    }
}
