use super::*;

#[test]
fn replica_pressure_is_bounded_light_work_and_does_not_overtake_waiting_ttl() {
    let budget = Arc::new(Budget::default());
    let mut data = budget.lane(Class::DataCheckpoint);
    let mut pressure = budget.lane(Class::ReplicaPressure);
    let mut ttl = budget.lane(Class::Ttl);
    let now = Instant::now();
    let heavy = (data.try_at(now).unwrap(), data.try_at(now).unwrap());
    let a = pressure.try_at(now).unwrap();
    let b = pressure.try_at(now).unwrap();
    assert!(pressure.try_at(now + REFILL).is_none());
    pressure.withdraw();
    assert!(ttl.try_at(now + REFILL).is_none());
    drop(a);
    assert!(pressure.try_at(now + REFILL * 2).is_none());
    let ttl_job = ttl.try_at(now + REFILL * 2).unwrap();
    assert_eq!(budget.snapshot().heavy_active, 2);
    drop((heavy, b, ttl_job, pressure, ttl, data));
    assert_eq!(budget.snapshot().active, 0);
    assert_eq!(budget.snapshot().waiting_classes, 0);
    let stats = budget.snapshot().classes[Class::ReplicaPressure.index()].clone();
    assert_eq!(stats.peak, 2);
    assert_eq!(stats.admitted, stats.released);
}

#[test]
fn checkpoint_retention_uses_one_light_slot_and_actual_job_keeps_it() {
    let budget = Arc::new(Budget::default());
    let mut data = budget.lane(Class::DataCheckpoint);
    let mut ttl = budget.lane(Class::Ttl);
    let mut sweep = budget.lane(Class::CheckpointRetention);
    let now = Instant::now();
    let heavy = (data.try_at(now).unwrap(), data.try_at(now).unwrap());
    let light = ttl.try_at(now).unwrap();
    let retention = sweep.try_at(now).unwrap();
    assert_eq!(budget.snapshot().active, 4);
    assert_eq!(budget.snapshot().heavy_active, 2);
    drop(light);
    assert!(
        sweep.try_at(now + REFILL).is_none(),
        "one per-node retention job"
    );
    drop(sweep);
    assert_eq!(budget.snapshot().active, 3);
    drop(retention);
    drop(heavy);
    let stats = budget.snapshot();
    assert_eq!(stats.active, 0);
    assert_eq!(
        stats.classes[Class::CheckpointRetention.index()].admitted,
        1
    );
    assert_eq!(
        stats.classes[Class::CheckpointRetention.index()].released,
        1
    );
}

#[test]
fn heavy_saturation_leaves_ttl_capacity_and_waiting_classes_cannot_be_overtaken() {
    let budget = Arc::new(Budget::default());
    let mut data = budget.lane(Class::DataCheckpoint);
    let mut metadata = budget.lane(Class::MetadataCheckpoint);
    let mut compact = budget.lane(Class::Compaction);
    let mut ttl = budget.lane(Class::Ttl);
    let now = Instant::now();
    let a = data.try_at(now).unwrap();
    let b = data.try_at(now).unwrap();
    assert!(metadata.try_at(now + REFILL).is_none());
    assert!(compact.try_at(now + REFILL * 2).is_none());
    let c = ttl.try_at(now + REFILL * 3).unwrap();
    let d = ttl.try_at(now + REFILL * 4).unwrap();
    assert_eq!(budget.snapshot().active, JOBS);
    assert_eq!(budget.snapshot().heavy_active, HEAVY);
    drop(a);
    assert!(data.try_at(now + REFILL * 5).is_none());
    assert!(compact.try_at(now + REFILL * 5).is_none());
    let meta = metadata.try_at(now + REFILL * 5).unwrap();
    drop(b);
    assert!(data.try_at(now + REFILL * 6).is_none());
    let compaction = compact.try_at(now + REFILL * 6).unwrap();
    drop((meta, compaction, c, d));
    drop((data, metadata, compact, ttl));
    let stats = budget.snapshot();
    assert_eq!(stats.active, 0);
    assert_eq!(stats.waiting_classes, 0);
    assert_eq!(stats.peak, JOBS);
    assert_eq!(stats.heavy_peak, HEAVY);
    assert!(stats.classes.iter().all(|c| c.admitted == c.released));
}

#[test]
fn idle_or_cancelled_dispatcher_withdraws_demand_and_cannot_release_running_job() {
    let budget = Arc::new(Budget::default());
    let mut data = budget.lane(Class::DataCheckpoint);
    let mut meta = budget.lane(Class::MetadataCheckpoint);
    let mut reclaim = budget.lane(Class::RaftReclaim);
    let now = Instant::now();
    let a = data.try_at(now).unwrap();
    let b = data.try_at(now).unwrap();
    assert!(meta.try_at(now).is_none());
    assert!(reclaim.try_at(now + REFILL).is_none());
    meta.withdraw();
    drop(data); // The actual jobs, rather than their dispatcher, own capacity.
    assert_eq!(budget.snapshot().active, 2);
    assert!(reclaim.try_at(now + REFILL * 2).is_none());
    drop(a);
    let job = reclaim.try_at(now + REFILL * 3).unwrap();
    drop(reclaim);
    assert_eq!(budget.snapshot().active, 2);
    drop((job, b));
    assert_eq!(budget.snapshot().active, 0);
}

#[test]
fn instantaneous_work_obeys_node_dispatch_rate_without_catchup_bursts() {
    let budget = Arc::new(Budget::default());
    let mut ttl = budget.lane(Class::Ttl);
    let now = budget.state.lock().unwrap().refilled;
    for _ in 0..BURST {
        drop(ttl.try_at(now).unwrap());
    }
    assert!(ttl.try_at(now).is_none());
    assert!(ttl.try_at(now + REFILL / 2).is_none());
    drop(ttl.try_at(now + REFILL).unwrap());
    assert!(ttl.try_at(now + REFILL).is_none());
    let later = now + Duration::from_secs(100);
    for _ in 0..BURST {
        drop(ttl.try_at(later).unwrap());
    }
    assert!(ttl.try_at(later).is_none());
    ttl.withdraw();
}

#[test]
fn split_has_one_heavy_slot_and_dispatcher_drop_cannot_release_actual_work() {
    let budget = Arc::new(Budget::default());
    let mut split = budget.lane(Class::Split);
    let mut data = budget.lane(Class::DataCheckpoint);
    let mut ttl = budget.lane(Class::Ttl);
    let now = Instant::now();
    let job = split.try_at(now).unwrap();
    assert!(split.try_at(now + REFILL).is_none());
    let upload = data.try_at(now + REFILL).unwrap();
    assert!(data.try_at(now + REFILL * 2).is_none());
    let cleanup = ttl.try_at(now + REFILL * 2).unwrap();
    drop(split);
    assert_eq!(budget.snapshot().heavy_active, 2);
    assert!(data.try_at(now + REFILL * 3).is_none());
    drop(job);
    let next_upload = data.try_at(now + REFILL * 4).unwrap();
    drop((upload, next_upload, cleanup));
    let stats = budget.snapshot();
    assert_eq!(stats.active, 0);
    let split_stats = &stats.classes[Class::Split.index()];
    assert_eq!(split_stats.class, "split");
    assert_eq!(split_stats.admitted, 1);
    assert_eq!(split_stats.released, 1);
}
