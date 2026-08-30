//! Explicit cluster root-of-trust and durable store identity.
//!
//! An "uninitialized" quorum describes state; it does not authorize creation. Creation
//! authority is the canonical [`RootDescriptor`] provisioned before Raft starts. Its digest
//! fences peers, while [`StoreIdentity`] prevents a replaced store from silently resuming an
//! old initial-voter identity.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{ClusterId, Error, NodeId, RegionId, Result, META_REGION_0};

pub const ROOT_DESCRIPTOR_VERSION: u16 = 1;
pub const ROOT_DESCRIPTOR_FILE: &str = "kv9-root-descriptor";
pub const STORE_IDENTITY_FILE: &str = "kv9-store-identity";

const ROOT_MAGIC: &[u8; 8] = b"KV9ROOT\0";
const STORE_MAGIC: &[u8; 8] = b"KV9STOR\0";

macro_rules! fixed_hex_id {
    ($name:ident, $n:expr, $label:literal) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name([u8; $n]);

        impl $name {
            pub fn from_bytes(bytes: [u8; $n]) -> Self {
                Self(bytes)
            }

            pub fn as_bytes(&self) -> &[u8; $n] {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                for byte in self.0 {
                    write!(f, "{byte:02x}")?;
                }
                Ok(())
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self)
            }
        }

        impl std::str::FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                if value.len() != $n * 2 {
                    return Err(Error::Config(format!(
                        concat!($label, " must be exactly {} hex characters"),
                        $n * 2
                    )));
                }
                if let Some(offset) = value.bytes().position(|byte| !byte.is_ascii_hexdigit()) {
                    return Err(Error::Config(format!(
                        concat!($label, " must be hex; invalid character at offset {}"),
                        offset
                    )));
                }
                let mut bytes = [0; $n];
                for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
                    let text = std::str::from_utf8(pair).expect("hex input is ascii");
                    bytes[index] = u8::from_str_radix(text, 16).expect("hex checked");
                }
                Ok(Self(bytes))
            }
        }
    };
}

fixed_hex_id!(BootstrapGeneration, 16, "bootstrap generation");
fixed_hex_id!(StoreIncarnation, 16, "store incarnation");
fixed_hex_id!(RootDigest, 32, "root digest");

impl BootstrapGeneration {
    pub fn mint() -> Result<Self> {
        Ok(Self(random_16("bootstrap generation")?))
    }
}

impl StoreIncarnation {
    pub fn mint() -> Result<Self> {
        Ok(Self(random_16("store incarnation")?))
    }
}

