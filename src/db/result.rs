//! Compact, allocation-friendly storage for query results.
//!
//! All cell text lives in one contiguous byte buffer; each cell is described
//! by a single `u64` end offset (high bit = NULL). A million-row, ten-column
//! result therefore costs ~80MB of offsets + the raw text, instead of ten
//! million individually allocated `String`s.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend::DataRowBody;

const NULL_BIT: u64 = 1 << 63;
static NEXT_RESULT: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Default)]
pub struct Column {
    pub name: String,
    pub type_oid: u32,
    /// Oid of the source table if the column is a plain table column, else 0.
    pub table_oid: u32,
    pub column_id: i16,
}

impl Column {
    pub fn text(name: &str) -> Column {
        Column { name: name.to_string(), type_oid: 25, ..Default::default() }
    }
}

#[derive(Clone, Debug)]
pub struct ResultSet {
    pub columns: Arc<Vec<Column>>,
    ncols: usize,
    data: Vec<u8>,
    ends: Vec<u64>,
    nrows: usize,
    /// Stable identity for view caches, including equal-sized replacement results.
    pub generation: u64,
    /// Command tag (`SELECT 3`, `INSERT 0 1`, ...).
    pub tag: String,
}

impl Default for ResultSet {
    fn default() -> Self {
        Self {
            columns: Arc::new(Vec::new()),
            ncols: 0,
            data: Vec::new(),
            ends: Vec::new(),
            nrows: 0,
            generation: NEXT_RESULT.fetch_add(1, Ordering::Relaxed),
            tag: String::new(),
        }
    }
}

impl ResultSet {
    pub fn new(columns: Vec<Column>) -> ResultSet {
        let ncols = columns.len();
        ResultSet { columns: Arc::new(columns), ncols, ..Default::default() }
    }

    pub fn with_ncols(ncols: usize) -> ResultSet {
        ResultSet { ncols, ..Default::default() }
    }

    pub fn ncols(&self) -> usize {
        self.ncols
    }

    pub fn nrows(&self) -> usize {
        self.nrows
    }

    pub fn is_empty(&self) -> bool {
        self.nrows == 0
    }

    /// Approximate memory used by the stored cells.
    pub fn byte_size(&self) -> usize {
        self.data.len() + self.ends.len() * 8
    }

    pub(crate) fn push_row(&mut self, row: &DataRowBody) -> Result<()> {
        let buf = row.buffer();
        let mut ranges = row.ranges();
        let mut n = 0;
        while let Some(r) = ranges.next()? {
            match r {
                Some(r) => {
                    self.data.extend_from_slice(&buf[r]);
                    self.ends.push(self.data.len() as u64);
                }
                None => self.ends.push(self.data.len() as u64 | NULL_BIT),
            }
            n += 1;
        }
        if self.ncols == 0 && self.nrows == 0 {
            self.ncols = n;
        }
        self.nrows += 1;
        Ok(())
    }

    pub fn push_text_row(&mut self, cells: &[Option<&str>]) {
        debug_assert_eq!(cells.len(), self.ncols);
        for c in cells {
            match c {
                Some(s) => {
                    self.data.extend_from_slice(s.as_bytes());
                    self.ends.push(self.data.len() as u64);
                }
                None => self.ends.push(self.data.len() as u64 | NULL_BIT),
            }
        }
        self.nrows += 1;
    }

    /// Appends all rows of `other` (which must have the same column count).
    pub fn append(&mut self, other: ResultSet) {
        if other.nrows == 0 {
            return;
        }
        if self.nrows == 0 && self.data.is_empty() {
            let cols = std::mem::take(&mut self.columns);
            let tag = std::mem::take(&mut self.tag);
            *self = other;
            if !cols.is_empty() {
                self.columns = cols;
            }
            if !tag.is_empty() {
                self.tag = tag;
            }
            return;
        }
        let base = self.data.len() as u64;
        self.data.extend_from_slice(&other.data);
        self.ends.reserve(other.ends.len());
        for e in other.ends {
            let null = e & NULL_BIT;
            self.ends.push(((e & !NULL_BIT) + base) | null);
        }
        self.nrows += other.nrows;
    }

    #[inline]
    pub fn get(&self, row: usize, col: usize) -> Option<&str> {
        if row >= self.nrows || col >= self.ncols {
            return None;
        }
        let idx = row * self.ncols + col;
        let end = *self.ends.get(idx)?;
        if end & NULL_BIT != 0 {
            return None;
        }
        let start = if idx == 0 { 0 } else { self.ends[idx - 1] & !NULL_BIT };
        let bytes = &self.data[start as usize..end as usize];
        // Server sends UTF-8 (client_encoding=UTF8); fall back defensively.
        Some(std::str::from_utf8(bytes).unwrap_or("<invalid utf-8>"))
    }

    /// Value or empty string; handy for catalog queries.
    pub fn s(&self, row: usize, col: usize) -> &str {
        self.get(row, col).unwrap_or("")
    }

    pub fn col_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    pub fn rows(&self) -> impl Iterator<Item = usize> {
        0..self.nrows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_column_does_not_read_the_next_row() {
        let mut rs = ResultSet::new(vec![Column::text("v")]);
        rs.push_text_row(&[Some("first")]);
        rs.push_text_row(&[Some("second")]);
        assert_eq!(rs.get(0, 1), None);
        assert_eq!(rs.get(usize::MAX, 0), None);
        assert_eq!(rs.get(0, usize::MAX), None);
    }

    #[test]
    fn append_and_get() {
        let mut a = ResultSet::new(vec![Column::text("a"), Column::text("b")]);
        a.push_text_row(&[Some("1"), None]);
        let mut b = ResultSet::with_ncols(2);
        b.push_text_row(&[Some("héllo"), Some("")]);
        a.append(b);
        assert_eq!(a.nrows(), 2);
        assert_eq!(a.get(0, 0), Some("1"));
        assert_eq!(a.get(0, 1), None);
        assert_eq!(a.get(1, 0), Some("héllo"));
        assert_eq!(a.get(1, 1), Some(""));
    }
}
