//! Public behavior shared by borrowed and consuming batch application.
//! These tests intentionally make no allocation or buffer-identity assertions.

use kv9_common::AppliedPosition;
use kv9_engine::{ColumnFamily, Engine, MemEngine, ReplicatedEngine, WriteBatch};

const MANIFEST: &[u8] = b"\x00kv9\x00manifest_";
const MANIFEST_CHILD: &[u8] = b"\x00kv9\x00manifest_checkpoint";

fn at(index: u64) -> AppliedPosition {
    AppliedPosition { term: 3, index }
}

fn apply_revision(engine: &MemEngine, batch: WriteBatch, index: &mut u64, delta: u64) {
    let before = engine.data_revision();
    *index += 1;
    engine.write_applied(batch, at(*index)).unwrap();
    assert_eq!(
        engine.data_revision(),
        before + delta,
        "positioned batch changed the revision by the wrong amount at index {index}"
    );
    assert_eq!(engine.volatile_applied_position(), Some(at(*index)));
}

#[test]
fn revision_tracks_non_manifest_mutations_once_per_positioned_batch() {
    let engine = MemEngine::new();
    let mut index = 0;
    apply_revision(&engine, WriteBatch::new(), &mut index, 0);

    // Revision classification is about the physical key, independently of CF.
    for cf in ColumnFamily::ALL {
        let mut manifest_puts = WriteBatch::new();
        manifest_puts.put(cf, MANIFEST.to_vec(), b"root".to_vec());
        manifest_puts.put(cf, MANIFEST_CHILD.to_vec(), b"child".to_vec());
        apply_revision(&engine, manifest_puts, &mut index, 0);
        assert_eq!(engine.get(cf, MANIFEST).unwrap(), Some(b"root".to_vec()));
        assert_eq!(
            engine.get(cf, MANIFEST_CHILD).unwrap(),
            Some(b"child".to_vec())
        );

        let mut manifest_deletes = WriteBatch::new();
        manifest_deletes.delete(cf, MANIFEST.to_vec());
        manifest_deletes.delete(cf, MANIFEST_CHILD.to_vec());
        apply_revision(&engine, manifest_deletes, &mut index, 0);
        assert_eq!(engine.get(cf, MANIFEST).unwrap(), None);
        assert_eq!(engine.get(cf, MANIFEST_CHILD).unwrap(), None);

        // Neither the last mutation's class nor the number of data mutations
        // determines the increment: both mixed orders advance exactly once.
        let mut ordinary_first = WriteBatch::new();
        ordinary_first.put(cf, b"a".to_vec(), b"first".to_vec());
        ordinary_first.put(cf, b"b".to_vec(), b"second".to_vec());
        ordinary_first.put(cf, MANIFEST_CHILD.to_vec(), b"metadata".to_vec());
        apply_revision(&engine, ordinary_first, &mut index, 1);
        assert_eq!(engine.get(cf, b"a").unwrap(), Some(b"first".to_vec()));
        assert_eq!(engine.get(cf, b"b").unwrap(), Some(b"second".to_vec()));

        let mut ordinary_last = WriteBatch::new();
        ordinary_last.delete(cf, MANIFEST_CHILD.to_vec());
        ordinary_last.delete(cf, b"a".to_vec());
        ordinary_last.put(cf, b"b".to_vec(), b"last".to_vec());
        apply_revision(&engine, ordinary_last, &mut index, 1);
        assert_eq!(engine.get(cf, b"a").unwrap(), None);
        assert_eq!(engine.get(cf, b"b").unwrap(), Some(b"last".to_vec()));

        let mut absent_delete = WriteBatch::new();
        absent_delete.delete(cf, b"absent".to_vec());
        apply_revision(&engine, absent_delete, &mut index, 1);
        assert_eq!(engine.get(cf, b"absent").unwrap(), None);

        let mut net_no_change = WriteBatch::new();
        net_no_change.put(cf, b"temporary".to_vec(), b"value".to_vec());
        net_no_change.delete(cf, b"temporary".to_vec());
        apply_revision(&engine, net_no_change, &mut index, 1);
        assert_eq!(engine.get(cf, b"temporary").unwrap(), None);

        // A shorter reserved-looking key and a non-prefix occurrence are data.
        for key in [b"\x00kv9\x00manifest".as_slice(), b"x\x00kv9\x00manifest_"] {
            let mut near_prefix = WriteBatch::new();
            near_prefix.put(cf, key.to_vec(), b"data".to_vec());
            apply_revision(&engine, near_prefix, &mut index, 1);
            assert_eq!(engine.get(cf, key).unwrap(), Some(b"data".to_vec()));
        }
    }

    // Empty application must preserve an already nonzero revision as well.
    apply_revision(&engine, WriteBatch::new(), &mut index, 0);
}

