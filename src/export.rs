//! Serialising (parts of) result sets: clipboard formats and file export.

use std::fmt::Write as _;
use std::io::Write as _;

use crate::db::meta::{quote_ident, quote_literal};
use crate::db::result::ResultSet;
use crate::db::types::Cat;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Tsv,
    Csv,
    Json,
    Markdown,
    Insert,
    /// `UPDATE … SET … WHERE <primary key>` per row
    Update,
    /// `col = value AND …` per row, rows joined with OR
    Where,
    /// TypeScript array of typed objects
    TypeScript,
    InList,
}

impl Format {
    pub const FILE_FORMATS: [Format; 5] = [Format::Csv, Format::Json, Format::Markdown, Format::Insert, Format::TypeScript];
    pub fn label(self) -> &'static str {
        match self {
            Format::Tsv => "TSV",
            Format::Csv => "CSV",
            Format::Json => "JSON",
            Format::Markdown => "Markdown",
            Format::Insert => "SQL INSERT",
            Format::Update => "SQL UPDATE",
            Format::Where => "SQL WHERE",
            Format::TypeScript => "TypeScript",
            Format::InList => "IN (...) list",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Format::Tsv => "tsv",
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Markdown => "md",
            Format::Insert | Format::Update | Format::Where => "sql",
            Format::TypeScript => "ts",
            Format::InList => "txt",
        }
    }
}

