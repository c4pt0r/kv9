use super::*;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn catalog_waiter_deadline_withdraws_without_late_ownership() {
    let lock = Arc::new(CatalogLock::new());
    let held = lock.lock();
    assert!(lock.until(Instant::now()).is_none());
    let copy = lock.clone();
    let waiter = std::thread::spawn(move || {
        copy.until(Instant::now() + Duration::from_millis(20))
            .is_none()
    });
    assert!(waiter.join().unwrap(), "expired waiter acquired held lock");
    assert_eq!(lock.waiting(), 0);
    drop(held);
    assert!(lock
        .until(Instant::now() + Duration::from_secs(1))
        .is_some());
    assert!(lock.try_lock().is_some(), "withdrawal left an orphan owner");
}

#[test]
fn catalog_serialization_survives_contended_mixed_waiters() {
    let lock = Arc::new(CatalogLock::new());
    let sequence = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0usize..8)
        .map(|n| {
            let lock = lock.clone();
            let sequence = sequence.clone();
            std::thread::spawn(move || {
                for _ in 0..32 {
                    let _guard = if n.is_multiple_of(2) {
                        lock.lock()
                    } else {
                        lock.until(Instant::now() + Duration::from_secs(5))
                            .expect("bounded waiter starved")
                    };
                    let old = sequence.load(Ordering::Relaxed);
                    std::thread::yield_now();
                    sequence.store(old + 1, Ordering::Relaxed);
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(sequence.load(Ordering::Relaxed), 256);
    assert_eq!(lock.waiting(), 0);
}

#[test]
fn catalog_panic_poisoning_survives_fair_handoff_and_all_entry_points() {
    let lock = Arc::new(CatalogLock::new());
    let copy = lock.clone();
    assert!(std::thread::spawn(move || {
        let _guard = copy.lock();
        panic!("planner failed while holding catalog ownership");
    })
    .join()
    .is_err());
    for path in 0..3 {
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = match path {
                0 => Some(lock.lock()),
                1 => lock.try_lock(),
                _ => lock.until(Instant::now() + Duration::from_secs(1)),
            };
        }))
        .is_err());
        assert_eq!(lock.waiting(), 0);
    }
}
