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
