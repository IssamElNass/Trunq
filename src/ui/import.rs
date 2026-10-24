//! CSV / TSV import: drop a file on the window, check the inferred table,
//! and it is streamed with COPY FROM STDIN inside a single transaction.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use egui::{RichText, vec2};

use super::theme::{self, icon};
use super::widgets as w;
use crate::config::Profile;
use crate::db::meta::{Catalog, RelKind, quote_ident, quote_literal};
use crate::db::session::{Promise, Session};
use crate::util;

const TYPES: &[&str] = &["text", "integer", "bigint", "numeric", "double precision", "boolean", "date", "timestamp", "timestamptz", "uuid", "jsonb"];

/// Minimal RFC 4180 parser (quotes, doubled quotes, newlines in quotes).
pub fn parse_csv(text: &str, delim: char, max_rows: usize) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            c if c == delim => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                if rows.len() >= max_rows {
                    return rows;
                }
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

pub fn detect_delimiter(first_line: &str) -> char {
    [',', ';', '\t', '|'].into_iter().max_by_key(|d| first_line.matches(*d).count()).unwrap_or(',')
}

/// Infers a PostgreSQL type from sample values.
pub fn infer_type<'a>(values: impl Iterator<Item = &'a str>) -> &'static str {
    let mut seen = false;
    let (mut int, mut big, mut num, mut boolean, mut date, mut ts, mut uuid, mut json) = (true, true, true, true, true, true, true, true);
    for v in values {
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        seen = true;
        let i = v.parse::<i64>();
        int &= i.as_ref().is_ok_and(|x| i32::try_from(*x).is_ok()) && !(v.len() > 1 && v.starts_with('0'));
        big &= i.is_ok() && !(v.len() > 1 && v.starts_with('0'));
        num &= v.parse::<f64>().is_ok_and(|f| f.is_finite()) && !(v.len() > 1 && v.starts_with('0') && !v.starts_with("0."));
        boolean &= matches!(v.to_ascii_lowercase().as_str(), "true" | "false" | "t" | "f" | "yes" | "no");
        date &= chrono::NaiveDate::parse_from_str(v, "%Y-%m-%d").is_ok();
        ts &=
            chrono::NaiveDateTime::parse_from_str(v.trim_end_matches('Z').replace('T', " ").split(['+']).next().unwrap_or(""), "%Y-%m-%d %H:%M:%S%.f").is_ok();
        uuid &= v.len() == 36 && v.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
        json &= (v.starts_with('{') || v.starts_with('[')) && serde_json::from_str::<serde_json::Value>(v).is_ok();
    }
    if !seen {
        return "text";
    }
    if int {
        "integer"
    } else if big {
        "bigint"
    } else if num {
        "numeric"
    } else if boolean {
        "boolean"
    } else if date {
        "date"
    } else if ts {
        "timestamptz"
    } else if uuid {
        "uuid"
    } else if json {
        "jsonb"
    } else {
        "text"
    }
}

/// Turns a header into a friendly SQL identifier.
pub fn sanitize_ident(s: &str, fallback: usize) -> String {
    let mut out = String::new();
    let mut last_us = false;
    for c in s.trim().trim_start_matches('\u{feff}').chars().flat_map(|c| c.to_lowercase()) {
        if c.is_alphanumeric() {
            out.push(c);
            last_us = false;
        } else if !last_us && !out.is_empty() {
            out.push('_');
            last_us = true;
        }
    }
    let out = out.trim_end_matches('_').to_string();
    if out.is_empty() {
        format!("column_{}", fallback + 1)
    } else if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("c_{out}")
    } else {
        out
    }
}

/// An import in flight: (result, bytes sent, cancel flag, start time).
type Running = (Promise<Result<String, String>>, Arc<AtomicU64>, Arc<AtomicBool>, Instant);

struct Col {
    name: String,
    ty: String,
    /// existing-table mode: target column
    target: String,
}

pub struct ImportDialog {
    path: PathBuf,
    size: u64,
    sample: String,
    delim: char,
    header: bool,
    preview: Vec<Vec<String>>,
    cols: Vec<Col>,
    pub profile_id: u64,
    pub database: String,
    schema: String,
    new_table: bool,
    table: String,
    truncate: bool,
    session: Option<Session>,
    running: Option<Running>,
    pub done: Option<Result<String, String>>,
    error: Option<String>,
}

pub enum ImportOutcome {
    Close,
    /// Imported: (profile, db, schema, table)
    Imported(u64, String, String, String),
}

