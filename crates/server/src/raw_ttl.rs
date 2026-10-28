//! Versioned, scoped public Raw TTL contract. No operation falls back to Raw v1.
use kv9_common::{AppliedPosition, Error, Result, UserKey, Value};
use serde::{Deserialize, Serialize};

// V2 confirms full catalog scan bounds and the distinct paged DeleteRange result.
// V1 capability replies cannot authorize either new contract.
pub const PROTOCOL_VERSION: u32 = 2;
pub const MAX_SCAN_EXAMINED: usize = 4096;
pub const MAX_BUDGET_MICROS: u64 = 30_000_000;
pub const MAX_RANGE_BOUND_BYTES: usize = kv9_common::data_range::MAX_BOUND;
pub const MAX_ACTIVATION_REQUEST_BYTES: usize = kv9_common::data_range::MAX_ENCODED_BYTES + 64;
pub fn valid_range_position(key: &[u8]) -> bool {
    key.len() <= MAX_RANGE_BOUND_BYTES
        || (key.len() == MAX_RANGE_BOUND_BYTES + 1 && key.last() == Some(&0))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Capabilities,
    Get {
        keys: Vec<UserKey>,
    },
    Put {
        pairs: Vec<(UserKey, Value)>,
        #[serde(default)]
        ttl_secs: Option<u64>,
    },
    Delete {
        key: UserKey,
    },
    Scan {
        start: UserKey,
        end: UserKey,
        limit: usize,
    },
    /// One bounded, per-range chunk. Continue only after its exact receipt.
    DeleteRange {
        start: UserKey,
        end: UserKey,
        limit: usize,
    },
}

impl Operation {
    pub fn is_read(&self) -> bool {
        matches!(
            self,
            Self::Capabilities | Self::Get { .. } | Self::Scan { .. }
        )
    }

    pub fn validate(&self) -> Result<()> {
        use crate::client::{valid_batch_keys, valid_batch_pairs, MAX_BATCH_ITEMS, MAX_KEY_BYTES};
        let valid = match self {
            Self::Capabilities => true,
            Self::Get { keys } => valid_batch_keys(keys),
            Self::Put { pairs, ttl_secs } => {
                valid_batch_pairs(pairs) && ttl_secs.is_none_or(|s| s.checked_mul(1000).is_some())
            }
            Self::Delete { key } => key.len() <= MAX_KEY_BYTES,
            Self::Scan { start, end, limit } | Self::DeleteRange { start, end, limit } => {
                valid_range_position(start)
                    && end.len() <= MAX_RANGE_BOUND_BYTES
                    && (1..=MAX_BATCH_ITEMS).contains(limit)
                    && (end.is_empty() || start < end)
            }
        };
        if valid {
            Ok(())
        } else {
            Err(Error::Config("invalid Raw TTL request bounds".into()))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Receipt {
    pub term: u64,
    pub index: u64,
}
impl From<AppliedPosition> for Receipt {
    fn from(at: AppliedPosition) -> Self {
        Self {
            term: at.term,
            index: at.index,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reply {
    Capabilities,
    Values {
        values: Vec<Option<Value>>,
    },
    Applied {
        term: u64,
        index: u64,
    },
    Page {
        rows: Vec<(UserKey, Value)>,
        next: Option<UserKey>,
        examined: usize,
    },
    Deleted {
        /// Number of selected delete mutations, not an existence/change count.
        processed_keys: usize,
        applied: Option<Receipt>,
        next: Option<UserKey>,
    },
}
impl From<AppliedPosition> for Reply {
    fn from(at: AppliedPosition) -> Self {
        Self::Applied {
            term: at.term,
            index: at.index,
        }
    }
}
pub type Completion = crate::api::OperationCompletion<Reply>;
pub type Preparation = crate::api::OperationPreparation<Reply>;

pub(crate) const REFUSED: &str = "kv9-ttl-refused";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Refusal {
    NotEnabled,
    LegacyCommand,
    InvalidScope,
    StaleScope,
    CrossRange,
    Backpressure,
    Clock {
        reason: kv9_common::raw_ttl::clock::ClockRefusal,
    },
}

impl From<Operation> for crate::proto::raw_ttl_request::Operation {
    fn from(op: Operation) -> Self {
        use crate::proto;
        match op {
            Operation::Capabilities => Self::Capabilities(proto::Empty {}),
            Operation::Get { keys } => Self::Get(proto::RoutedKeys { keys }),
            Operation::Put { pairs, ttl_secs } => Self::Put(proto::RawTtlPut {
                pairs: pairs
                    .into_iter()
                    .map(|(key, value)| proto::KeyValue { key, value })
                    .collect(),
                ttl_secs,
            }),
            Operation::Delete { key } => Self::DeleteKey(key),
            Operation::Scan { start, end, limit } => Self::Scan(proto::RawTtlScan {
                start,
                end,
                limit: limit.try_into().unwrap_or(u32::MAX),
            }),
            Operation::DeleteRange { start, end, limit } => Self::DeleteRange(proto::RawTtlScan {
                start,
                end,
                limit: limit.try_into().unwrap_or(u32::MAX),
            }),
        }
    }
}
