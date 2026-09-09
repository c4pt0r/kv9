use super::*;
use kv9_raft::grpc::{grpc_confirm_endpoint, EndpointConfirmError, EndpointConfirmationReceipt};
use std::io::{Read, Write};

const FILE: &str = "kv9-serving-endpoint";
const MAGIC: &[u8; 8] = b"KV9ENDP2";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedEndpoint {
    node: u64,
    incarnation: String,
    root: String,
    address: std::net::SocketAddr,
    generation: u64,
    term: u64,
    index: u64,
}

impl SavedEndpoint {
    fn save(&self, directory: &Path) -> Result<()> {
        self.save_observed(directory, |_| Ok(()))
    }

    fn save_observed(
        &self,
        directory: &Path,
        mut observed: impl FnMut(&str) -> std::io::Result<()>,
    ) -> Result<()> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(serde_json::to_vec(self).map_err(|e| Error::Config(e.to_string()))?);
        bytes.extend_from_slice(RootDigest::sha256(&bytes).as_bytes());
        let temporary = directory.join(format!(".{FILE}.{}.tmp", StoreIncarnation::mint()?));
        let mut operation = || -> std::io::Result<()> {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            observed("written")?;
            file.sync_all()?;
            observed("file_synced")?;
            fs::rename(&temporary, directory.join(FILE))?;
            observed("renamed")?;
            kv9_common::fs::sync_ancestors(&kv9_common::fs::OsFileSystem, directory)?;
            observed("ancestors_synced")
        };
        let result =
            operation().map_err(|e| Error::Config(format!("publish serving endpoint: {e}")));
        let _ = fs::remove_file(temporary);
        result
    }
}

pub(super) struct EndpointRecovery {
    saved: Option<SavedEndpoint>,
    pending: Option<(EndpointConfirmationReceipt, Instant)>,
    cursor: RegistrationSeedCursor,
    next_attempt: Instant,
    pub(super) attempts: u64,
    pub(super) last: &'static str,
    pub(super) receipt: Option<AppliedPosition>,
}

impl EndpointRecovery {
    pub(super) fn load(directory: &Path, identity: &StoreIdentity) -> Result<Self> {
        let path = directory.join(FILE);
        let read = || -> std::io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            fs::File::open(&path)?.take(4097).read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        let saved = match read() {
            Ok(bytes) => {
                if bytes.len() < 40
                    || bytes.len() > 4096
                    || &bytes[..8] != MAGIC
                    || RootDigest::sha256(&bytes[..bytes.len() - 32]).as_bytes()
                        != &bytes[bytes.len() - 32..]
                {
                    return Err(Error::Config("invalid serving endpoint record".into()));
                }
                let saved: SavedEndpoint = serde_json::from_slice(&bytes[8..bytes.len() - 32])
                    .map_err(|e| Error::Config(format!("decode serving endpoint: {e}")))?;
                if saved.node != identity.node_id.0
                    || saved.incarnation != identity.store_incarnation.to_string()
                    || saved.root != identity.root_digest.to_string()
                    || saved.address.port() == 0
                    || saved.address.ip().is_unspecified()
                    || (saved.term == 0) != (saved.index == 0)
                    || (saved.generation > 0 && saved.index == 0)
                {
                    return Err(Error::Config(
                        "serving endpoint does not match this store or its receipt".into(),
                    ));
                }
                fs::File::open(&path)
                    .and_then(|file| file.sync_all())
                    .and_then(|_| {
                        kv9_common::fs::sync_ancestors(&kv9_common::fs::OsFileSystem, directory)
                    })
                    .map_err(|e| {
                        Error::Config(format!("stabilize serving endpoint record: {e}"))
                    })?;
                Some(saved)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::Config(format!("read serving endpoint: {e}"))),
        };
        Ok(Self {
            saved,
            pending: None,
            cursor: RegistrationSeedCursor::default(),
            next_attempt: Instant::now(),
            attempts: 0,
            last: "not_checked",
            receipt: None,
        })
    }
}