impl ImportDialog {
    pub fn open(path: PathBuf, profile_id: u64, database: String) -> Result<ImportDialog, String> {
        let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        // Read up to 1 MB for preview & inference.
        let bytes = {
            use std::io::Read;
            let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 1 << 20];
            let n = f.read(&mut buf).map_err(|e| e.to_string())?;
            buf.truncate(n);
            buf
        };
        let mut sample = String::from_utf8_lossy(&bytes).into_owned();
        if bytes.len() == 1 << 20
            && let Some(p) = sample.rfind('\n')
        {
            sample.truncate(p + 1);
        }
        let first_line = sample.lines().next().unwrap_or("");
        let delim = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("tsv")) { '\t' } else { detect_delimiter(first_line) };
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "import".into());
        let mut d = ImportDialog {
            size: meta.len(),
            sample,
            delim,
            header: true,
            preview: Vec::new(),
            cols: Vec::new(),
            profile_id,
            database,
            schema: "public".into(),
            new_table: true,
            table: sanitize_ident(&stem, 0),
            truncate: false,
            session: None,
            running: None,
            done: None,
            error: None,
            path,
        };
        d.reparse();
        Ok(d)
    }

    /// Starts the import on a dedicated connection (closed when done).
    pub fn start(&mut self, ctx: &egui::Context, profiles: &[Profile], passwords: &std::collections::HashMap<u64, String>) {
        match profiles.iter().find(|p| p.id == self.profile_id) {
            Some(p) if !self.table.trim().is_empty() => {
                let (setup, copy) = self.sql();
                let s = Session::connect(p, &self.database, "CSV import", passwords.get(&p.id).cloned(), ctx);
                let progress = Arc::new(AtomicU64::new(0));
                let cancel = Arc::new(AtomicBool::new(false));
                let promise = s.import(setup, copy, self.path.clone(), progress.clone(), cancel.clone());
                self.session = Some(s);
                self.running = Some((promise, progress, cancel, Instant::now()));
                self.done = None;
                self.error = None;
            }
            _ => self.error = Some("Choose a connection and a table name".into()),
        }
    }

    pub fn set_schema(&mut self, schema: String) {
        self.schema = schema;
    }

    fn reparse(&mut self) {
        let rows = parse_csv(&self.sample, self.delim, 2001);
        let ncols = rows.iter().map(Vec::len).max().unwrap_or(0);
        let body = if self.header { rows.get(1..).unwrap_or(&[]) } else { &rows[..] };
        self.cols = (0..ncols)
            .map(|c| {
                let raw = if self.header { rows.first().and_then(|r| r.get(c)).cloned().unwrap_or_default() } else { String::new() };
                let name = if self.header { sanitize_ident(&raw, c) } else { format!("column_{}", c + 1) };
                let ty = infer_type(body.iter().filter_map(|r| r.get(c).map(String::as_str))).to_string();
                Col { target: name.clone(), name, ty }
            })
            .collect();
        // de-duplicate names
        let mut seen = std::collections::HashMap::new();
        for c in &mut self.cols {
            let n = seen.entry(c.name.clone()).or_insert(0);
            *n += 1;
            if *n > 1 {
                c.name = format!("{}_{}", c.name, n);
                c.target = c.name.clone();
            }
        }
        self.preview = body.iter().take(8).cloned().collect();
    }

    fn sql(&self) -> (String, String) {
        let qtable = format!("{}.{}", quote_ident(&self.schema), quote_ident(&self.table));
        let names: Vec<String> = self.cols.iter().map(|c| quote_ident(if self.new_table { &c.name } else { &c.target })).collect();
        let setup = if self.new_table {
            format!(
                "CREATE TABLE {qtable} (\n{}\n)",
                self.cols.iter().map(|c| format!("    {} {}", quote_ident(&c.name), c.ty)).collect::<Vec<_>>().join(",\n")
            )
        } else if self.truncate {
            format!("TRUNCATE {qtable}")
        } else {
            String::new()
        };
        let delim = quote_literal(&self.delim.to_string());
        let copy = format!(
            "COPY {qtable} ({}) FROM STDIN WITH (FORMAT csv, HEADER {}, DELIMITER {delim})",
            names.join(", "),
            if self.header { "true" } else { "false" }
        );
        (setup, copy)
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        profiles: &[Profile],
        catalog: Option<&Catalog>,
        passwords: &std::collections::HashMap<u64, String>,
    ) -> Option<ImportOutcome> {
        let pal = theme::p();
        let mut outcome = None;
        if let Some((p, _, _, _)) = &self.running
            && let Some(r) = p.take()
        {
            self.running = None;
            self.session = None; // release the connection
            if r.is_ok() {
                outcome = Some(ImportOutcome::Imported(self.profile_id, self.database.clone(), self.schema.clone(), self.table.clone()));
            }
            self.done = Some(r);
        }
        if let Some(s) = &mut self.session {
            for e in s.poll() {
                if let crate::db::session::Event::ConnectFailed { error, .. } = e {
                    self.error = Some(error);
                    self.running = None;
                }
            }
        }
        let busy = self.running.is_some();
        let modal = egui::Modal::new(egui::Id::unique("import_dialog")).frame(w::modal_frame()).show(ctx, |ui| {
            ui.set_width(820.0);
            let file = self.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            w::dialog_header(
                ui,
                icon::IMPORT,
                pal.accent,
                &format!("Import {file}"),
                &format!("{} · streamed with COPY in one transaction", util::bytes(self.size as usize)),
            );
            ui.add_enabled_ui(!busy, |ui| {
                w::section(ui, "Destination");
                ui.add_space(6.0);
                w::form_row(ui, "Connection", |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let names: Vec<(u64, String)> = profiles.iter().map(|p| (p.id, p.display_name())).collect();
                    let view: Vec<(u64, &str)> = names.iter().map(|(i, n)| (*i, n.as_str())).collect();
                    if w::dropdown(ui, &mut self.profile_id, &view, 260.0)
                        && let Some(p) = profiles.iter().find(|p| p.id == self.profile_id)
                    {
                        self.database = p.database.clone();
                    }
                    ui.label(RichText::new("Database").color(pal.dim));
                    w::input(ui, &mut self.database, "postgres", Some(icon::DATABASE), 0.0);
                });
                w::form_row(ui, "Schema", |ui| {
                    let schemas: Vec<String> = catalog.map(|c| c.schemas.iter().filter(|s| !s.is_system).map(|s| s.name.clone()).collect()).unwrap_or_default();
                    if schemas.is_empty() {
                        w::input(ui, &mut self.schema, "public", Some(icon::FOLDER), 260.0);
                    } else {
                        let view: Vec<&str> = schemas.iter().map(String::as_str).collect();
                        w::dropdown_str(ui, &mut self.schema, &view, 260.0);
                    }
                });
                w::form_row(ui, "Table", |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    w::segmented(ui, &mut self.new_table, &[(true, icon::PLUS, "New table"), (false, icon::TABLE, "Existing table")]);
                    if self.new_table {
                        w::code_input(ui, &mut self.table, "table_name", None, 0.0);
                    } else {
                        let tables: Vec<&crate::db::meta::Relation> = catalog
                            .and_then(|c| c.schemas.iter().find(|s| s.name == self.schema))
                            .map(|s| s.relations.iter().filter(|r| r.kind == RelKind::Table || r.kind == RelKind::Partitioned).collect())
                            .unwrap_or_default();
                        let view: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
                        if w::dropdown_str(ui, &mut self.table, &view, 220.0)
                            && let Some(t) = tables.iter().find(|t| t.name == self.table)
                        {
                            // map file columns to table columns by name, else by position
                            for (i, c) in self.cols.iter_mut().enumerate() {
                                c.target = t
                                    .columns
                                    .iter()
                                    .find(|x| x.name.eq_ignore_ascii_case(&c.name))
                                    .or_else(|| t.columns.get(i))
                                    .map(|x| x.name.clone())
                                    .unwrap_or_default();
                            }
                        }
                        w::checkbox(ui, &mut self.truncate, "Empty it first");
                    }
                });
                ui.add_space(8.0);
                w::section(ui, "File format");
                ui.add_space(6.0);
                w::form_row(ui, "Delimiter", |ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    let before = (self.delim, self.header);
                    w::segmented(ui, &mut self.delim, &[(',', "", "Comma"), (';', "", "Semicolon"), ('\t', "", "Tab"), ('|', "", "Pipe")]);
                    w::checkbox(ui, &mut self.header, "First row is a header");
                    if before != (self.delim, self.header) {
                        self.reparse();
                    }
                });
                ui.add_space(8.0);
                w::section(ui, if self.new_table { "Columns and types" } else { "Column mapping" });
                ui.add_space(6.0);
                let existing_cols: Vec<String> = if self.new_table {
                    vec![]
                } else {
                    catalog
                        .and_then(|c| c.find_relation(Some(&self.schema), &self.table))
                        .map(|r| r.columns.iter().map(|c| c.name.clone()).collect())
                        .unwrap_or_default()
                };
                egui::Frame::NONE.fill(pal.bg0).stroke(egui::Stroke::new(1.0, pal.border)).corner_radius(8.0).inner_margin(egui::Margin::same(10)).show(
                    ui,
                    |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::both().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
                            egui::Grid::new("import_cols").striped(true).spacing(vec2(8.0, 6.0)).show(ui, |ui| {
                                for (i, c) in self.cols.iter_mut().enumerate() {
                                    ui.push_id(i, |ui| {
                                        ui.vertical(|ui| {
                                            ui.set_width(150.0);
                                            if self.new_table {
                                                w::code_input(ui, &mut c.name, "column", None, 150.0);
                                                ui.add_space(4.0);
                                                w::dropdown_str(ui, &mut c.ty, TYPES, 150.0);
                                            } else {
                                                ui.label(RichText::new(&c.name).font(theme::mono(12.0)).color(pal.faint));
                                                let view: Vec<&str> = existing_cols.iter().map(String::as_str).collect();
                                                w::dropdown_str(ui, &mut c.target, &view, 150.0);
                                            }
                                        });
                                    });
                                }
                                ui.end_row();
                                for row in &self.preview {
                                    for i in 0..self.cols.len() {
                                        let v: String = row.get(i).map(|s| s.chars().take(40).collect()).unwrap_or_default();
                                        let t = if v.is_empty() { RichText::new("NULL").color(pal.faint).italics() } else { RichText::new(v).color(pal.text) };
                                        ui.label(t.font(theme::mono(12.0)));
                                    }
                                    ui.end_row();
                                }
                            });
                        });
                    },
                );
            });

            if let Some((_, progress, _, started)) = &self.running {
                let sent = progress.load(Ordering::Relaxed) as f32;
                let frac = if self.size > 0 { sent / self.size as f32 } else { 0.0 };
                let secs = started.elapsed().as_secs_f32().max(0.01);
                ui.add_space(10.0);
                ui.add(egui::ProgressBar::new(frac.min(1.0)).show_percentage().corner_radius(6.0).fill(pal.primary).text(format!(
                    "{} of {} · {}/s",
                    util::bytes(sent as usize),
                    util::bytes(self.size as usize),
                    util::bytes((sent / secs) as usize)
                )));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }
            match &self.done {
                Some(Ok(tag)) => {
                    ui.add_space(8.0);
                    w::callout(ui, pal.success, icon::CIRCLE_CHECK, &format!("Done: {tag} into {}.{}", self.schema, self.table));
                }
                Some(Err(e)) => {
                    ui.add_space(8.0);
                    w::callout(ui, pal.danger, icon::CIRCLE_X, &format!("Import failed, nothing was changed: {e}"));
                }
                None => {}
            }
            if let Some(e) = &self.error {
                ui.add_space(8.0);
                w::callout(ui, pal.danger, icon::CIRCLE_X, e);
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let ok = self.done.as_ref().is_some_and(|d| d.is_ok());
                if !busy && !ok {
                    w::menu_button(ui, w::Kind::Secondary, icon::CODE, "Show SQL", |ui| {
                        let (setup, copy) = self.sql();
                        super::editor::code_view(ui, &format!("BEGIN;\n{setup};\n{copy};\nCOMMIT;"), 12.0);
                    });
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if busy {
                        if w::button(ui, w::Kind::Danger, icon::STOP, "Cancel import").clicked()
                            && let Some((_, _, cancel, _)) = &self.running
                        {
                            cancel.store(true, Ordering::Relaxed);
                        }
                        ui.spinner();
                    } else {
                        if !ok && w::button(ui, w::Kind::Primary, icon::IMPORT, "Import").clicked() {
                            self.start(ui.ctx(), profiles, passwords);
                        }
                        if w::button(ui, if ok { w::Kind::Primary } else { w::Kind::Ghost }, "", if ok { "Close" } else { "Cancel" }).clicked() {
                            outcome = Some(ImportOutcome::Close);
                        }
                    }
                });
            });
        });
        if modal.should_close() && !busy && outcome.is_none() {
            outcome = Some(ImportOutcome::Close);
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_parsing() {
        let rows = parse_csv("a,b\n1,\"x, \"\"y\"\"\nz\"\n2,\n", ',', 100);
        assert_eq!(rows, vec![vec!["a", "b"], vec!["1", "x, \"y\"\nz"], vec!["2", ""]]);
        assert_eq!(detect_delimiter("a;b;c"), ';');
        assert_eq!(detect_delimiter("a\tb"), '\t');
    }

    #[test]
    fn inference() {
        assert_eq!(infer_type(["1", "2", ""].into_iter()), "integer");
        assert_eq!(infer_type(["1", "99999999999"].into_iter()), "bigint");
        assert_eq!(infer_type(["1.5", "2"].into_iter()), "numeric");
        assert_eq!(infer_type(["007"].into_iter()), "text");
        assert_eq!(infer_type(["2024-01-02"].into_iter()), "date");
        assert_eq!(infer_type(["2024-01-02T10:00:00Z"].into_iter()), "timestamptz");
        assert_eq!(infer_type(["true", "no"].into_iter()), "boolean");
        assert_eq!(infer_type(["{\"a\":1}"].into_iter()), "jsonb");
        assert_eq!(sanitize_ident("\u{feff}First Name", 0), "first_name");
        assert_eq!(sanitize_ident("2024 sales ($)", 0), "c_2024_sales");
    }
}
