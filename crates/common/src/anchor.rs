//! Bounded portable description of the initial whole-engine recovery anchor.
//!
//! This is data, never an install, serving or deletion capability. Version 1
//! requires retained local protocol history from index 1 and has no bound outer
//! retention ledger. A remote receiver must obtain and validate those missing
//! authorities separately. Destination store identity is deliberately absent.
use crate::{AppliedPosition, RootDescriptor, RootDigest};

const MAGIC: &[u8; 8] = b"KV9ANCH\0";
const VERSION: u16 = 1;
/// Whole initial engine, retained protocol history, and unbound retention.
/// Unknown bits and missing required bits both refuse this version.
const REQUIRED_CAPABILITIES: u64 = 0b111;
const HEADER: usize = 8 + 2 + 8 + 4;
pub const MAX_ANCHOR_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ANCHOR_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_ROOT_BYTES: usize = 64 * 1024;
pub const MAX_ANCHOR_MEMBERS: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AnchorError {
    #[error("unsupported recovery anchor version or required capabilities")]
    Unsupported,
    #[error("invalid recovery anchor length, count or encoding")]
    Format,
    #[error("recovery anchor checksum mismatch")]
    Checksum,
    #[error("invalid recovery anchor root descriptor")]
    Root,
    #[error("invalid recovery anchor positions or generation")]
    Position,
    #[error("invalid recovery anchor configuration")]
    Configuration,
    #[error("recovery anchor scope or manifest is invalid")]
    Scope,
}

/// Content identity of one complete anchor frame, distinct from a root digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnchorDigest([u8; 32]);
impl AnchorDigest {
    pub fn of(encoded: &[u8]) -> Self {
        Self(*RootDigest::sha256(encoded).as_bytes())
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}
impl std::fmt::Display for AnchorDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Complete configuration sets, including joint-consensus state. Each list is
/// canonical: strictly increasing nonzero node IDs, never silently deduplicated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorConfiguration {
    pub voters: Vec<u64>,
    pub voters_outgoing: Vec<u64>,
    pub learners: Vec<u64>,
    pub learners_next: Vec<u64>,
    pub auto_leave: bool,
}
impl AnchorConfiguration {
    fn validate(&self) -> Result<(), AnchorError> {
        for nodes in [
            &self.voters,
            &self.voters_outgoing,
            &self.learners,
            &self.learners_next,
        ] {
            if nodes.len() > MAX_ANCHOR_MEMBERS
                || nodes.first() == Some(&0)
                || nodes.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(AnchorError::Configuration);
            }
        }
        if self.voters.is_empty()
            || self.learners.iter().any(|n| {
                self.voters.binary_search(n).is_ok()
                    || self.voters_outgoing.binary_search(n).is_ok()
            })
            || self.learners_next.iter().any(|n| {
                self.voters_outgoing.binary_search(n).is_err()
                    || self.voters.binary_search(n).is_ok()
                    || self.learners.binary_search(n).is_ok()
            })
            || (self.voters_outgoing.is_empty()
                && (self.auto_leave || !self.learners_next.is_empty()))
        {
            return Err(AnchorError::Configuration);
        }
        Ok(())
    }
}

/// Serializable descriptor. Public fields intentionally confer no authority.
/// The image manifest identifies the complete SST closure and its hashes;
/// the enclosing metadata/engine layer validates its canonical codec and scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryAnchor {
    pub root: RootDescriptor,
    /// The initial whole engine under the root's metadata Raft group. Both
    /// physical range bounds are empty by definition, covering every keyspace.
    pub conf_ver: u64,
    pub version: u64,
    pub image_cut: AppliedPosition,
    pub publication: AppliedPosition,
    pub generation: u64,
    pub change_id: [u8; 32],
    /// None means the certified initial root configuration, not an invented 0/0.
    pub configuration_at: Option<AppliedPosition>,
    pub configuration: AnchorConfiguration,
    pub manifest: Vec<u8>,
}

impl RecoveryAnchor {
    pub fn root_digest(&self) -> RootDigest {
        self.root.digest()
    }

