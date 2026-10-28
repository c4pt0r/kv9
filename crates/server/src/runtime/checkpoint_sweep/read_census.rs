//! Image admission and lifecycle censuses independent of attempt settlement.
use super::*;

/// Commit at most one census page in the caller's existing maintenance slot.
/// A closed read fence only authorizes this observation, never owner release.
pub(super) fn pass(
    backend: &RuntimeBackend,
    ledger: &impl CheckpointLedger,
    root: &RootDescriptor,
    pending: OwnerId,
    check: &impl Fn() -> Result<()>,
    stats: &mut Stats,
) -> Result<bool> {
    check()?;
    let view = backend.node.meta_raft.store.begin()?.into_view();
    use kv9_meta::retention::metadata_image_write_gate::{self, Work as GateWork};
    let gate = metadata_image_write_gate::observe(view.as_ref(), root, pending)?;
    drop(view);
    match gate {
        GateWork::Ready => {
            check()?;
            ledger.apply(encode_request(
                root.digest(),
                &LedgerRequest::CloseMetadataImageWrites(kv9_common::retention::OwnerToken {
                    owner: pending,
                    generation: std::num::NonZeroU64::new(1).unwrap(),
                }),
            )?)?;
            stats.metadata_image_write_gates_written =
                stats.metadata_image_write_gates_written.saturating_add(1);
            return Ok(true);
        }
        GateWork::Closed(_) => {
            stats.metadata_image_write_gates_closed =
                stats.metadata_image_write_gates_closed.saturating_add(1)
        }
        GateWork::Open => {}
    }
    // Both censuses use the existing maintenance slot. A registered
    // closure is an observation; Pending and Version stay protected.
    check()?;
    let view = backend.node.meta_raft.store.begin()?.into_view();
    use kv9_meta::retention::reader_closure::{self, Work as ReaderWork};
    let work = reader_closure::observe(view.as_ref(), root, pending)?;
    drop(view);
    let closed = work.registered_generations_closed();
    match work {
        ReaderWork::Advance { cursor, restart } => {
            check()?;
            ledger.apply(encode_request(root.digest(), &cursor.request(pending))?)?;
            stats.reader_closure_pages = stats.reader_closure_pages.saturating_add(1);
            stats.reader_closure_restarts = stats
                .reader_closure_restarts
                .saturating_add(u64::from(restart));
            return Ok(true);
        }
        ReaderWork::Complete(_) => {
            stats.reader_closure_closed = stats
                .reader_closure_closed
                .saturating_add(u64::from(closed));
            stats.reader_closure_blocked = stats
                .reader_closure_blocked
                .saturating_add(u64::from(!closed));
        }
        ReaderWork::Open | ReaderWork::WaitingForCatalog => {}
    }
    check()?;
    let view = backend.node.meta_raft.store.begin()?.into_view();
    use kv9_meta::retention::bootstrap_closure::{self, Work as BootstrapWork};
    let work = bootstrap_closure::observe(view.as_ref(), root, pending)?;
    drop(view);
    let retired = work.registered_grants_retired();
    match work {
        BootstrapWork::Advance { cursor, restart } => {
            check()?;
            ledger.apply(encode_request(root.digest(), &cursor.request(pending))?)?;
            stats.bootstrap_closure_pages = stats.bootstrap_closure_pages.saturating_add(1);
            stats.bootstrap_closure_restarts = stats
                .bootstrap_closure_restarts
                .saturating_add(u64::from(restart));
            return Ok(true);
        }
        BootstrapWork::Complete(_) => {
            stats.bootstrap_closure_retired = stats
                .bootstrap_closure_retired
                .saturating_add(u64::from(retired));
            stats.bootstrap_closure_blocked = stats
                .bootstrap_closure_blocked
                .saturating_add(u64::from(!retired));
        }
        BootstrapWork::Open | BootstrapWork::WaitingForCatalog => {}
    }
    check()?;
    let view = backend.node.meta_raft.store.begin()?.into_view();
    use kv9_meta::retention::image_reader_closure::{self, Work as ImageWork};
    let work = image_reader_closure::observe(view.as_ref(), root, pending)?;
    drop(view);
    let closed = work.registered_generations_closed();
    match work {
        ImageWork::Advance { cursor, restart } => {
            check()?;
            ledger.apply(encode_request(root.digest(), &cursor.request(pending))?)?;
            stats.image_reader_closure_pages = stats.image_reader_closure_pages.saturating_add(1);
            stats.image_reader_closure_restarts = stats
                .image_reader_closure_restarts
                .saturating_add(u64::from(restart));
            return Ok(true);
        }
        ImageWork::Complete(_) => {
            stats.image_reader_closure_closed = stats
                .image_reader_closure_closed
                .saturating_add(u64::from(closed));
            stats.image_reader_closure_blocked = stats
                .image_reader_closure_blocked
                .saturating_add(u64::from(!closed));
        }
        ImageWork::Open | ImageWork::WaitingForCatalog => {}
    }
    check()?;
    let view = backend.node.meta_raft.store.begin()?.into_view();
    use kv9_meta::retention::image_publisher_closure::{self, Work as PublisherWork};
    let work = image_publisher_closure::observe(view.as_ref(), root, pending)?;
    drop(view);
    let closed = work.registered_zero_put_closed();
    match work {
        PublisherWork::Advance { cursor, restart } => {
            check()?;
            ledger.apply(encode_request(root.digest(), &cursor.request(pending))?)?;
            stats.image_publisher_closure_pages =
                stats.image_publisher_closure_pages.saturating_add(1);
            stats.image_publisher_closure_restarts = stats
                .image_publisher_closure_restarts
                .saturating_add(u64::from(restart));
            return Ok(true);
        }
        PublisherWork::Complete(_) => {
            stats.image_publisher_closure_zero_put = stats
                .image_publisher_closure_zero_put
                .saturating_add(u64::from(closed));
            stats.image_publisher_closure_blocked = stats
                .image_publisher_closure_blocked
                .saturating_add(u64::from(!closed));
        }
        PublisherWork::Open | PublisherWork::WaitingForCatalog => {}
    }
    Ok(false)
}
