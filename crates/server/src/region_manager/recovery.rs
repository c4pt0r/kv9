//! Completed-open facts stay with the owning group, not an independently keyed
//! cache that could outlive replacement or retirement. Current selection is an
//! on-demand local observation; status output only emits the fixed startup facts.
use super::*;
use kv9_common::AppliedPosition;
use kv9_raft::state_machine::checkpoint_recovery::data_group::{RecoveryImage, RecoveryImageState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverySelectionRelation {
    /// No completed active-group recovery observation (for example a newly
    /// prepared group). This never means that recovery dependencies are empty.
    NoCompletedOpen,
    /// Only the selected image matches; does not certify subsequent WAL or
    /// protocol changes, current object availability, or transferable recovery.
    SameImageAsOpen,
    ChangedSinceOpen,
}

#[derive(Debug, Clone, Copy)]
pub struct GroupRecoveryStatus {
    store: StoreIdentity,
    at_open: Option<DataRecoveryDependencies>,
    selected: Option<(AppliedPosition, RootDigest)>,
    relation: RecoverySelectionRelation,
    driver_started: bool,
}
impl GroupRecoveryStatus {
    pub fn store(&self) -> StoreIdentity {
        self.store
    }
    pub fn at_open(&self) -> Option<&DataRecoveryDependencies> {
        self.at_open.as_ref()
    }
    /// Exact manifest digest and cut from the healthy live engine. Absence is
    /// distinct from failure; poisoned/unavailable engines return an error.
    pub fn selected(&self) -> Option<(AppliedPosition, RootDigest)> {
        self.selected
    }
    pub fn selection_relation(&self) -> RecoverySelectionRelation {
        self.relation
    }
    /// Ordinary recovery can precede catalog compaction admission and driver
    /// startup. Even a started driver is not public range-serving authority.
    pub fn driver_started(&self) -> bool {
        self.driver_started
    }
}

impl RegionManager {
    pub(crate) fn recovery_status(&self, region: RegionId) -> Result<GroupRecoveryStatus> {
        let Some(LocalGroup::Ready(prepared)) = self.groups.get(&region) else {
            return Err(invalid("group has no available recovery owner"));
        };
        if let Some(cause) = prepared.driver.as_ref().and_then(|d| d.status().fatal) {
            return Err(Error::Raft(cause));
        }
        // selected_checkpoint holds the history/WAL locks and checks uncertain
        // publication. No unguarded checkpoint-reference file read is allowed.
        let selected = prepared
            .engine
            .selected_checkpoint()?
            .as_ref()
            .map(|m| -> Result<_> { Ok((m.position(), RootDigest::sha256(&m.encode()?))) })
            .transpose()?;
        let relation = prepared.recovery.as_ref().map_or(
            RecoverySelectionRelation::NoCompletedOpen,
            |recovery| {
                if recovery.selected().map(|s| (s.cut(), s.manifest())) == selected {
                    RecoverySelectionRelation::SameImageAsOpen
                } else {
                    RecoverySelectionRelation::ChangedSinceOpen
                }
            },
        );
        Ok(GroupRecoveryStatus {
            store: self.identity,
            at_open: prepared.recovery,
            selected,
            relation,
            driver_started: prepared.driver.is_some(),
        })
    }
}

fn position(at: AppliedPosition) -> serde_json::Value {
    serde_json::json!({"term": at.term, "index": at.index})
}
fn image(image: &RecoveryImage) -> serde_json::Value {
    let (kind, digest) = match image.state() {
        RecoveryImageState::Bound(d) => ("bound", d),
    };
    serde_json::json!({
        "manifest": image.manifest().to_string(), "cut": position(image.cut()),
        "image_state": kind, "state_digest": digest.to_string(),
        "ttl_watermark": image.watermark().map(|w| hex(&w.encode())),
        "configuration": image.configuration().to_string(),
        "configuration_at": image.configuration_at().map(position),
        "publication": image.publication().map(|p| serde_json::json!({
            "at": position(p.at()), "generation": p.generation(), "change_id": hex(p.change_id()),
        })),
    })
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("write string");
    }
    output
}
pub(super) fn at_open_json(recovery: Option<&DataRecoveryDependencies>) -> serde_json::Value {
    let Some(recovery) = recovery else {
        return serde_json::Value::Null;
    };
    let identity = recovery.identity();
    serde_json::json!({
        "cluster": identity.cluster.to_string(), "root": identity.root.to_string(),
        "region": identity.region.0, "creation": identity.creation.to_string(),
        "recovered_through": match recovery.recovered_through() {
            DurableAppliedPosition::Volatile => Some(serde_json::json!({"error": "volatile engine"})),
            DurableAppliedPosition::AppliedNothing => None,
            DurableAppliedPosition::AppliedThrough(at) => Some(position(at)),
        },
        "protocol_base": recovery.protocol_base().map(position),
        "selected": recovery.selected().map(image),
        "installed": recovery.installed().map(|i| serde_json::json!({
            "node": i.store().node_id.0, "store_incarnation": i.store().store_incarnation.to_string(),
            "generation": i.generation().to_string(), "image_record": i.image_record().to_string(),
            "original_image": image(i.image()),
            "engine_open_used_original_image": recovery.used_original_install_image(),
            "adoption_verified_original_image": i.verified_original_during_adoption(),
            "local_adoption_records_required": true,
        })),
    })
}
