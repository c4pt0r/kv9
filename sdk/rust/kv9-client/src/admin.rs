//! The administrative surface of the SDK (docs/KV9-ADMIN.md): the node directory,
//! a node's status document, and the metadata-group membership verbs. These are the
//! RPCs `kv9-admin` is built on; applications normally never need them.

use crate::{proto, Applied, Client, Error};

/// One node's status document: the lines of its status file, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeStatus {
    pub fields: Vec<(String, String)>,
    /// The node's local, unbarriered view of the directory: `(node_id, address)`.
    pub peers: Vec<(u64, String)>,
}

impl NodeStatus {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn u64(&self, name: &str) -> Option<u64> {
        self.get(name).and_then(|v| v.parse().ok())
    }

    /// A comma-separated id list field (`meta_voters`, `meta_learners`, ...).
    pub fn ids(&self, name: &str) -> Vec<u64> {
        self.get(name)
            .unwrap_or_default()
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect()
    }
}

/// One row of the node directory as the metadata leader sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeRow {
    pub node_id: u64,
    pub address: String,
    pub store_incarnation: Vec<u8>,
    pub endpoint_generation: u64,
    pub active: bool,
    pub last_heartbeat_unix: u64,
    /// `voter`, `learner` or `none`.
    pub raft_role: String,
    pub matched_index: u64,
    pub recent_active: bool,
    /// `pending`, `consumed`, `revoked`, `superseded` or empty.
    pub admission_state: String,
    pub admission_expires_unix: u64,
    /// A committed drain names this node's current store (docs/DRAIN-NODE.md).
    pub draining: bool,
    /// Draining and already out of every data group and the metadata group.
    pub drained: bool,
}

/// The receipt of [`Client::backup_keyspace`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupPlanned {
    pub task: u64,
    pub operation: Vec<u8>,
    /// `false` confirms the keyspace's live backup.
    pub changed: bool,
    pub applied: Applied,
    pub sequence: u64,
    pub regions: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupAnchorRow {
    pub task: u64,
    pub cut: Applied,
    pub image_digest: Vec<u8>,
    pub objects: u64,
    pub object_bytes: u64,
    pub rows: u64,
    pub rows_digest: Vec<u8>,
    pub restorable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRangeRow {
    pub region: u64,
    pub creation_task: u64,
    pub bind_task: u64,
    pub operation: Vec<u8>,
    pub anchor: Option<BackupAnchorRow>,
}

/// One committed backup (docs/BACKUP-RESTORE.md §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRow {
    pub task: u64,
    pub operation: Vec<u8>,
    pub keyspace_id: u32,
    pub tenant_id: u64,
    pub label: String,
    pub sequence: u64,
    pub requested_unix: u64,
    /// intended, capturing, anchored, complete, aborted, retired.
    pub stage: String,
    pub ranges: Vec<BackupRangeRow>,
    pub bundle_key: String,
    pub bundle_digest: Vec<u8>,
    pub bundle_bytes: u64,
    pub completed_unix: u64,
    pub retired_unix: u64,
    pub restorable: bool,
}

/// The receipt of [`Client::retire_backup`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRetired {
    pub retirement_task: u64,
    pub changed: bool,
    pub applied: Applied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupRangeVerdict {
    pub region: u64,
    pub ok: bool,
    pub detail: String,
}

/// The verdict of [`Client::verify_backup`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupVerdict {
    pub task: u64,
    pub bundle_key: String,
    pub bundle_ok: bool,
    pub ranges: Vec<BackupRangeVerdict>,
    pub ok: bool,
}

/// The receipt of [`Client::restore_keyspace`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestorePlanned {
    pub task: u64,
    pub operation: Vec<u8>,
    pub changed: bool,
    pub applied: Applied,
    /// Reserved; published once every target is evidenced.
    pub keyspace_id: u32,
    pub target_regions: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreTargetRow {
    pub source_region: u64,
    pub creation_task: u64,
    pub region: u64,
    pub start: Vec<u8>,
    pub end: Vec<u8>,
    pub evidence_task: u64,
    pub rows: u64,
}

