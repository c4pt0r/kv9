//! Compose an actual local engine recovery into a portable anchor descriptor.
//!
//! Only this function owns the base observation and completed publication from
//! the same open. A decoded descriptor is never accepted instead. The resulting
//! local observation does not certify a destination, retained transfer bundle,
//! future epoch transition chain or replicated retention owner.
use kv9_common::anchor::{AnchorDigest, RecoveryAnchor};
use kv9_common::{AppliedPosition, Error, Result, RootDescriptor, META_REGION_0};
use kv9_engine::checkpoint::{CheckpointManifest, RemoteUploader};
use kv9_engine::{DurableAppliedPosition, EngineReplay, ReplicatedEngine, WalEngine};
use kv9_raft::storage::DiskRaftStorage;

/// Minted only after the complete matching local recovery below. Its descriptor
/// may be serialized/copied, but decoding cannot recreate this observation.
/// ```compile_fail
/// fn forge(descriptor: kv9_common::anchor::RecoveryAnchor) {
///     let _ = kv9_meta::recovery::LocalRecoveryAnchor {
///         descriptor, encoded: vec![],
///         recovered_through: kv9_common::AppliedPosition { term: 1, index: 1 },
///     };
/// }
/// ```
#[derive(Debug)]
pub struct LocalRecoveryAnchor {
    descriptor: RecoveryAnchor,
    encoded: Vec<u8>,
    recovered_through: AppliedPosition,
}
impl LocalRecoveryAnchor {
    pub fn descriptor(&self) -> &RecoveryAnchor {
        &self.descriptor
    }
    /// Local replay endpoint; it is not part of the portable image identity.
    pub fn recovered_through(&self) -> AppliedPosition {
        self.recovered_through
    }
    pub fn encoded(&self) -> &[u8] {
        &self.encoded
    }
    pub fn digest(&self) -> AnchorDigest {
        AnchorDigest::of(&self.encoded)
    }
}

/// Decode a structurally and semantically consistent descriptor for this root.
/// This is still untrusted data: no committed-history, retention, destination or
/// live engine authority is obtained by decoding, even when all digests match.
pub fn decode_initial_anchor(encoded: &[u8], root: &RootDescriptor) -> Result<RecoveryAnchor> {
    if root.voters.len() > kv9_common::anchor::MAX_ANCHOR_MEMBERS {
        return Err(invalid("expected root exceeds the anchor member bound"));
    }
    root.validate()?;
    let descriptor = RecoveryAnchor::decode(encoded).map_err(|e| invalid(&e.to_string()))?;
    if &descriptor.root != root {
        return Err(invalid("descriptor belongs to another root"));
    }
    let manifest = CheckpointManifest::decode(&descriptor.manifest)?;
    if manifest.encode()? != descriptor.manifest
        || manifest.scope.cluster != root.cluster_id.to_string()
        || manifest.scope.region != META_REGION_0.0
        || manifest.scope.conf_ver != descriptor.conf_ver
        || manifest.scope.version != descriptor.version
        || manifest.position() != descriptor.image_cut
        || manifest.change_id(descriptor.generation - 1)?.as_slice() != descriptor.change_id
    {
        return Err(invalid(
            "descriptor and nested manifest do not identify the same image/publication",
        ));
    }
    Ok(descriptor)
}

pub fn open_initial_checkpoint_engine(
    storage: &mut DiskRaftStorage,
    path: impl AsRef<std::path::Path>,
    uploader: Option<&RemoteUploader>,
    root: &RootDescriptor,
) -> Result<(WalEngine, EngineReplay, Option<LocalRecoveryAnchor>)> {
    let mut base = None;
    let (engine, report, publication) =
        kv9_raft::state_machine::checkpoint_recovery::open_checkpoint_engine_with_base(
            storage,
            path,
            uploader,
            root.cluster_id.to_string(),
            META_REGION_0.0,
            |manifest, view| {
                let observed = crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
                observed.check_manifest(manifest)?;
                if base.replace(observed).is_some() {
                    return Err(invalid("base observed more than once"));
                }
                Ok(())
            },
        )?;
    let anchor = match (base, publication) {
        (None, None) => None,
        (Some(base), Some(publication)) => {
            let manifest = publication.manifest();
            let configuration = publication.configuration();
            base.check_manifest(manifest)?;
            if base.root_digest() != root.digest() || configuration.cut() != manifest.position() {
                return Err(invalid("root or configuration belongs to another image"));
            }
            let DurableAppliedPosition::AppliedThrough(recovered_through) =
                engine.applied_position()?
            else {
                return Err(invalid("recovered engine has no durable applied position"));
            };
            let published = publication.publication();
            if recovered_through.index < published.index
                || recovered_through.term < published.term
                || (recovered_through.index == published.index
                    && recovered_through.term != published.term)
            {
                return Err(invalid(
                    "local replay does not cover the winning publication",
                ));
            }
            let predecessor = publication
                .generation()
                .checked_sub(1)
                .ok_or_else(|| invalid("zero publication generation"))?;
            let change_id = manifest
                .change_id(predecessor)?
                .try_into()
                .map_err(|_| invalid("manifest change identity has the wrong size"))?;
            let descriptor = RecoveryAnchor {
                root: root.clone(),
                conf_ver: manifest.scope.conf_ver,
                version: manifest.scope.version,
                image_cut: manifest.position(),
                publication: publication.publication(),
                generation: publication.generation(),
                change_id,
                configuration_at: configuration.applied_at(),
                configuration: configuration.anchor_state()?,
                manifest: manifest.encode()?,
            };
            let encoded = descriptor.encode().map_err(|e| invalid(&e.to_string()))?;
            let decoded = decode_initial_anchor(&encoded, root)?;
            if decoded != descriptor {
                return Err(invalid("anchor codec changed the recovered descriptor"));
            }
            Some(LocalRecoveryAnchor {
                descriptor,
                encoded,
                recovered_through,
            })
        }
        _ => return Err(invalid("base and publication observations do not match")),
    };
    Ok((engine, report, anchor))
}