fn csv_field(out: &mut String, v: &str, sep: char) {
    if v.contains(sep) || v.contains('"') || v.contains('\n') || v.contains('\r') {
        out.push('"');
        out.push_str(&v.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(v);
    }
}

fn json_value(out: &mut String, v: Option<&str>, cat: Cat) {
    match v {
        None => out.push_str("null"),
        Some(v) => match cat {
            Cat::Number if v.parse::<f64>().is_ok_and(|f| f.is_finite()) => out.push_str(v),
            Cat::Bool => out.push_str(if v == "t" { "true" } else { "false" }),
            Cat::Json if serde_json::from_str::<serde_json::Value>(v).is_ok() => out.push_str(v),
            _ => out.push_str(&serde_json::to_string(v).unwrap_or_default()),
        },
    }
}

fn where_cond(name: &str, v: Option<&str>, cat: Cat) -> String {
    match v {
        None => format!("{} IS NULL", quote_ident(name)),
        Some(_) => format!("{} = {}", quote_ident(name), sql_value(v, cat)),
    }
}

fn sql_value(v: Option<&str>, cat: Cat) -> String {
    match v {
        None => "NULL".into(),
        Some(v) => match cat {
            Cat::Number if v.parse::<f64>().is_ok() => v.to_string(),
            Cat::Bool => {
                if v == "t" {
                    "true".into()
                } else {
                    "false".into()
                }
            }
            _ => quote_literal(v),
        },
    }
}

/// Formats the given rows × columns of `rs`.
pub fn format(rs: &ResultSet, rows: &[usize], cols: &[usize], cats: &[Cat], fmt: Format, headers: bool, table: &str) -> String {
    format_with_pk(rs, rows, cols, cats, fmt, headers, table, &[])
}

/// Like [`format`]; `pk` (source column indexes) identifies rows for UPDATE.
#[allow(clippy::too_many_arguments)]
pub fn format_with_pk(rs: &ResultSet, rows: &[usize], cols: &[usize], cats: &[Cat], fmt: Format, headers: bool, table: &str, pk: &[usize]) -> String {
    let mut out = String::new();
    let get = |r: usize, c: usize| rs.get(r, c);
    let names: Vec<&str> = cols.iter().map(|&c| rs.columns.get(c).map_or("?", |x| x.name.as_str())).collect();
    match fmt {
        Format::Tsv | Format::Csv => {
            let sep = if fmt == Format::Tsv { '\t' } else { ',' };
            if headers {
                for (i, n) in names.iter().enumerate() {
                    if i > 0 {
                        out.push(sep);
                    }
                    csv_field(&mut out, n, sep);
                }
                out.push('\n');
            }
            for (ri, &r) in rows.iter().enumerate() {
                for (i, &c) in cols.iter().enumerate() {
                    if i > 0 {
                        out.push(sep);
                    }
                    match get(r, c) {
                        Some(v) if fmt == Format::Tsv && !v.contains(['\t', '\n']) => out.push_str(v),
                        Some(v) => csv_field(&mut out, v, sep),
                        None if fmt == Format::Tsv => out.push_str("NULL"),
                        None => {}
                    }
                }
                if ri + 1 < rows.len() || headers || fmt == Format::Csv {
                    out.push('\n');
                }
            }
        }
        Format::Json => {
            out.push_str("[\n");
            for (ri, &r) in rows.iter().enumerate() {
                out.push_str("  {");
                for (i, &c) in cols.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&serde_json::to_string(names[i]).unwrap_or_default());
                    out.push_str(": ");
                    json_value(&mut out, get(r, c), cats.get(c).copied().unwrap_or(Cat::Other));
                }
                out.push('}');
                if ri + 1 < rows.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("]\n");
        }
        Format::Markdown => {
            let esc = |s: &str| s.replace('|', "\\|").replace('\n', "<br>");
            out.push('|');
            for n in &names {
                let _ = write!(out, " {} |", esc(n));
            }
            out.push_str("\n|");
            for &c in cols {
                out.push_str(if cats.get(c) == Some(&Cat::Number) { " ---: |" } else { " --- |" });
            }
            out.push('\n');
            for &r in rows {
                out.push('|');
                for &c in cols {
                    let _ = write!(out, " {} |", get(r, c).map(esc).unwrap_or_else(|| "NULL".into()));
                }
                out.push('\n');
            }
        }
        Format::Insert => {
            let t = if table.is_empty() { "table_name" } else { table };
            let col_list = names.iter().map(|n| quote_ident(n)).collect::<Vec<_>>().join(", ");
            for &r in rows {
                let vals = cols.iter().map(|&c| sql_value(get(r, c), cats.get(c).copied().unwrap_or(Cat::Other))).collect::<Vec<_>>().join(", ");
                let _ = writeln!(out, "INSERT INTO {t} ({col_list}) VALUES ({vals});");
            }
        }
        Format::Update => {
            let t = if table.is_empty() { "table_name" } else { table };
            // Identify rows by the primary key, else by the first selected column.
            let key: Vec<usize> = if pk.is_empty() { cols.iter().take(1).copied().collect() } else { pk.to_vec() };
            let cat = |c: usize| cats.get(c).copied().unwrap_or(Cat::Other);
            for &r in rows {
                let sets: Vec<String> = cols
                    .iter()
                    .filter(|c| !key.contains(c))
                    .map(|&c| format!("{} = {}", quote_ident(&rs.columns[c].name), sql_value(get(r, c), cat(c))))
                    .collect();
                if sets.is_empty() {
                    continue;
                }
                let cond: Vec<String> = key.iter().map(|&c| where_cond(&rs.columns[c].name, get(r, c), cat(c))).collect();
                let _ = writeln!(out, "UPDATE {t} SET {} WHERE {};", sets.join(", "), cond.join(" AND "));
            }
        }
        Format::Where => {
            let cat = |c: usize| cats.get(c).copied().unwrap_or(Cat::Other);
            let conds: Vec<String> = rows
                .iter()
                .map(|&r| {
                    let parts: Vec<String> = cols.iter().map(|&c| where_cond(&rs.columns[c].name, get(r, c), cat(c))).collect();
                    if parts.len() > 1 && rows.len() > 1 { format!("({})", parts.join(" AND ")) } else { parts.join(" AND ") }
                })
                .collect();
            let _ = write!(out, "WHERE {}", conds.join("\n   OR "));
        }
        Format::TypeScript => {
            out.push_str("const rows = [\n");
            for &r in rows {
                out.push_str("  { ");
                for (i, &c) in cols.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    let name = &rs.columns[c].name;
                    let is_ident =
                        name.chars().next().is_some_and(|ch| ch.is_alphabetic() || ch == '_') && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_');
                    if is_ident {
                        out.push_str(name);
                    } else {
                        out.push_str(&serde_json::to_string(name).unwrap_or_default());
                    }
                    out.push_str(": ");
                    json_value(&mut out, get(r, c), cats.get(c).copied().unwrap_or(Cat::Other));
                }
                out.push_str(" },\n");
            }
            out.push_str("];\n");
        }
        Format::InList => {
            let vals: Vec<String> = rows
                .iter()
                .flat_map(|&r| cols.iter().map(move |&c| (r, c)))
                .map(|(r, c)| sql_value(get(r, c), cats.get(c).copied().unwrap_or(Cat::Other)))
                .collect();
            let _ = write!(out, "({})", vals.join(", "));
        }
    }
    out
}

/// Writes result chunks to a file as they arrive, so exports of any size
/// need only one chunk in memory.
pub struct StreamWriter {
    w: std::io::BufWriter<std::fs::File>,
    fmt: Format,
    table: String,
    started: bool,
    rows: u64,
}

