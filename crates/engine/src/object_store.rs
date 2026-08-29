//! Object storage: the source of truth for flushed bulk data (DESIGN §6.5).
//!
//! # What belongs here, and what must never
//!
//! Object storage holds **immutable, write-once** objects: SSTs, snapshots, and later the
//! `META_REGION_0` anchor. It does **not** hold anything on the acknowledgement path of
//! consensus — HardState (the vote), raft log entries, or the ConfState pairing record all
//! require a local fsync *before* a reply leaves the node, and an object store is a network
//! round-trip with eventual-consistency semantics. It satisfies neither requirement. A write
//! is acked at raft-majority acceptance and drains here afterwards; the local tail is the
//! staging buffer, never a competing truth.
//!
//! Nor does it hold the **manifest**. Each region's manifest — the file-id list and LSM
//! structure — is the one piece of *mutable* state, and it lives in raft-replicated region
//! state precisely so that object storage sees only immutable creates and deletes. That
//! split is what lets kv9 sidestep object-store consistency weaknesses instead of fighting
//! them: the mutable pointer is raft-committed, and the objects it points at never change
//! under it. Putting the manifest here would re-import the entire problem.
//!
//! # Why this trait has no update operation
//!
//! There is no `overwrite`, no `append`, no `put_if_match`. Objects are written once under a
//! unique file-id and thereafter only read or deleted. The absence is the design: an API that
//! *cannot* express an in-place update cannot be talked into one later by someone who has not
//! read DESIGN §6.5.
//!
//! [`ObjectStore::put`] is nonetheless **idempotent for identical content**, because
//! retransmission is a normal event — an upload may be retried after a timeout, or reissued
//! by a node that has since lost leadership. Re-putting the same bytes under the same key is
//! therefore not an error. Putting *different* bytes under a key that already exists is a
//! different matter entirely: it means two distinct objects were assigned one file-id, which
//! breaks the immutability the manifest depends on. That is rejected with
//! [`Error::ObjectContentMismatch`] rather than silently accepted, because silently accepting
//! it would corrupt data that a committed manifest already points at.
//!
//! [`ObjectStore::delete`] is idempotent for the symmetric reason. A physical delete may
//! legitimately arrive late — after the deciding node has been deposed, or twice from a
//! retried GC pass — and a late delete of an already-deleted object is a no-op, not a fault.

use std::collections::BTreeMap;
use std::sync::Mutex;

use kv9_common::{Error, Result};

/// The name of one immutable object.
///
/// A newtype rather than a bare `String` so that a key cannot be built by accident from
/// whatever string happens to be in scope. Construction is deliberately explicit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectKey(String);

