use super::*;

#[test]
fn borrowed_encoding_matches_fixed_v1_frame_and_owned_api_for_all_cfs() {
    // Independent v1 frame including IEEE CRC (also reproducible with zlib).
    let golden = [
        75, 86, 57, 83, 1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 255, 1, 0, 0, 0, 97, 1, 0, 0,
        0, 122, 0, 0, 0, 0, 1, 0, 0, 0, 97, 102, 29, 33, 224,
    ];
    let rows = [(b"".as_slice(), b"\0\xff".as_slice()), (b"a", b"z")];
    for cf in ColumnFamily::ALL {
        let bytes = encode_borrowed(cf, rows.len(), golden.len(), rows.into_iter()).unwrap();
        assert_eq!(bytes.len(), golden.len());
        assert_eq!(bytes.capacity(), golden.len());
        if cf == ColumnFamily::Default {
            assert_eq!(bytes, golden);
        }
        let parsed = SstView::parse(&bytes).unwrap();
        assert_eq!(parsed.column_family(), cf);
        assert_eq!(parsed.iter().collect::<Vec<_>>(), rows);
        let mut writer = SstWriter::new(cf);
        for (key, value) in rows {
            writer.add(key.to_vec(), value.to_vec()).unwrap();
        }
        assert_eq!(writer.finish().unwrap(), bytes);
    }
}

#[test]
fn encoder_handles_a_smaller_final_footer_without_over_reserving() {
    let long = vec![b'a'; 256];
    let rows = [(long.as_slice(), b"v".as_slice()), (b"b", b"v")];
    let exact = SstWriter::table_bytes(8 + 256 + 1 + 8 + 1 + 1, 256, 1).unwrap();
    assert!(SstWriter::table_bytes(8 + 256 + 1, 256, 256).unwrap() > exact);
    let bytes = encode_borrowed(ColumnFamily::Default, 2, exact, rows.into_iter()).unwrap();
    assert_eq!(bytes.len(), exact);
    assert_eq!(bytes.capacity(), exact);
    assert_eq!(
        SstView::parse(&bytes).unwrap().iter().collect::<Vec<_>>(),
        rows
    );
}

#[test]
fn mismatched_preflight_and_unordered_rows_never_return_an_object() {
    let row = [(b"a".as_slice(), b"v".as_slice())];
    for wrong_size in (0..34).chain(35..40) {
        assert!(encode_borrowed(ColumnFamily::Default, 1, wrong_size, row.into_iter()).is_err());
    }
    for wrong_count in [0, 2] {
        assert!(encode_borrowed(ColumnFamily::Default, wrong_count, 34, row.into_iter()).is_err());
    }
    for second in [b"a".as_slice(), b""] {
        let rows = [row[0], (second, b"v".as_slice())];
        assert!(encode_borrowed(ColumnFamily::Default, 2, 44, rows.into_iter()).is_err());
    }
    let extra = [row[0], (b"b".as_slice(), b"v".as_slice())];
    assert!(encode_borrowed(ColumnFamily::Default, 1, 44, extra.into_iter()).is_err());
}

#[test]
fn count_and_size_limits_refuse_before_polling_rows() {
    let limits = Limits {
        entries: 2,
        total_bytes: 40,
        ..Limits::PRODUCTION
    };
    for (count, size) in [(0, 34), (3, 34), (1, 41)] {
        let rows = std::iter::from_fn(|| panic!("rejected layout must not read rows"));
        assert!(encode_rows(ColumnFamily::Default, count, size, limits, rows).is_err());
    }
}

#[test]
fn borrowed_encoder_enforces_key_and_value_field_limits() {
    let limits = Limits {
        field_len: 4,
        ..Limits::PRODUCTION
    };
    for (key, value) in [(b"12345".as_slice(), b"v".as_slice()), (b"k", b"12345")] {
        let rows = [(key, value)];
        let size =
            SstWriter::table_bytes(8 + key.len() + value.len(), key.len(), key.len()).unwrap();
        let error =
            encode_rows(ColumnFamily::Default, 1, size, limits, rows.into_iter()).unwrap_err();
        assert!(error
            .to_string()
            .contains("5 bytes exceeds the 4 byte limit"));
    }
}
