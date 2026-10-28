//! Bounded positive-only history lookup. A missing/exhausted page never proves
//! that an attempt failed; immutable exact-window history is a separate proof.
use super::*;
use kv9_common::Error;
use kv9_engine::checkpoint::CheckpointManifest;

pub const MAX_MANIFEST_EFFECT_ROWS: u64 = 4;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManifestEffectPage {
    pub found: bool,
    /// Inclusive older generation for the next page. None means this pass
    /// exhausted its observed history, not a certified negative result.
    pub next: Option<u64>,
    pub examined: u64,
}

#[cfg(test)]
mod tests;

impl<E: ApplyStore> MemStateMachine<E> {
    /// Inspect at most four immutable history rows under the state-machine lock.
    /// Continue an older cursor even if newer generations arrive, so appenders
    /// cannot starve an old match. Once a pass ends, a later call with None can
    /// see newly appended effects. The caller retains the unresolved attempt.
    pub fn manifest_effect_page(
        &self,
        region: u64,
        intended: &[u8],
        next: Option<u64>,
    ) -> Result<ManifestEffectPage> {
        if !CheckpointManifest::is_checkpoint(intended) {
            return Ok(ManifestEffectPage::default());
        }
        let intended = CheckpointManifest::decode(intended)?;
        if intended.scope.region != region {
            return Err(Error::Raft(
                "manifest effect query names a different group".into(),
            ));
        }
        let latest = self.manifest_pair(region)?.generation;
        let first = next.unwrap_or(latest).min(latest);
        let stop = first.saturating_sub(MAX_MANIFEST_EFFECT_ROWS);
        let mut page = ManifestEffectPage::default();
        for generation in (stop + 1..=first).rev() {
            page.examined += 1;
            if let Some((term, index, bytes)) = self.manifest_at(region, generation)? {
                if CheckpointManifest::is_checkpoint(&bytes) {
                    let actual = CheckpointManifest::decode(&bytes)?;
                    if actual.scope.region != region
                        || actual.position() != (AppliedPosition { term, index })
                    {
                        return Err(Error::Raft(
                            "manifest effect history binding differs".into(),
                        ));
                    }
                    if actual.covers_effect(&intended) {
                        page.found = true;
                        return Ok(page);
                    }
                }
            }
        }
        page.next = (stop > 0).then_some(stop);
        Ok(page)
    }
}
