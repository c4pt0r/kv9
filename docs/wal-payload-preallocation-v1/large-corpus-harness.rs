use super::{decode_batch, encode_batch, encode_batch_with_capacity};
use crate::wal_segment::{encoded_size, FRAME_HEADER_BYTES};
use crate::{ColumnFamily, WriteBatch};

fn payload_size(batch: &WriteBatch) -> usize {
    (encoded_size(batch).unwrap() - FRAME_HEADER_BYTES as u64 - 4) as usize
}

#[test]
fn capacity_does_not_change_golden_bytes_or_mutation_order() {
    let mut batch = WriteBatch::new();
    batch.put(ColumnFamily::Default, b"k".to_vec(), vec![0, 255]);
    batch.delete(ColumnFamily::Lock, vec![]);
    batch.put(ColumnFamily::Write, b"x".to_vec(), vec![]);
    let golden = vec![
        3, 0, 0, 0, // count
        0, 0, 1, 0, 0, 0, b'k', 2, 0, 0, 0, 0, 255, // Put default
        1, 1, 0, 0, 0, 0, // Delete lock with empty key
        0, 2, 1, 0, 0, 0, b'x', 0, 0, 0, 0, // Put write with empty value
    ];
    assert_eq!(payload_size(&batch), golden.len());
    for capacity in [0, 1, 7, golden.len() - 1, golden.len(), golden.len() + 37] {
        assert_eq!(encode_batch_with_capacity(&batch, capacity), golden);
    }
    assert_eq!(encode_batch(&batch), golden);
    assert_eq!(encode_batch(&WriteBatch::new()), vec![0; 4]);
}

#[test]
fn validated_size_matches_mixed_binary_batches_and_growing_values() {
    let mut batch = WriteBatch::new();
    for step in 0..180 {
        let size = [0, 1, 7, 8, 63, 64, 127, 128, 255, 256, 4095, 4096][step % 12];
        let cf = ColumnFamily::ALL[step % 3];
        let key = vec![(step % 17) as u8; step % 9];
        if step % 4 == 0 {
            batch.delete(cf, key);
        } else {
            batch.put(cf, key, vec![(step % 251) as u8; size]);
        }
        let expected = encode_batch(&batch);
        let size = payload_size(&batch);
        assert_eq!(size, expected.len());
        let reserved = encode_batch_with_capacity(&batch, size);
        assert_eq!(reserved, expected);
        assert_eq!(encode_batch(&decode_batch(&reserved).unwrap()), expected);
    }
}

#[test]
fn record_limit_is_checked_before_reserving_or_writing_payload() {
    use crate::wal_segment::{SegmentHeader, WalSegment, HEADER_BYTES};
    use crate::wal_v2::MAX_RECORD_LEN;
    use kv9_common::metrics::WalIoMetrics;
    use std::time::{SystemTime, UNIX_EPOCH};

    let mut batch = WriteBatch::new();
    let value_len = MAX_RECORD_LEN as usize - 4 - 10 - 1;
    batch.put(ColumnFamily::Default, vec![0], vec![0; value_len]);
    assert_eq!(payload_size(&batch), MAX_RECORD_LEN as usize);
    let encoded = encode_batch_with_capacity(&batch, payload_size(&batch));
    assert_eq!(encoded.len(), MAX_RECORD_LEN as usize);
    drop(encoded);
    batch.delete(ColumnFamily::Default, vec![]);
    assert!(encoded_size(&batch).is_err());

    let name = format!(
        "kv9-wal-preallocation-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let directory = std::env::temp_dir().join(name);
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("segment.wal");
    let header = SegmentHeader {
        stream_id: [9; 16],
        sequence: 1,
        previous: None,
    };
    let mut writer = WalSegment::create(&path, header, WalIoMetrics::shared()).unwrap();
    assert!(writer.append(&batch, None).is_err());
    assert_eq!(std::fs::metadata(&path).unwrap().len(), HEADER_BYTES as u64);
    assert_eq!(writer.summary().records, 0);
    // Validation refusal does not poison an otherwise healthy writer.
    writer.append(&WriteBatch::new(), None).unwrap();
    drop(writer);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "local encoder CPU experiment; requires KV9_WAL_CORPUS and recorded allocator configuration"]
fn retained_corpus_encoder_microbenchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    let data = std::fs::read(std::env::var("KV9_WAL_CORPUS").expect("KV9_WAL_CORPUS")).unwrap();
    let mut remaining = data.as_slice();
    let mut batches = Vec::new();
    while !remaining.is_empty() {
        let size = u32::from_le_bytes(remaining[..4].try_into().unwrap()) as usize;
        let original = &remaining[4..4 + size];
        let batch = decode_batch(original).unwrap();
        assert_eq!(payload_size(&batch), size);
        assert_eq!(encode_batch(&batch), original);
        assert_eq!(encode_batch_with_capacity(&batch, size), original);
        batches.push(batch);
        remaining = &remaining[4 + size..];
    }
    for reserved in [false, true, true, false] {
        // Include the existing size validation in both timed arms. Neither arm
        // precomputes its sizes outside the timed operation.
        for batch in &batches {
            black_box(encode_batch(batch));
        }
        let mut timings = Vec::new();
        let mut bytes = 0usize;
        let mut allocated_capacity = 0usize;
        for _ in 0..128 {
            for batch in &batches {
                let start = Instant::now();
                let size = payload_size(batch);
                let payload = if reserved {
                    encode_batch_with_capacity(batch, size)
                } else {
                    encode_batch(batch)
                };
                black_box(&payload);
                bytes += payload.len();
                allocated_capacity += payload.capacity();
                drop(payload);
                timings.push(start.elapsed().as_nanos() as u64);
            }
        }
        timings.sort_unstable();
        let sum: u64 = timings.iter().sum();
        println!(
            "WAL_ENCODER_MICRO={}",
            serde_json::json!({
                "reserved": reserved, "batches": timings.len(), "payload_bytes": bytes,
                "sum_final_capacity_bytes": allocated_capacity, "total_ns": sum,
                "mean_ns": sum as f64 / timings.len() as f64,
                "p99_ns": timings[(timings.len() * 99).div_ceil(100) - 1],
                "scope": "validation, encoding and buffer drop only; no CRC, WAL I/O or database throughput"
            })
        );
    }
}

#[cfg(target_os = "linux")]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;
