use super::*;

#[tokio::test]
async fn shared_rate_bounds_bursts_and_cancelled_waits_spend_no_credit() {
    let budget = ObjectIoBudget::new(1, 100).unwrap();
    budget.admit(Direction::Put, 100).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), budget.admit(Direction::Get, 100))
            .await
            .is_err()
    );
    let stats = budget.snapshot();
    assert_eq!(stats.waiting_operations, 0);
    assert_eq!(stats.cancelled_before_admission, 1);
    assert_eq!(stats.admitted_get_bytes, 0);
    let budget = std::sync::Arc::new(ObjectIoBudget::new(1000, 100).unwrap());
    let start = Instant::now();
    budget.admit(Direction::Put, 100).await.unwrap();
    for _ in 0..3 {
        budget.admit(Direction::Get, 100).await.unwrap();
    }
    // The first PUT spends the initial burst; all three GETs must be funded
    // by new elapsed time in this fresh budget.
    assert!(start.elapsed() >= Duration::from_millis(300));
    assert_eq!(budget.snapshot().admitted_get_bytes, 300);
    assert_eq!(budget.snapshot().admitted_operations, 4);
}

#[tokio::test]
async fn oversize_never_waits_forever_and_invalid_limits_are_rejected() {
    assert!(ObjectIoBudget::new(0, 1).is_err());
    assert!(ObjectIoBudget::new(1, 0).is_err());
    let budget = ObjectIoBudget::new(u64::MAX, 128).unwrap();
    assert!(budget.admit(Direction::Put, 129).await.is_err());
    budget.admit(Direction::Get, 128).await.unwrap();
    budget.admit(Direction::Get, 128).await.unwrap();
    let stats = budget.snapshot();
    assert_eq!(stats.oversized_operations, 1);
    assert_eq!(stats.admitted_put_bytes, 0);
    assert_eq!(stats.admitted_get_bytes, 256);
    assert_eq!(stats.waiting_operations, 0);
}