impl NodeRuntime {
    pub(super) fn advance_endpoint_recovery(&mut self) -> Result<bool> {
        if !init_marker_exists(&self.data_dir)
            || self.node.local_cluster_identity()? != Some(self.root.cluster_id)
            || !self.local_membership_is_active()?
        {
            self.endpoint_recovery.last = "membership_pending";
            return Ok(false);
        }
        if let Some((receipt, started)) = self.endpoint_recovery.pending {
            let at = ProposedAt {
                term: receipt.applied.term,
                index: kv9_raft::LogIndex(receipt.applied.index),
            };
            match self.driver.wait_applied(at, Duration::from_millis(1)) {
                Ok(ApplyWaitOutcome::Applied(observed)) if observed == receipt.applied => {
                    let txn = self.node.meta_raft.store.begin()?;
                    let current = kv9_meta::endpoint::node_endpoint(&txn, self.node.id)?;
                    drop(txn);
                    if current.is_some_and(|row| {
                        row.active
                            && row.incarnation == self.store_identity.store_incarnation
                            && row.address == self.advertised_addr
                            && row.generation == receipt.subject.generation
                    }) {
                        self.save_serving_endpoint(
                            receipt.subject.generation,
                            Some(receipt.applied),
                        )?;
                        self.endpoint_recovery.pending = None;
                        self.endpoint_recovery.last = "confirmed";
                        self.endpoint_recovery.receipt = Some(receipt.applied);
                        return Ok(true);
                    }
                    self.endpoint_recovery.pending = None;
                    self.endpoint_recovery.last = "confirmation_superseded";
                    return Ok(false);
                }
                Err(ApplyWaitError::Unconfirmed { .. }) => {
                    // An evicted exact receipt is not a confirmation. Request a
                    // new idempotent barrier after this bounded wait instead.
                    if started.elapsed() >= Duration::from_secs(5) {
                        self.endpoint_recovery.pending = None;
                        self.endpoint_recovery.last = "confirmation_expired";
                    } else {
                        self.endpoint_recovery.last = "confirmation_apply_pending";
                    }
                    return Ok(false);
                }
                Err(ApplyWaitError::Failed(error)) => return Err(error),
                other => {
                    return Err(Error::Raft(format!(
                        "endpoint confirmation receipt was not applied exactly: {other:?}"
                    )))
                }
            }
        }
        let txn = self.node.meta_raft.store.begin()?;
        let current = kv9_meta::endpoint::node_endpoint(&txn, self.node.id)?
            .ok_or_else(|| Error::Config("active member has no endpoint".into()))?;
        if current.address == self.advertised_addr {
            if let Some(saved) = &self.endpoint_recovery.saved {
                if saved.address == self.advertised_addr
                    && saved.generation <= current.generation
                    // ConfirmEndpoint appends Command::Noop, whose position
                    // is persisted with the catalog. The per-process unified
                    // watermark deliberately starts empty after restart.
                    && self.driver.status().applied_index >= saved.index
                {
                    self.endpoint_recovery.last = "stable_local_authority";
                    return Ok(true);
                }
            } else if current.generation == 0 {
                // Preserve the established generation-zero initial/registration
                // authority. A migrated row with no marker needs a new receipt.
                drop(txn);
                self.save_serving_endpoint(0, None)?;
                self.endpoint_recovery.last = "initial_local_authority";
                return Ok(true);
            }
        }
        if Instant::now() < self.endpoint_recovery.next_attempt {
            return Ok(false);
        }
        self.endpoint_recovery.next_attempt = Instant::now() + DISCOVERY_INTERVAL;
        let mut candidates = BTreeMap::new();
        for seed in &self.seeds {
            candidates.insert(seed.node_id, seed.addr);
        }
        let rows = txn.scan(&NODES_DESC, 1025)?;
        if rows.len() > 1024 {
            return Err(Error::Config("endpoint candidate limit exceeded".into()));
        }
        for row in rows {
            if let Some(ColumnValue::Uint(id)) = row.value.get(ColumnId(1)) {
                if let Some(endpoint) = kv9_meta::endpoint::node_endpoint(&txn, NodeId(*id))? {
                    if endpoint.active {
                        candidates.insert(endpoint.node, endpoint.address);
                    }
                }
            }
        }
        drop(txn);
        if candidates.len() > 1024 {
            return Err(Error::Config("endpoint candidate limit exceeded".into()));
        }
        for (id, address) in &mut candidates {
            if let Some(installed) = self.transport.peer_address(*id) {
                *address = installed;
            }
        }
        // Local control RPCs remain reachable even while public readiness or
        // a Service endpoint is closed. Remote nodes still use advertised routes.
        let local_ip = if self.addr.ip().is_unspecified() {
            if self.addr.is_ipv4() {
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
            } else {
                std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
            }
        } else {
            self.addr.ip()
        };
        candidates.insert(
            self.node.id,
            std::net::SocketAddr::new(local_ip, self.addr.port()),
        );
        let allowed: HashSet<_> = candidates.keys().copied().collect();
        let candidates: Vec<_> = candidates.into_iter().collect();
        let mut queue: std::collections::VecDeque<_> =
            self.endpoint_recovery.cursor.order(&candidates).into();
        let mut visited = HashSet::new();
        let deadline = Instant::now() + REGISTRATION_PASS_TIMEOUT;
        while let Some(peer) = queue.pop_front() {
            if Instant::now() >= deadline || visited.len() >= candidates.len() + 4 {
                break;
            }
            if !visited.insert(peer) {
                continue;
            }
            self.endpoint_recovery.attempts = self.endpoint_recovery.attempts.saturating_add(1);
            match grpc_confirm_endpoint(
                self.grpc_runtime.handle(),
                self.node.id,
                peer,
                self.root.cluster_id,
                self.root.digest(),
                self.store_identity.store_incarnation,
                self.advertised_addr,
                self.cluster_token.clone(),
                deadline.saturating_duration_since(Instant::now()),
            ) {
                Ok(receipt) => {
                    let _guard = self.node.meta_raft.lock_catalog_txn();
                    self.transport.register_catalog_peer(
                        receipt.responder.node,
                        receipt.responder.address,
                        receipt.responder.generation,
                    )?;
                    self.endpoint_recovery.pending = Some((receipt, Instant::now()));
                    self.endpoint_recovery.receipt = Some(receipt.applied);
                    self.endpoint_recovery.last = "confirmation_received";
                    return Ok(false);
                }
                Err(EndpointConfirmError::NotLeader(Some(hint))) if allowed.contains(&hint.id) => {
                    if let Some(address) = hint.addr {
                        if !visited.contains(&(hint.id, address)) {
                            queue.push_front((hint.id, address));
                        }
                    }
                    self.endpoint_recovery.last = "not_leader";
                }
                Err(EndpointConfirmError::NotLeader(_)) => {
                    self.endpoint_recovery.last = "not_leader"
                }
                Err(EndpointConfirmError::Unconfirmed(_)) => {
                    self.endpoint_recovery.last = "unconfirmed"
                }
            }
        }
        Ok(false)
    }

