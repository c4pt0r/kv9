use super::*;
use crate::checkpoint::{object_name, FlushScope, SstReference};
use serde::ser::SerializeStruct;
use sha2::{Digest, Sha256};
use std::cell::Cell;

fn manifest() -> CheckpointManifest {
    let scope = FlushScope {
        cluster: "escaped\\\"\n雪".into(),
        region: u64::MAX,
        conf_ver: 1,
        version: 2,
    };
    let hash = "ab".repeat(32);
    let files = (0..3)
        .map(|cf| SstReference {
            key: object_name(&scope, &hash),
            sha256: hash.clone(),
            cf,
            smallest: (0..=255).collect(),
            largest: vec![255; 4],
            size: 4096,
            count: 2,
        })
        .collect();
    CheckpointManifest {
        scope,
        term: u64::MAX,
        index: u64::MAX,
        files,
    }
}

fn legacy_bytes(manifest: &CheckpointManifest) -> Vec<u8> {
    let mut bytes = MAGIC.to_vec();
    bytes.extend(serde_json::to_vec(manifest).unwrap());
    bytes
}

#[test]
fn large_manifest_enforces_object_closure_per_sst_and_canonical_version() {
    use crate::checkpoint::{MAX_CHECKPOINT_BYTES, MAX_STREAMED_SSTS};
    let mut image = manifest();
    let file = image.files[0].clone();
    image.files = (0..MAX_STREAMED_SSTS)
        .map(|i| SstReference {
            smallest: vec![i as u8],
            largest: vec![i as u8],
            size: MAX_CHECKPOINT_BYTES as u64,
            ..file.clone()
        })
        .collect();
    let bytes = image.encode().unwrap();
    assert!(bytes.starts_with(LARGE_MAGIC));
    assert_eq!(CheckpointManifest::decode(&bytes).unwrap(), image);
    assert!(CheckpointManifest::decode_legacy(&bytes).is_err());
    assert!(CheckpointManifest::decode(&legacy_bytes(&image)).is_err());
    image.files.push(SstReference {
        smallest: vec![MAX_STREAMED_SSTS as u8],
        largest: vec![MAX_STREAMED_SSTS as u8],
        ..file
    });
    assert!(CheckpointManifest::decode(&image.encode().unwrap()).is_err());
    image.files.pop();
    image.files[0].size += 1;
    assert!(CheckpointManifest::decode(&image.encode().unwrap()).is_err());
    image.files.truncate(1);
    image.files[0].size = MAX_CHECKPOINT_BYTES as u64;
    let small = image.encode().unwrap();
    assert!(small.starts_with(MAGIC));
    assert_eq!(CheckpointManifest::decode_legacy(&small).unwrap(), image);
    let noncanonical = encode_frame(&image, MAX_MANIFEST_BYTES, LARGE_MAGIC).unwrap();
    assert!(CheckpointManifest::decode(&noncanonical).is_err());
    image.files[0].size = u64::MAX;
    image.files.push(image.files[0].clone());
    assert!(image.encode().is_err());
}

#[test]
fn streaming_encoding_preserves_bytes_decode_and_change_identity() {
    let manifest = manifest();
    let old = legacy_bytes(&manifest);
    assert_eq!(manifest.encode().unwrap(), old);
    assert_eq!(CheckpointManifest::decode(&old).unwrap(), manifest);
    for generation in [0, 1, u64::MAX] {
        let mut hash = Sha256::new();
        hash.update(b"kv9-manifest-v1");
        hash.update(generation.to_be_bytes());
        hash.update(&old);
        assert_eq!(
            manifest.change_id(generation).unwrap(),
            hash.finalize().to_vec()
        );
    }
}

#[test]
fn complete_frame_limit_counts_magic_and_accepts_exact_boundary() {
    let mut manifest = manifest();
    manifest.files.truncate(1);
    let mut base = legacy_bytes(&manifest).len();
    // The cluster occurs in scope and in one object name. Adjust a numeric
    // field's width to make the remaining space an even number of ASCII bytes.
    if !(MAX_MANIFEST_BYTES - base).is_multiple_of(2) {
        manifest.scope.version = 10;
        base += 1;
    }
    let padding = (MAX_MANIFEST_BYTES - base) / 2;
    manifest.scope.cluster.push_str(&"a".repeat(padding));
    manifest.files[0].key = object_name(&manifest.scope, &manifest.files[0].sha256);
    let exact = manifest.encode().unwrap();
    assert_eq!(exact.len(), MAX_MANIFEST_BYTES);
    assert_eq!(exact, legacy_bytes(&manifest));
    assert_eq!(CheckpointManifest::decode(&exact).unwrap(), manifest);
    assert!(encode_with_limit(&manifest, exact.len() - 1).is_err());
    assert_eq!(
        encode_with_limit(&manifest, exact.len() + 1).unwrap(),
        exact
    );
    manifest.scope.conf_ver = 10; // One extra encoded byte.
    assert!(manifest
        .encode()
        .unwrap_err()
        .to_string()
        .contains("too large"));
}

#[test]
fn output_capacity_stays_bounded_and_rejected_chunks_do_not_allocate_or_append() {
    let mut output = BoundedOutput::new(1023);
    for width in [1, 2, 4, 8, 16, 32, 64, 128, 256, 512] {
        output.write_all(&vec![b'x'; width]).unwrap();
        assert!(output.bytes.capacity() <= output.limit);
    }
    assert_eq!(output.bytes.len(), output.limit);
    let capacity = output.bytes.capacity();
    let ptr = output.bytes.as_ptr();
    assert!(output.write_all(&[b'y'; 2048]).is_err());
    assert_eq!(output.bytes.len(), 1023);
    assert_eq!(output.bytes.capacity(), capacity);
    assert_eq!(output.bytes.as_ptr(), ptr);
    assert!(output.bytes.iter().all(|b| *b == b'x'));
}

#[test]
fn oversized_field_stops_serialization_before_visiting_later_fields() {
    struct Probe {
        oversized: String,
        visited_later: Cell<bool>,
    }
    impl Serialize for Probe {
        fn serialize<S: serde::Serializer>(
            &self,
            serializer: S,
        ) -> std::result::Result<S::Ok, S::Error> {
            let mut fields = serializer.serialize_struct("Probe", 2)?;
            fields.serialize_field("oversized", &self.oversized)?;
            self.visited_later.set(true);
            fields.serialize_field("later", &42)?;
            fields.end()
        }
    }
    let probe = Probe {
        oversized: "x".repeat(MAX_MANIFEST_BYTES + 1),
        visited_later: Cell::new(false),
    };
    assert!(encode_with_limit(&probe, MAX_MANIFEST_BYTES).is_err());
    assert!(!probe.visited_later.get());
}