impl ObjectKey {
    /// Build a key. Rejects the empty name, which no backend can address.
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        if name.is_empty() {
            return Err(Error::Config("object key must not be empty".into()));
        }
        Ok(ObjectKey(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ObjectKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A store of immutable objects (DESIGN §6.5).
///
/// `Send + Sync` because the drain worker and reads run on different threads.
pub trait ObjectStore: Send + Sync + std::fmt::Debug {
    /// Write an object.
    ///
    /// Idempotent when `bytes` matches what is already stored under `key`. Returns
    /// [`Error::ObjectContentMismatch`] when `key` exists with *different* content — see the
    /// module docs for why that is refused rather than overwritten.
    fn put(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()>;

    /// Read an object. `Ok(None)` means it is not present, which is a normal answer (it may
    /// have been collected), not an error.
    fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>>;

    /// Remove an object. Idempotent: deleting an absent object is `Ok(())`, because a late or
    /// repeated delete is expected rather than exceptional.
    fn delete(&self, key: &ObjectKey) -> Result<()>;

    /// Keys beginning with `prefix`, in sorted order.
    ///
    /// Sorted so that callers (leak scans, in particular) get a deterministic sequence
    /// instead of one that varies with backend internals and hides ordering bugs in tests.
    fn list(&self, prefix: &str) -> Result<Vec<ObjectKey>>;
}

/// An in-memory [`ObjectStore`], for tests and for running kv9 without object storage.
///
/// Deliberately the first backend: it pins the semantics above with no network, no
/// credentials, and no MinIO, so the trait's contract can be tested before a real backend
/// exists to blur it.
///
/// `BTreeMap` rather than `HashMap` so `list` is sorted by construction — see [`ObjectStore::list`].
#[derive(Debug, Default)]
pub struct MemoryObjectStore {
    objects: Mutex<BTreeMap<ObjectKey, Vec<u8>>>,
}

impl MemoryObjectStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of objects held. Test/diagnostic use.
    pub fn len(&self) -> usize {
        self.objects
            .lock()
            .expect("object store mutex poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ObjectStore for MemoryObjectStore {
    fn put(&self, key: &ObjectKey, bytes: &[u8]) -> Result<()> {
        let mut objects = self.objects.lock().expect("object store mutex poisoned");
        match objects.get(key) {
            // Retransmission of identical content: expected, not an error.
            Some(existing) if existing == bytes => Ok(()),
            // Two different objects under one file-id. Refusing is the whole point; see the
            // module docs. The key is named because it is a file-id, not user data.
            Some(_) => Err(Error::ObjectContentMismatch {
                key: key.to_string(),
            }),
            None => {
                objects.insert(key.clone(), bytes.to_vec());
                Ok(())
            }
        }
    }

    fn get(&self, key: &ObjectKey) -> Result<Option<Vec<u8>>> {
        let objects = self.objects.lock().expect("object store mutex poisoned");
        Ok(objects.get(key).cloned())
    }

    fn delete(&self, key: &ObjectKey) -> Result<()> {
        let mut objects = self.objects.lock().expect("object store mutex poisoned");
        objects.remove(key);
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<ObjectKey>> {
        let objects = self.objects.lock().expect("object store mutex poisoned");
        Ok(objects
            .keys()
            .filter(|k| k.as_str().starts_with(prefix))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> ObjectKey {
        ObjectKey::new(s).expect("non-empty test key")
    }

    #[test]
    fn put_then_get_returns_the_bytes() {
        let store = MemoryObjectStore::new();
        store.put(&key("sst/000001"), b"payload").unwrap();
        assert_eq!(
            store.get(&key("sst/000001")).unwrap().as_deref(),
            Some(&b"payload"[..])
        );
    }

    #[test]
    fn absent_object_reads_as_none_not_error() {
        // A collected object is a normal answer. If this were an error, every leak scan and
        // every racing reader would have to distinguish "gone" from "broken".
        let store = MemoryObjectStore::new();
        assert!(store.get(&key("sst/nope")).unwrap().is_none());
    }

    #[test]
    fn reput_of_identical_content_is_idempotent() {
        // Upload retried after a timeout, or reissued by a node that has lost leadership.
        let store = MemoryObjectStore::new();
        store.put(&key("sst/000001"), b"payload").unwrap();
        store.put(&key("sst/000001"), b"payload").unwrap();
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn reput_with_different_content_is_refused() {
        // Two distinct objects assigned one file-id. Accepting the second would overwrite
        // bytes a committed manifest already points at.
        let store = MemoryObjectStore::new();
        store.put(&key("sst/000001"), b"original").unwrap();
        let err = store.put(&key("sst/000001"), b"different").unwrap_err();
        assert!(
            matches!(err, Error::ObjectContentMismatch { .. }),
            "expected a typed mismatch, got {err:?}"
        );
        // And the original must survive: the refusal is not a partial write.
        assert_eq!(
            store.get(&key("sst/000001")).unwrap().as_deref(),
            Some(&b"original"[..])
        );
    }

    #[test]
    fn delete_is_idempotent_and_absent_delete_is_ok() {
        // A late delete from a deposed leader, or a retried GC pass, must not fault.
        let store = MemoryObjectStore::new();
        store.put(&key("sst/000001"), b"payload").unwrap();
        store.delete(&key("sst/000001")).unwrap();
        store.delete(&key("sst/000001")).unwrap();
        store.delete(&key("sst/never-existed")).unwrap();
        assert!(store.is_empty());
    }

    #[test]
    fn deleted_key_may_be_reused_with_different_content() {
        // States what the store does, not what the layer above requires. Delete drops the
        // bytes, so the immutability check has nothing left to compare against and a
        // subsequent put of *different* content succeeds. That is the edge worth pinning:
        // single-use file-ids are a convention of the manifest layer, and this store cannot
        // enforce them alone -- a name implying it rejected mismatched content here would
        // describe a guarantee that does not exist.
        let store = MemoryObjectStore::new();
        store.put(&key("sst/000001"), b"original").unwrap();
        store.delete(&key("sst/000001")).unwrap();
        store.put(&key("sst/000001"), b"different").unwrap();
        assert_eq!(
            store.get(&key("sst/000001")).unwrap().as_deref(),
            Some(&b"different"[..])
        );
    }

    #[test]
    fn list_is_prefix_filtered_and_sorted() {
        let store = MemoryObjectStore::new();
        for k in ["sst/000003", "sst/000001", "snap/000002", "sst/000002"] {
            store.put(&key(k), b"x").unwrap();
        }
        let listed: Vec<String> = store
            .list("sst/")
            .unwrap()
            .iter()
            .map(|k| k.to_string())
            .collect();
        assert_eq!(listed, ["sst/000001", "sst/000002", "sst/000003"]);
    }

    #[test]
    fn empty_key_is_rejected_at_construction() {
        assert!(ObjectKey::new("").is_err());
    }

    #[test]
    fn object_store_is_object_safe() {
        // The drain worker will hold this behind a trait object; if that stops compiling the
        // failure should surface here and not in whatever wires it up months from now.
        let store: Box<dyn ObjectStore> = Box::new(MemoryObjectStore::new());
        store.put(&key("sst/000001"), b"payload").unwrap();
        assert_eq!(store.list("sst/").unwrap().len(), 1);
    }
}
