//! Negative tests for the wire frame decoder (task #15, third item).
//!
//! The contract under test, as specified: bad magic, unknown version, unknown kind, an
//! out-of-range length, and an EOF-truncated read must each produce a typed
//! `Error::Raft` — never a panic, never a silently coerced value. A frame header arrives
//! from a socket, so it is untrusted input in the plainest sense: whatever is on the other
//! end may be a peer, a port scanner, or a half-open connection from a crashed process.
//!
//! Two things these tests deliberately check that a "does it error?" suite would not:
//!
//! * **Rejection happens before allocation.** A frame claiming 16 MiB must be refused on
//!   the length check rather than by trying to read 16 MiB and failing at EOF. The two
//!   are indistinguishable by "did it error", so the tests distinguish them by *which*
//!   error comes back — see `oversized_discovery_is_refused_before_reading_payload`.
//! * **The suite is sensitive.** `every_kind_round_trips` is the control: without it, a
//!   decoder that rejected everything unconditionally would pass every test below.

use std::io::Cursor;

use kv9_common::{Error, NodeId};
use kv9_raft::transport::{encode_frame, read_frame, Frame, FRAME_MAGIC, FRAME_VERSION};

/// Build a raw header. Kept explicit so each test states exactly which byte it is
/// corrupting rather than mutating an opaque buffer by offset.
fn header(magic: u16, version: u8, kind: u8, len: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.extend_from_slice(&magic.to_be_bytes());
    out.push(version);
    out.push(kind);
    out.extend_from_slice(&len.to_be_bytes());
    out
}

fn read(bytes: &[u8]) -> Result<Frame, Error> {
    read_frame(&mut Cursor::new(bytes.to_vec()))
}

fn err(bytes: &[u8]) -> String {
    match read(bytes) {
        Err(e) => e.to_string(),
        Ok(f) => panic!("expected a typed error, decoded {f:?} instead"),
    }
}

/// The control. Every negative assertion below is meaningless if the decoder simply
/// refuses everything, so pin that valid frames of all three kinds still round-trip.
#[test]
fn every_kind_round_trips() {
    let frames = [
        Frame::Raft(vec![0xDE, 0xAD, 0xBE, 0xEF]),
        Frame::DiscoveryReq { from: NodeId(7) },
        Frame::DiscoveryResp {
            node: NodeId(9),
            initialized: true,
        },
        Frame::DiscoveryResp {
            node: NodeId(9),
            initialized: false,
        },
        // A zero-length raft payload is legal, and an easy off-by-one to get wrong.
        Frame::Raft(Vec::new()),
    ];
    for f in frames {
        let bytes = encode_frame(&f);
        assert_eq!(read(&bytes).unwrap(), f, "round trip failed for {f:?}");
    }
}

#[test]
fn bad_magic_is_rejected() {
    let mut bytes = header(0x0000, FRAME_VERSION, 1, 0);
    assert!(err(&bytes).contains("magic"));
    // Also the near-miss: one bit off from the real magic.
    bytes = header(FRAME_MAGIC ^ 1, FRAME_VERSION, 1, 0);
    assert!(err(&bytes).contains("magic"));
}

#[test]
fn unknown_version_is_rejected() {
    for v in [0u8, FRAME_VERSION + 1, 0xFF] {
        let bytes = header(FRAME_MAGIC, v, 1, 0);
        assert!(
            err(&bytes).contains("version"),
            "version {v} should be refused by version check"
        );
    }
}

#[test]
fn unknown_kind_is_rejected() {
    for kind in [0u8, 4, 99, 0xFF] {
        let bytes = header(FRAME_MAGIC, FRAME_VERSION, kind, 0);
        assert!(
            err(&bytes).contains("kind"),
            "kind {kind} should be refused"
        );
    }
}

/// A raft payload larger than the cap is corrupt by definition.
#[test]
fn oversized_raft_length_is_rejected() {
    let bytes = header(FRAME_MAGIC, FRAME_VERSION, 1, u32::MAX);
    assert!(err(&bytes).contains("length"));
}

/// Discovery frames are fixed size, so any other length is invalid — not merely
/// "unexpected". Both directions matter: too short and too long.
#[test]
fn wrong_discovery_lengths_are_rejected() {
    for (kind, correct) in [(2u8, 9u32), (3u8, 10u32)] {
        for wrong in [0, correct - 1, correct + 1, 1024] {
            let bytes = header(FRAME_MAGIC, FRAME_VERSION, kind, wrong);
            assert!(
                err(&bytes).contains("length"),
                "kind {kind} len {wrong} should be refused (correct is {correct})"
            );
        }
    }
}

