use super::*;

#[test]
fn configuration_is_explicit_and_retry_backoff_has_a_finite_ceiling() {
    assert!(enabled(Err(std::env::VarError::NotPresent)).unwrap());
    assert!(!enabled(Ok("0".into())).unwrap());
    assert!(enabled(Ok("1".into())).unwrap());
    for value in ["", "true", "2", "-1", " 1", "01"] {
        assert!(enabled(Ok(value.into())).is_err());
    }
    assert_eq!(backoff(1), Duration::from_millis(250));
    assert_eq!(backoff(2), Duration::from_millis(500));
    assert_eq!(backoff(u32::MAX), Duration::from_secs(30));
}
