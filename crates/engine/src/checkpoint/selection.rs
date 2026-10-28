use super::CheckpointManifest;
use kv9_common::{AppliedPosition, Error, Result};

impl CheckpointManifest {
    /// Local consistency only. The caller must independently establish the
    /// exact committed publication and the availability of its objects.
    pub(crate) fn check_selection(
        &self,
        applied: Option<AppliedPosition>,
        previous: Option<&Self>,
    ) -> Result<bool> {
        Self::decode(&self.encode()?)?;
        self.check_applied_position(applied)?;
        if let Some(old) = previous {
            if old.scope.cluster != self.scope.cluster || old.scope.region != self.scope.region {
                return Err(invalid("checkpoint scope changed within one stream"));
            }
            if old.index > self.index {
                return Ok(false);
            }
            if old.index == self.index && old != self {
                return Err(invalid("conflicting checkpoint at one applied index"));
            }
            if old.term > self.term {
                return Err(invalid("checkpoint term regresses"));
            }
            if old.scope.conf_ver > self.scope.conf_ver || old.scope.version > self.scope.version {
                return Err(invalid("checkpoint epoch regresses"));
            }
        }
        Ok(true)
    }

    pub(crate) fn check_applied_position(&self, applied: Option<AppliedPosition>) -> Result<()> {
        let applied = applied.ok_or_else(|| invalid("checkpoint before positioned apply"))?;
        if self.index > applied.index
            || self.term > applied.term
            || (self.index == applied.index && self.term != applied.term)
        {
            return Err(invalid(
                "checkpoint is ahead of applied position or has a conflicting term",
            ));
        }
        Ok(())
    }
}

fn invalid(message: &str) -> Error {
    Error::Engine(format!("checkpoint selection: {message}"))
}
