//! Local, bounded group demultiplexing on the node's shared peer streams.
//!
//! Registration is explicit and append-only for this first D01 increment. It
//! is not catalog authority, durable creation, or permission to reuse a retired
//! region ID. Those lifecycle transitions belong to the RegionManager.

use super::*;
use crate::work::{RaftInbox, WorkSignal};
use kv9_common::{RegionId, META_REGION_0};
use std::sync::atomic::AtomicBool;

/// Initial hard ceiling, including metadata. Per-group inbox limits still
/// apply; this is not a substitute for the future node-wide byte budget.
const MAX_LOCAL_GROUPS: usize = 256;

pub struct RegionInboxes {
    metadata: RaftInbox,
    inboxes: Mutex<HashMap<u64, RaftInbox>>,
}

impl RegionInboxes {
    pub(super) fn new(metadata: RaftInbox) -> Self {
        // Existing V2 nodes send metadata as wire ID 0, although its catalog
        // ID is 1. Preserve that wire encoding and reserve both IDs.
        Self {
            metadata,
            inboxes: Mutex::new(HashMap::new()),
        }
    }

    fn register(&self, region: RegionId) -> kv9_common::Result<RaftInbox> {
        let mut inboxes = self.inboxes.lock().expect("region inboxes poisoned");
        if region.0 == 0 || region == META_REGION_0 {
            return Err(Error::Config("metadata transport IDs are reserved".into()));
        }
        if inboxes.contains_key(&region.0) {
            return Err(Error::Config("region transport already registered".into()));
        }
        if inboxes.len() >= MAX_LOCAL_GROUPS - 1 {
            return Err(Error::Config("local region transport limit reached".into()));
        }
        let inbox = RaftInbox::default();
        inboxes.insert(region.0, inbox.clone());
        Ok(inbox)
    }

    pub(super) fn send(&self, region: u64, message: Message) -> bool {
        // Preserve the metadata path without a registry lookup or data-group
        // lock; registration and a hot group cannot hold up this admission.
        if region == 0 {
            return self.metadata.send(message).is_ok();
        }
        // Release the registry lock before acquiring a queue or notifying a
        // driver. No Raft, persistence, network I/O or await under this lock.
        let inbox = self
            .inboxes
            .lock()
            .expect("region inboxes poisoned")
            .get(&region)
            .cloned();
        inbox.is_some_and(|inbox| inbox.send(message).is_ok())
    }
}

/// One group's inbox and wakeup; peer connections and endpoint generations
/// remain owned by the shared node transport. A handle can bind only one driver.
pub struct GroupTransport {
    shared: Arc<GrpcTransport>,
    region: RegionId,
    inbox: RaftInbox,
    bound: AtomicBool,
}

impl GrpcTransport {
    /// Install this router in the node's one Raft service before starting data
    /// groups. Unknown groups are dropped, never lazily created by wire traffic.
    pub fn inbound_router(&self) -> Arc<RegionInboxes> {
        self.regions.clone()
    }

    /// Allocate an independent local inbox without a new connection or thread.
    /// Call only after the owner has established durable creation authority.
    /// This infrastructure API does not create or publish a serving region.
    pub fn register_group(
        self: &Arc<Self>,
        region: RegionId,
    ) -> kv9_common::Result<Arc<GroupTransport>> {
        let inbox = self.regions.register(region)?;
        Ok(Arc::new(GroupTransport {
            shared: self.clone(),
            region,
            inbox,
            bound: AtomicBool::new(false),
        }))
    }
}

impl RaftTransport for GroupTransport {
    fn bind_driver(&self, region: RegionId, signal: Arc<WorkSignal>) -> kv9_common::Result<()> {
        if region != self.region {
            return Err(Error::Config("driver and transport region mismatch".into()));
        }
        if self
            .bound
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(Error::Config(
                "region transport already bound to a driver".into(),
            ));
        }
        self.inbox.set_signal(signal);
        Ok(())
    }

    fn send(&self, to: NodeId, message: Message) {
        self.shared.send_region(self.region.0, to, message);
    }

    fn drain(&self) -> Vec<Message> {
        self.shared.drain_inbox(&self.inbox)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_group_registration_is_bounded_unique_and_reserves_metadata() {
        let metadata = RaftInbox::default();
        let router = RegionInboxes::new(metadata.clone());
        assert!(router.register(RegionId(0)).is_err());
        assert!(router.register(META_REGION_0).is_err());
        let original = router.register(RegionId(2)).unwrap();
        assert!(router.register(RegionId(2)).is_err());
        assert!(router.send(
            2,
            Message {
                term: 7,
                ..Default::default()
            }
        ));
        assert_eq!(
            original.drain()[0].term,
            7,
            "duplicate registration must not replace the inbox"
        );
        for id in 3..=MAX_LOCAL_GROUPS as u64 {
            router.register(RegionId(id)).unwrap();
        }
        assert!(
            router
                .register(RegionId(MAX_LOCAL_GROUPS as u64 + 1))
                .is_err(),
            "local group cap must be enforced"
        );
        assert!(
            !router.send(999, Message::default()),
            "wire traffic must not create groups"
        );
        assert!(
            metadata.drain().is_empty(),
            "unknown groups must not fall back to metadata"
        );
    }

    #[test]
    fn multi_group_legacy_inbox_refuses_nonmetadata_envelopes() {
        let inbox = RaftInbox::default();
        let sender = InboundSender::from(inbox.clone());
        assert!(
            !sender.send(2, Message::default()),
            "legacy inbox must not consume data-group traffic"
        );
        assert!(sender.send(0, Message::default()));
        assert_eq!(inbox.drain().len(), 1);
    }

    #[test]
    fn multi_group_driver_binding_rejects_wrong_group_and_second_consumer() {
        use crate::{driver::NodeDriver, MemStateMachine, RaftPeer};
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = GrpcTransport::new(
            NodeId(1),
            None,
            rt.handle().clone(),
            RootDigest::from_bytes([0; 32]),
        );
        let group = shared.register_group(RegionId(10)).unwrap();
        let make_peer = |region| Arc::new(RaftPeer::new(NodeId(1), region, &[NodeId(1)]).unwrap());
        let wrong = NodeDriver::new(
            make_peer(RegionId(11)),
            group.clone(),
            MemStateMachine::new(),
        );
        assert!(
            matches!(wrong, Err(Error::Config(ref s)) if s == "driver and transport region mismatch")
        );
        let _first = NodeDriver::new(
            make_peer(RegionId(10)),
            group.clone(),
            MemStateMachine::new(),
        )
        .unwrap();
        let second = NodeDriver::new(make_peer(RegionId(10)), group, MemStateMachine::new());
        assert!(
            matches!(second, Err(Error::Config(ref s)) if s == "region transport already bound to a driver")
        );
    }
}
