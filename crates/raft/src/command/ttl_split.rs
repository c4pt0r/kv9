//! Bounded transfer entries. Ordinary submission always refuses; the driver's
//! source-derived commands and exact all-member confirmation gate are required.
use super::{put_bytes, Reader};
use kv9_common::data_range::DataRange;
use kv9_common::raw_ttl::split::{ImportRow, ImportSpec, SplitPlan, MAX_PAGE_ROWS};
use kv9_common::raw_ttl::ExpiryRecord;
use kv9_common::{AppliedPosition, Error, Result, RootDigest};

pub const MAX_COMMAND_BYTES: usize = 16 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawTtlSplitAction {
    Seal(SplitPlan),
    Begin {
        spec: ImportSpec,
        empty_at: AppliedPosition,
    },
    Append {
        target: DataRange,
        expected: RootDigest,
        rows: Vec<ImportRow>,
    },
    Finish {
        target: DataRange,
        expected: RootDigest,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTtlSplit {
    pub(crate) action: Box<RawTtlSplitAction>,
}
fn invalid() -> Error {
    Error::Raft("invalid or oversized TTL split command".into())
}
impl RawTtlSplit {
    /// Exact row space after the tag, target, CAS digest and row count.
    pub fn append_row_bytes_budget(target: &DataRange) -> Result<usize> {
        target.validate()?;
        MAX_COMMAND_BYTES
            .checked_sub(3 + 4 + target.encode().len() + 32 + 4)
            .ok_or_else(invalid)
    }
    pub fn new(action: RawTtlSplitAction) -> Result<Self> {
        let cmd = Self {
            action: Box::new(action),
        };
        cmd.validate()?;
        Ok(cmd)
    }
    pub fn action(&self) -> &RawTtlSplitAction {
        &self.action
    }
    pub(crate) fn target(&self) -> &DataRange {
        match self.action.as_ref() {
            RawTtlSplitAction::Seal(plan) => &plan.parent,
            RawTtlSplitAction::Begin { spec, .. } => spec.target(),
            RawTtlSplitAction::Append { target, .. } | RawTtlSplitAction::Finish { target, .. } => {
                target
            }
        }
    }
    fn validate(&self) -> Result<()> {
        self.target().validate()?;
        let bytes = match self.action.as_ref() {
            RawTtlSplitAction::Seal(plan) => {
                plan.validate()?;
                3 + 4 + plan.encode().len()
            }
            RawTtlSplitAction::Begin { spec, empty_at } => {
                spec.validate()?;
                if empty_at.term == 0 || empty_at.index == 0 {
                    return Err(invalid());
                }
                3 + 4 + spec.encode().len() + 16
            }
            RawTtlSplitAction::Append {
                target,
                expected,
                rows,
            } => {
                if expected.as_bytes() == &[0; 32] || rows.is_empty() || rows.len() > MAX_PAGE_ROWS
                {
                    return Err(invalid());
                }
                let mut bytes = 3 + 4 + target.encode().len() + 32 + 4;
                let mut last: Option<&[u8]> = None;
                for row in rows {
                    row.validate(target)?;
                    if last.is_some_and(|last| last >= row.key.as_slice()) {
                        return Err(invalid());
                    }
                    bytes = bytes
                        .checked_add(row.encoded_bytes()?)
                        .ok_or_else(invalid)?;
                    if bytes > MAX_COMMAND_BYTES {
                        return Err(invalid());
                    }
                    last = Some(&row.key);
                }
                bytes
            }
            RawTtlSplitAction::Finish { target, expected } => {
                if expected.as_bytes() == &[0; 32] {
                    return Err(invalid());
                }
                3 + 4 + target.encode().len() + 32
            }
        };
        if bytes > MAX_COMMAND_BYTES || self.target().sealed {
            return Err(invalid());
        }
        Ok(())
    }
    pub(super) fn encode_into(&self, out: &mut Vec<u8>) {
        match self.action.as_ref() {
            RawTtlSplitAction::Seal(plan) => {
                out.push(0);
                put_bytes(out, &plan.encode());
            }
            RawTtlSplitAction::Begin { spec, empty_at } => {
                out.push(1);
                put_bytes(out, &spec.encode());
                out.extend_from_slice(&empty_at.term.to_be_bytes());
                out.extend_from_slice(&empty_at.index.to_be_bytes());
            }
            RawTtlSplitAction::Append {
                target,
                expected,
                rows,
            } => {
                out.push(2);
                put_bytes(out, &target.encode());
                out.extend_from_slice(expected.as_bytes());
                out.extend_from_slice(&(rows.len() as u32).to_be_bytes());
                for row in rows {
                    put_bytes(out, &row.key);
                    put_bytes(out, &row.value);
                    out.push(u8::from(row.expiry.is_some()));
                    if let Some(expiry) = row.expiry {
                        out.extend_from_slice(&expiry.encode());
                    }
                }
            }
            RawTtlSplitAction::Finish { target, expected } => {
                out.push(3);
                put_bytes(out, &target.encode());
                out.extend_from_slice(expected.as_bytes());
            }
        }
    }
    pub(super) fn decode_from(r: &mut Reader<'_>) -> Result<Self> {
        let action = match r.u8()? {
            0 => RawTtlSplitAction::Seal(SplitPlan::decode(&r.bytes()?)?),
            1 => RawTtlSplitAction::Begin {
                spec: ImportSpec::decode(&r.bytes()?)?,
                empty_at: AppliedPosition {
                    term: r.u64()?,
                    index: r.u64()?,
                },
            },
            tag @ (2 | 3) => {
                let target = DataRange::decode(&r.bytes()?)?;
                let expected = RootDigest::from_bytes(r.take(32)?.try_into().unwrap());
                if tag == 3 {
                    RawTtlSplitAction::Finish { target, expected }
                } else {
                    let n = r.u32()? as usize;
                    if n == 0 || n > MAX_PAGE_ROWS {
                        return Err(invalid());
                    }
                    let mut rows = Vec::with_capacity(n);
                    for _ in 0..n {
                        let key = r.bytes()?;
                        let value = r.bytes()?;
                        let expiry = match r.u8()? {
                            0 => None,
                            1 => Some(ExpiryRecord::decode(r.take(64)?)?),
                            _ => return Err(invalid()),
                        };
                        rows.push(ImportRow { key, value, expiry });
                    }
                    RawTtlSplitAction::Append {
                        target,
                        expected,
                        rows,
                    }
                }
            }
            _ => return Err(invalid()),
        };
        Self::new(action)
    }
}