    /// SHA-256 of the exact canonical manifest, not a flat hash of user values.
    pub fn manifest_digest(&self) -> [u8; 32] {
        *RootDigest::sha256(&self.manifest).as_bytes()
    }

    fn validate(&self) -> Result<(), AnchorError> {
        if self.manifest.is_empty()
            || self.manifest.len() > MAX_ANCHOR_MANIFEST_BYTES
            || self.conf_ver == 0
            || self.version == 0
        {
            return Err(AnchorError::Scope);
        }
        // Bound before root validation/encoding allocates per-voter state.
        if self.root.voters.len() > MAX_ANCHOR_MEMBERS || self.root.validate().is_err() {
            return Err(AnchorError::Root);
        }
        self.configuration.validate()?;
        if !valid_position(self.image_cut)
            || !valid_position(self.publication)
            || self.generation == 0
            || self.publication.index <= self.image_cut.index
            || !covers(self.publication, self.image_cut)
        {
            return Err(AnchorError::Position);
        }
        match self.configuration_at {
            Some(at) if valid_position(at) && covers(self.image_cut, at) => {}
            Some(_) => return Err(AnchorError::Position),
            None => {
                let expected: Vec<_> = self.root.voters.iter().map(|v| v.node_id.0).collect();
                if self.configuration.voters != expected
                    || !self.configuration.voters_outgoing.is_empty()
                    || !self.configuration.learners.is_empty()
                    || !self.configuration.learners_next.is_empty()
                    || self.configuration.auto_leave
                {
                    return Err(AnchorError::Configuration);
                }
            }
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, AnchorError> {
        self.validate()?;
        let root = self.root.canonical_bytes();
        if root.len() > MAX_ROOT_BYTES {
            return Err(AnchorError::Root);
        }
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_be_bytes());
        out.extend_from_slice(&REQUIRED_CAPABILITIES.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // Body length, patched below.
        bytes(&mut out, &root);
        out.extend_from_slice(self.root_digest().as_bytes());
        for value in [self.conf_ver, self.version] {
            uint(&mut out, value);
        }
        for at in [self.image_cut, self.publication] {
            position(&mut out, at);
        }
        uint(&mut out, self.generation);
        out.extend_from_slice(&self.change_id);
        out.push(u8::from(self.configuration_at.is_some()));
        if let Some(at) = self.configuration_at {
            position(&mut out, at);
        }
        for nodes in [
            &self.configuration.voters,
            &self.configuration.voters_outgoing,
            &self.configuration.learners,
            &self.configuration.learners_next,
        ] {
            out.extend_from_slice(&(nodes.len() as u32).to_be_bytes());
            for node in nodes {
                uint(&mut out, *node);
            }
        }
        out.push(u8::from(self.configuration.auto_leave));
        bytes(&mut out, &self.manifest);
        out.extend_from_slice(&self.manifest_digest());
        if out.len() + 32 > MAX_ANCHOR_BYTES {
            return Err(AnchorError::Format);
        }
        let length = (out.len() - HEADER) as u32;
        out[HEADER - 4..HEADER].copy_from_slice(&length.to_be_bytes());
        let digest = RootDigest::sha256(&out);
        out.extend_from_slice(digest.as_bytes());
        Ok(out)
    }

    /// Check the complete outer frame before allocating variable-length fields.
    /// Nested root/list/manifest bounds are checked before their own allocations.
    pub fn decode(input: &[u8]) -> Result<Self, AnchorError> {
        if input.len() < HEADER + 32 || input.len() > MAX_ANCHOR_BYTES || &input[..8] != MAGIC {
            return Err(AnchorError::Format);
        }
        let mut header = Reader(&input[8..HEADER]);
        if u16::from_be_bytes(header.array()?) != VERSION || header.uint()? != REQUIRED_CAPABILITIES
        {
            return Err(AnchorError::Unsupported);
        }
        let length = header.count()?;
        if length != input.len() - HEADER - 32 {
            return Err(AnchorError::Format);
        }
        let end = input.len() - 32;
        if RootDigest::sha256(&input[..end]).as_bytes() != &input[end..] {
            return Err(AnchorError::Checksum);
        }
        let mut reader = Reader(&input[HEADER..end]);
        let root_bytes = reader.bytes(MAX_ROOT_BYTES)?;
        let root = RootDescriptor::decode(root_bytes).map_err(|_| AnchorError::Root)?;
        // Require the nested format's unique representation too.
        if root.canonical_bytes() != root_bytes {
            return Err(AnchorError::Root);
        }
        let expected_root = reader.array::<32>()?;
        if root.digest().as_bytes() != &expected_root {
            return Err(AnchorError::Root);
        }
        let conf_ver = reader.uint()?;
        let version = reader.uint()?;
        let image_cut = reader.position()?;
        let publication = reader.position()?;
        let generation = reader.uint()?;
        let change_id = reader.array()?;
        let configuration_at = if reader.boolean()? {
            Some(reader.position()?)
        } else {
            None
        };
        let configuration = AnchorConfiguration {
            voters: reader.nodes()?,
            voters_outgoing: reader.nodes()?,
            learners: reader.nodes()?,
            learners_next: reader.nodes()?,
            auto_leave: reader.boolean()?,
        };
        let manifest = reader.bytes(MAX_ANCHOR_MANIFEST_BYTES)?.to_vec();
        let expected_manifest = reader.array::<32>()?;
        if !reader.0.is_empty() {
            return Err(AnchorError::Format);
        }
        if RootDigest::sha256(&manifest).as_bytes() != &expected_manifest {
            return Err(AnchorError::Scope);
        }
        let anchor = Self {
            root,
            conf_ver,
            version,
            image_cut,
            publication,
            generation,
            change_id,
            configuration_at,
            configuration,
            manifest,
        };
        anchor.validate()?;
        Ok(anchor)
    }
}

fn valid_position(at: AppliedPosition) -> bool {
    at.term > 0 && at.index > 0 && at.index < u64::MAX
}
fn covers(new: AppliedPosition, old: AppliedPosition) -> bool {
    new.index >= old.index
        && new.term >= old.term
        && (new.index != old.index || new.term == old.term)
}
fn uint(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn position(out: &mut Vec<u8>, value: AppliedPosition) {
    uint(out, value.term);
    uint(out, value.index);
}
fn bytes(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], AnchorError> {
        if n > self.0.len() {
            return Err(AnchorError::Format);
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], AnchorError> {
        Ok(self.take(N)?.try_into().expect("checked length"))
    }
    fn uint(&mut self) -> Result<u64, AnchorError> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    fn count(&mut self) -> Result<usize, AnchorError> {
        Ok(u32::from_be_bytes(self.array()?) as usize)
    }
    fn boolean(&mut self) -> Result<bool, AnchorError> {
        match self.take(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(AnchorError::Format),
        }
    }
    fn position(&mut self) -> Result<AppliedPosition, AnchorError> {
        Ok(AppliedPosition {
            term: self.uint()?,
            index: self.uint()?,
        })
    }
    fn bytes(&mut self, max: usize) -> Result<&'a [u8], AnchorError> {
        let n = self.count()?;
        if n > max {
            return Err(AnchorError::Format);
        }
        self.take(n)
    }
    fn nodes(&mut self) -> Result<Vec<u64>, AnchorError> {
        let n = self.count()?;
        if n > MAX_ANCHOR_MEMBERS || n > self.0.len() / 8 {
            return Err(AnchorError::Format);
        }
        (0..n).map(|_| self.uint()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BootstrapGeneration, ClusterId, NodeId, RootVoter, StoreIncarnation};

    fn at(term: u64, index: u64) -> AppliedPosition {
        AppliedPosition { term, index }
    }
    fn fixture() -> RecoveryAnchor {
        RecoveryAnchor {
            root: RootDescriptor::new(
                ClusterId::from_bytes([1; 16]),
                BootstrapGeneration::from_bytes([2; 16]),
                (1..=3)
                    .map(|id| RootVoter {
                        node_id: NodeId(id),
                        addr: format!("127.0.0.1:{}", 21000 + id).parse().unwrap(),
                        store_incarnation: StoreIncarnation::from_bytes([id as u8; 16]),
                    })
                    .collect(),
                b"anchor-unit-credential",
            )
            .unwrap(),
            conf_ver: 2,
            version: 1,
            image_cut: at(2, 5),
            publication: at(3, 7),
            generation: 3,
            change_id: [4; 32],
            configuration_at: Some(at(2, 4)),
            configuration: AnchorConfiguration {
                voters: vec![1, 2, 4],
                voters_outgoing: vec![1, 2, 3],
                learners: vec![5],
                learners_next: vec![3],
                auto_leave: true,
            },
            // Common-layer fixture: nested manifest semantics are checked by
            // the enclosing metadata/engine recovery, not by this frame codec.
            manifest: b"opaque-manifest-component-fixture".to_vec(),
        }
    }
    fn checksum(bytes: &mut [u8]) {
        let end = bytes.len() - 32;
        let digest = RootDigest::sha256(&bytes[..end]);
        bytes[end..].copy_from_slice(digest.as_bytes());
    }
    fn scope_offset(bytes: &[u8]) -> usize {
        HEADER + 4 + u32::from_be_bytes(bytes[HEADER..HEADER + 4].try_into().unwrap()) as usize + 32
    }

    #[test]
    fn joint_configuration_and_all_three_positions_survive_roundtrip() {
        let anchor = fixture();
        let bytes = anchor.encode().unwrap();
        let decoded = RecoveryAnchor::decode(&bytes).unwrap();
        assert_eq!(decoded, anchor);
        assert_eq!(decoded.encode().unwrap(), bytes);
        assert_eq!(decoded.configuration.voters_outgoing, vec![1, 2, 3]);
        assert_eq!(decoded.configuration.learners_next, vec![3]);
        assert!(decoded.configuration.auto_leave);
        assert_ne!(decoded.image_cut, decoded.publication);
        assert_ne!(decoded.configuration_at, Some(decoded.image_cut));
        assert_eq!(&bytes[..8], b"KV9ANCH\0");
        assert_eq!(&bytes[8..10], &1u16.to_be_bytes());
    }

    #[test]
    fn initial_configuration_is_the_exact_certified_root_membership() {
        let mut anchor = fixture();
        anchor.configuration_at = None;
        assert_eq!(anchor.encode(), Err(AnchorError::Configuration));
        anchor.configuration = AnchorConfiguration {
            voters: vec![1, 2, 3],
            voters_outgoing: vec![],
            learners: vec![],
            learners_next: vec![],
            auto_leave: false,
        };
        let bytes = anchor.encode().unwrap();
        assert_eq!(
            RecoveryAnchor::decode(&bytes).unwrap().configuration_at,
            None
        );
        anchor.configuration.voters.pop();
        assert_eq!(anchor.encode(), Err(AnchorError::Configuration));
    }

    #[test]
    fn every_truncation_and_single_bit_corruption_is_refused() {
        let bytes = fixture().encode().unwrap();
        for end in 0..bytes.len() {
            assert!(
                RecoveryAnchor::decode(&bytes[..end]).is_err(),
                "truncation at {end}"
            );
        }
        for index in 0..bytes.len() {
            let mut corrupt = bytes.clone();
            corrupt[index] ^= 1;
            assert!(
                RecoveryAnchor::decode(&corrupt).is_err(),
                "corruption at {index}"
            );
        }
        let mut appended = bytes;
        appended.push(0);
        assert!(RecoveryAnchor::decode(&appended).is_err());
    }

    #[test]
    fn checksum_valid_unsupported_headers_and_extra_bytes_are_refused() {
        let bytes = fixture().encode().unwrap();
        for (index, value) in [(9, 2), (17, 0), (17, 15)] {
            let mut corrupt = bytes.clone();
            corrupt[index] = value;
            checksum(&mut corrupt);
            assert_eq!(
                RecoveryAnchor::decode(&corrupt),
                Err(AnchorError::Unsupported)
            );
        }
        let mut extra = bytes[..bytes.len() - 32].to_vec();
        extra.push(0);
        extra.extend_from_slice(&[0; 32]);
        let len = (extra.len() - HEADER - 32) as u32;
        extra[HEADER - 4..HEADER].copy_from_slice(&len.to_be_bytes());
        checksum(&mut extra);
        assert_eq!(RecoveryAnchor::decode(&extra), Err(AnchorError::Format));
    }

    #[test]
    fn checksum_valid_nested_lengths_cannot_request_unbounded_allocation() {
        let bytes = fixture().encode().unwrap();
        let scope = scope_offset(&bytes);
        // Wire layout: epochs16, positions32, generation8, change32,
        // optional-config tag1 plus position16; then first membership count.
        let voters = scope + 16 + 32 + 8 + 32 + 1 + 16;
        for offset in [HEADER, voters] {
            let mut corrupt = bytes.clone();
            corrupt[offset..offset + 4].copy_from_slice(&u32::MAX.to_be_bytes());
            checksum(&mut corrupt);
            assert_eq!(RecoveryAnchor::decode(&corrupt), Err(AnchorError::Format));
        }
        let mut corrupt = bytes.clone();
        let count = HEADER + 4 + 8 + 2 + 16 + 16 + 8;
        corrupt[count..count + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        checksum(&mut corrupt);
        assert_eq!(RecoveryAnchor::decode(&corrupt), Err(AnchorError::Root));
        assert_eq!(
            RecoveryAnchor::decode(&vec![0; MAX_ANCHOR_BYTES + 1]),
            Err(AnchorError::Format)
        );
    }

    #[test]
    fn checksum_valid_mismatched_nested_digests_are_refused() {
        let bytes = fixture().encode().unwrap();
        let mut wrong_root = bytes.clone();
        wrong_root[scope_offset(&bytes) - 1] ^= 1;
        checksum(&mut wrong_root);
        assert_eq!(RecoveryAnchor::decode(&wrong_root), Err(AnchorError::Root));
        let mut wrong_manifest = bytes.clone();
        let offset = wrong_manifest.len() - 33;
        wrong_manifest[offset] ^= 1;
        checksum(&mut wrong_manifest);
        assert_eq!(
            RecoveryAnchor::decode(&wrong_manifest),
            Err(AnchorError::Scope)
        );
    }

    #[test]
    fn descriptor_identity_binds_each_recovery_authority_component() {
        let original = fixture();
        let digest = AnchorDigest::of(&original.encode().unwrap());
        for field in 0..8 {
            let mut changed = original.clone();
            match field {
                0 => changed.root.bootstrap_generation = BootstrapGeneration::from_bytes([9; 16]),
                1 => changed.conf_ver += 1,
                2 => changed.image_cut.index += 1,
                3 => changed.publication.index -= 1,
                4 => changed.generation += 1,
                5 => changed.change_id[0] ^= 1,
                6 => changed.configuration.learners.push(6),
                _ => changed.manifest.push(1),
            }
            assert_ne!(AnchorDigest::of(&changed.encode().unwrap()), digest);
        }
    }

    #[test]
    fn impossible_position_relations_and_ambiguous_configuration_are_refused() {
        let original = fixture();
        for field in 0..8 {
            let mut changed = original.clone();
            match field {
                0 => changed.image_cut.index = 0,
                1 => changed.publication = changed.image_cut,
                2 => changed.publication.term = 1,
                3 => changed.image_cut = at(0, 5),
                4 => changed.publication.index = u64::MAX,
                5 => changed.configuration_at = Some(at(2, 6)),
                6 => changed.configuration_at = Some(at(1, 5)),
                _ => changed.generation = 0,
            }
            assert_eq!(changed.encode(), Err(AnchorError::Position));
        }
        for field in 0..7 {
            let mut changed = original.clone();
            match field {
                0 => changed.configuration.voters = vec![1, 1, 4],
                1 => changed.configuration.voters.reverse(),
                2 => changed.configuration.learners = vec![2],
                3 => changed.configuration.learners_next = vec![4],
                4 => changed.configuration.voters_outgoing.clear(),
                5 => changed.configuration.voters.clear(),
                _ => changed.configuration.voters = vec![1; MAX_ANCHOR_MEMBERS + 1],
            }
            assert_eq!(changed.encode(), Err(AnchorError::Configuration));
        }
    }
}