/// One committed restore (docs/BACKUP-RESTORE.md §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreRow {
    pub task: u64,
    pub operation: Vec<u8>,
    pub backup_task: u64,
    pub keyspace_id: u32,
    pub tenant_id: u64,
    pub name: String,
    pub requested_unix: u64,
    /// intended, populating, populated, published, aborted.
    pub stage: String,
    pub targets: Vec<RestoreTargetRow>,
}

/// The receipt of [`Client::drain_node`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainPlanned {
    pub task: u64,
    pub operation: Vec<u8>,
    /// `false` confirms an identical committed drain.
    pub changed: bool,
    pub applied: Applied,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeDirectory {
    pub leader_node_id: u64,
    pub raft_committed: u64,
    pub nodes: Vec<NodeRow>,
}

/// The receipt of a membership verb: the applied position and the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipChange {
    pub applied: Applied,
    pub voters: Vec<u64>,
    pub learners: Vec<u64>,
    /// Present only on `admit_node`: the one-time join ticket, returned once.
    pub join_ticket: Option<String>,
    /// `false` when the target state already held (a repeated verb confirms).
    pub changed: bool,
}

/// The outcome of [`Client::transfer_leader`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeaderTransfer {
    pub previous_leader: u64,
    pub leader: Option<u64>,
    pub term: u64,
    pub completed: bool,
}

/// The metadata group's region id, for [`Client::transfer_leader`].
pub const META_REGION_ID: u64 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeEndpoint {
    pub node_id: u64,
    pub store_incarnation: Vec<u8>,
    pub address: String,
    pub generation: u64,
    pub previous_address: Option<String>,
    pub active: bool,
}

fn membership(r: proto::MembershipChangeResponse) -> MembershipChange {
    MembershipChange {
        applied: Applied {
            term: r.applied_term,
            index: r.applied_index,
        },
        voters: r.voters,
        learners: r.learners,
        join_ticket: Some(r.join_ticket).filter(|t| !t.is_empty()),
        changed: r.changed,
    }
}

impl Client {
    /// The same client addressed at another node: same token, peers, timeout and
    /// redirect policy. `NodeStatus` is answered by the node asked, so a fan-out
    /// over the directory builds one of these per address.
    pub fn at(&self, address: impl Into<String>) -> Client {
        let address = address.into();
        self.clone().rebuild(|inner| {
            *inner.address.get_mut().expect("address") = address;
        })
    }

    /// This node's status document. Served by every node about itself; never
    /// redirected.
    pub async fn node_status(&self) -> Result<NodeStatus, Error> {
        let r = self
            .invoke(true, proto::NodeStatusRequest {}, |mut c, r| async move {
                c.node_status(r).await
            })
            .await?;
        Ok(NodeStatus {
            fields: r.fields.into_iter().map(|f| (f.name, f.value)).collect(),
            peers: r.peers.into_iter().map(|p| (p.node_id, p.address)).collect(),
        })
    }

    /// The node directory, linearizably, from the metadata leader (follows a
    /// `NotLeader` hint through the peers map).
    pub async fn list_nodes(&self) -> Result<NodeDirectory, Error> {
        let r = self
            .invoke(true, proto::ListNodesRequest {}, |mut c, r| async move {
                c.list_nodes(r).await
            })
            .await?;
        Ok(NodeDirectory {
            leader_node_id: r.leader_node_id,
            raft_committed: r.raft_committed,
            nodes: r
                .nodes
                .into_iter()
                .map(|n| NodeRow {
                    node_id: n.node_id,
                    address: n.address,
                    store_incarnation: n.store_incarnation,
                    endpoint_generation: n.endpoint_generation,
                    active: n.active,
                    last_heartbeat_unix: n.last_heartbeat_unix,
                    raft_role: n.raft_role,
                    matched_index: n.matched_index,
                    recent_active: n.recent_active,
                    admission_state: n.admission_state,
                    admission_expires_unix: n.admission_expires_unix,
                    draining: n.draining,
                    drained: n.drained,
                })
                .collect(),
        })
    }

