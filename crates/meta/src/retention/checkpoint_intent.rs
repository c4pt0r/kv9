//! Immutable checkpoint descriptor and predecessor, discoverable by Pending
//! owner after its local journal is gone. This is data, not a PreparedFlush or
//! a certificate that an attempt won/lost, publishers closed, or objects exist.
use super::*;
use kv9_engine::checkpoint::{CheckpointManifest, MAX_MANIFEST_BYTES};

const MAGIC: &[u8; 8] = b"KV9CPI02";
const LEGACY_MAGIC: &[u8; 8] = b"KV9CPI01";
pub const MAX_INTENT_BYTES: usize = MAX_MANIFEST_BYTES + 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointIntent {
    root: RootDigest,
    pending: OwnerId,
    predecessor: u64,
    manifest: Vec<u8>,
    gated: bool,
}
impl CheckpointIntent {
    pub fn new(
        root: &RootDescriptor,
        manifest: &CheckpointManifest,
        predecessor: u64,
    ) -> Result<Self> {
        let manifest = CheckpointManifest::decode(&manifest.encode()?)?;
        if manifest.scope.cluster != root.cluster_id.to_string() {
            return Err(invalid("checkpoint intent cluster differs"));
        }
        Self::from_bytes(root.digest(), manifest.encode()?, predecessor)
    }
    fn from_bytes(root: RootDigest, bytes: Vec<u8>, predecessor: u64) -> Result<Self> {
        let manifest = CheckpointManifest::decode(&bytes)?;
        if manifest.encode()? != bytes {
            return Err(invalid("checkpoint intent descriptor is not canonical"));
        }
        if root.as_bytes() == &[0; 32]
            || (manifest.scope.region != kv9_common::META_REGION_0.0 && manifest.scope.region < 100)
        {
            return Err(invalid("checkpoint intent root/scope differs"));
        }
        let operation = manifest.change_id(predecessor)?;
        let pending = owner_id(root, &operation, b"kv9-checkpoint-pending-v1")?;
        let intent = Self {
            root,
            pending,
            predecessor,
            manifest: bytes,
            gated: true,
        };
        intent.binding(OwnerKind::Pending)?.validate(root)?;
        Ok(intent)
    }
    pub fn root(&self) -> RootDigest {
        self.root
    }
    pub fn pending_id(&self) -> OwnerId {
        self.pending
    }
    pub fn predecessor(&self) -> u64 {
        self.predecessor
    }
    pub fn manifest(&self) -> Result<CheckpointManifest> {
        CheckpointManifest::decode(&self.manifest)
    }
    pub fn manifest_bytes(&self) -> &[u8] {
        &self.manifest
    }
    pub fn binding(&self, kind: OwnerKind) -> Result<OwnerBinding> {
        let label = match kind {
            OwnerKind::Pending => b"kv9-checkpoint-pending-v1".as_slice(),
            OwnerKind::Version => b"kv9-checkpoint-version-v1".as_slice(),
            _ => return Err(invalid("checkpoint intent owner kind")),
        };
        let manifest = self.manifest()?;
        let operation: [u8; 32] = manifest.change_id(self.predecessor)?.try_into().unwrap();
        let binding = OwnerBinding {
            descriptor: OwnerDescriptor {
                root: self.root,
                id: owner_id(self.root, &operation, label)?,
                kind,
                region: manifest.scope.region,
                conf_ver: manifest.scope.conf_ver,
                version: manifest.scope.version,
                operation,
                subject: *RootDigest::sha256(&self.manifest).as_bytes(),
                subject_is_anchor: false,
                local: None,
            },
            generation: NonZeroU64::new(1).unwrap(),
            resources: manifest_resources(self.root, &manifest)?,
        };
        binding.validate(self.root)?;
        Ok(binding)
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = if self.gated { MAGIC } else { LEGACY_MAGIC }.to_vec();
        bytes.extend_from_slice(self.root.as_bytes());
        bytes.extend_from_slice(self.pending.as_bytes());
        uint(&mut bytes, self.predecessor);
        bytes.extend_from_slice(&(self.manifest.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&self.manifest);
        seal(bytes)
    }
    pub(super) fn decode_request(bytes: &[u8], root: RootDigest) -> Result<Self> {
        let gated = bytes.starts_with(MAGIC);
        let mut r = Reader(checked(
            bytes,
            if gated { MAGIC } else { LEGACY_MAGIC },
            MAX_INTENT_BYTES,
        )?);
        if r.array::<32>()? != *root.as_bytes() {
            return Err(invalid("checkpoint intent root differs"));
        }
        let owner = OwnerId::new(r.array()?).map_err(pin_error)?;
        let predecessor = r.uint()?;
        let len = u32::from_be_bytes(r.array()?) as usize;
        if len > MAX_MANIFEST_BYTES || len != r.0.len() {
            return Err(invalid("checkpoint intent descriptor length"));
        }
        let mut intent = Self::from_bytes(root, r.take(len)?.to_vec(), predecessor)?;
        intent.gated = gated;
        if intent.pending != owner || intent.encode() != bytes {
            return Err(invalid(
                "checkpoint intent identity or canonical bytes differ",
            ));
        }
        Ok(intent)
    }
    pub fn decode(bytes: &[u8], root: &RootDescriptor, pending: OwnerId) -> Result<Self> {
        let intent = Self::decode_request(bytes, root.digest())?;
        if intent.pending != pending
            || intent.manifest()?.scope.cluster != root.cluster_id.to_string()
        {
            return Err(invalid("checkpoint intent observation binding differs"));
        }
        Ok(intent)
    }
}

pub(super) fn owner_id(root: RootDigest, operation: &[u8], label: &[u8]) -> Result<OwnerId> {
    let mut bytes = label.to_vec();
    bytes.extend_from_slice(root.as_bytes());
    bytes.extend_from_slice(operation);
    OwnerId::new(
        RootDigest::sha256(&bytes).as_bytes()[..16]
            .try_into()
            .unwrap(),
    )
    .map_err(pin_error)
}

/// The full canonical SST closure, shared by ledger validation and the worker.
pub fn manifest_resources(
    root: RootDigest,
    manifest: &CheckpointManifest,
) -> Result<Vec<ResourceIdentity>> {
    // Validate the public descriptor before indexing any digest bytes.
    let manifest = CheckpointManifest::decode(&manifest.encode()?)?;
    let mut resources = Vec::with_capacity(manifest.files.len());
    for file in &manifest.files {
        let mut content = [0; 32];
        for (n, pair) in file.sha256.as_bytes().chunks_exact(2).enumerate() {
            let nibble = |c: u8| {
                if c.is_ascii_digit() {
                    c - b'0'
                } else {
                    c - b'a' + 10
                }
            };
            content[n] = nibble(pair[0]) * 16 + nibble(pair[1]);
        }
        resources.push(
            ResourceIdentity::new(
                root,
                ResourceKind::Sst,
                RootDigest::sha256(file.key.as_bytes()).as_bytes()[..16]
                    .try_into()
                    .unwrap(),
                content,
            )
            .map_err(pin_error)?,
        );
    }
    resources.sort_by_key(|r| *r.instance());
    validate_resources(&resources, root)?;
    Ok(resources)
}

pub(super) fn intent_key(owner: OwnerId) -> Vec<u8> {
    key(8, owner.as_bytes())
}

pub(super) fn check_owners(p: &Planner<'_>, intent: &CheckpointIntent) -> Result<()> {
    let pending = intent.binding(OwnerKind::Pending)?;
    let owner = p.exact_owner(pending.token())?;
    if owner.binding != pending {
        return Err(invalid("checkpoint intent pending binding differs"));
    }
    if owner.phase == PinPhase::Published {
        return Ok(());
    }
    if !matches!(owner.phase, PinPhase::Quiesced | PinPhase::Released) {
        return Err(conflict("checkpoint intent lacks published protection"));
    }
    let version = intent.binding(OwnerKind::Version)?;
    let owner = p.exact_owner(version.token())?;
    if owner.binding != version || owner.phase != PinPhase::Published {
        return Err(conflict("checkpoint intent lacks exact published version"));
    }
    Ok(())
}
pub(super) fn record(
    p: &mut Planner<'_>,
    root: &RootDescriptor,
    intent: &CheckpointIntent,
) -> Result<()> {
    check_owners(p, intent)?;
    persist(p, root, intent)
}
/// The caller must have validated complete Published protection, either from
/// its established view or as part of the same atomic Begin effect.
pub(super) fn persist(
    p: &mut Planner<'_>,
    root: &RootDescriptor,
    intent: &CheckpointIntent,
) -> Result<()> {
    let mut intent = intent.clone();
    intent.gated = true;
    let bytes = intent.encode();
    CheckpointIntent::decode(&bytes, root, intent.pending)?;
    let key = intent_key(intent.pending);
    match p.view.get(ColumnFamily::Default, &key)? {
        Some(old) => {
            let mut prior = CheckpointIntent::decode(&old, root, intent.pending)?;
            let gated = prior.gated;
            prior.gated = true;
            if prior != intent {
                return Err(invalid("immutable checkpoint intent differs or is corrupt"));
            }
            if gated {
                checkpoint_admission::load(p, &intent)?;
            } else {
                checkpoint_admission::create(p, &intent)?;
                p.writes.insert(key, bytes);
            }
            Ok(())
        }
        None => {
            checkpoint_admission::create(p, &intent)?;
            p.writes.insert(key, bytes);
            Ok(())
        }
    }
}

/// One established metadata snapshot, with all owner/resource cross-links
/// checked. Absence is an observation, never a negative settlement certificate.
pub fn checkpoint_intent(
    view: &dyn ReadView,
    root: &RootDescriptor,
    pending: OwnerId,
) -> Result<Option<CheckpointIntent>> {
    crate::checkpoint::inspect_initial_checkpoint_base(view, root)?;
    let digest = root.digest();
    let header = view
        .get(ColumnFamily::Default, &key(1, &[]))?
        .ok_or_else(|| invalid("ledger header missing"))?;
    let (revision, rows_enabled) = read_header(&header, digest)?;
    let Some(bytes) = view.get(ColumnFamily::Default, &intent_key(pending))? else {
        return Ok(None);
    };
    let intent = CheckpointIntent::decode(&bytes, root, pending)?;
    let p = Planner {
        view,
        root: digest,
        revision,
        rows_enabled,
        writes: BTreeMap::new(),
    };
    check_owners(&p, &intent)?;
    if intent.gated {
        checkpoint_admission::load(&p, &intent)?;
    }
    Ok(Some(intent))
}

pub(super) fn load(p: &Planner<'_>, pending: OwnerId) -> Result<Option<CheckpointIntent>> {
    let Some(intent) = retained(p, pending)? else {
        return Ok(None);
    };
    check_owners(p, &intent)?;
    Ok(Some(intent))
}

/// Canonical historical descriptor only. It grants no current Published
/// coverage and must not authorize entry or a first terminal transition.
pub(super) fn retained(p: &Planner<'_>, pending: OwnerId) -> Result<Option<CheckpointIntent>> {
    let Some(bytes) = p.view.get(ColumnFamily::Default, &intent_key(pending))? else {
        return Ok(None);
    };
    let intent = CheckpointIntent::decode_request(&bytes, p.root)?;
    if intent.pending != pending || !intent.gated {
        return Err(invalid("checkpoint admission requires gated intent"));
    }
    Ok(Some(intent))
}

#[cfg(test)]
pub(super) mod tests;
