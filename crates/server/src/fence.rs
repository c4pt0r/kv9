//! The catalog-backed fence adjudicator (task #48 layer 2, server half).
//!
//! `Command::Fenced` carries the epoch the proposer believed the region had. Deciding
//! whether that belief still holds must happen inside *ordered apply*, not before the
//! propose: a split committed to the log but not yet applied locally is invisible to a
//! pre-propose check, and by the time the fenced write applies the split may already have
//! landed ahead of it. Inside apply the ordering is total, so the epoch read here is the
//! state established by every entry before this one.
//!
//! That is also why the verdict is a pure function of log state. Every replica applies the
//! same entries in the same order, reads the same catalog, and must reach the same answer.
//! Anything that varies per node — configuration, local IO health — may not become a
//! verdict, which is what the three-state result is for.

use std::sync::Arc;

use kv9_common::{RegionId, Result};
use kv9_engine::Engine;
use kv9_meta::tables::Tables;
use kv9_raft::{FenceAdjudicator, RegionFence};
use kv9_region::RegionEpoch;

use crate::Node;

/// Answers "is the proposer's expected region epoch still current?" from the region
/// catalog, using the same predicate the router's `check_epoch` applies on the propose
/// side so the two cannot drift.
pub struct CatalogFenceAdjudicator<E: Engine> {
    node: Arc<Node<E>>,
}

impl<E: Engine> CatalogFenceAdjudicator<E> {
    pub fn new(node: Arc<Node<E>>) -> Self {
        CatalogFenceAdjudicator { node }
    }
}

impl<E: Engine + 'static> FenceAdjudicator for CatalogFenceAdjudicator<E> {
    /// The three states are a deliberate split between what the *log* says and what this
    /// *machine* says, and conflating them is how replicas diverge.
    ///
    /// - `Ok(true)` / `Ok(false)` are log facts: the catalog is built by the same replicated
    ///   log, so every replica reads the same row and returns the same verdict.
    /// - `Err` is a local fact — this node could not read its own copy. It is not a verdict
    ///   and must not consume the log position.
    ///
    /// **A region row that is authoritatively absent is `Ok(false)`, not `Err`** (@Rafa's
    /// correction to my first classification). "This region no longer exists" is established
    /// by the log — a split consumed it — so every replica agrees, and the proposer deserves
    /// a stale-epoch receipt it can act on by refreshing its routing. Returning `Err` there
    /// would poison a node on every fenced write that races a split once M2 lands, turning
    /// ordinary concurrency into an apply failure.
    ///
    /// The two failures NOT reachable from here are the ones that made this signature
    /// `Result<bool>` instead of `bool`: a read error crushed into `false` would dress a
    /// local disk problem as a deterministic rejection and advance the watermark, while
    /// crushed into `true` it would apply an unfenced write. Either way a node whose read
    /// failed ends up in a different state from one whose read succeeded, on the same
    /// committed entry.
    fn is_fresh(&self, fence: &RegionFence) -> Result<bool> {
        // One snapshot for the read. `region_by_id_in` takes a caller-owned txn precisely so
        // the adjudicator observes the same state the apply it serves is observing.
        let txn = self.node.meta_raft.store.begin()?;
        let Some(region) = Tables::<E>::region_by_id_in(&txn, RegionId(fence.region_id))? else {
            return Ok(false);
        };

        let authoritative = RegionEpoch {
            conf_ver: region.epoch_conf,
            version: region.epoch_ver,
        };
        let proposed = RegionEpoch {
            conf_ver: fence.conf_ver,
            version: fence.version,
        };
        // Direction matters and is easy to write backwards: the *proposer's* epoch must be at
        // least as fresh as the catalog's. Same orientation as `RegionRouter::check_epoch`,
        // which does `req_epoch.is_fresh_as(&cached.epoch)`.
        Ok(proposed.is_fresh_as(&authoritative))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kv9_common::{ApiType, Config, NodeId, TenantId};
    use kv9_engine::MemEngine;

    /// A node with one keyspace, and the id + epoch of the region the catalog created.
    fn seeded_node() -> (Arc<Node<MemEngine>>, u64, u64, u64) {
        let node = Arc::new(Node::new(NodeId(1), Config::default()).unwrap());
        node.bootstrap().unwrap();
        let keyspace = node
            .create_keyspace("fenced", TenantId::DEFAULT, ApiType::Raw)
            .unwrap();
        let region = Tables::new(&node.meta_raft.store)
            .region_for_key(keyspace, b"")
            .unwrap()
            .expect("CreateKeyspace creates the initial region");
        (node, region.id.0, region.epoch_conf, region.epoch_ver)
    }

    fn fence(region_id: u64, conf_ver: u64, version: u64) -> RegionFence {
        RegionFence {
            region_id,
            conf_ver,
            version,
        }
    }

    /// The epoch the catalog holds is accepted; the comparison is against the catalog row,
    /// not against anything the caller supplied.
    #[test]
    fn the_current_epoch_is_fresh() {
        let (node, region, conf, ver) = seeded_node();
        let adj = CatalogFenceAdjudicator::new(node);
        assert!(adj.is_fresh(&fence(region, conf, ver)).unwrap());
    }

    /// An epoch older than the catalog's in EITHER component is stale.
    ///
    /// Both components are checked because they move for different reasons — `version` on
    /// split/merge, `conf_ver` on membership change (DESIGN §6.1) — so a comparison that
    /// only consulted one would accept a write fenced against the wrong half.
    #[test]
    fn an_epoch_older_in_either_component_is_stale() {
        let (node, region, conf, ver) = seeded_node();
        let adj = CatalogFenceAdjudicator::new(node);
        assert!(
            !adj.is_fresh(&fence(region, conf.saturating_sub(1), ver))
                .unwrap(),
            "a lower conf_ver must be refused"
        );
        assert!(
            !adj.is_fresh(&fence(region, conf, ver.saturating_sub(1)))
                .unwrap(),
            "a lower version must be refused"
        );
    }

    /// A region the catalog authoritatively does not have is `Ok(false)` — a verdict — and
    /// **not** `Err`.
    ///
    /// This is the boundary I originally got wrong (@Rafa's correction). "The region is
    /// gone" is established by the log: a split consumed it, and every replica reads the
    /// same absence, so it is a rejection the proposer can act on by refreshing its routing.
    /// Returning `Err` here would poison a node on every fenced write that races a split
    /// once M2 lands — ordinary concurrency reported as an apply failure. The distinction is
    /// log fact vs local fact, not "is this about epochs".
    #[test]
    fn an_absent_region_is_a_rejection_not_a_read_failure() {
        let (node, _region, conf, ver) = seeded_node();
        let adj = CatalogFenceAdjudicator::new(node);
        let verdict = adj.is_fresh(&fence(9_999_999, conf, ver));
        assert!(
            matches!(verdict, Ok(false)),
            "an absent region must be a verdict the proposer can act on, got {verdict:?}"
        );
    }

    /// The comparison must be "is the PROPOSER at least as fresh as the catalog", not the
    /// reverse. Written backwards, a stale proposer would be accepted and a fenced write
    /// would apply against a region that had already moved — the exact hole the fence
    /// exists to close, and it would pass every test that only ever uses the current epoch.
    #[test]
    fn the_comparison_is_oriented_proposer_against_catalog() {
        let (node, region, conf, ver) = seeded_node();
        let adj = CatalogFenceAdjudicator::new(node);
        // A proposer AHEAD of the catalog is still fresh (it cannot have seen a future the
        // catalog has not applied, but the predicate must not reject it either) ...
        assert!(adj.is_fresh(&fence(region, conf + 1, ver + 1)).unwrap());
        // ... while a proposer BEHIND is refused. Reversing the operands swaps these two.
        assert!(!adj
            .is_fresh(&fence(region, conf, ver.saturating_sub(1)))
            .unwrap());
    }
}

