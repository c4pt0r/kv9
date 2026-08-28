//! Schema versioning & migrations (METADATA-CATALOG §7).
//!
//! `schema_version` holds the current version. New binaries carry migration steps
//! `vN → vN+1` (add table / add column / add index), each run **once** as a `system`
//! transaction, gated by cluster version so a mixed-version cluster stays compatible
//! (rolling upgrade). Column adds are non-breaking by construction (tag-length row
//! encoding, [`crate::codec`]); index adds backfill in a task.

use kv9_common::{Error, Result};

use crate::schema::SCHEMA_VERSION;

/// One migration step `from → to` (METADATA-CATALOG §7).
#[derive(Debug, Clone, Copy)]
pub struct MigrationStep {
    pub from: u32,
    pub to: u32,
    /// Human description for logs / `pd-ctl`-style admin output.
    pub description: &'static str,
}

/// The ordered migration steps this binary knows how to apply (METADATA-CATALOG §7).
///
/// Empty at `v1` (the initial hardcoded schema). Later binaries append steps here; the
/// runner applies exactly the contiguous chain `persisted → SCHEMA_VERSION`.
pub const STEPS: &[MigrationStep] = &[];

/// Plan the migrations needed to move the persisted schema from `from_v` to `to_v`
/// (METADATA-CATALOG §7). Returns the contiguous chain of steps, or an error if the
/// chain is not contiguous / not known to this binary.
pub fn plan(from_v: u32, to_v: u32) -> Result<Vec<MigrationStep>> {
    if from_v == to_v {
        return Ok(Vec::new());
    }
    if from_v > to_v {
        return Err(Error::Config(format!(
            "cannot migrate backwards: {from_v} -> {to_v}"
        )));
    }
    let mut chain = Vec::new();
    let mut cur = from_v;
    while cur < to_v {
        let step = STEPS
            .iter()
            .find(|s| s.from == cur)
            .ok_or_else(|| Error::Config(format!("no migration step from v{cur}")))?;
        chain.push(*step);
        cur = step.to;
    }
    Ok(chain)
}

/// Run migrations `from_v → to_v`, each as one `system` transaction (METADATA-CATALOG §7).
///
/// Phase-1 stub: computes the plan (real, exercised) but the per-step apply — which adds
/// tables/columns/indexes and bumps the persisted `schema_version` row atomically — is
/// `unimplemented!()`. `to_v` defaults to the compiled-in [`SCHEMA_VERSION`].
pub fn migrate(from_v: u32, to_v: u32) -> Result<()> {
    let steps = plan(from_v, to_v)?;
    if steps.is_empty() {
        return Ok(());
    }
    // TODO(phase1): for each step, open a MetaStore txn, apply the DDL delta, write the
    // new schema_version row, commit; index adds enqueue a backfill task.
    unimplemented!("migrate: apply {} step(s) toward v{to_v}", steps.len())
}

/// The version this binary compiles against (METADATA-CATALOG §7).
pub fn target_version() -> u32 {
    SCHEMA_VERSION
}