    /// Commit a split intent for the bound range of `parent_region` at `split_key`
    /// (docs/RANGE-SPLIT.md §2): the two children are created and activated on
    /// the parent's stores in the same transaction. Nothing seals or reroutes yet.
    pub async fn split_range(
        &self,
        root: [u8; 32],
        operation: [u8; 16],
        parent_region: u64,
        split_key: &[u8],
    ) -> Result<SplitPlanned, Error> {
        let r = self
            .invoke(
                false,
                proto::SplitRangeRequest {
                    root_digest: root.to_vec(),
                    operation_id: operation.to_vec(),
                    parent_region,
                    split_key: split_key.to_vec(),
                },
                |mut c, r| async move { c.split_range(r).await },
            )
            .await?;
        Ok(SplitPlanned {
            intent_task: r.intent_task,
            child_low_task: r.child_low_task,
            child_high_task: r.child_high_task,
            child_low_region: r.child_low_region,
            child_high_region: r.child_high_region,
            changed: r.changed,
            applied: Applied {
                term: r.applied_term,
                index: r.applied_index,
            },
        })
    }

    /// Abort the long-running intent `task` names (docs/RANGE-SPLIT.md §12).
    pub async fn abort_task(&self, task: u64) -> Result<Aborted, Error> {
        let r = self
            .invoke(false, proto::AbortTaskRequest { task }, |mut c, r| async move { c.abort_task(r).await })
            .await?;
        Ok(Aborted {
            target: r.target,
            target_task: r.target_task,
            operation: r.operation,
            abort_task: r.abort_task,
            changed: r.changed,
            applied: Applied { term: r.applied_term, index: r.applied_index },
            path: r.path,
        })
    }

    /// Commit a merge intent for the adjacent bound ranges `low` and `high` on the
    /// same stores (docs/RANGE-SPLIT.md §6); the merged child is created and
    /// activated in the same transaction and the cluster does the rest.
    pub async fn merge_ranges(
        &self,
        root: [u8; 32],
        operation: [u8; 16],
        low_region: u64,
        high_region: u64,
    ) -> Result<MergePlanned, Error> {
        let r = self
            .invoke(
                false,
                proto::MergeRangesRequest {
                    root_digest: root.to_vec(),
                    operation_id: operation.to_vec(),
                    low_region,
                    high_region,
                },
                |mut c, r| async move { c.merge_ranges(r).await },
            )
            .await?;
        Ok(MergePlanned {
            intent_task: r.intent_task,
            child_task: r.child_task,
            child_region: r.child_region,
            changed: r.changed,
            applied: Applied {
                term: r.applied_term,
                index: r.applied_index,
            },
        })
    }

    /// Commit the intent that `node`'s current store leaves the cluster
    /// (docs/DRAIN-NODE.md). Idempotent per store incarnation; the cluster does
    /// the moves. Poll [`Client::list_nodes`] for `drained`.
    pub async fn drain_node(&self, node: u64) -> Result<DrainPlanned, Error> {
        let r = self
            .invoke(false, proto::DrainNodeRequest { node_id: node }, |mut c, r| async move {
                c.drain_node(r).await
            })
            .await?;
        Ok(DrainPlanned {
            task: r.task,
            operation: r.operation,
            changed: r.changed,
            applied: Applied {
                term: r.applied_term,
                index: r.applied_index,
            },
        })
    }

    /// Commit the backup intent of a keyspace (docs/BACKUP-RESTORE.md §1); the
    /// cluster captures every range and writes the bundle. Poll
    /// [`Client::list_backups`] for `complete`.
    pub async fn backup_keyspace(&self, keyspace_id: u32, label: &str) -> Result<BackupPlanned, Error> {
        let r = self
            .invoke(
                false,
                proto::BackupKeyspaceRequest { keyspace_id, label: label.to_owned() },
                |mut c, r| async move { c.backup_keyspace(r).await },
            )
            .await?;
        Ok(BackupPlanned {
            task: r.task,
            operation: r.operation,
            changed: r.changed,
            applied: Applied { term: r.applied_term, index: r.applied_index },
            sequence: r.sequence,
            regions: r.regions,
        })
    }

