//! Multi-tenant physical key encoding (DESIGN §3.4, §13 principles 3–4).
//!
//! Every stored key is prefixed:
//!
//! ```text
//! mode_byte (1)  keyspace_id (3 bytes, big-endian)  user_key...
//! mode_byte ∈ { 't' = txn, 'r' = raw, 's' = system }
//! ```
//!
//! The 3-byte keyspace id gives up to 2^24 keyspaces. kv9 **validates** the id range at
//! encode time instead of silently truncating (DESIGN §3.4, §13 principle 4): a scheme
//! that truncated an out-of-range id would misroute keys across tenants. The prefix
//! makes each keyspace a contiguous range, so routing/GC/backup/encryption all key off
//! it, and region split points are constrained never to cross a keyspace boundary
//! (DESIGN §3.3, §13 principle 3).

use crate::error::{Error, Result};
use crate::ids::KeyspaceId;

/// The mode byte of a physical key (DESIGN §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMode {
    /// `'t'` — a `txn` keyspace key.
    Txn,
    /// `'r'` — a `raw` keyspace key.
    Raw,
    /// `'s'` — the reserved system keyspace (DESIGN §5).
    System,
}

impl KeyMode {
    #[inline]
    pub fn as_byte(self) -> u8 {
        match self {
            KeyMode::Txn => b't',
            KeyMode::Raw => b'r',
            KeyMode::System => b's',
        }
    }

    #[inline]
    pub fn from_byte(b: u8) -> Result<KeyMode> {
        match b {
            b't' => Ok(KeyMode::Txn),
            b'r' => Ok(KeyMode::Raw),
            b's' => Ok(KeyMode::System),
            other => Err(Error::InvalidKeyMode(other)),
        }
    }
}

/// Length in bytes of the fixed prefix: 1 mode byte + 3 keyspace-id bytes.
pub const KEY_PREFIX_LEN: usize = 4;

/// Width in bytes of the on-disk keyspace id (DESIGN §3.4).
pub const KEYSPACE_ID_WIDTH: usize = 3;

/// Validate that a keyspace id fits the 3-byte on-disk width (DESIGN §3.4, §13 principle 4).
///
/// kv9 rejects an out-of-range id rather than silently truncating it (which would
/// misroute keys across tenants).
pub fn validate_keyspace_id(id: KeyspaceId) -> Result<()> {
    if id.0 > KeyspaceId::MAX {
        Err(Error::KeyspaceIdOutOfRange(id.0))
    } else {
        Ok(())
    }
}

/// Encode `mode + keyspace_id + user_key` into a physical key (DESIGN §3.4).
pub fn encode_key(mode: KeyMode, keyspace: KeyspaceId, user_key: &[u8]) -> Result<Vec<u8>> {
    validate_keyspace_id(keyspace)?;
    let mut out = Vec::with_capacity(KEY_PREFIX_LEN + user_key.len());
    out.push(mode.as_byte());
    // 3-byte big-endian keyspace id.
    out.push(((keyspace.0 >> 16) & 0xFF) as u8);
    out.push(((keyspace.0 >> 8) & 0xFF) as u8);
    out.push((keyspace.0 & 0xFF) as u8);
    out.extend_from_slice(user_key);
    Ok(out)
}

/// The just-past-the-end prefix for a keyspace, i.e. `mode + (keyspace_id + 1)`.
///
/// Gives the exclusive upper bound of a keyspace's contiguous range, used to keep
/// region boundaries aligned to keyspace boundaries (DESIGN §3.3, §13 principle 3).
pub fn keyspace_range(mode: KeyMode, keyspace: KeyspaceId) -> Result<(Vec<u8>, Vec<u8>)> {
    let start = encode_key(mode, keyspace, &[])?;
    let end = if keyspace.0 == KeyspaceId::MAX {
        // Roll the mode byte forward for the last keyspace.
        vec![mode.as_byte().saturating_add(1)]
    } else {
        encode_key(mode, KeyspaceId(keyspace.0 + 1), &[])?
    };
    Ok((start, end))
}

/// A decoded physical key: its mode, keyspace, and user-key suffix (DESIGN §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedKey<'a> {
    pub mode: KeyMode,
    pub keyspace: KeyspaceId,
    pub user_key: &'a [u8],
}