#[cfg(test)]
mod read_failure_tests {
    use super::*;
    use kv9_common::{
        ApiType, Config, NodeId, RegionId as CommonRegionId, TenantId, META_REGION_0,
    };
    use kv9_engine::testing::FaultyEngine;
    use kv9_engine::MemEngine;
    use kv9_raft::SingleNodeRaft;

    /// A node whose engine can be made to fail reads *after* the catalog is seeded, so the
    /// fixture stays realistic and only the adjudicator's own read fails.
    /// The node, its engine (for arming failures), and the seeded region's id + epoch.
    struct Armable {
        node: Arc<Node<FaultyEngine<MemEngine>>>,
        engine: Arc<FaultyEngine<MemEngine>>,
        region: u64,
        conf: u64,
        ver: u64,
    }

    fn node_with_armable_reads() -> Armable {
        let engine = Arc::new(FaultyEngine::new(MemEngine::new()));
        let raft = Arc::new(SingleNodeRaft::new(NodeId(1), META_REGION_0));
        let node = Arc::new(
            Node::with_raft_and_engine(NodeId(1), Config::default(), raft, engine.clone()).unwrap(),
        );
        node.bootstrap().unwrap();
        let keyspace = node
            .create_keyspace("fenced", TenantId::DEFAULT, ApiType::Raw)
            .unwrap();
        let region = Tables::new(&node.meta_raft.store)
            .region_for_key(keyspace, b"")
            .unwrap()
            .expect("CreateKeyspace creates the initial region");
        Armable {
            node,
            engine,
            region: region.id.0,
            conf: region.epoch_conf,
            ver: region.epoch_ver,
        }
    }