#[test]
fn positioned_and_plain_batches_preserve_order_and_retained_views() {
    for positioned in [false, true] {
        let engine = MemEngine::new();
        let mut initial = WriteBatch::new();
        for cf in ColumnFamily::ALL {
            initial.put(cf, b"key".to_vec(), b"before".to_vec());
            initial.put(cf, b"deleted".to_vec(), b"before".to_vec());
        }
        engine.write_applied(initial, at(10)).unwrap();
        let before_revision = engine.data_revision();
        let retained = engine.snapshot().unwrap();

        let mut next = WriteBatch::new();
        for cf in ColumnFamily::ALL {
            next.put(cf, b"key".to_vec(), b"intermediate".to_vec());
            next.delete(cf, b"key".to_vec());
            next.put(cf, b"key".to_vec(), Vec::new());
            next.put(cf, b"deleted".to_vec(), b"intermediate".to_vec());
            next.delete(cf, b"deleted".to_vec());
        }
        if positioned {
            engine.write_applied(next, at(11)).unwrap();
        } else {
            engine.write(next).unwrap();
        }

        let current = engine.snapshot().unwrap();
        for cf in ColumnFamily::ALL {
            assert_eq!(
                retained.get(cf, b"key").unwrap(),
                Some(b"before".to_vec()),
                "later batch changed a retained snapshot"
            );
            assert_eq!(
                retained.get(cf, b"deleted").unwrap(),
                Some(b"before".to_vec())
            );
            assert_eq!(
                current.get(cf, b"key").unwrap(),
                Some(Vec::new()),
                "Put/Delete/Put lost input order or confused empty with absent"
            );
            assert_eq!(current.get(cf, b"deleted").unwrap(), None);
        }
        assert_eq!(
            engine.data_revision(),
            before_revision + u64::from(positioned),
            "plain write must leave revision unchanged; positioned write advances once"
        );
        assert_eq!(
            engine.volatile_applied_position(),
            Some(at(if positioned { 11 } else { 10 }))
        );
    }
}

#[test]
fn refused_position_preserves_all_cfs_revision_and_position() {
    let engine = MemEngine::new();
    let mut initial = WriteBatch::new();
    for cf in ColumnFamily::ALL {
        initial.put(cf, b"key".to_vec(), b"before".to_vec());
        initial.put(cf, MANIFEST_CHILD.to_vec(), b"before".to_vec());
    }
    engine.write_applied(initial, at(10)).unwrap();
    let before_revision = engine.data_revision();
    let retained = engine.snapshot().unwrap();

    for refused in [
        at(10),
        AppliedPosition {
            term: 99,
            index: 10,
        },
        at(9),
    ] {
        let mut batch = WriteBatch::new();
        for cf in ColumnFamily::ALL {
            batch.delete(cf, b"key".to_vec());
            batch.put(cf, b"new".to_vec(), b"unpublished".to_vec());
            batch.put(cf, MANIFEST_CHILD.to_vec(), b"unpublished".to_vec());
        }
        let error = engine.write_applied(batch, refused).unwrap_err();
        assert!(error.to_string().contains("must advance"));
        assert_eq!(
            engine.data_revision(),
            before_revision,
            "refused position advanced the data revision"
        );
        assert_eq!(engine.volatile_applied_position(), Some(at(10)));
        let current = engine.snapshot().unwrap();
        for cf in ColumnFamily::ALL {
            let expected = vec![
                (MANIFEST_CHILD.to_vec(), b"before".to_vec()),
                (b"key".to_vec(), b"before".to_vec()),
            ];
            assert_eq!(
                current.scan(cf, b"", b"\xff", 16).unwrap(),
                expected,
                "refused position changed {cf:?} contents"
            );
            assert_eq!(retained.scan(cf, b"", b"\xff", 16).unwrap(), expected);
        }
    }
}