    /// Every committed backup with its stage, anchors and bundle; retired ones
    /// only with `include_retired`.
    pub async fn list_backups(&self, include_retired: bool) -> Result<Vec<BackupRow>, Error> {
        let r = self
            .invoke(true, proto::ListBackupsRequest { include_retired }, |mut c, r| async move {
                c.list_backups(r).await
            })
            .await?;
        Ok(r
            .backups
            .into_iter()
            .map(|b| BackupRow {
                task: b.task,
                operation: b.operation,
                keyspace_id: b.keyspace_id,
                tenant_id: b.tenant_id,
                label: b.label,
                sequence: b.sequence,
                requested_unix: b.requested_unix,
                stage: b.stage,
                ranges: b
                    .ranges
                    .into_iter()
                    .map(|r| BackupRangeRow {
                        region: r.region,
                        creation_task: r.creation_task,
                        bind_task: r.bind_task,
                        operation: r.operation,
                        anchor: r.anchor.map(|a| BackupAnchorRow {
                            task: a.task,
                            cut: Applied { term: a.cut_term, index: a.cut_index },
                            image_digest: a.image_digest,
                            objects: a.objects,
                            object_bytes: a.object_bytes,
                            rows: a.rows,
                            rows_digest: a.rows_digest,
                            restorable: a.restorable,
                        }),
                    })
                    .collect(),
                bundle_key: b.bundle_key,
                bundle_digest: b.bundle_digest,
                bundle_bytes: b.bundle_bytes,
                completed_unix: b.completed_unix,
                retired_unix: b.retired_unix,
                restorable: b.restorable,
            })
            .collect())
    }

    /// Retire a complete backup: the only release of its pins.
    pub async fn retire_backup(&self, task: u64) -> Result<BackupRetired, Error> {
        let r = self
            .invoke(false, proto::RetireBackupRequest { task }, |mut c, r| async move {
                c.retire_backup(r).await
            })
            .await?;
        Ok(BackupRetired {
            retirement_task: r.retirement_task,
            changed: r.changed,
            applied: Applied { term: r.applied_term, index: r.applied_index },
        })
    }

    /// GET-verify a complete backup's bundle and every range's object closure.
    pub async fn verify_backup(&self, task: u64) -> Result<BackupVerdict, Error> {
        let r = self
            .invoke(true, proto::VerifyBackupRequest { task }, |mut c, r| async move {
                c.verify_backup(r).await
            })
            .await?;
        Ok(BackupVerdict {
            task: r.task,
            bundle_key: r.bundle_key,
            bundle_ok: r.bundle_ok,
            ranges: r
                .ranges
                .into_iter()
                .map(|v| BackupRangeVerdict { region: v.region, ok: v.ok, detail: v.detail })
                .collect(),
            ok: r.ok,
        })
    }

    /// Commit the restore of a complete backup as a NEW keyspace
    /// (docs/BACKUP-RESTORE.md §5). Empty `voters` lets placement choose. Poll
    /// [`Client::list_restores`] for `published`.
    pub async fn restore_keyspace(&self, backup_task: u64, name: &str, tenant_id: u64, voters: &[u64]) -> Result<RestorePlanned, Error> {
        let r = self
            .invoke(
                false,
                proto::RestoreKeyspaceRequest { backup_task, name: name.to_owned(), tenant_id, voters: voters.to_vec() },
                |mut c, r| async move { c.restore_keyspace(r).await },
            )
            .await?;
        Ok(RestorePlanned {
            task: r.task,
            operation: r.operation,
            changed: r.changed,
            applied: Applied { term: r.applied_term, index: r.applied_index },
            keyspace_id: r.keyspace_id,
            target_regions: r.target_regions,
        })
    }

    /// Every committed restore with its stage and targets.
    pub async fn list_restores(&self) -> Result<Vec<RestoreRow>, Error> {
        let r = self
            .invoke(true, proto::ListRestoresRequest {}, |mut c, r| async move { c.list_restores(r).await })
            .await?;
        Ok(r
            .restores
            .into_iter()
            .map(|x| RestoreRow {
                task: x.task,
                operation: x.operation,
                backup_task: x.backup_task,
                keyspace_id: x.keyspace_id,
                tenant_id: x.tenant_id,
                name: x.name,
                requested_unix: x.requested_unix,
                stage: x.stage,
                targets: x
                    .targets
                    .into_iter()
                    .map(|t| RestoreTargetRow {
                        source_region: t.source_region,
                        creation_task: t.creation_task,
                        region: t.region,
                        start: t.start,
                        end: t.end,
                        evidence_task: t.evidence_task,
                        rows: t.rows,
                    })
                    .collect(),
            })
            .collect())
    }

