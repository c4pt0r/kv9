use super::*;

fn sequence() -> PayloadSequence {
    PayloadSequence::new(
        pb::MigrationCaptureTransferRequest {
            challenge: vec![7; 16],
            descriptor_digest: vec![8; 32],
            ..Default::default()
        },
        6,
    )
}
fn frame(offset: u64, last: bool) -> pb::MigrationCaptureChunk {
    pb::MigrationCaptureChunk {
        challenge: vec![7; 16],
        descriptor_digest: vec![8; 32],
        offset,
        total_bytes: 6,
        payload: vec![9; 3],
        last,
    }
}
#[test]
fn stream_refuses_wrong_binding_length_order_and_missing_or_extra_terminal_frames() {
    let mut good = sequence();
    assert_eq!(good.accept(Some(frame(0, false))).unwrap(), vec![9; 3]);
    assert_eq!(good.accept(Some(frame(3, true))).unwrap(), vec![9; 3]);
    assert!(!good.done, "last flag alone is not transport EOF");
    assert!(good.accept(None).unwrap().is_empty());
    assert!(good.done);
    assert!(good.accept(Some(frame(3, true))).is_err());
    for case in 0..9 {
        let mut value = frame(0, false);
        match case {
            0 => value.challenge[0] ^= 1,
            1 => value.descriptor_digest[0] ^= 1,
            2 => value.offset = 1,
            3 => value.total_bytes += 1,
            4 => value.payload.clear(),
            5 => value.payload.resize(MAX_CHUNK_BYTES + 1, 9),
            6 => value.payload.resize(7, 9),
            7 => value.last = true,
            _ => value.challenge.resize(MAX_REPLY_BYTES, 7),
        }
        assert!(sequence().accept(Some(value)).is_err(), "case {case}");
    }
    assert!(sequence().accept(None).is_err());
    let mut truncated = sequence();
    truncated.accept(Some(frame(0, false))).unwrap();
    assert!(truncated.accept(None).is_err());
    assert!(truncated.accept(Some(frame(0, false))).is_err());
    assert!(truncated.accept(Some(frame(3, false))).is_err());
    let mut extra = sequence();
    extra.accept(Some(frame(0, false))).unwrap();
    extra.accept(Some(frame(3, true))).unwrap();
    assert!(extra.accept(Some(frame(6, true))).is_err());
    let mut overflow = sequence();
    overflow.expected = u64::MAX;
    overflow.offset = u64::MAX - 1;
    let mut value = frame(u64::MAX - 1, true);
    value.total_bytes = u64::MAX;
    assert!(overflow.accept(Some(value)).is_err());
}
