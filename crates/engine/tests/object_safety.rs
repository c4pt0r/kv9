//! `Engine` and `ReadView` must stay usable as trait objects.
//!
//! A node that can be configured to run either the volatile or the durable engine has to
//! hold one of them behind a single handle — either a type parameter threaded through
//! every struct that touches storage, or an `Arc<dyn Engine>`. The second only works while
//! the trait is object-safe, and object safety is easy to lose by accident: one generic
//! method, one `where Self: Sized`, one method returning `Self`, and the option disappears
//! along with any caller that depended on it.
//!
//! These are compile-time assertions wearing test clothing. If they stop building, a trait
//! change has removed a choice from the callers, and that should be a deliberate decision
//! rather than a surprise discovered downstream.

use std::sync::Arc;

use kv9_engine::{ColumnFamily, Engine, MemEngine, ReadView, WalEngine, WriteBatch};

fn tmpdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kv9-objsafe-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Exercises the trait *through* the vtable, so this is a real dynamic-dispatch check and
/// not merely a coercion that the compiler could optimise away.
fn round_trip_through_dyn(engine: &dyn Engine) {
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, b"k".to_vec(), b"v".to_vec());
    engine.write(batch).unwrap();
    assert_eq!(
        engine.get(ColumnFamily::Default, b"k").unwrap(),
        Some(b"v".to_vec())
    );

    let view: Box<dyn ReadView + '_> = engine.snapshot().unwrap();
    assert_eq!(
        view.get(ColumnFamily::Default, b"k").unwrap(),
        Some(b"v".to_vec())
    );
    assert_eq!(view.scan(ColumnFamily::Default, b"", b"z", 10).unwrap().len(), 1);
    assert!(view.seek_le(ColumnFamily::Default, b"k").unwrap().is_some());
    assert_eq!(view.iter(ColumnFamily::Default, b"", b"z").unwrap().count(), 1);
    assert_eq!(
        view.iter_rev(ColumnFamily::Default, b"", b"z").unwrap().count(),
        1
    );
}

#[test]
fn both_engines_fit_behind_one_dyn_handle() {
    let mem: Arc<dyn Engine> = Arc::new(MemEngine::new());
    let (wal, _) = WalEngine::open(tmpdir("dyn").join("wal")).unwrap();
    let wal: Arc<dyn Engine> = Arc::new(wal);

    // The point: one variable can hold either, which is what lets a node choose its
    // engine at runtime from configuration.
    for engine in [mem, wal] {
        round_trip_through_dyn(engine.as_ref());
    }
}

/// The durability answer must survive dynamic dispatch too — it is the one method a
/// caller consults before truncating a raft log, so losing it to a vtable problem would
/// be quiet and expensive.
#[test]
fn durability_is_visible_through_a_trait_object() {
    use kv9_engine::Durability;

    let mem: Arc<dyn Engine> = Arc::new(MemEngine::new());
    let (wal, _) = WalEngine::open(tmpdir("durability").join("wal")).unwrap();
    let wal: Arc<dyn Engine> = Arc::new(wal);

    assert_eq!(mem.durability(), Durability::Volatile);
    assert_eq!(wal.durability(), Durability::DurableThroughLastWrite);
}
