//! One serializer for owned writers and borrowed checkpoint rows. The caller
//! supplies the preflight size, which is verified before every append and again
//! against the complete frame. No collection of owned rows is constructed here.
use super::*;

pub(crate) fn encode_borrowed<'a>(
    cf: ColumnFamily,
    count: usize,
    encoded: usize,
    rows: impl Iterator<Item = (&'a [u8], &'a [u8])>,
) -> Result<Vec<u8>> {
    encode_rows(cf, count, encoded, Limits::PRODUCTION, rows)
}

pub(super) fn encode_rows<'a>(
    cf: ColumnFamily,
    count: usize,
    encoded: usize,
    limits: Limits,
    rows: impl Iterator<Item = (&'a [u8], &'a [u8])>,
) -> Result<Vec<u8>> {
    if count == 0 {
        return Err(Error::Engine(
            "sst: refusing to write an empty table — it would have no key range".into(),
        ));
    }
    let disk_count = on_disk_count(count, limits.entries)?;
    check_total_size(encoded, limits.total_bytes)?;
    check_append(0, 10, encoded)?;
    let mut out = Vec::with_capacity(encoded);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(cf_code(cf));
    out.extend_from_slice(&disk_count.to_le_bytes());

    let mut first = None;
    let mut last = None;
    let mut seen = 0;
    for (key, value) in rows {
        if seen == count {
            return Err(mismatch());
        }
        let key_len = on_disk_len(key.len(), "key", limits.field_len)?;
        let value_len = on_disk_len(value.len(), "value", limits.field_len)?;
        if last.is_some_and(|previous| key <= previous) {
            return Err(Error::Engine(
                "sst: entries must be added in strictly ascending key order".into(),
            ));
        }
        let row_bytes = key
            .len()
            .checked_add(value.len())
            .and_then(|n| n.checked_add(8))
            .ok_or_else(mismatch)?;
        check_append(out.len(), row_bytes, encoded)?;
        out.extend_from_slice(&key_len.to_le_bytes());
        out.extend_from_slice(key);
        out.extend_from_slice(&value_len.to_le_bytes());
        out.extend_from_slice(value);
        first.get_or_insert(key);
        last = Some(key);
        seen += 1;
    }
    if seen != count {
        return Err(mismatch());
    }
    let smallest = first.ok_or_else(mismatch)?;
    let largest = last.ok_or_else(mismatch)?;
    let footer_bytes = smallest
        .len()
        .checked_add(largest.len())
        .and_then(|n| n.checked_add(12))
        .ok_or_else(mismatch)?;
    if check_append(out.len(), footer_bytes, encoded)? != encoded {
        return Err(mismatch());
    }
    out.extend_from_slice(
        &on_disk_len(smallest.len(), "smallest key", limits.field_len)?.to_le_bytes(),
    );
    out.extend_from_slice(smallest);
    out.extend_from_slice(
        &on_disk_len(largest.len(), "largest key", limits.field_len)?.to_le_bytes(),
    );
    out.extend_from_slice(largest);
    let crc = crc32(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    Ok(out)
}

fn mismatch() -> Error {
    Error::Engine("sst: preflight size or entry count disagrees with rows".into())
}

fn check_append(current: usize, extra: usize, encoded: usize) -> Result<usize> {
    current
        .checked_add(extra)
        .filter(|end| *end <= encoded)
        .ok_or_else(mismatch)
}

#[cfg(test)]
mod tests;