impl RootDigest {
    pub fn sha256(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}

fn random_16(label: &str) -> Result<[u8; 16]> {
    let mut bytes = [0; 16];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| Error::Config(format!("{label} entropy: {error}")))?;
    Ok(bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootVoter {
    pub node_id: NodeId,
    pub addr: SocketAddr,
    pub store_incarnation: StoreIncarnation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootDescriptor {
    pub version: u16,
    pub cluster_id: ClusterId,
    pub bootstrap_generation: BootstrapGeneration,
    pub meta_region_id: RegionId,
    pub voters: Vec<RootVoter>,
    /// SHA-256 of the bootstrap credential. The credential itself is never persisted.
    pub bootstrap_credential_sha256: RootDigest,
}

impl RootDescriptor {
    pub fn new(
        cluster_id: ClusterId,
        bootstrap_generation: BootstrapGeneration,
        mut voters: Vec<RootVoter>,
        bootstrap_credential: &[u8],
    ) -> Result<Self> {
        voters.sort_by_key(|voter| voter.node_id);
        let descriptor = Self {
            version: ROOT_DESCRIPTOR_VERSION,
            cluster_id,
            bootstrap_generation,
            meta_region_id: META_REGION_0,
            voters,
            bootstrap_credential_sha256: RootDigest::sha256(bootstrap_credential),
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != ROOT_DESCRIPTOR_VERSION {
            return Err(Error::Config(format!(
                "unsupported root descriptor version {}",
                self.version
            )));
        }
        if self.meta_region_id != META_REGION_0 {
            return Err(Error::Config(format!(
                "root descriptor meta region must be {}",
                META_REGION_0.0
            )));
        }
        if self.voters.is_empty() {
            return Err(Error::Config(
                "root descriptor needs at least one initial voter".into(),
            ));
        }
        let mut ids = HashSet::new();
        let mut addrs = HashSet::new();
        let mut incarnations = HashSet::new();
        let mut previous = None;
        for voter in &self.voters {
            if voter.node_id.0 == 0 {
                return Err(Error::Config("root voter node id must be non-zero".into()));
            }
            if previous.is_some_and(|id| voter.node_id.0 <= id) {
                return Err(Error::Config(
                    "root voters must be sorted by unique node id".into(),
                ));
            }
            previous = Some(voter.node_id.0);
            if !ids.insert(voter.node_id)
                || !addrs.insert(voter.addr)
                || !incarnations.insert(voter.store_incarnation)
            {
                return Err(Error::Config(
                    "root voters need unique node ids, addresses, and store incarnations".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn voter(&self, node_id: NodeId) -> Option<&RootVoter> {
        self.voters.iter().find(|voter| voter.node_id == node_id)
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(96 + self.voters.len() * 48);
        out.extend_from_slice(ROOT_MAGIC);
        out.extend_from_slice(&self.version.to_be_bytes());
        out.extend_from_slice(self.cluster_id.as_bytes());
        out.extend_from_slice(self.bootstrap_generation.as_bytes());
        out.extend_from_slice(&self.meta_region_id.0.to_be_bytes());
        out.extend_from_slice(&(self.voters.len() as u32).to_be_bytes());
        for voter in &self.voters {
            out.extend_from_slice(&voter.node_id.0.to_be_bytes());
            encode_addr(&mut out, voter.addr);
            out.extend_from_slice(voter.store_incarnation.as_bytes());
        }
        out.extend_from_slice(self.bootstrap_credential_sha256.as_bytes());
        out
    }

    pub fn digest(&self) -> RootDigest {
        RootDigest::sha256(&self.canonical_bytes())
    }

    pub fn decode(mut input: &[u8]) -> Result<Self> {
        expect_magic(&mut input, ROOT_MAGIC, "root descriptor")?;
        let version = take_u16(&mut input, "root version")?;
        let cluster_id = ClusterId::from_bytes(take_array(&mut input, "cluster id")?);
        let bootstrap_generation =
            BootstrapGeneration::from_bytes(take_array(&mut input, "bootstrap generation")?);
        let meta_region_id = RegionId(take_u64(&mut input, "meta region id")?);
        let count = take_u32(&mut input, "root voter count")? as usize;
        if count == 0 || count > 1024 {
            return Err(Error::Config("invalid root voter count".into()));
        }
        let mut voters = Vec::with_capacity(count);
        for _ in 0..count {
            voters.push(RootVoter {
                node_id: NodeId(take_u64(&mut input, "root voter id")?),
                addr: decode_addr(&mut input)?,
                store_incarnation: StoreIncarnation::from_bytes(take_array(
                    &mut input,
                    "store incarnation",
                )?),
            });
        }
        let bootstrap_credential_sha256 =
            RootDigest::from_bytes(take_array(&mut input, "bootstrap credential digest")?);
        if !input.is_empty() {
            return Err(Error::Config("root descriptor has trailing bytes".into()));
        }
        let descriptor = Self {
            version,
            cluster_id,
            bootstrap_generation,
            meta_region_id,
            voters,
            bootstrap_credential_sha256,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreIdentity {
    pub cluster_id: ClusterId,
    pub node_id: NodeId,
    pub store_incarnation: StoreIncarnation,
    pub root_digest: RootDigest,
}

impl StoreIdentity {
    pub fn for_voter(root: &RootDescriptor, node_id: NodeId) -> Result<Self> {
        let voter = root.voter(node_id).ok_or_else(|| {
            Error::Config(format!("node {} is not an initial root voter", node_id.0))
        })?;
        Ok(Self {
            cluster_id: root.cluster_id,
            node_id,
            store_incarnation: voter.store_incarnation,
            root_digest: root.digest(),
        })
    }

    /// Bind a newly admitted store to an existing root. The store is deliberately
    /// absent from `root.voters`: dynamic membership is certified by the catalog,
    /// while this record prevents the local data directory from changing cluster,
    /// node id, incarnation, or root across restarts.
    pub fn for_joiner(
        root: &RootDescriptor,
        node_id: NodeId,
        store_incarnation: StoreIncarnation,
    ) -> Result<Self> {
        if node_id.0 == 0 {
            return Err(Error::Config("store node id must be non-zero".into()));
        }
        if root.voter(node_id).is_some() {
            return Err(Error::Config(format!(
                "node {} is an initial voter; use its provisioned incarnation",
                node_id.0
            )));
        }
        Ok(Self {
            cluster_id: root.cluster_id,
            node_id,
            store_incarnation,
            root_digest: root.digest(),
        })
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(80);
        out.extend_from_slice(STORE_MAGIC);
        out.extend_from_slice(self.cluster_id.as_bytes());
        out.extend_from_slice(&self.node_id.0.to_be_bytes());
        out.extend_from_slice(self.store_incarnation.as_bytes());
        out.extend_from_slice(self.root_digest.as_bytes());
        out
    }

    pub fn decode(mut input: &[u8]) -> Result<Self> {
        expect_magic(&mut input, STORE_MAGIC, "store identity")?;
        let identity = Self {
            cluster_id: ClusterId::from_bytes(take_array(&mut input, "cluster id")?),
            node_id: NodeId(take_u64(&mut input, "node id")?),
            store_incarnation: StoreIncarnation::from_bytes(take_array(
                &mut input,
                "store incarnation",
            )?),
            root_digest: RootDigest::from_bytes(take_array(&mut input, "root digest")?),
        };
        if !input.is_empty() {
            return Err(Error::Config("store identity has trailing bytes".into()));
        }
        Ok(identity)
    }

    pub fn verify(&self, root: &RootDescriptor, node_id: NodeId) -> Result<()> {
        if self.node_id != node_id
            || self.node_id.0 == 0
            || self.cluster_id != root.cluster_id
            || self.root_digest != root.digest()
        {
            return Err(Error::Config(
                "store identity does not match the provisioned root descriptor".into(),
            ));
        }
        if let Some(voter) = root.voter(node_id) {
            if self.store_incarnation != voter.store_incarnation {
                return Err(Error::Config(
                    "initial voter's store incarnation does not match the root descriptor".into(),
                ));
            }
        }
        Ok(())
    }
}

pub fn persist_root_bundle(
    data_dir: &Path,
    root: &RootDescriptor,
    identity: &StoreIdentity,
) -> Result<()> {
    root.validate()?;
    identity.verify(root, identity.node_id)?;
    fs::create_dir_all(data_dir)
        .map_err(|error| Error::Config(format!("create {}: {error}", data_dir.display())))?;
    let root_path = data_dir.join(ROOT_DESCRIPTOR_FILE);
    let store_path = data_dir.join(STORE_IDENTITY_FILE);
    match (root_path.exists(), store_path.exists()) {
        (false, false) => {}
        (true, true) => {
            let (saved_root, saved_identity) = load_root_bundle(data_dir)?;
            if saved_root == *root && saved_identity == *identity {
                return Ok(());
            }
            return Err(Error::Config(
                "data directory is already bound to a different root/store identity".into(),
            ));
        }
        // The root is published first. A crash between the two individually
        // atomic writes leaves this exact state. Resume only when the durable
        // root bytes equal the caller's root; identity.verify above then proves
        // the missing second file is the identity authorized by that root.
        (true, false) => {
            let saved_root = RootDescriptor::decode(&read_file(&root_path)?)?;
            if saved_root != *root {
                return Err(Error::Config(
                    "data directory contains a different root and no store identity".into(),
                ));
            }
            atomic_write(data_dir, STORE_IDENTITY_FILE, &identity.canonical_bytes())?;
            return Ok(());
        }
        (false, true) => {
            return Err(Error::Config(
                "data directory contains a store identity without a root descriptor".into(),
            ))
        }
    }
    atomic_write(data_dir, ROOT_DESCRIPTOR_FILE, &root.canonical_bytes())?;
    atomic_write(data_dir, STORE_IDENTITY_FILE, &identity.canonical_bytes())?;
    Ok(())
}

/// Publish a newly created root exactly once. The bytes are fully synced
/// before an atomic no-replace hard link makes the chosen path visible, so a
/// crash cannot leave a partially written file that later looks authoritative.
pub fn write_new_root_descriptor(path: &Path, root: &RootDescriptor) -> Result<()> {
    root.validate()?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| Error::Config(format!("create {}: {error}", parent.display())))?;
    let name = path
        .file_name()
        .ok_or_else(|| Error::Config("root descriptor output needs a file name".into()))?;
    let temp = temporary_path(parent, &name.to_string_lossy())?;
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|error| Error::Config(format!("create {}: {error}", temp.display())))?;
        file.write_all(&root.canonical_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| Error::Config(format!("write {}: {error}", temp.display())))?;
        fs::hard_link(&temp, path).map_err(|error| {
            Error::Config(format!(
                "publish {} without replacing an existing root: {error}",
                path.display()
            ))
        })?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| Error::Config(format!("sync {}: {error}", parent.display())))
    })();
    let _ = fs::remove_file(&temp);
    result
}

pub fn load_root_bundle(data_dir: &Path) -> Result<(RootDescriptor, StoreIdentity)> {
    let root = RootDescriptor::decode(&read_file(&data_dir.join(ROOT_DESCRIPTOR_FILE))?)?;
    let identity = StoreIdentity::decode(&read_file(&data_dir.join(STORE_IDENTITY_FILE))?)?;
    identity.verify(&root, identity.node_id)?;
    Ok((root, identity))
}

fn read_file(path: &Path) -> Result<Vec<u8>> {
    fs::read(path).map_err(|error| Error::Config(format!("read {}: {error}", path.display())))
}

fn atomic_write(data_dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let target = data_dir.join(name);
    let tmp = temporary_path(data_dir, name)?;
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)
            .map_err(|error| Error::Config(format!("create {}: {error}", tmp.display())))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| Error::Config(format!("write {}: {error}", tmp.display())))?;
        fs::rename(&tmp, &target)
            .map_err(|error| Error::Config(format!("rename {}: {error}", target.display())))?;
        File::open(data_dir)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| Error::Config(format!("sync {}: {error}", data_dir.display())))
    })();
    let _ = fs::remove_file(&tmp);
    result
}

fn temporary_path(parent: &Path, name: &str) -> Result<std::path::PathBuf> {
    let nonce = random_16("temporary file")?
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(parent.join(format!(".{name}.{}.{nonce}.tmp", std::process::id())))
}

fn encode_addr(out: &mut Vec<u8>, addr: SocketAddr) {
    match addr.ip() {
        IpAddr::V4(ip) => {
            out.push(4);
            out.extend_from_slice(&ip.octets());
        }
        IpAddr::V6(ip) => {
            out.push(6);
            out.extend_from_slice(&ip.octets());
        }
    }
    out.extend_from_slice(&addr.port().to_be_bytes());
}

fn decode_addr(input: &mut &[u8]) -> Result<SocketAddr> {
    let family = take_array::<1>(input, "address family")?[0];
    let ip = match family {
        4 => IpAddr::V4(Ipv4Addr::from(take_array::<4>(input, "ipv4 address")?)),
        6 => IpAddr::V6(Ipv6Addr::from(take_array::<16>(input, "ipv6 address")?)),
        _ => return Err(Error::Config("invalid root voter address family".into())),
    };
    Ok(SocketAddr::new(ip, take_u16(input, "address port")?))
}

fn expect_magic(input: &mut &[u8], magic: &[u8], label: &str) -> Result<()> {
    if input.len() < magic.len() || &input[..magic.len()] != magic {
        return Err(Error::Config(format!("invalid {label} magic")));
    }
    *input = &input[magic.len()..];
    Ok(())
}

fn take_array<const N: usize>(input: &mut &[u8], label: &str) -> Result<[u8; N]> {
    if input.len() < N {
        return Err(Error::Config(format!("truncated {label}")));
    }
    let (head, tail) = input.split_at(N);
    *input = tail;
    Ok(head.try_into().expect("length checked"))
}

fn take_u16(input: &mut &[u8], label: &str) -> Result<u16> {
    Ok(u16::from_be_bytes(take_array(input, label)?))
}

fn take_u32(input: &mut &[u8], label: &str) -> Result<u32> {
    Ok(u32::from_be_bytes(take_array(input, label)?))
}

fn take_u64(input: &mut &[u8], label: &str) -> Result<u64> {
    Ok(u64::from_be_bytes(take_array(input, label)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> RootDescriptor {
        RootDescriptor::new(
            ClusterId::from_bytes([1; 16]),
            BootstrapGeneration::from_bytes([2; 16]),
            vec![
                RootVoter {
                    node_id: NodeId(2),
                    addr: "127.0.0.1:20202".parse().unwrap(),
                    store_incarnation: StoreIncarnation::from_bytes([4; 16]),
                },
                RootVoter {
                    node_id: NodeId(1),
                    addr: "127.0.0.1:20201".parse().unwrap(),
                    store_incarnation: StoreIncarnation::from_bytes([3; 16]),
                },
            ],
            b"bootstrap-secret",
        )
        .unwrap()
    }

    #[test]
    fn canonical_root_round_trips_and_is_sorted() {
        let root = root();
        assert_eq!(root.voters[0].node_id, NodeId(1));
        assert_eq!(
            RootDescriptor::decode(&root.canonical_bytes()).unwrap(),
            root
        );
        assert_eq!(root.digest(), RootDigest::sha256(&root.canonical_bytes()));
    }

    #[test]
    fn root_digest_changes_with_generation_incarnation_and_credential() {
        let root = root();
        let mut generation = root.clone();
        generation.bootstrap_generation = BootstrapGeneration::from_bytes([9; 16]);
        let mut incarnation = root.clone();
        incarnation.voters[0].store_incarnation = StoreIncarnation::from_bytes([8; 16]);
        let mut credential = root.clone();
        credential.bootstrap_credential_sha256 = RootDigest::sha256(b"different");
        assert_ne!(root.digest(), generation.digest());
        assert_ne!(root.digest(), incarnation.digest());
        assert_ne!(root.digest(), credential.digest());
    }

    #[test]
    fn durable_bundle_is_idempotent_but_never_rebinds_a_store() {
        let dir = std::env::temp_dir().join(format!("kv9-root-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let root = root();
        let identity = StoreIdentity::for_voter(&root, NodeId(1)).unwrap();
        persist_root_bundle(&dir, &root, &identity).unwrap();
        persist_root_bundle(&dir, &root, &identity).unwrap();
        assert_eq!(load_root_bundle(&dir).unwrap(), (root.clone(), identity));
        let other = StoreIdentity::for_voter(&root, NodeId(2)).unwrap();
        assert!(persist_root_bundle(&dir, &root, &other).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn root_only_bundle_resumes_but_other_partial_states_fail_closed() {
        let dir = std::env::temp_dir().join(format!("kv9-root-partial-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let expected_root = root();
        let identity = StoreIdentity::for_voter(&expected_root, NodeId(1)).unwrap();
        fs::write(
            dir.join(ROOT_DESCRIPTOR_FILE),
            expected_root.canonical_bytes(),
        )
        .unwrap();
        assert!(load_root_bundle(&dir).is_err());
        persist_root_bundle(&dir, &expected_root, &identity).unwrap();
        assert_eq!(
            load_root_bundle(&dir).unwrap(),
            (expected_root.clone(), identity)
        );

        fs::remove_dir_all(&dir).unwrap();
        fs::create_dir_all(&dir).unwrap();
        let identity = StoreIdentity::for_voter(&expected_root, NodeId(1)).unwrap();
        fs::write(dir.join(STORE_IDENTITY_FILE), identity.canonical_bytes()).unwrap();
        assert!(persist_root_bundle(&dir, &expected_root, &identity).is_err());

        fs::remove_dir_all(&dir).unwrap();
        fs::create_dir_all(&dir).unwrap();
        let mut other_root = expected_root.clone();
        other_root.bootstrap_generation = BootstrapGeneration::from_bytes([8; 16]);
        fs::write(dir.join(ROOT_DESCRIPTOR_FILE), other_root.canonical_bytes()).unwrap();
        assert!(persist_root_bundle(&dir, &expected_root, &identity).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn new_root_publication_is_atomic_and_never_replaces() {
        let dir = std::env::temp_dir().join(format!("kv9-root-create-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("root.bin");
        let original = root();
        write_new_root_descriptor(&path, &original).unwrap();
        assert_eq!(
            RootDescriptor::decode(&fs::read(&path).unwrap()).unwrap(),
            original
        );
        let mut replacement = root();
        replacement.bootstrap_generation = BootstrapGeneration::from_bytes([8; 16]);
        assert!(write_new_root_descriptor(&path, &replacement).is_err());
        assert_eq!(
            RootDescriptor::decode(&fs::read(&path).unwrap()).unwrap(),
            root()
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