    fn save_serving_endpoint(
        &mut self,
        generation: u64,
        at: Option<AppliedPosition>,
    ) -> Result<()> {
        let saved = SavedEndpoint {
            node: self.node.id.0,
            incarnation: self.store_identity.store_incarnation.to_string(),
            root: self.root.digest().to_string(),
            address: self.advertised_addr,
            generation,
            term: at.map_or(0, |p| p.term),
            index: at.map_or(0, |p| p.index),
        };
        saved.save(&self.data_dir)?;
        self.endpoint_recovery.saved = Some(saved);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (PathBuf, StoreIdentity, SavedEndpoint) {
        let directory = std::env::temp_dir().join(format!(
            "kv9-endpoint-record-{}",
            StoreIncarnation::mint().unwrap()
        ));
        fs::create_dir(&directory).unwrap();
        let identity = StoreIdentity {
            cluster_id: ClusterId::from_bytes([1; 16]),
            node_id: NodeId(4),
            store_incarnation: StoreIncarnation::from_bytes([4; 16]),
            root_digest: RootDigest::from_bytes([7; 32]),
        };
        let saved = SavedEndpoint {
            node: 4,
            incarnation: identity.store_incarnation.to_string(),
            root: identity.root_digest.to_string(),
            address: "127.0.0.1:45004".parse().unwrap(),
            generation: 1,
            term: 2,
            index: 31,
        };
        (directory, identity, saved)
    }

    #[test]
    fn endpoint_record_rejects_corruption_foreign_binding_and_invalid_receipts() {
        let (directory, identity, saved) = fixture();
        saved.save(&directory).unwrap();
        assert_eq!(
            EndpointRecovery::load(&directory, &identity)
                .unwrap()
                .saved
                .unwrap()
                .index,
            31
        );
        let original = fs::read(directory.join(FILE)).unwrap();
        for bytes in [
            vec![],
            original[..original.len() - 1].to_vec(),
            vec![0; 4097],
        ] {
            fs::write(directory.join(FILE), bytes).unwrap();
            assert!(
                EndpointRecovery::load(&directory, &identity).is_err(),
                "corrupt endpoint record admitted"
            );
        }
        let mut corrupt = original;
        corrupt[20] ^= 1;
        fs::write(directory.join(FILE), corrupt).unwrap();
        assert!(
            EndpointRecovery::load(&directory, &identity).is_err(),
            "bad endpoint checksum admitted"
        );
        for invalid in [
            SavedEndpoint {
                node: 5,
                ..saved.clone()
            },
            SavedEndpoint {
                incarnation: StoreIncarnation::from_bytes([5; 16]).to_string(),
                ..saved.clone()
            },
            SavedEndpoint {
                root: RootDigest::from_bytes([8; 32]).to_string(),
                ..saved.clone()
            },
            SavedEndpoint {
                term: 0,
                ..saved.clone()
            },
            SavedEndpoint {
                term: 0,
                index: 0,
                ..saved.clone()
            },
            SavedEndpoint {
                address: "0.0.0.0:45004".parse().unwrap(),
                ..saved.clone()
            },
        ] {
            invalid.save(&directory).unwrap();
            assert!(
                EndpointRecovery::load(&directory, &identity).is_err(),
                "foreign or unconfirmed endpoint record admitted"
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn endpoint_publication_failure_never_returns_authority_and_reopen_stabilizes_visible_record() {
        for cut in ["written", "file_synced", "renamed", "ancestors_synced"] {
            let (directory, identity, saved) = fixture();
            let mut reached = false;
            let result = saved.save_observed(&directory, |point| {
                if point == cut {
                    reached = true;
                    Err(std::io::Error::other(
                        "injected endpoint publication failure",
                    ))
                } else {
                    Ok(())
                }
            });
            assert!(
                reached && result.is_err(),
                "failed endpoint publication returned authority"
            );
            let reopened = EndpointRecovery::load(&directory, &identity).unwrap();
            assert_eq!(
                reopened.saved.is_some(),
                matches!(cut, "renamed" | "ancestors_synced")
            );
            assert_eq!(reopened.attempts, 0);
            assert!(reopened.pending.is_none() && reopened.receipt.is_none());
            fs::remove_dir_all(directory).unwrap();
        }
    }
}
