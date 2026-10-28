use super::*;
use std::sync::Arc;

fn encoded(cf: ColumnFamily) -> Vec<u8> {
    let mut writer = SstWriter::new(cf);
    for (key, value) in [
        (b"".as_slice(), b"".as_slice()),
        (b"a", b"\0binary\xff"),
        (b"aa", b"last"),
    ] {
        writer.add(key.to_vec(), value.to_vec()).unwrap();
    }
    writer.finish().unwrap()
}

#[test]
fn borrowed_rows_and_footer_keys_alias_the_original_encoded_buffer() {
    for cf in ColumnFamily::ALL {
        let bytes = encoded(cf);
        let view = SstView::parse(&bytes).unwrap();
        let legacy = Sst::parse(&bytes).unwrap();
        assert_eq!(view.column_family(), cf);
        assert_eq!(view.len(), 3);
        assert_eq!(
            view.iter().collect::<Vec<_>>(),
            legacy.iter().collect::<Vec<_>>()
        );
        assert_eq!(view.smallest_key(), legacy.smallest_key());
        assert_eq!(view.largest_key(), legacy.largest_key());
        let begin = bytes.as_ptr() as usize;
        let end = begin + bytes.len();
        for slice in view
            .iter()
            .flat_map(|(k, v)| [k, v])
            .chain([view.smallest_key(), view.largest_key()])
        {
            let at = slice.as_ptr() as usize;
            assert!(at >= begin && at + slice.len() <= end);
        }
        assert_eq!(view.iter().count(), view.len());
    }
}

#[test]
fn shared_sst_retains_the_original_remote_owner_through_iteration() {
    let bytes = encoded(ColumnFamily::Write);
    let budget = Arc::new(crate::object_memory::BufferBudget::new(bytes.len() as u64).unwrap());
    let owner = budget.copy(&bytes).unwrap();
    let ptr = owner.as_ptr() as usize;
    let shared = SharedSst::parse(owner.clone()).unwrap();
    drop(owner);
    assert_eq!(budget.snapshot().retained_bytes, bytes.len() as u64);
    assert!(budget.copy(&[1]).is_err());
    assert_eq!(shared.column_family(), ColumnFamily::Write);
    for (key, value) in shared.iter() {
        for slice in [key, value] {
            let at = slice.as_ptr() as usize;
            assert!(at >= ptr && at + slice.len() <= ptr + bytes.len());
        }
    }
    drop(shared);
    assert_eq!(budget.snapshot().retained_bytes, 0);
    assert_eq!(budget.snapshot().active_buffers, 0);
}

#[test]
fn every_bulk_reader_rejects_truncation_and_checksum_valid_structural_corruption() {
    let bytes = encoded(ColumnFamily::Default);
    for cut in 0..bytes.len() {
        assert!(SstView::parse(&bytes[..cut]).is_err(), "borrowed cut {cut}");
        assert!(
            SharedSst::parse(Bytes::copy_from_slice(&bytes[..cut])).is_err(),
            "shared cut {cut}"
        );
    }
    let mut writer = SstWriter::new(ColumnFamily::Default);
    writer.add(b"a".to_vec(), b"1".to_vec()).unwrap();
    writer.add(b"b".to_vec(), b"2".to_vec()).unwrap();
    let bytes = writer.finish().unwrap();
    let good = SstView::parse(&bytes).unwrap();
    for variant in 0..7 {
        let mut body = bytes[..bytes.len() - 4].to_vec();
        match variant {
            0 => body[0] ^= 1,
            1 => body[4] = VERSION + 1,
            2 => body[5] = 99,
            3 => body[6..10].copy_from_slice(&u32::MAX.to_le_bytes()),
            4 => body[10..14].copy_from_slice(&u32::MAX.to_le_bytes()),
            5 => {
                // Duplicate keys with matching footer: only strict-order
                // validation rejects this otherwise canonical frame.
                let mut cursor = Cursor {
                    bytes: &body,
                    pos: 10,
                    limits: Limits::PRODUCTION,
                };
                cursor.len_prefixed().unwrap();
                cursor.len_prefixed().unwrap();
                cursor.u32().unwrap();
                let second_key = cursor.pos;
                body[second_key] = b'a';
                body[good.validated.largest.start] = b'a';
            }
            _ => body.push(0),
        }
        body.extend(crc32(&body).to_le_bytes());
        assert!(SstView::parse(&body).is_err(), "borrowed variant {variant}");
        assert!(
            SharedSst::parse(Bytes::copy_from_slice(&body)).is_err(),
            "shared variant {variant}"
        );
        assert!(Sst::parse(&body).is_err(), "owned variant {variant}");
    }
}