fn invalid(reason: &str) -> Error {
    Error::Engine(format!("recovery anchor: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::anchor::AnchorConfiguration;
    use kv9_common::{
        AppliedPosition, BootstrapGeneration, ClusterId, NodeId, RootVoter, StoreIncarnation,
    };
    use kv9_engine::checkpoint::{FlushScope, SstReference};

    fn fixture() -> RecoveryAnchor {
        let root = RootDescriptor::new(
            ClusterId::from_bytes([1; 16]),
            BootstrapGeneration::from_bytes([2; 16]),
            vec![RootVoter {
                node_id: NodeId(1),
                addr: "127.0.0.1:20160".parse().unwrap(),
                store_incarnation: StoreIncarnation::from_bytes([3; 16]),
            }],
            b"anchor-component-credential",
        )
        .unwrap();
        let hash = "0".repeat(64);
        let manifest = CheckpointManifest {
            scope: FlushScope {
                cluster: root.cluster_id.to_string(),
                region: META_REGION_0.0,
                conf_ver: 1,
                version: 1,
            },
            term: 1,
            index: 2,
            files: vec![SstReference {
                key: format!("clusters/{}/regions/1/sst/{hash}", root.cluster_id),
                sha256: hash,
                cf: 0,
                smallest: vec![1],
                largest: vec![1],
                size: 1,
                count: 1,
            }],
        };
        RecoveryAnchor {
            root,
            conf_ver: 1,
            version: 1,
            image_cut: manifest.position(),
            publication: AppliedPosition { term: 1, index: 4 },
            generation: 1,
            change_id: manifest.change_id(0).unwrap().try_into().unwrap(),
            configuration_at: None,
            configuration: AnchorConfiguration {
                voters: vec![1],
                voters_outgoing: vec![],
                learners: vec![],
                learners_next: vec![],
                auto_leave: false,
            },
            manifest: manifest.encode().unwrap(),
        }
    }

    #[test]
    fn outer_checksum_does_not_hide_conflicting_nested_identity() {
        let original = fixture();
        assert_eq!(
            decode_initial_anchor(&original.encode().unwrap(), &original.root).unwrap(),
            original
        );
        for field in 0..5 {
            let mut wrong = original.clone();
            match field {
                0 => wrong.conf_ver += 1,
                1 => wrong.version += 1,
                2 => wrong.image_cut.index += 1,
                3 => wrong.generation += 1,
                _ => wrong.change_id[0] ^= 1,
            }
            assert!(decode_initial_anchor(&wrong.encode().unwrap(), &original.root).is_err());
        }
        let mut other = original.root.clone();
        other.bootstrap_generation = BootstrapGeneration::from_bytes([9; 16]);
        assert!(decode_initial_anchor(&original.encode().unwrap(), &other).is_err());
    }

    #[test]
    fn nested_manifest_must_be_canonical_and_owned_by_the_initial_engine() {
        let original = fixture();
        let mut wrong = original.clone();
        // The old manifest JSON reader permits unknown fields. The new envelope
        // does not let a discarded field acquire the same canonical identity.
        let end = wrong.manifest.pop().unwrap();
        assert_eq!(end, b'}');
        wrong
            .manifest
            .extend_from_slice(b",\"future_required_authority\":true}");
        assert!(CheckpointManifest::decode(&wrong.manifest).is_ok());
        assert!(decode_initial_anchor(&wrong.encode().unwrap(), &original.root).is_err());
        let mut wrong = original.clone();
        let mut manifest = CheckpointManifest::decode(&wrong.manifest).unwrap();
        manifest.scope.region = 2;
        manifest.files[0].key = manifest.files[0].key.replace("/regions/1/", "/regions/2/");
        wrong.manifest = manifest.encode().unwrap();
        wrong.change_id = manifest.change_id(0).unwrap().try_into().unwrap();
        assert!(decode_initial_anchor(&wrong.encode().unwrap(), &original.root).is_err());
    }

    #[test]
    fn decoded_description_does_not_certify_object_or_publication_presence() {
        let mut descriptor = fixture();
        // No object store or Raft log exists in this component test. A valid
        // checksum is data integrity, never evidence that publication100 won.
        descriptor.publication.index = 100;
        let decoded: RecoveryAnchor =
            decode_initial_anchor(&descriptor.encode().unwrap(), &descriptor.root).unwrap();
        assert_eq!(decoded.publication.index, 100);
        // The separate compile-fail example prevents callers from constructing
        // LocalRecoveryAnchor out of these freely forged descriptor fields.
    }
}