/// Decode a physical key back into its parts (DESIGN §3.4).
pub fn decode_key(raw: &[u8]) -> Result<DecodedKey<'_>> {
    if raw.len() < KEY_PREFIX_LEN {
        return Err(Error::MalformedKey(format!(
            "length {} < prefix length {}",
            raw.len(),
            KEY_PREFIX_LEN
        )));
    }
    let mode = KeyMode::from_byte(raw[0])?;
    let keyspace = KeyspaceId(
        (u32::from(raw[1]) << 16) | (u32::from(raw[2]) << 8) | u32::from(raw[3]),
    );
    Ok(DecodedKey {
        mode,
        keyspace,
        user_key: &raw[KEY_PREFIX_LEN..],
    })
}

/// Extract just the keyspace id from a physical key without full decode.
///
/// Region routing derives the keyspace from the key prefix (DESIGN §3.4). Because
/// regions never span keyspace boundaries (DESIGN §3.3, §13 principle 3), the keyspace
/// derives unambiguously from the encoded prefix.
pub fn keyspace_of(raw: &[u8]) -> Result<KeyspaceId> {
    Ok(decode_key(raw)?.keyspace)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODES: [KeyMode; 3] = [KeyMode::Txn, KeyMode::Raw, KeyMode::System];

    /// Interesting user keys: empty, boundary bytes, and prefix relationships.
    fn sample_user_keys() -> Vec<Vec<u8>> {
        vec![
            vec![],
            vec![0x00],
            vec![0x00, 0x00],
            vec![0x01],
            vec![b'a'],
            vec![b'a', b'b'],
            vec![b'b'],
            vec![0xFE],
            vec![0xFF],
            vec![0xFF, 0x00],
            vec![0xFF, 0xFF],
        ]
    }

    /// An out-of-range keyspace id must be **rejected**, never silently truncated
    /// (DESIGN §3.4, §13 principle 4: a truncating scheme would misroute keys across
    /// tenants). This is the single enforcement point for that invariant.
    #[test]
    fn out_of_range_keyspace_id_is_rejected_not_truncated() {
        let first_invalid = KeyspaceId(KeyspaceId::MAX + 1);
        assert!(matches!(
            validate_keyspace_id(first_invalid),
            Err(Error::KeyspaceIdOutOfRange(_))
        ));
        for mode in MODES {
            assert!(
                encode_key(mode, first_invalid, b"k").is_err(),
                "encode accepted an out-of-range keyspace id"
            );
            assert!(keyspace_range(mode, first_invalid).is_err());
        }
        // A value whose low 3 bytes collide with a *valid* id is the dangerous case: if
        // encoding truncated, id 0x01_00_00_2A would land in tenant 0x00_00_2A's range.
        let colliding = KeyspaceId(0x0100_002A);
        assert!(encode_key(KeyMode::Txn, colliding, b"k").is_err());

        // The largest valid id still encodes.
        assert!(encode_key(KeyMode::Txn, KeyspaceId(KeyspaceId::MAX), b"k").is_ok());
    }

    #[test]
    fn roundtrip_preserves_mode_keyspace_and_user_key() {
        for mode in MODES {
            for id in [0u32, 1, 0x00FF, 0x00FF_FFFF] {
                for uk in sample_user_keys() {
                    let raw = encode_key(mode, KeyspaceId(id), &uk).unwrap();
                    let decoded = decode_key(&raw).unwrap();
                    assert_eq!(decoded.mode, mode);
                    assert_eq!(decoded.keyspace, KeyspaceId(id));
                    assert_eq!(decoded.user_key, &uk[..]);
                    assert_eq!(keyspace_of(&raw).unwrap(), KeyspaceId(id));
                }
            }
        }
    }

    /// The encoding must be **order-preserving**: comparing encoded bytes must give the
    /// same answer as comparing `(keyspace_id, user_key)`.
    ///
    /// Everything that resolves a key by range depends on this — region routing's
    /// "last start_key ≤ K" reverse seek returns a silently wrong region if byte order
    /// and logical order disagree.
    #[test]
    fn encoding_is_order_preserving_within_a_mode() {
        /// `((keyspace_id, user_key), encoded_bytes)` — logical key beside its encoding.
        type LogicalAndEncoded = ((u32, Vec<u8>), Vec<u8>);

        for mode in MODES {
            let mut pairs: Vec<LogicalAndEncoded> = Vec::new();
            for id in [0u32, 1, 2, 0x00FF, 0x0100, 0x00FF_FFFE, 0x00FF_FFFF] {
                for uk in sample_user_keys() {
                    let raw = encode_key(mode, KeyspaceId(id), &uk).unwrap();
                    pairs.push(((id, uk), raw));
                }
            }
            for (a_logical, a_raw) in &pairs {
                for (b_logical, b_raw) in &pairs {
                    assert_eq!(
                        a_raw.cmp(b_raw),
                        a_logical.cmp(b_logical),
                        "byte order disagrees with logical order for {a_logical:?} vs {b_logical:?}"
                    );
                }
            }
        }
    }

    /// A keyspace occupies a contiguous range, and consecutive keyspaces abut exactly —
    /// no gap (which would strand keys) and no overlap (which would be a cross-tenant
    /// leak). DESIGN §3.4.
    #[test]
    fn keyspace_ranges_are_contiguous_and_bound_their_own_keys() {
        for mode in MODES {
            for id in [0u32, 1, 0x00FF, 0x00FF_FFFE] {
                let (start, end) = keyspace_range(mode, KeyspaceId(id)).unwrap();
                let (next_start, _) = keyspace_range(mode, KeyspaceId(id + 1)).unwrap();
                assert_eq!(end, next_start, "keyspace {id} and {} do not abut", id + 1);

                for uk in sample_user_keys() {
                    let k = encode_key(mode, KeyspaceId(id), &uk).unwrap();
                    assert!(k >= start && k < end, "key escaped its keyspace range");
                    // ...and a neighbour's key must not fall inside this range.
                    let neighbour = encode_key(mode, KeyspaceId(id + 1), &uk).unwrap();
                    assert!(neighbour >= end, "neighbour key leaked into this range");
                }
            }
        }
    }

    /// The last keyspace has no `id + 1`, so its upper bound rolls the mode byte forward.
    /// That bound must still exclude nothing of its own and include none of the next mode.
    #[test]
    fn last_keyspace_range_bounds_correctly() {
        for mode in MODES {
            let max = KeyspaceId(KeyspaceId::MAX);
            let (start, end) = keyspace_range(mode, max).unwrap();
            for uk in sample_user_keys() {
                let k = encode_key(mode, max, &uk).unwrap();
                assert!(k >= start && k < end);
            }
            // The bound is exactly one past this mode's byte, so no key of this mode can
            // reach it and every key of the next mode is at or beyond it.
            assert_eq!(end, vec![mode.as_byte() + 1]);
        }
    }

    /// Malformed input must produce a typed error, never a panic and never a plausible
    /// wrong answer (DESIGN §13 principle 12: never panic on the unknown).
    #[test]
    fn decode_rejects_malformed_keys() {
        for short in [&b""[..], b"t", b"t\x00", b"t\x00\x00"] {
            assert!(
                matches!(decode_key(short), Err(Error::MalformedKey(_))),
                "decode accepted a key shorter than the prefix: {short:?}"
            );
        }
        // A prefix-length key with no user key is valid (empty user key).
        assert!(decode_key(b"t\x00\x00\x00").is_ok());

        // Unknown mode bytes are rejected rather than guessed at.
        for bad in [0x00u8, b'a', b'q', b'u', 0xFF] {
            let raw = [bad, 0, 0, 1];
            assert!(
                matches!(decode_key(&raw), Err(Error::InvalidKeyMode(b)) if b == bad),
                "decode accepted invalid mode byte {bad:#x}"
            );
        }
    }

    #[test]
    fn mode_byte_roundtrip() {
        for mode in MODES {
            assert_eq!(KeyMode::from_byte(mode.as_byte()).unwrap(), mode);
        }
        assert_eq!(KeyMode::Txn.as_byte(), b't');
        assert_eq!(KeyMode::Raw.as_byte(), b'r');
        assert_eq!(KeyMode::System.as_byte(), b's');
    }
}