    /// A catalog read that FAILS must propagate as `Err`, never be substituted with a
    /// verdict.
    ///
    /// This is the state the whole `Result<bool>` signature exists for, and it is the one my
    /// first four tests did not touch: they all ran over `MemEngine`, where the read cannot
    /// fail, so the `?` on the catalog read was decoration. @Tess demonstrated it by turning
    /// that `?` into `.ok().flatten()` — all 47 server tests stayed green while a real read
    /// failure silently became "stale", consuming the log position.
    ///
    /// Substituting either default is a correctness bug rather than a degraded read:
    /// `false` dresses a node-local disk problem as a deterministic rejection and advances
    /// the watermark; `true` applies an unfenced write. Either way a replica whose read
    /// failed diverges from one whose read succeeded, on the same committed entry.
    #[test]
    fn a_failed_catalog_read_is_an_error_not_a_verdict() {
        let Armable {
            node,
            engine,
            region,
            conf,
            ver,
        } = node_with_armable_reads();
        let adj = CatalogFenceAdjudicator::new(node);

        // Control: the same call answers cleanly while reads are healthy, so a later Err
        // cannot be blamed on the fence being malformed.
        assert!(
            adj.is_fresh(&RegionFence {
                region_id: region,
                conf_ver: conf,
                version: ver
            })
            .unwrap(),
            "control: with reads healthy this fence is fresh"
        );

        engine.start_failing_reads();
        let verdict = adj.is_fresh(&RegionFence {
            region_id: region,
            conf_ver: conf,
            version: ver,
        });

        match verdict {
            Err(kv9_common::Error::Engine(message)) => assert!(
                message.contains("injected read failure"),
                "the underlying read failure must stay identifiable, got: {message}"
            ),
            other => panic!(
                "a catalog read failure must propagate as Err, never become a verdict, got {other:?}"
            ),
        }
    }

    /// The OTHER read failure point: the snapshot cannot be opened at all.
    ///
    /// There are two `?`s on this path — `store.begin()` and `region_by_id_in` — and they
    /// are separate failure points that a single test cannot cover. My first attempt at the
    /// read-failure test armed a switch that failed both, so it never reached the second
    /// one: @Tess's mutation of `region_by_id_in(..)?` into `.ok().flatten()` stayed green
    /// while the test passed for the wrong reason. Splitting the switch made her mutation
    /// red and left THIS path bare, which is what this test is for.
    ///
    /// Failing to open a view is as local a fact as failing to read through one, and
    /// substituting a verdict for it has the same consequence: replicas that could open a
    /// snapshot decide differently from replicas that could not.
    #[test]
    fn a_failure_to_open_the_snapshot_is_also_an_error_not_a_verdict() {
        let Armable {
            node,
            engine,
            region,
            conf,
            ver,
        } = node_with_armable_reads();
        let adj = CatalogFenceAdjudicator::new(node);
        let fence = RegionFence {
            region_id: region,
            conf_ver: conf,
            version: ver,
        };
        assert!(
            adj.is_fresh(&fence).unwrap(),
            "control: healthy before arming"
        );

        engine.start_failing_snapshots();
        let verdict = adj.is_fresh(&fence);
        match verdict {
            Err(kv9_common::Error::Engine(message)) => assert!(
                message.contains("injected snapshot failure"),
                "the snapshot failure must stay identifiable, got: {message}"
            ),
            other => panic!("failing to open a view must propagate as Err, got {other:?}"),
        }
    }

    /// The same failure, stated as a property of the outcome rather than of the call.
    ///
    /// Deliberately does NOT inspect `Ok`/`Err` (@Rafa's shape, after his own negative test
    /// reddened on `expect_err` before its watermark assertion was ever evaluated). Crushing
    /// the read failure into a verdict would leave this assertion as the only witness, so it
    /// asserts only the thing that must not happen: a failed read must not decide anything.
    #[test]
    fn a_failed_read_yields_no_verdict_at_all() {
        let Armable {
            node,
            engine,
            region,
            conf,
            ver,
        } = node_with_armable_reads();
        let adj = CatalogFenceAdjudicator::new(node);
        engine.start_failing_reads();

        let outcome = adj.is_fresh(&RegionFence {
            region_id: region,
            conf_ver: conf,
            version: ver,
        });
        assert!(
            outcome.is_err(),
            "a read failure must not produce a verdict of either polarity; got {outcome:?}"
        );

        // And it is genuinely the read that failed, not the fence: healing reads restores a
        // clean answer without touching the fence or the catalog.
        engine.stop_failing_reads();
        assert!(adj
            .is_fresh(&RegionFence {
                region_id: region,
                conf_ver: conf,
                version: ver
            })
            .unwrap());
        let _ = CommonRegionId(0);
    }
}