/// The anti-DoS property: a bogus length is refused *by the length check*, not by
/// attempting the read and hitting EOF.
///
/// Both paths error, so "did it error" cannot tell them apart. The distinguisher is which
/// error: a length rejection mentions the length, whereas an attempted read that runs out
/// of bytes reports a payload-read failure. If this ever starts reporting the latter, the
/// decoder has begun allocating on an attacker-supplied size.
#[test]
fn oversized_discovery_is_refused_before_reading_payload() {
    // Claim 16 MiB for a frame whose payload is 9 bytes by definition, and supply no
    // payload at all.
    let bytes = header(FRAME_MAGIC, FRAME_VERSION, 2, 16 * 1024 * 1024);
    let message = err(&bytes);
    assert!(
        message.contains("length"),
        "expected refusal on the length check, got: {message}"
    );
    assert!(
        !message.contains("payload read"),
        "decoder tried to read the payload before validating the length: {message}"
    );
}

#[test]
fn truncated_header_is_rejected() {
    let full = encode_frame(&Frame::DiscoveryReq { from: NodeId(3) });
    for cut in 0..8 {
        let message = err(&full[..cut]);
        assert!(
            message.contains("header"),
            "an {cut}-byte header should fail as a header read, got: {message}"
        );
    }
}

/// "The length prefix lies": the header promises N payload bytes and the stream ends
/// early. This is the shape a half-open or crashed connection produces.
#[test]
fn lying_length_prefix_is_rejected() {
    let full = encode_frame(&Frame::DiscoveryResp {
        node: NodeId(5),
        initialized: true,
    });
    // Keep the full header, drop part of the payload.
    for cut in 8..full.len() {
        let message = err(&full[..cut]);
        assert!(
            message.contains("payload"),
            "a truncated payload should fail as a payload read, got: {message}"
        );
    }
}

/// The discovery payload carries its own version byte; an unknown one must be refused
/// rather than parsed as if it were version 1.
#[test]
fn unknown_discovery_payload_version_is_rejected() {
    for (kind, len) in [(2u8, 9usize), (3u8, 10usize)] {
        let mut bytes = header(FRAME_MAGIC, FRAME_VERSION, kind, len as u32);
        let mut payload = vec![0u8; len];
        payload[0] = 2; // inner version, only 1 is legal
        bytes.extend_from_slice(&payload);
        assert!(
            read(&bytes).is_err(),
            "kind {kind} with inner version 2 must be refused"
        );
    }
}

/// The one that feeds bootstrap fencing: `initialized` is 0 or 1, and nothing else.
///
/// A decoder written as `!= 0 => true` would turn a garbled byte into "this cluster is
/// already initialized" — an answer, not an error. That value decides whether a node
/// joins or bootstraps, so guessing at it is exactly the wrong move.
#[test]
fn garbled_initialized_byte_is_an_error_not_an_answer() {
    for bad in [2u8, 3, 0x37, 0xFF] {
        let mut bytes = header(FRAME_MAGIC, FRAME_VERSION, 3, 10);
        bytes.push(1); // inner version
        bytes.extend_from_slice(&9u64.to_be_bytes()); // node id
        bytes.push(bad);
        let message = err(&bytes);
        assert!(
            message.contains("initialized"),
            "byte {bad} must be refused as an invalid initialized flag, got: {message}"
        );
    }

    // Control: the two legal values still decode, and to the right booleans — otherwise
    // the assertions above would hold against a decoder that rejected 0 and 1 too.
    for (byte, want) in [(0u8, false), (1u8, true)] {
        let mut bytes = header(FRAME_MAGIC, FRAME_VERSION, 3, 10);
        bytes.push(1);
        bytes.extend_from_slice(&9u64.to_be_bytes());
        bytes.push(byte);
        assert_eq!(
            read(&bytes).unwrap(),
            Frame::DiscoveryResp {
                node: NodeId(9),
                initialized: want
            }
        );
    }
}

/// Whole-stream garbage must not panic — the decoder is the first thing an unknown
/// connection touches.
#[test]
fn arbitrary_garbage_never_panics() {
    let samples: [&[u8]; 6] = [
        b"",
        b"\x00",
        b"GET / HTTP/1.1\r\n\r\n",
        b"\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff",
        b"K9",
        b"\x4b\x39\x01\x02\xff\xff\xff\xff",
    ];
    for s in samples {
        // Any outcome is acceptable except a panic; the point is that the decoder is
        // total over arbitrary bytes.
        let _ = read(s);
    }
}

/// Bytes following a complete frame belong to the *next* frame and must not be consumed
/// or mistaken for part of this one.
#[test]
fn a_frame_does_not_consume_the_next_one() {
    let mut stream = encode_frame(&Frame::DiscoveryReq { from: NodeId(1) });
    stream.extend_from_slice(&encode_frame(&Frame::DiscoveryResp {
        node: NodeId(2),
        initialized: false,
    }));

    let mut cursor = Cursor::new(stream);
    assert_eq!(
        read_frame(&mut cursor).unwrap(),
        Frame::DiscoveryReq { from: NodeId(1) }
    );
    assert_eq!(
        read_frame(&mut cursor).unwrap(),
        Frame::DiscoveryResp {
            node: NodeId(2),
            initialized: false
        }
    );
}