    pub async fn node_endpoint(&self, node_id: u64) -> Result<Option<NodeEndpoint>, Error> {
        let r = self
            .invoke(
                true,
                proto::GetNodeEndpointRequest { node_id },
                |mut c, r| async move { c.get_node_endpoint(r).await },
            )
            .await?;
        Ok(r.endpoint.map(|e| NodeEndpoint {
            node_id: e.node_id,
            store_incarnation: e.store_incarnation,
            address: e.address,
            generation: e.generation,
            previous_address: e.previous_address,
            active: e.active,
        }))
    }

    /// Admit `node_id` at `addr` as a learner of the metadata group. The returned
    /// join ticket is shown exactly once; the catalog keeps only its digest.
    pub async fn admit_node(
        &self,
        node_id: u64,
        addr: &str,
        ttl_seconds: u64,
    ) -> Result<MembershipChange, Error> {
        let r = self
            .invoke(
                false,
                proto::AdmitNodeRequest {
                    node_id,
                    addr: addr.to_owned(),
                    ttl_seconds,
                },
                |mut c, r| async move { c.admit_node(r).await },
            )
            .await?;
        Ok(membership(r))
    }

    pub async fn revoke_admission(&self, node_id: u64) -> Result<MembershipChange, Error> {
        let r = self
            .invoke(
                false,
                proto::RevokeAdmissionRequest { node_id },
                |mut c, r| async move { c.revoke_admission(r).await },
            )
            .await?;
        Ok(membership(r))
    }

    pub async fn promote_node(&self, node_id: u64) -> Result<MembershipChange, Error> {
        let r = self
            .invoke(
                false,
                proto::PromoteNodeRequest { node_id },
                |mut c, r| async move { c.promote_node(r).await },
            )
            .await?;
        Ok(membership(r))
    }

    /// Hand leadership of `region_id` (1 = the metadata group) to a caught-up voter.
    /// Leader-only (follows the hint); `completed == false` means raft abandoned the
    /// transfer and the previous leader leads again.
    pub async fn transfer_leader(&self, region_id: u64, transferee: u64) -> Result<LeaderTransfer, Error> {
        let r = self
            .invoke(
                false,
                proto::TransferLeaderRequest { region_id, transferee },
                |mut c, r| async move { c.transfer_leader(r).await },
            )
            .await?;
        Ok(LeaderTransfer {
            previous_leader: r.previous_leader,
            leader: (r.leader != 0).then_some(r.leader),
            term: r.term,
            completed: r.completed,
        })
    }

    /// Remove a voter. The server refuses below three voters and never removes the
    /// leader itself; promote a replacement first.
    pub async fn remove_node(&self, node_id: u64) -> Result<MembershipChange, Error> {
        let r = self
            .invoke(
                false,
                proto::RemoveNodeRequest { node_id },
                |mut c, r| async move { c.remove_node(r).await },
            )
            .await?;
        Ok(membership(r))
    }
}