impl StreamWriter {
    pub fn create(path: &std::path::Path, fmt: Format, table: &str) -> std::io::Result<StreamWriter> {
        let f = std::fs::File::create(path)?;
        Ok(StreamWriter { w: std::io::BufWriter::with_capacity(1 << 20, f), fmt, table: table.to_string(), started: false, rows: 0 })
    }

    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// Appends `rows` of `rs` (all columns).
    pub fn write(&mut self, rs: &ResultSet, rows: &[usize], cats: &[Cat]) -> std::io::Result<()> {
        let cols: Vec<usize> = (0..rs.ncols()).collect();
        if !self.started {
            self.started = true;
            match self.fmt {
                Format::Csv | Format::Markdown => self.w.write_all(format(rs, &[], &cols, cats, self.fmt, true, &self.table).as_bytes())?,
                Format::Json => self.w.write_all(b"[\n")?,
                Format::TypeScript => self.w.write_all(b"const rows = [\n")?,
                _ => {}
            }
        }
        if rows.is_empty() {
            return Ok(());
        }
        let mut s = format(rs, rows, &cols, cats, self.fmt, false, &self.table);
        match self.fmt {
            Format::Json => {
                s = s.trim_start_matches("[\n").trim_end_matches("]\n").trim_end_matches('\n').to_string();
                if self.rows > 0 {
                    self.w.write_all(b",\n")?;
                }
            }
            Format::TypeScript => s = s.trim_start_matches("const rows = [\n").trim_end_matches("];\n").to_string(),
            Format::Markdown => s = s.lines().skip(2).map(|l| format!("{l}\n")).collect(),
            _ => {}
        }
        self.w.write_all(s.as_bytes())?;
        self.rows += rows.len() as u64;
        Ok(())
    }

    pub fn finish(mut self, rs_for_header: Option<&ResultSet>, cats: &[Cat]) -> std::io::Result<u64> {
        if !self.started
            && let Some(rs) = rs_for_header
        {
            self.write(rs, &[], cats)?;
        }
        match self.fmt {
            Format::Json => self.w.write_all(if self.rows > 0 { b"\n]\n" } else { b"]\n" })?,
            Format::TypeScript => self.w.write_all(b"];\n")?,
            _ => {}
        }
        self.w.flush()?;
        Ok(self.rows)
    }
}

/// Writes the given rows of a loaded result set to a file.
pub fn write_file(path: &std::path::Path, rs: &ResultSet, rows: &[usize], cats: &[Cat], fmt: Format, table: &str) -> std::io::Result<u64> {
    let mut w = StreamWriter::create(path, fmt, table)?;
    for chunk in rows.chunks(10_000) {
        w.write(rs, chunk, cats)?;
    }
    w.finish(Some(rs), cats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::result::Column;

    fn sample() -> ResultSet {
        let mut rs = ResultSet::new(vec![Column::text("id"), Column::text("name")]);
        rs.push_text_row(&[Some("1"), Some("a,\"b\"")]);
        rs.push_text_row(&[Some("2"), None]);
        rs
    }

    #[test]
    fn csv_json_insert() {
        let rs = sample();
        let cats = [Cat::Number, Cat::String];
        let csv = format(&rs, &[0, 1], &[0, 1], &cats, Format::Csv, true, "");
        assert_eq!(csv, "id,name\n1,\"a,\"\"b\"\"\"\n2,\n");
        let json = format(&rs, &[0, 1], &[0, 1], &cats, Format::Json, true, "");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v[0]["id"], 1);
        assert!(v[1]["name"].is_null());
        let ins = format(&rs, &[1], &[0, 1], &cats, Format::Insert, true, "t");
        assert_eq!(ins, "INSERT INTO t (id, name) VALUES (2, NULL);\n");
        let upd = format_with_pk(&rs, &[0], &[0, 1], &cats, Format::Update, true, "t", &[0]);
        assert_eq!(upd, "UPDATE t SET name = 'a,\"b\"' WHERE id = 1;\n");
        let wh = format(&rs, &[0, 1], &[0], &cats, Format::Where, true, "t");
        assert_eq!(wh, "WHERE id = 1\n   OR id = 2");
        let ts = format(&rs, &[1], &[0, 1], &cats, Format::TypeScript, true, "t");
        assert_eq!(ts, "const rows = [\n  { id: 2, name: null },\n];\n");
    }

    #[test]
    fn file_json_valid() {
        let rs = sample();
        let p = std::env::temp_dir().join("trunq_export_test.json");
        let cats = [Cat::Number, Cat::String];
        // two chunks must still produce one valid JSON array
        let mut w = StreamWriter::create(&p, Format::Json, "").unwrap();
        w.write(&rs, &[0], &cats).unwrap();
        w.write(&rs, &[1], &cats).unwrap();
        assert_eq!(w.finish(None, &cats).unwrap(), 2);
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
        let p = std::env::temp_dir().join("trunq_export_test.csv");
        assert_eq!(write_file(&p, &rs, &[0, 1], &cats, Format::Csv, "").unwrap(), 2);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "id,name\n1,\"a,\"\"b\"\"\"\n2,\n");
    }
}
