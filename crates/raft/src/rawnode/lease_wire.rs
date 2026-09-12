//! Exact lease messages over the existing Raft transport. An ordinary heartbeat
//! echo cannot manufacture a Grant. No absolute clock sample crosses the wire.

use crate::lease::{Authority, Renewal};
use crate::lease_policy::LeasePolicy;
use raft::eraftpb::{Message, MessageType};

const MAGIC: &[u8; 8] = b"KV9LSE01";
// Ordinary Raft heartbeats leave log_term zero. This separates the namespace
// from arbitrary Safe ReadIndex contexts (including a coincidental magic prefix).
const MARKER: u64 = u64::MAX;
const FIXED: usize = 102;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Request = 1,
    Grant = 2,
}

pub(super) fn reserved(message: &Message) -> bool {
    message.log_term == MARKER
        && matches!(
            message.get_msg_type(),
            MessageType::MsgHeartbeat | MessageType::MsgHeartbeatResponse
        )
}

pub(super) fn encode(kind: Kind, renewal: Renewal, policy: &LeasePolicy, to: u64) -> Message {
    let a = renewal.authority;
    let mut bytes = Vec::with_capacity(FIXED + 8 * policy.voters.len());
    bytes.extend_from_slice(MAGIC);
    bytes.push(kind as u8);
    bytes.extend_from_slice(&a.group.to_be_bytes());
    bytes.extend_from_slice(&a.configuration.to_be_bytes());
    bytes.extend_from_slice(&a.leader.to_be_bytes());
    bytes.extend_from_slice(&a.term.to_be_bytes());
    bytes.extend_from_slice(&a.incarnation.to_be_bytes());
    bytes.extend_from_slice(&renewal.generation.to_be_bytes());
    bytes.extend_from_slice(&renewal.sequence.to_be_bytes());
    bytes.extend_from_slice(&renewal.promise_ns.to_be_bytes());
    bytes.extend_from_slice(&policy.drift_ppb.to_be_bytes());
    bytes.extend_from_slice(&policy.margin_ns.to_be_bytes());
    bytes.push(policy.voters.len() as u8);
    for voter in &policy.voters {
        bytes.extend_from_slice(&voter.to_be_bytes());
    }
    Message {
        msg_type: if kind == Kind::Request {
            MessageType::MsgHeartbeat
        } else {
            MessageType::MsgHeartbeatResponse
        },
        from: policy.node,
        to,
        term: a.term,
        log_term: MARKER,
        context: bytes.into(),
        ..Default::default()
    }
}