// ------------------------------------------------------------------ data groups and tasks

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupReplica {
    pub node_id: u64,
    pub store_incarnation: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupKeyspace {
    pub keyspace_id: u32,
    pub tenant_id: u64,
    pub bind_task: u64,
    pub conf_ver: u64,
    pub version: u64,
    pub start: Vec<u8>,
    pub end: Vec<u8>,
    pub sealed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupMigration {
    pub task: u64,
    pub operation: Vec<u8>,
    pub destination: DataGroupReplica,
    pub evidence_task: u64,
    pub abort_task: u64,
    pub removal_task: u64,
    pub truncation_task: u64,
}

/// One committed replica plan (kind 111) of a group and its catalog-only stage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupPlan {
    pub task: u64,
    pub operation: Vec<u8>,
    pub migration_task: u64,
    /// `add` or `move`.
    pub mode: String,
    pub destination: DataGroupReplica,
    pub source: Option<DataGroupReplica>,
    /// `planned`, `captured`, `pinned`, `evidenced`, `decided` or `aborted`.
    pub stage: String,
}

/// One committed data group as the metadata leader sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroup {
    pub creation_task: u64,
    pub region_id: u64,
    pub operation: Vec<u8>,
    pub replicas: Vec<DataGroupReplica>,
    pub activated: bool,
    pub keyspaces: Vec<DataGroupKeyspace>,
    pub migrations: Vec<DataGroupMigration>,
    pub compaction_floor: Option<Applied>,
    pub plans: Vec<DataGroupPlan>,
    /// Committed membership now: initial replicas plus evidenced destinations,
    /// minus retired sources. Empty only from a server older than this field.
    pub members: Vec<DataGroupReplica>,
    /// Committed split intents whose parent is this group (docs/RANGE-SPLIT.md).
    pub splits: Vec<DataGroupSplit>,
    /// Committed merge intents in which this group is a parent (docs/RANGE-SPLIT.md §6).
    pub merges: Vec<DataGroupMerge>,
}

/// One committed merge intent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupMerge {
    pub task: u64,
    pub operation: Vec<u8>,
    pub low_region: u64,
    pub high_region: u64,
    pub child_task: u64,
    pub child_region: u64,
    /// `intended`, `populated`, `published`.
    pub stage: String,
    pub evidence_task: u64,
    pub evidence_digest: Vec<u8>,
    pub evidence_rows: u64,
}

/// The receipt of [`Client::abort_task`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aborted {
    /// `split`, `merge`, `drain` or `plan`.
    pub target: String,
    pub target_task: u64,
    pub operation: Vec<u8>,
    pub abort_task: u64,
    pub changed: bool,
    pub applied: Applied,
    pub path: String,
}

/// The receipt of [`Client::merge_ranges`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePlanned {
    pub intent_task: u64,
    pub child_task: u64,
    pub child_region: u64,
    pub changed: bool,
    pub applied: Applied,
}

/// [`Client::list_data_groups_detailed`]: the groups and the leader's failed readbacks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupListing {
    pub groups: Vec<DataGroup>,
    pub readback_errors: Vec<String>,
}

/// One committed split intent of a group.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupSplit {
    pub task: u64,
    pub operation: Vec<u8>,
    pub split_key: Vec<u8>,
    pub child_low_task: u64,
    pub child_high_task: u64,
    pub child_low_region: u64,
    pub child_high_region: u64,
    /// `intended`, `populating`, `populated`, later `published`.
    pub stage: String,
    pub evidence: Vec<DataGroupSplitEvidence>,
}

/// One child's verified copy of the sealed parent half (kind 114).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGroupSplitEvidence {
    pub task: u64,
    pub child_task: u64,
    pub digest: Vec<u8>,
    pub rows: u64,
}

/// The receipt of [`Client::split_range`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitPlanned {
    pub intent_task: u64,
    pub child_low_task: u64,
    pub child_high_task: u64,
    pub child_low_region: u64,
    pub child_high_region: u64,
    /// `false` confirms an identical committed intent.
    pub changed: bool,
    pub applied: Applied,
}

impl DataGroup {
    /// The nodes that hold the group today: `members`, or the initial replicas
    /// when the server does not report membership.
    pub fn current_members(&self) -> &[DataGroupReplica] {
        if self.members.is_empty() {
            &self.replicas
        } else {
            &self.members
        }
    }
}

