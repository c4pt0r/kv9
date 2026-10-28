use super::*;
use crate::checkpoint::upload_planned;
use crate::MemoryObjectStore;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

// Independent reference for the already persisted format, without calling the
// streaming writer. Includes the original v1/v2 hash coverage and endianness.
fn legacy_bytes(
    generation: u64,
    manifest: &CheckpointManifest,
    objects: Option<&[Vec<u8>]>,
) -> Vec<u8> {
    let mut bytes = if objects.is_some() {
        PLANNED_MAGIC
    } else {
        MAGIC
    }
    .to_vec();
    bytes.extend(generation.to_le_bytes());
    let encoded = manifest.encode().unwrap();
    if objects.is_some() {
        bytes.extend((encoded.len() as u32).to_le_bytes());
    }
    bytes.extend(encoded);
    if let Some(objects) = objects {
        for object in objects {
            bytes.extend(object);
        }
    }
    bytes.extend(Sha256::digest(&bytes));
    bytes
}

#[test]
fn streaming_disk_codec_preserves_existing_v1_v2_bytes_and_identity() {
    let (dir, engine, plan) = tests::fixture();
    let mut journal = FlushJournal::new(dir.join("pending"));
    let legacy = legacy_bytes(8, &plan.manifest, Some(&plan.objects));
    fs::write(dir.join("pending"), &legacy).unwrap();
    let recovered = journal.load().unwrap().unwrap();
    assert_eq!(recovered.objects.as_ref(), Some(&plan.objects));
    assert_eq!(recovered.generation, 8);
    assert_eq!(recovered.manifest, plan.manifest);
    journal.stage_planned(&plan, 8).unwrap();
    assert_eq!(fs::read(dir.join("pending")).unwrap(), legacy);
    journal.clear_settled().unwrap();
    journal.stage_planned(&plan, 8).unwrap();
    assert_eq!(fs::read(dir.join("pending")).unwrap(), legacy);

    let prepared =
        upload_planned(&MemoryObjectStore::new(), tests::recovered_plan(recovered)).unwrap();
    journal.stage(&prepared, 8).unwrap();
    let legacy = legacy_bytes(8, &plan.manifest, None);
    assert_eq!(fs::read(dir.join("pending")).unwrap(), legacy);
    fs::write(dir.join("pending"), legacy).unwrap();
    let recovered = journal.load().unwrap().unwrap();
    assert_eq!(recovered.generation, 8);
    assert_eq!(recovered.manifest, plan.manifest);
    assert!(!recovered.requires_upload());
    drop(engine);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streaming_writer_borrows_objects_and_handles_short_writes_and_failure() {
    struct Sink<'a> {
        output: Vec<u8>,
        objects: &'a [Vec<u8>],
        borrowed: usize,
        fail_after: Option<usize>,
    }
    impl Write for Sink<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.fail_after.is_some_and(|n| self.output.len() >= n) {
                return Err(std::io::Error::other("injected write failure"));
            }
            if self
                .objects
                .iter()
                .any(|o| o.as_ptr() == bytes.as_ptr() && o.len() == bytes.len())
            {
                self.borrowed += 1;
            }
            let n = bytes.len().min(7);
            self.output.extend_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let (dir, engine, plan) = tests::fixture();
    let pending = PendingRef {
        generation: 8,
        manifest: &plan.manifest,
        objects: Some(&plan.objects),
    };
    let manifest = pending.validate().unwrap();
    let mut sink = Sink {
        output: Vec::new(),
        objects: &plan.objects,
        borrowed: 0,
        fail_after: None,
    };
    pending.write_to(&mut sink, &manifest).unwrap();
    assert_eq!(sink.borrowed, plan.objects.len());
    assert_eq!(
        sink.output,
        legacy_bytes(8, &plan.manifest, Some(&plan.objects))
    );
    sink.output.clear();
    sink.fail_after = Some(MAGIC.len() + 8 + 4 + manifest.len() + 10);
    assert!(pending.write_to(&mut sink, &manifest).is_err());
    assert!(PendingFlush::decode(&sink.output).is_err());
    drop(engine);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streaming_reader_refuses_lengths_before_reading_and_requires_exact_eof() {
    struct NeverRead;
    impl Read for NeverRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("oversize file must be rejected before reading")
        }
    }
    assert!(PendingFlush::read_from(NeverRead, MAX_FILE_BYTES + 1).is_err());
    let (dir, engine, plan) = tests::fixture();
    let bytes = legacy_bytes(8, &plan.manifest, Some(&plan.objects));
    let mut grown = bytes.clone();
    grown.push(0);
    assert!(PendingFlush::read_from(grown.as_slice(), bytes.len() as u64).is_err());
    assert!(PendingFlush::read_from(&bytes[..bytes.len() - 1], bytes.len() as u64).is_err());
    // A fabricated long descriptor cannot make the reader allocate a body
    // beyond the captured file length, even before outer checksum verification.
    let mut manifest = plan.manifest.clone();
    manifest.files[0].size += 1024;
    let forged = legacy_bytes(8, &manifest, Some(&plan.objects));
    assert!(PendingFlush::decode(&forged).is_err());
    let mut corrupt = bytes;
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(PendingFlush::decode(&corrupt).is_err());
    drop(engine);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejected_plans_do_not_create_temporary_files_or_replace_durable_identity() {
    let (dir, engine, mut plan) = tests::fixture();
    let mut journal = FlushJournal::new(dir.join("pending"));
    journal.stage_planned(&plan, 8).unwrap();
    let original = fs::read(dir.join("pending")).unwrap();
    let object = plan.objects.pop().unwrap();
    assert!(journal.stage_planned(&plan, 8).is_err());
    plan.objects.push(object);
    plan.objects[0][0] ^= 1;
    assert!(journal.stage_planned(&plan, 8).is_err());
    plan.objects[0][0] ^= 1;
    assert!(journal.stage_planned(&plan, u64::MAX).is_err());
    assert_eq!(fs::read(dir.join("pending")).unwrap(), original);
    assert!(!dir.join("pending.tmp").exists());
    journal.clear_settled().unwrap();
    plan.objects.push(vec![1]);
    assert!(journal.stage_planned(&plan, 8).is_err());
    assert!(!dir.join("pending").exists());
    assert!(!dir.join("pending.tmp").exists());
    drop(engine);
    fs::remove_dir_all(dir).unwrap();
}
