//! Borrowed bulk validation and iteration. Successful parsing allocates no
//! entry collection or key/value copies. SharedSst retains the caller's Bytes
//! owner (including a remote buffer reservation) until iteration is finished.
use super::*;
use bytes::Bytes;
use std::ops::Range;

#[derive(Clone, Debug)]
struct Validated {
    cf: ColumnFamily,
    limits: Limits,
    count: usize,
    entries: Range<usize>,
    smallest: Range<usize>,
    largest: Range<usize>,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    limits: Limits,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| Error::Engine("sst: length overflow".into()))?;
        if end > self.bytes.len() {
            return Err(Error::Engine(format!(
                "sst: truncated — wanted {n} bytes at offset {}, {} remain",
                self.pos,
                self.bytes.len() - self.pos
            )));
        }
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn len_prefixed(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        if n > self.limits.field_len {
            return Err(Error::Engine(format!(
                "sst: length field {n} exceeds the {} byte limit",
                self.limits.field_len
            )));
        }
        self.take(n)
    }
}

fn validate(bytes: &[u8], limits: Limits) -> Result<Validated> {
    if bytes.len() > limits.total_bytes {
        return Err(Error::Engine(format!(
            "sst: buffer of {} bytes exceeds the {} byte limit",
            bytes.len(),
            limits.total_bytes
        )));
    }
    // Checksum first: nothing else in the buffer may be trusted until it matches.
    if bytes.len() < 4 {
        return Err(Error::Engine("sst: too short to contain a checksum".into()));
    }
    let split = bytes.len() - 4;
    let (body, stored) = bytes.split_at(split);
    let stored = u32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]]);
    let actual = crc32(body);
    if stored != actual {
        return Err(Error::Engine(format!(
            "sst: checksum mismatch — stored {stored:#010x}, computed {actual:#010x}"
        )));
    }

    let mut c = Cursor {
        bytes: body,
        pos: 0,
        limits,
    };
    if c.take(4)? != MAGIC {
        return Err(Error::Engine("sst: bad magic — not an SST".into()));
    }
    let version = c.take(1)?[0];
    if version != VERSION {
        return Err(Error::Engine(format!(
            "sst: unknown format version {version} (this build writes {VERSION}) — \
                 refusing to guess"
        )));
    }
    let cf = cf_from_code(c.take(1)?[0])?;
    let count = c.u32()? as usize;
    if count > limits.entries {
        return Err(Error::Engine(format!(
            "sst: entry count {count} exceeds the {} entry limit",
            limits.entries
        )));
    }

    let entries_start = c.pos;
    let mut first = None;
    let mut prev: Option<&[u8]> = None;
    for i in 0..count {
        let key = c.len_prefixed()?;
        c.len_prefixed()?;
        if let Some(p) = &prev {
            if key <= *p {
                return Err(Error::Engine(format!(
                    "sst: entries out of order at index {i} — keys must strictly ascend"
                )));
            }
        }
        first.get_or_insert(key);
        prev = Some(key);
    }

    let entries_end = c.pos;
    let smallest = c.len_prefixed()?;
    let smallest_start = c.pos - smallest.len();
    let largest = c.len_prefixed()?;
    let largest_start = c.pos - largest.len();

    if c.pos != body.len() {
        return Err(Error::Engine(format!(
            "sst: {} trailing bytes after the footer",
            body.len() - c.pos
        )));
    }

    if count == 0 {
        return Err(Error::Engine("sst: no entries — refusing".into()));
    }

    // The declared range is checked against the data rather than believed. A file
    // whose footer disagrees with its entries has two answers to one question.
    if Some(smallest) != first || Some(largest) != prev {
        return Err(Error::Engine(
            "sst: declared key range disagrees with the entries it bounds".into(),
        ));
    }

    Ok(Validated {
        cf,
        limits,
        count,
        entries: entries_start..entries_end,
        smallest: smallest_start..smallest_start + smallest.len(),
        largest: largest_start..largest_start + largest.len(),
    })
}

#[derive(Debug)]
pub(crate) struct SstView<'a> {
    bytes: &'a [u8],
    validated: Validated,
}
impl<'a> SstView<'a> {
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_limits(bytes, Limits::PRODUCTION)
    }
    pub(super) fn parse_with_limits(bytes: &'a [u8], limits: Limits) -> Result<Self> {
        Ok(Self {
            bytes,
            validated: validate(bytes, limits)?,
        })
    }
    pub(crate) fn column_family(&self) -> ColumnFamily {
        self.validated.cf
    }
    pub(crate) fn len(&self) -> usize {
        self.validated.count
    }
    pub(crate) fn smallest_key(&self) -> &[u8] {
        &self.bytes[self.validated.smallest.clone()]
    }
    pub(crate) fn largest_key(&self) -> &[u8] {
        &self.bytes[self.validated.largest.clone()]
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&'a [u8], &'a [u8])> + 'a {
        let mut c = Cursor {
            bytes: &self.bytes[self.validated.entries.clone()],
            pos: 0,
            limits: self.validated.limits,
        };
        // This exact immutable input has already passed length, count and order
        // checks. Iteration cannot expose an unverified prefix or read footer data.
        (0..self.validated.count).map(move |_| {
            (
                c.len_prefixed().expect("validated immutable SST key"),
                c.len_prefixed().expect("validated immutable SST value"),
            )
        })
    }
}

pub(crate) struct SharedSst {
    bytes: Bytes,
    validated: Validated,
}
impl SharedSst {
    pub(crate) fn parse(bytes: Bytes) -> Result<Self> {
        let validated = validate(&bytes, Limits::PRODUCTION)?;
        Ok(Self { bytes, validated })
    }
    pub(crate) fn view(&self) -> SstView<'_> {
        SstView {
            bytes: &self.bytes,
            validated: self.validated.clone(),
        }
    }
    pub(crate) fn column_family(&self) -> ColumnFamily {
        self.validated.cf
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.view().iter()
    }
}

#[cfg(test)]
mod tests;