/// The receipt of [`Client::add_replica`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaPlanned {
    pub plan_task: u64,
    pub migration_task: u64,
    pub region_id: u64,
    /// `false` confirms an identical committed plan.
    pub changed: bool,
    pub applied: Applied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataGroupCreated {
    pub creation_task: u64,
    pub region_id: u64,
    /// `false` is a confirmation of an identical earlier request.
    pub changed: bool,
    pub applied: Applied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataKeyspaceCreated {
    pub keyspace_id: u32,
    pub region_id: u64,
    pub changed: bool,
    pub applied: Applied,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskRow {
    pub id: u64,
    pub kind: u64,
    pub kind_name: String,
    pub state: u64,
    pub created_unix: u64,
    pub payload_bytes: u64,
    pub operation: Vec<u8>,
    pub reference_task: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskPage {
    pub tasks: Vec<TaskRow>,
    /// Pass back as `after` for the next page; 0 means exhausted.
    pub next_after: u64,
}

fn replica(r: Option<proto::DataGroupReplica>) -> DataGroupReplica {
    r.map(|r| DataGroupReplica {
        node_id: r.node_id,
        store_incarnation: r.store_incarnation,
    })
    .unwrap_or_default()
}

impl Client {
    /// Create (or confirm) an empty data group on exactly 3, 5 or 7 registered
    /// stores. `root` is the certified metadata root digest (status `root_digest`),
    /// `operation` the immutable 16-byte retry identity.
    pub async fn create_data_group(
        &self,
        root: [u8; 32],
        operation: [u8; 16],
        voters: Vec<u64>,
    ) -> Result<DataGroupCreated, Error> {
        let r = self
            .invoke(
                false,
                proto::CreateDataGroupRequest {
                    root_digest: root.to_vec(),
                    operation_id: operation.to_vec(),
                    voters,
                },
                |mut c, r| async move { c.create_data_group(r).await },
            )
            .await?;
        Ok(DataGroupCreated {
            creation_task: r.creation_task,
            region_id: r.region_id,
            changed: r.changed,
            applied: Applied {
                term: r.applied_term,
                index: r.applied_index,
            },
        })
    }

    /// Create (or confirm) a raw keyspace bound to the data group `creation_task`.
    pub async fn create_data_keyspace(
        &self,
        root: [u8; 32],
        creation_task: u64,
        name: &str,
        tenant_id: u64,
    ) -> Result<DataKeyspaceCreated, Error> {
        let r = self
            .invoke(
                false,
                proto::CreateDataKeyspaceRequest {
                    root_digest: root.to_vec(),
                    creation_task,
                    name: name.to_owned(),
                    tenant_id,
                },
                |mut c, r| async move { c.create_data_keyspace(r).await },
            )
            .await?;
        Ok(DataKeyspaceCreated {
            keyspace_id: r.keyspace_id,
            region_id: r.region_id,
            changed: r.changed,
            applied: Applied {
                term: r.applied_term,
                index: r.applied_index,
            },
        })
    }

    /// Every committed data group, linearizably, from the metadata leader. Readback
    /// errors are dropped here; [`Client::list_data_groups_detailed`] keeps them.
    pub async fn list_data_groups(&self) -> Result<Vec<DataGroup>, Error> {
        Ok(self.list_data_groups_detailed().await?.groups)
    }

    /// The listing plus every catalog readback that failed while the leader built
    /// it (a row family that no longer validates): operators must see those.
    pub async fn list_data_groups_detailed(&self) -> Result<DataGroupListing, Error> {
        let r = self
            .invoke(true, proto::ListDataGroupsRequest {}, |mut c, r| async move {
                c.list_data_groups(r).await
            })
            .await?;
        let readback_errors = r.readback_errors;
        let groups = r.groups
            .into_iter()
            .map(|g| DataGroup {
                creation_task: g.creation_task,
                region_id: g.region_id,
                operation: g.operation,
                replicas: g.replicas.into_iter().map(|r| replica(Some(r))).collect(),
                members: g.members.into_iter().map(|r| replica(Some(r))).collect(),
                splits: g
                    .splits
                    .into_iter()
                    .map(|s| DataGroupSplit {
                        task: s.task,
                        operation: s.operation,
                        split_key: s.split_key,
                        child_low_task: s.child_low_task,
                        child_high_task: s.child_high_task,
                        child_low_region: s.child_low_region,
                        child_high_region: s.child_high_region,
                        stage: s.stage,
                        evidence: s
                            .evidence
                            .into_iter()
                            .map(|e| DataGroupSplitEvidence {
                                task: e.task,
                                child_task: e.child_task,
                                digest: e.digest,
                                rows: e.rows,
                            })
                            .collect(),
                    })
                    .collect(),
                merges: g
                    .merges
                    .into_iter()
                    .map(|m| DataGroupMerge {
                        task: m.task,
                        operation: m.operation,
                        low_region: m.low_region,
                        high_region: m.high_region,
                        child_task: m.child_task,
                        child_region: m.child_region,
                        stage: m.stage,
                        evidence_task: m.evidence_task,
                        evidence_digest: m.evidence_digest,
                        evidence_rows: m.evidence_rows,
                    })
                    .collect(),
                activated: g.activated,
                keyspaces: g
                    .keyspaces
                    .into_iter()
                    .map(|k| DataGroupKeyspace {
                        keyspace_id: k.keyspace_id,
                        tenant_id: k.tenant_id,
                        bind_task: k.bind_task,
                        conf_ver: k.conf_ver,
                        version: k.version,
                        start: k.start,
                        end: k.end,
                        sealed: k.sealed,
                    })
                    .collect(),
                migrations: g
                    .migrations
                    .into_iter()
                    .map(|m| DataGroupMigration {
                        task: m.task,
                        operation: m.operation,
                        destination: replica(m.destination),
                        evidence_task: m.evidence_task,
                        abort_task: m.abort_task,
                        removal_task: m.removal_task,
                        truncation_task: m.truncation_task,
                    })
                    .collect(),
                compaction_floor: match (g.compaction_floor_term, g.compaction_floor_index) {
                    (Some(term), Some(index)) => Some(Applied { term, index }),
                    _ => None,
                },
                plans: g
                    .plans
                    .into_iter()
                    .map(|p| DataGroupPlan {
                        task: p.task,
                        operation: p.operation,
                        migration_task: p.migration_task,
                        mode: p.mode,
                        destination: replica(p.destination),
                        source: p.source.map(|s| replica(Some(s))),
                        stage: p.stage,
                    })
                    .collect(),
            })
            .collect();
        Ok(DataGroupListing { groups, readback_errors })
    }

    /// Commit one replica plan: `mode` is `"add"` (grow the group by `destination`) or
    /// `"move"` (grow, then retire `source`, an initial replica of the group). The
    /// cluster drives the rest by itself (docs/REPLICA-ADDITION.md).
    pub async fn add_replica(
        &self,
        root: [u8; 32],
        operation: [u8; 16],
        creation_task: u64,
        destination: u64,
        mode: &str,
        source: Option<u64>,
    ) -> Result<ReplicaPlanned, Error> {
        let r = self
            .invoke(
                false,
                proto::AddReplicaRequest {
                    root_digest: root.to_vec(),
                    operation_id: operation.to_vec(),
                    creation_task,
                    destination_node: destination,
                    mode: mode.to_owned(),
                    source_node: source.unwrap_or(0),
                },
                |mut c, r| async move { c.add_replica(r).await },
            )
            .await?;
        Ok(ReplicaPlanned {
            plan_task: r.plan_task,
            migration_task: r.migration_task,
            region_id: r.region_id,
            changed: r.changed,
            applied: Applied {
                term: r.applied_term,
                index: r.applied_index,
            },
        })
    }

    /// One page of the committed task history after `after` (0 = start), at most
    /// `limit` rows (1..=256), optionally one `kind` (0 = all).
    pub async fn list_tasks(&self, after: u64, limit: u32, kind: u64) -> Result<TaskPage, Error> {
        let r = self
            .invoke(
                true,
                proto::ListTasksRequest { after, limit, kind },
                |mut c, r| async move { c.list_tasks(r).await },
            )
            .await?;
        Ok(TaskPage {
            tasks: r
                .tasks
                .into_iter()
                .map(|t| TaskRow {
                    id: t.id,
                    kind: t.kind,
                    kind_name: t.kind_name,
                    state: t.state,
                    created_unix: t.created_unix,
                    payload_bytes: t.payload_bytes,
                    operation: t.operation,
                    reference_task: t.reference_task,
                })
                .collect(),
            next_after: r.next_after,
        })
    }
}
