//! A transfer source minted from one actual applied, sealed parent snapshot.
//! No public constructor, deserialization, or caller-supplied digest authority.
use super::*;
use crate::command::RawTtlSplit;
use kv9_common::codec::{decode_key, encode_key, keyspace_range, KeyMode};
use kv9_common::data_range::RANGE_KEY;
use kv9_common::raw_ttl::split::{
    ImportRow, ImportSpec, ImportState, SplitSeal, IMPORT_KEY, MAX_PAGE_ROWS, SEAL_KEY,
};
use kv9_common::raw_ttl::{ExpiryRecord, CLOCK_KEY};
use kv9_common::RootDigest;
use kv9_engine::ReadView;

pub struct SealedTtlSplitSource {
    view: Box<dyn ReadView>,
    specs: [ImportSpec; 2],
}
fn invalid(message: &str) -> Error {
    Error::Raft(format!("TTL split source: {message}"))
}
pub(super) fn unexpired(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err(Error::MetaNotReady(
            "TTL split preparation deadline expired".into(),
        ));
    }
    Ok(())
}
fn resident<'a>(view: &'a dyn ReadView, cf: ColumnFamily, key: &[u8]) -> Result<Option<&'a [u8]>> {
    view.get_resident(cf, key)
        .ok_or_else(|| invalid("resident view capability missing"))
}
fn count(view: &dyn ReadView, cf: ColumnFamily) -> Result<u64> {
    view.resident_len(cf)
        .and_then(|n| n.try_into().ok())
        .ok_or_else(|| invalid("exact CF count capability missing"))
}
fn row(view: &dyn ReadView, key: Vec<u8>, value: Vec<u8>) -> Result<ImportRow> {
    Ok(ImportRow {
        expiry: resident(view, ColumnFamily::Write, &key)?
            .map(ExpiryRecord::decode)
            .transpose()?,
        key,
        value,
    })
}
impl SealedTtlSplitSource {
    pub fn spec(&self, high: bool) -> &ImportSpec {
        &self.specs[usize::from(high)]
    }
    pub(super) fn capture(
        view: Box<dyn ReadView>,
        seal: SplitSeal,
        deadline: Instant,
    ) -> Result<Self> {
        let owner = seal.plan.parent.region;
        Self::capture_contents(view, seal, owner, deadline)
    }
    fn capture_contents(
        view: Box<dyn ReadView>,
        seal: SplitSeal,
        owner: kv9_common::RegionId,
        deadline: Instant,
    ) -> Result<Self> {
        let empty = |high| {
            let mut s = ImportSpec {
                seal: seal.clone(),
                high,
                rows: 0,
                digest: RootDigest::from_bytes([0; 32]),
            };
            s.digest = s.seed();
            s
        };
        let mut specs = [empty(false), empty(true)];
        let budgets = [
            RawTtlSplit::append_row_bytes_budget(specs[0].target())?,
            RawTtlSplit::append_row_bytes_budget(specs[1].target())?,
        ];
        let pair = crate::state_machine::manifest_pair_key(owner.0);
        let mut history = crate::state_machine::manifest_gen_key(owner.0, 0);
        history.truncate(history.len() - 8);
        let mut defaults = 0u64;
        let mut expiries = 0u64;
        // All legal Raw and local system keys are < ff. Exact resident counts
        // below also reject any unknown key outside this traversal interval.
        for entry in view.iter(ColumnFamily::Default, b"", b"\xff")? {
            unexpired(deadline)?;
            let (key, value) = entry?;
            defaults = defaults
                .checked_add(1)
                .ok_or_else(|| invalid("row count overflow"))?;
            if [
                RANGE_KEY,
                CLOCK_KEY,
                SEAL_KEY,
                IMPORT_KEY,
                COMPACTION_CONFIRMED_KEY,
            ]
            .contains(&key.as_slice())
                || key == pair
                || key.starts_with(&history) && key.len() == history.len() + 8
            {
                // Local checkpoint/transfer metadata is not user payload. Its
                // authority is established by apply/recovery, never copied.
                continue;
            }
            let item = row(view.as_ref(), key, value)?;
            item.validate(&seal.plan.parent)?;
            let high = decode_key(&item.key)?.user_key >= seal.plan.high.start.as_slice();
            let spec = &mut specs[usize::from(high)];
            item.validate(spec.target())?;
            if item.encoded_bytes()? > budgets[usize::from(high)] {
                return Err(invalid("one source row exceeds transfer frame budget"));
            }
            spec.rows = spec
                .rows
                .checked_add(1)
                .ok_or_else(|| invalid("row count overflow"))?;
            spec.digest = item.fold(spec.digest);
            expiries += u64::from(item.expiry.is_some());
        }
        if count(view.as_ref(), ColumnFamily::Default)? != defaults
            || count(view.as_ref(), ColumnFamily::Write)? != expiries
            || count(view.as_ref(), ColumnFamily::Lock)? != 0
        {
            return Err(invalid(
                "orphan expiry, foreign key or unsupported CF contents",
            ));
        }
        for spec in &specs {
            spec.validate()?;
        }
        unexpired(deadline)?;
        Ok(Self { view, specs })
    }
    pub(super) fn verify_target(
        &self,
        view: Box<dyn ReadView>,
        high: bool,
        deadline: Instant,
    ) -> Result<()> {
        let expected = self.spec(high);
        let state = resident(view.as_ref(), ColumnFamily::Default, IMPORT_KEY)?
            .map(ImportState::decode)
            .transpose()?
            .filter(|s| s.complete && s.spec == *expected)
            .ok_or_else(|| invalid("target has no matching completed import"))?;
        if resident(view.as_ref(), ColumnFamily::Default, RANGE_KEY)?
            != Some(expected.target().encode().as_slice())
            || resident(view.as_ref(), ColumnFamily::Default, SEAL_KEY)?.is_some()
        {
            return Err(invalid("target ownership differs or is already sealed"));
        }
        let clock = resident(view.as_ref(), ColumnFamily::Default, CLOCK_KEY)?
            .map(ClockWatermark::decode)
            .transpose()?;
        validate_local(
            (
                expected.target().root,
                expected.target().region,
                expected.target().creation,
            ),
            Some(expected.target()),
            clock,
            None,
            Some(&state),
        )?;
        let actual = Self::capture_contents(
            view,
            expected.seal.clone(),
            expected.target().region,
            deadline,
        )?;
        if actual.spec(high) != expected || actual.spec(!high).rows != 0 {
            return Err(invalid(
                "target contents differ from committed source image",
            ));
        }
        Ok(())
    }
    pub(super) fn next_page(
        &self,
        state: &ImportState,
        deadline: Instant,
    ) -> Result<Vec<ImportRow>> {
        state.validate()?;
        if state.complete || &state.spec != self.spec(state.spec.high) {
            return Err(invalid("import differs from source or already completed"));
        }
        let target = state.spec.target();
        let start = match &state.last {
            Some(last) => {
                let mut next = last.clone();
                next.push(0);
                next
            }
            None => encode_key(KeyMode::Raw, target.keyspace, &target.start)?,
        };
        let end = if target.end.is_empty() {
            keyspace_range(KeyMode::Raw, target.keyspace)?.1
        } else {
            encode_key(KeyMode::Raw, target.keyspace, &target.end)?
        };
        let mut bytes = RawTtlSplit::append_row_bytes_budget(target)?;
        let mut rows = Vec::new();
        for entry in self.view.iter(ColumnFamily::Default, &start, &end)? {
            unexpired(deadline)?;
            let (key, value) = entry?;
            let item = row(self.view.as_ref(), key, value)?;
            let size = item.encoded_bytes()?;
            if size > bytes {
                break;
            }
            bytes -= size;
            rows.push(item);
            if rows.len() == MAX_PAGE_ROWS {
                break;
            }
        }
        if rows.is_empty() && state.copied != state.spec.rows {
            return Err(invalid(
                "import cursor or page budget does not reach source",
            ));
        }
        unexpired(deadline)?;
        Ok(rows)
    }
}
