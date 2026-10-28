use super::*;

#[test]
fn terminal_migration_publisher_ack_uses_its_own_retained_report() {
    let (e, r, d, b) = fixture(2);
    apply(&e, &r, enter(&d, &b));
    let command = finish(&d, &b, 4, 1);
    apply(&e, &r, command.clone());
    let report = fact(&e, &r, &b);
    let mut next = b.clone();
    next.generation = NonZeroU64::new(2).unwrap();
    apply(&e, &r, enter(&d, &next));
    // Simulate a future source-history archive. This test only proves that an
    // exact terminal acknowledgement has no remaining source dependency.
    let (source, destination) = image_owners(&d).unwrap();
    let mut batch = WriteBatch::new();
    batch.delete(ColumnFamily::Default, catalog_key(d.request().operation));
    for s in [source, destination] {
        batch.delete(ColumnFamily::Default, owner_key(s.descriptor.id));
        batch.delete(
            ColumnFamily::Default,
            owner_registration_key(s.descriptor.id),
        );
    }
    e.write(batch).unwrap();
    let before = all(&e);
    assert!(!plan(&e, &r, &command)
        .expect("terminal Publisher acknowledgement still reads original catalog")
        .changed());
    assert_eq!(all(&e), before);
    assert_eq!(fact(&e, &r, &b), report);
    assert_eq!(owner(&e, &r, b.descriptor.id).phase, PinPhase::Published);
    assert!(plan(&e, &r, &finish(&d, &next, 0, 0)).is_err());
    assert!(plan(&e, &r, &finish(&d, &b, 0, 0)).is_err());
    let mut wrong = command;
    if let LedgerRequest::FinishMigrationPublisher { operation, .. } = &mut wrong {
        operation[0] ^= 1;
    }
    assert!(plan(&e, &r, &wrong).is_err());
}

#[test]
fn terminal_migration_ack_preserves_registration_pins_phase_and_report_checks() {
    for damage in 0..5 {
        let (e, r, d, b) = fixture(2);
        apply(&e, &r, enter(&d, &b));
        let command = finish(&d, &b, 2, 0);
        apply(&e, &r, command.clone());
        let mut batch = WriteBatch::new();
        let key = match damage {
            0 => owner_key(b.descriptor.id),
            1 => owner_registration_key(b.descriptor.id),
            2 => resource_key(&b.resources[0]),
            3 => record_key(b.token()),
            _ => {
                let mut row = owner(&e, &r, b.descriptor.id);
                row.phase = PinPhase::Published;
                batch.put(
                    ColumnFamily::Default,
                    owner_key(b.descriptor.id),
                    encode_owner(&row),
                );
                e.write(batch).unwrap();
                assert!(plan(&e, &r, &command).is_err());
                continue;
            }
        };
        batch.delete(ColumnFamily::Default, key);
        e.write(batch).unwrap();
        let before = all(&e);
        assert!(plan(&e, &r, &command).is_err(), "damage {damage}");
        assert_eq!(all(&e), before);
    }
}