pub(super) fn decode(message: &Message, policy: &LeasePolicy) -> Option<(Kind, Renewal)> {
    let bytes = message.context.as_ref();
    if !reserved(message)
        || message.to != policy.node
        || message.from == policy.node
        || policy.voters.binary_search(&message.from).is_err()
        || message.reject
        || message.commit != 0
        || message.index != 0
        || !message.entries.is_empty()
        || message.has_snapshot()
        || bytes.len() < FIXED
        || &bytes[..8] != MAGIC
        || bytes[101] > 64
        || bytes.len() != FIXED + 8 * usize::from(bytes[101])
    {
        return None;
    }
    let kind = match (bytes[8], message.get_msg_type()) {
        (1, MessageType::MsgHeartbeat) => Kind::Request,
        (2, MessageType::MsgHeartbeatResponse) => Kind::Grant,
        _ => return None,
    };
    let n =
        |offset| u64::from_be_bytes(bytes[offset..offset + 8].try_into().expect("checked shape"));
    let renewal = Renewal {
        authority: Authority {
            group: n(9),
            configuration: u128::from_be_bytes(bytes[17..33].try_into().expect("checked shape")),
            leader: n(33),
            term: n(41),
            incarnation: u128::from_be_bytes(bytes[49..65].try_into().expect("checked shape")),
        },
        generation: n(65),
        sequence: n(73),
        promise_ns: n(81),
    };
    let a = renewal.authority;
    if a.group != policy.group
        || a.configuration != policy.configuration
        || a.term == 0
        || a.term != message.term
        || a.incarnation == 0
        || renewal.sequence == 0
        || renewal.generation == u64::MAX
        || renewal.promise_ns != policy.promise_ns
        || u32::from_be_bytes(bytes[89..93].try_into().expect("checked shape")) != policy.drift_ppb
        || n(93) != policy.margin_ns
        || policy.voters.len() != usize::from(bytes[101])
        || policy
            .voters
            .iter()
            .enumerate()
            .any(|(i, voter)| n(FIXED + 8 * i) != *voter)
        || (kind == Kind::Request && a.leader != message.from)
        || (kind == Kind::Grant && a.leader != policy.node)
    {
        return None;
    }
    Some((kind, renewal))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(node: u64, count: u64) -> LeasePolicy {
        LeasePolicy {
            node,
            group: 17,
            configuration: 23,
            voters: (1..=count).collect(),
            promise_ns: 100,
            drift_ppb: 10,
            margin_ns: 1,
        }
    }

    fn renewal() -> Renewal {
        Renewal {
            authority: Authority {
                group: 17,
                configuration: 23,
                leader: 1,
                term: 5,
                incarnation: 19,
            },
            generation: 0,
            sequence: 1,
            promise_ns: 100,
        }
    }

    #[test]
    fn exact_envelopes_round_trip_at_maximum_membership_and_reject_bad_shapes() {
        for count in [3, 64] {
            let request = encode(Kind::Request, renewal(), &policy(1, count), 2);
            assert_eq!(request.context.len(), FIXED + 8 * count as usize);
            assert_eq!(
                decode(&request, &policy(2, count)),
                Some((Kind::Request, renewal()))
            );
            let grant = encode(Kind::Grant, renewal(), &policy(2, count), 1);
            assert_eq!(
                decode(&grant, &policy(1, count)),
                Some((Kind::Grant, renewal()))
            );
            for len in 0..request.context.len() {
                let mut truncated = request.clone();
                truncated.context = request.context[..len].to_vec().into();
                assert_eq!(decode(&truncated, &policy(2, count)), None, "length {len}");
            }
            let mut extra = request.clone();
            let mut bytes = extra.context.to_vec();
            bytes.push(0);
            extra.context = bytes.into();
            assert_eq!(decode(&extra, &policy(2, count)), None);
        }
    }

    #[test]
    fn ordinary_heartbeat_echo_and_read_context_never_become_grants() {
        let mut echo = encode(Kind::Request, renewal(), &policy(1, 3), 2);
        echo.from = 2;
        echo.to = 1;
        echo.msg_type = MessageType::MsgHeartbeatResponse;
        // A legacy peer echoes context without the marker. Even retaining the
        // marker cannot convert Request kind into the required Grant kind.
        assert_eq!(decode(&echo, &policy(1, 3)), None);
        echo.log_term = 0;
        assert!(!reserved(&echo));
        assert_eq!(decode(&echo, &policy(1, 3)), None);
        let mut ordinary = encode(Kind::Grant, renewal(), &policy(2, 3), 1);
        ordinary.log_term = 0;
        assert!(!reserved(&ordinary));
        assert_eq!(decode(&ordinary, &policy(1, 3)), None);
    }

    #[test]
    fn envelope_rejects_wrong_identity_policy_and_raft_payload() {
        let original = encode(Kind::Request, renewal(), &policy(1, 3), 2);
        let recipient = policy(2, 3);
        // Mutate exact checked fields, rather than assuming all payload bits
        // are invalid: sequence/incarnation changes can name a valid new round.
        for offset in [0, 8, 9, 17, 33, 41, 81, 89, 93, 101, 102] {
            let mut changed = original.clone();
            let mut bytes = changed.context.to_vec();
            bytes[offset] ^= 1;
            changed.context = bytes.into();
            assert_eq!(decode(&changed, &recipient), None, "offset {offset}");
        }
        for change in 0..9 {
            let mut changed = original.clone();
            match change {
                0 => changed.from = 4,
                1 => changed.to = 3,
                2 => changed.term += 1,
                3 => changed.reject = true,
                4 => changed.commit = 1,
                5 => changed.index = 1,
                6 => changed.entries.push(Default::default()),
                7 => changed.mut_snapshot().mut_metadata().index = 1,
                8 => changed.log_term = 0,
                _ => unreachable!(),
            }
            assert_eq!(decode(&changed, &recipient), None, "mutation {change}");
        }
    }
}
