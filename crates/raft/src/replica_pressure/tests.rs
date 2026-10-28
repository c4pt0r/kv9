use super::*;
use crate::grpc::{log_pressure::ConfirmedPressure, pb};
use kv9_common::{ClusterId, RootDigest, StoreIncarnation};

fn identity(node: u64) -> StoreIdentity {
    StoreIdentity {
        cluster_id: ClusterId::from_bytes([1; 16]),
        root_digest: RootDigest::from_bytes([2; 32]),
        node_id: NodeId(node),
        store_incarnation: StoreIncarnation::from_bytes([node as u8; 16]),
    }
}
fn route(node: u64) -> PressureRoute {
    PressureRoute {
        identity: identity(node),
        address: "127.0.0.1:20001".parse().unwrap(),
        generation: 1,
    }
}
fn response(ticket: &PressureTicket, capacity: Option<u64>) -> pb::LogPressureResponse {
    let id = ticket.route.identity;
    pb::LogPressureResponse {
        root_digest: id.root_digest.as_bytes().to_vec(),
        cluster_id: id.cluster_id.as_bytes().to_vec(),
        node_id: id.node_id.0,
        store_incarnation: id.store_incarnation.as_bytes().to_vec(),
        challenge: ticket.challenge.to_vec(),
        observation: Some(match capacity {
            Some(bytes) => pb::log_pressure_response::Observation::AvailableBytes(bytes),
            None => pb::log_pressure_response::Observation::Uncertain(true),
        }),
    }
}
fn finish(cache: &ReplicaPressure, ticket: PressureTicket, capacity: Option<u64>) {
    let reply = ConfirmedPressure::from_response(&ticket, response(&ticket, capacity)).unwrap();
    cache.finish(ticket, Ok(reply));
}
fn cache() -> ReplicaPressure {
    let cache = ReplicaPressure::new(identity(1));
    cache.set_routes(vec![route(2)]).unwrap();
    cache
}
fn report(cache: &ReplicaPressure, capacity: Option<u64>) {
    finish(cache, cache.begin(NodeId(2)).unwrap().unwrap(), capacity);
}

#[test]
fn unobserved_preserves_availability_but_errors_and_missing_routes_keep_pressure() {
    let cache = cache();
    assert!(!cache.refuses([2], 100));
    report(&cache, Some(99));
    assert!(cache.refuses([2], 100));
    assert!(!cache.refuses([2], 99));
    assert!(!cache.refuses([3], 100));
    let ticket = cache.begin(NodeId(2)).unwrap().unwrap();
    cache.finish(ticket, Err(Error::MetaNotReady("timeout".into())));
    assert!(cache.refuses([2], 100));
    cache.set_routes(vec![]).unwrap();
    assert!(cache.polling_nodes().is_empty());
    assert!(cache.refuses([2], 100));
    assert_eq!(cache.snapshot()[0].errors, 1);
    cache.set_routes(vec![route(2)]).unwrap();
    report(&cache, None);
    assert!(cache.refuses([2], 1));
    assert!(cache.snapshot()[0].uncertain);
    report(&cache, Some(100));
    assert!(!cache.refuses([2], 100));
}

#[test]
fn late_out_of_order_and_moved_route_replies_cannot_clear_pressure() {
    let cache = cache();
    report(&cache, Some(0));
    let old = cache.begin(NodeId(2)).unwrap().unwrap();
    let current = cache.begin(NodeId(2)).unwrap().unwrap();
    finish(&cache, current, Some(0));
    finish(&cache, old, Some(u64::MAX));
    assert!(cache.refuses([2], 1));
    let mut expired = cache.begin(NodeId(2)).unwrap().unwrap();
    expired.issued = Instant::now() - MAX_REPLY_AGE - Duration::from_millis(1);
    finish(&cache, expired, Some(u64::MAX));
    assert!(cache.refuses([2], 1));
    let old_route = cache.begin(NodeId(2)).unwrap().unwrap();
    let mut next = route(2);
    next.generation += 1;
    next.address.set_port(20002);
    cache.set_routes(vec![next]).unwrap();
    finish(&cache, old_route, Some(u64::MAX));
    assert!(cache.refuses([2], 1));
    report(&cache, Some(100));
    assert!(!cache.refuses([2], 100));
    report(&cache, Some(0));
    let prior_disk = cache.begin(NodeId(2)).unwrap().unwrap();
    next.identity.store_incarnation = StoreIncarnation::from_bytes([9; 16]);
    next.generation = 0;
    cache.set_routes(vec![next]).unwrap();
    assert!(!cache.snapshot()[0].observed);
    finish(&cache, prior_disk, Some(0));
    assert!(!cache.snapshot()[0].observed);
}

#[test]
fn reply_validation_rejects_scope_challenge_and_missing_or_false_observation() {
    let cache = cache();
    let ticket = cache.begin(NodeId(2)).unwrap().unwrap();
    for field in 0..8 {
        let mut bad = response(&ticket, Some(100));
        match field {
            0 => bad.root_digest[0] ^= 1,
            1 => bad.cluster_id.push(0),
            2 => bad.node_id += 1,
            3 => bad.store_incarnation[0] ^= 1,
            4 => bad.challenge[0] ^= 1,
            5 => bad.observation = None,
            6 => bad.observation = Some(pb::log_pressure_response::Observation::Uncertain(false)),
            _ => bad.challenge = vec![1; 513],
        }
        assert!(
            ConfirmedPressure::from_response(&ticket, bad).is_err(),
            "field {field}"
        );
    }
}

#[test]
fn directory_updates_are_atomic_bounded_and_cannot_regress_a_disk_route() {
    let cache = cache();
    report(&cache, Some(0));
    let mut moved = route(2);
    moved.generation = 2;
    moved.address.set_port(20002);
    cache.set_routes(vec![moved]).unwrap();
    for invalid in [
        vec![route(2)],
        vec![moved, moved],
        vec![route(1)],
        vec![PressureRoute {
            generation: 2,
            ..route(2)
        }],
        vec![PressureRoute {
            identity: StoreIdentity {
                root_digest: RootDigest::from_bytes([3; 32]),
                ..identity(3)
            },
            ..route(3)
        }],
    ] {
        assert!(cache.set_routes(invalid).is_err());
        assert_eq!(cache.polling_nodes(), vec![NodeId(2)]);
        assert!(cache.refuses([2], 1));
    }
    // The cache retains inactive node slots, so its lifetime bound is explicit.
    let routes: Vec<_> = (2..MAX_PEERS as u64 + 2)
        .map(|node| {
            let mut r = route(node);
            r.identity.store_incarnation = StoreIncarnation::from_bytes([4; 16]);
            r
        })
        .collect();
    cache.set_routes(routes).unwrap();
    assert_eq!(cache.snapshot().len(), MAX_PEERS);
    assert!(cache.set_routes(vec![route(MAX_PEERS as u64 + 2)]).is_err());
    assert_eq!(cache.snapshot().len(), MAX_PEERS);
}
