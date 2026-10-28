//! One-use format collection bound to one exact transfer command. A source
//! snapshot and local state checks are supplied by the driver constructors.
use super::*;
use crate::command::RawTtlSplit;
use crate::grpc::ConfirmedRawTtlFormat;
use crate::rawnode::ttl_activation::{
    PreparedRawTtlActivation, RawTtlActivationConfirmations, RawTtlActivationProbe,
};
use kv9_common::data_range::DataRange;
use kv9_common::StoreIdentity;
use std::time::Instant;

pub struct TtlSplitProbe {
    probe: RawTtlActivationProbe,
    command: RawTtlSplit,
}
pub struct TtlSplitConfirmations {
    confirmations: RawTtlActivationConfirmations,
    command: RawTtlSplit,
}
pub struct PreparedTtlSplit {
    confirmation: PreparedRawTtlActivation,
    command: RawTtlSplit,
}
impl TtlSplitProbe {
    pub fn target(&self) -> &DataRange {
        self.probe.range()
    }
    pub fn replicas(&self) -> impl ExactSizeIterator<Item = NodeId> + '_ {
        self.probe.replicas()
    }
    pub fn deadline(&self) -> Instant {
        self.probe.deadline()
    }
    pub fn bind_stores(self, identities: &[StoreIdentity]) -> Result<TtlSplitConfirmations> {
        Ok(TtlSplitConfirmations {
            confirmations: self.probe.bind_stores(identities)?,
            command: self.command,
        })
    }
}
impl TtlSplitConfirmations {
    pub fn confirm(&mut self, reply: ConfirmedRawTtlFormat) -> Result<()> {
        self.confirmations.confirm(reply)
    }
    pub fn finish(self) -> Result<PreparedTtlSplit> {
        Ok(PreparedTtlSplit {
            confirmation: self.confirmations.finish()?,
            command: self.command,
        })
    }
}
impl PreparedTtlSplit {
    pub(crate) fn command(&self) -> &RawTtlSplit {
        &self.command
    }
    pub(crate) fn deadline(&self) -> Instant {
        self.confirmation.deadline()
    }
}
impl<S: PersistentRaftStorage> RaftPeer<S> {
    pub(crate) fn begin_ttl_split_confirmation(
        &self,
        command: RawTtlSplit,
        deadline: Instant,
    ) -> Result<TtlSplitProbe> {
        Ok(TtlSplitProbe {
            probe: self.begin_raw_ttl_activation(command.target().clone(), deadline)?,
            command,
        })
    }
    pub(crate) fn propose_ttl_split_confirmed(
        &self,
        prepared: PreparedTtlSplit,
    ) -> Result<ProposedAt> {
        let command = crate::Command::RawTtlSplit(prepared.command);
        self.propose_format_confirmed(
            prepared.confirmation,
            command.encode(),
            command.requires_write_capacity(),
        )
    }
}
