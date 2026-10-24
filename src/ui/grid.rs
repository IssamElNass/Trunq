//! A virtualised data grid: only visible cells are laid out, so result sets
//! with millions of rows and hundreds of columns scroll smoothly.
//!
//! Display columns are a view over the source columns, so columns can be
//! hidden (manually or "hide empty"), and rows are a view over the source
//! rows after local search, column filters and sorting.

use std::collections::{HashMap, HashSet};

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align2, Color32, CursorIcon, FontId, Id, Key, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};

use super::theme::{self, icon};
use crate::db::result::ResultSet;
use crate::db::types::Cat;
use crate::export::{self, Format};
use crate::ui::widgets as w;

const HEADER_H: f32 = 40.0;
const MIN_W: f32 = 40.0;
/// JSON values longer than this get a size badge.
const JSON_BADGE_BYTES: usize = 160;

/// Pending modifications of an editable grid.
#[derive(Default, Clone)]
pub struct Edits {
    revision: u64,
    pub cells: HashMap<(usize, usize), Option<String>>,
    pub deleted: HashSet<usize>,
    /// New rows (index into this vec + rs.nrows() = grid row id)
    pub inserted: Vec<Vec<Option<String>>>,
}

impl Edits {
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty() && self.deleted.is_empty() && self.inserted.is_empty()
    }
    pub fn count(&self) -> usize {
        let rows: HashSet<usize> = self.cells.keys().map(|k| k.0).collect();
        rows.len() + self.deleted.len() + self.inserted.len()
    }
}

/// How timestamptz values are displayed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TsMode {
    /// Exactly as the server sent it.
    #[default]
    Raw,
    Local,
    Utc,
}

impl TsMode {
    pub fn label(self) -> &'static str {
        match self {
            TsMode::Raw => "Raw",
            TsMode::Local => "Local time",
            TsMode::Utc => "UTC",
        }
    }
}

/// A client-side column filter ("click to filter").
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilterOp {
    Eq(String),
    Ne(String),
    IsNull,
    NotNull,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColFilter {
    /// Source column name (stable across re-runs).
    pub column: String,
    pub op: FilterOp,
}

impl ColFilter {
    pub fn label(&self) -> String {
        let v = |s: &str| if s.chars().count() > 24 { format!("{}…", s.chars().take(24).collect::<String>()) } else { s.to_string() };
        match &self.op {
            FilterOp::Eq(s) => format!("{} = {}", self.column, v(s)),
            FilterOp::Ne(s) => format!("{} ≠ {}", self.column, v(s)),
            FilterOp::IsNull => format!("{} is NULL", self.column),
            FilterOp::NotNull => format!("{} is not NULL", self.column),
        }
    }

    /// The equivalent SQL condition.
    pub fn sql(&self) -> String {
        let c = crate::db::meta::quote_ident(&self.column);
        match &self.op {
            FilterOp::Eq(s) => format!("{c} = {}", crate::db::meta::quote_literal(s)),
            FilterOp::Ne(s) => format!("{c} IS DISTINCT FROM {}", crate::db::meta::quote_literal(s)),
            FilterOp::IsNull => format!("{c} IS NULL"),
            FilterOp::NotNull => format!("{c} IS NOT NULL"),
        }
    }
}

/// Grid layout remembered per table ("Remember my table").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridPrefs {
    pub widths: HashMap<String, f32>,
    pub hidden: Vec<String>,
    pub sort: Option<(String, bool)>,
    pub hide_empty: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Index into the *view* (after filter / sort).
    pub row: usize,
    /// Display column index.
    pub col: usize,
}

#[derive(Default)]
pub struct GridState {
    /// Widths by source column.
    pub widths: Vec<f32>,
    /// view row -> source row
    view: Vec<usize>,
    view_key: Option<u64>,
    stats_cache: Option<(u64, Option<String>)>,
    /// display col -> source col
    dcols: Vec<usize>,
    /// (source col, ascending)
    pub sort: Option<(usize, bool)>,
    /// Local search over loaded rows.
    pub filter: String,
    pub col_filters: Vec<ColFilter>,
    pub hidden: HashSet<usize>,
    pub hide_empty: bool,
    empty_cols: (u64, HashSet<usize>),
    anchor: Option<Cell>,
    cursor: Option<Cell>,
    editing: Option<(usize, usize, String, bool)>,
    scroll_to_cursor: bool,
    drag_resize: Option<(usize, f32, f32)>,
    selecting: bool,
    last_click: Option<(Cell, f64)>,
    /// Column finder (⌘F) query, when open.
    finder: Option<(String, usize)>,
    pending_prefs: Option<GridPrefs>,
    /// Set while rendering: the last rows are visible (used to auto-fetch more).
    pub near_end: bool,
    /// Set while rendering: a foreign-key cell hovered for a moment
    /// (source row, source col, screen position).
    pub hover_fk: Option<(usize, usize, Pos2)>,
    hover_since: Option<((usize, usize), f64)>,
    /// Time spent drawing the last frame (for the timing breakdown).
    pub render_ms: f32,
}

pub enum GridAction {
    /// Show the full value of a cell (source row, source col).
    ViewValue(usize, usize),
    /// Filter on a value (source col). The owner decides: SQL WHERE for a
    /// table view, client-side filter for query results.
    Filter(usize, FilterOp),
    DeleteRows(Vec<usize>),
    DuplicateRow(usize),
    /// Follow the foreign key in (source row, source col).
    FollowFk(usize, usize),
    /// Compare these source rows side by side.
    CompareRows(Vec<usize>),
}

pub struct GridCtx<'a> {
    pub rs: &'a ResultSet,
    pub cats: &'a [Cat],
    pub type_names: &'a [String],
    pub edits: Option<&'a mut Edits>,
    pub font_size: f32,
    pub max_chars: usize,
    pub table_name: &'a str,
    /// Highlight these source columns as primary key.
    pub pk_cols: &'a [usize],
    /// Source columns that are (single-column) foreign keys.
    pub fk_cols: &'a [usize],
    pub ts_mode: TsMode,
}

impl GridState {
    /// Number of rows currently visible (after filters).
    pub fn visible_rows(&self) -> usize {
        self.view.len()
    }

    pub fn selection(&self) -> Option<(usize, usize, usize, usize)> {
        let (a, c) = (self.anchor?, self.cursor?);
        Some((a.row.min(c.row), a.row.max(c.row), a.col.min(c.col), a.col.max(c.col)))
    }

    /// Recompute aggregates only when the selection, data, edits or view changes.
    pub fn selection_stats(&mut self, rs: &ResultSet, cats: &[Cat], edits: Option<&Edits>) -> Option<String> {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (self.selection(), self.view_key, &self.dcols, rs.generation, rs.nrows(), edits.map(|e| e.revision)).hash(&mut hash);
        cats.hash(&mut hash);
        let key = hash.finish();
        if let Some((cached_key, value)) = &self.stats_cache
            && *cached_key == key
        {
            return value.clone();
        }
        let value = self.compute_stats(rs, cats, edits);
        self.stats_cache = Some((key, value.clone()));
        value
    }

    fn compute_stats(&self, rs: &ResultSet, cats: &[Cat], edits: Option<&Edits>) -> Option<String> {
        let (r0, r1, c0, c1) = self.selection()?;
        if self.selected_count() > 2_000_000 {
            return None;
        }
        let (mut count, mut n) = (0usize, 0usize);
        let (mut sum, mut min, mut max) = (0f64, f64::INFINITY, f64::NEG_INFINITY);
        for &c in self.dcols.get(c0..=c1)? {
            for &r in self.view.get(r0..=r1)? {
                let value = if let Some(e) = edits { cell_value_raw(rs, e, r, c) } else { rs.get(r, c) };
                let Some(v) = value else { continue };
                count += 1;
                if cats.get(c) == Some(&Cat::Number)
                    && let Ok(x) = v.parse::<f64>()
                    && x.is_finite()
                {
                    n += 1;
                    sum += x;
                    min = min.min(x);
                    max = max.max(x);
                }
            }
        }
        let mut out = format!("· count {}", crate::util::thousands(count));
        if n > 0 {
            let f = |v: f64| {
                if v.fract() == 0.0 && v.abs() < 1e15 {
                    format!("{}", v as i64)
                } else {
                    format!("{v:.4}").trim_end_matches('0').trim_end_matches('.').to_string()
                }
            };
            out.push_str(&format!(" · Σ {} · avg {} · min {} · max {}", f(sum), f(sum / n as f64), f(min), f(max)));
        }
        Some(out)
    }

    pub fn selected_count(&self) -> usize {
        self.selection().map_or(0, |(r0, r1, c0, c1)| (r1 - r0 + 1) * (c1 - c0 + 1))
    }

    /// Cursor as (source row, source col).
    pub fn cursor_source(&self) -> Option<(usize, usize)> {
        let c = self.cursor?;
        Some((*self.view.get(c.row)?, *self.dcols.get(c.col)?))
    }

    /// Selection as (source rows, source cols) for copy/export.
    pub fn selection_rows_cols(&self) -> Option<(Vec<usize>, Vec<usize>)> {
        let (r0, r1, c0, c1) = self.selection()?;
        let rows = (r0..=r1).filter_map(|r| self.view.get(r).copied()).collect();
        let cols = (c0..=c1).filter_map(|c| self.dcols.get(c).copied()).collect();
        Some((rows, cols))
    }

    /// Source rows in the current view order (respects filters & sort).
    pub fn view_rows(&self) -> &[usize] {
        &self.view
    }

    /// Visible source columns in display order.
    pub fn display_cols(&self) -> &[usize] {
        &self.dcols
    }

    pub fn select_all(&mut self) {
        if self.view.is_empty() || self.dcols.is_empty() {
            return;
        }
        self.anchor = Some(Cell { row: 0, col: 0 });
        self.cursor = Some(Cell { row: self.view.len() - 1, col: self.dcols.len() - 1 });
    }

    /// Moves the cursor to a source row (used by card view navigation).
    pub fn focus_row(&mut self, view_row: usize) {
        if view_row < self.view.len() {
            let col = self.cursor.map_or(0, |c| c.col);
            self.cursor = Some(Cell { row: view_row, col });
            self.anchor = self.cursor;
            self.scroll_to_cursor = true;
        }
    }

    /// Computes the row / column view without drawing (used by the card view).
    pub fn refresh_view(&mut self, rs: &ResultSet, cats: &[Cat], type_names: &[String]) {
        let g =
            GridCtx { rs, cats, type_names, edits: None, font_size: 12.0, max_chars: 100, table_name: "", pk_cols: &[], fk_cols: &[], ts_mode: TsMode::Raw };
        self.ensure_view(&g);
    }

    pub fn cursor_view_row(&self) -> Option<usize> {
        self.cursor.map(|c| c.row)
    }

    pub fn open_finder(&mut self) {
        self.finder = Some((String::new(), 0));
    }

    pub fn add_filter(&mut self, f: ColFilter) {
        if !self.col_filters.contains(&f) {
            self.col_filters.push(f);
        }
    }

    pub fn prefs(&self, rs: &ResultSet) -> GridPrefs {
        GridPrefs {
            widths: rs.columns.iter().zip(&self.widths).map(|(c, w)| (c.name.clone(), *w)).collect(),
            hidden: self.hidden.iter().filter_map(|&i| rs.columns.get(i).map(|c| c.name.clone())).collect(),
            sort: self.sort.and_then(|(c, a)| Some((rs.columns.get(c)?.name.clone(), a))),
            hide_empty: self.hide_empty,
        }
    }

    /// Applies remembered prefs once the result's columns are known.
    pub fn apply_prefs(&mut self, prefs: GridPrefs) {
        self.pending_prefs = Some(prefs);
        self.widths.clear();
    }

    fn view_hash(&self, g: &GridCtx) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let extra = g.edits.as_ref().map_or(0, |e| e.inserted.len());
        (
            g.rs.generation,
            g.rs.nrows() + extra,
            g.rs.ncols(),
            g.rs.byte_size(),
            g.edits.as_ref().map(|e| e.revision),
            &self.filter,
            self.sort,
            &self.col_filters,
        )
            .hash(&mut h);
        g.cats.hash(&mut h);
        h.finish()
    }

    fn ensure_view(&mut self, g: &GridCtx) {
        let key = self.view_hash(g);
        // display columns
        let n = g.rs.ncols();
        let rows = g.rs.nrows() + g.edits.as_ref().map_or(0, |e| e.inserted.len());
        if self.hide_empty && self.empty_cols.0 != key {
            let empty = (0..n).filter(|&c| rows > 0 && (0..rows).all(|r| cell_value(g, r, c).is_none())).collect();
            self.empty_cols = (key, empty);
        }
        self.dcols = (0..n).filter(|c| !(self.hidden.contains(c) || self.hide_empty && self.empty_cols.1.contains(c))).collect();

        if self.view_key == Some(key) {
            return;
        }
        self.view_key = Some(key);
        let get = |r: usize, c: usize| cell_value(g, r, c);
        let filters: Vec<(usize, &FilterOp)> = self.col_filters.iter().filter_map(|f| Some((g.rs.col_index(&f.column)?, &f.op))).collect();
        let needle = self.filter.trim().to_lowercase();
        let ascii = needle.is_ascii();
        let mut view: Vec<usize> = (0..rows)
            .filter(|&r| {
                filters.iter().all(|(c, op)| match (op, get(r, *c)) {
                    (FilterOp::Eq(s), Some(v)) => v == s,
                    (FilterOp::Eq(_), None) => false,
                    (FilterOp::Ne(s), v) => v != Some(s.as_str()),
                    (FilterOp::IsNull, v) => v.is_none(),
                    (FilterOp::NotNull, v) => v.is_some(),
                })
            })
            .filter(|&r| {
                needle.is_empty()
                    || (0..g.rs.ncols()).any(|c| match get(r, c) {
                        Some(v) if ascii => contains_ascii_ci(v.as_bytes(), needle.as_bytes()),
                        Some(v) => v.to_lowercase().contains(&needle),
                        None => needle == "null",
                    })
            })
            .collect();
        if let Some((col, asc)) = self.sort
            && col < g.rs.ncols()
        {
            if g.cats.get(col) == Some(&Cat::Number) {
                let mut keyed: Vec<(Option<f64>, usize)> = view.iter().map(|&r| (get(r, col).and_then(|v| v.parse::<f64>().ok()), r)).collect();
                keyed.sort_by(|a, b| match (a.0, b.0) {
                    (None, None) => std::cmp::Ordering::Equal,
                    (None, _) => std::cmp::Ordering::Greater,
                    (_, None) => std::cmp::Ordering::Less,
                    (Some(x), Some(y)) => {
                        let o = x
                            .total_cmp(&y)
                            .then_with(|| decimal_cmp(get(a.1, col).unwrap_or(""), get(b.1, col).unwrap_or("")).unwrap_or(std::cmp::Ordering::Equal));
                        if asc { o } else { o.reverse() }
                    }
                });
                view = keyed.into_iter().map(|k| k.1).collect();
            } else {
                view.sort_by(|&a, &b| match (get(a, col), get(b, col)) {
                    (None, None) => std::cmp::Ordering::Equal,
                    (None, _) => std::cmp::Ordering::Greater,
                    (_, None) => std::cmp::Ordering::Less,
                    (Some(x), Some(y)) => {
                        let o = natural_cmp(x, y);
                        if asc { o } else { o.reverse() }
                    }
                });
            }
        }
        self.view = view;
        if let Some(c) = self.cursor
            && (c.row >= self.view.len() || c.col >= self.dcols.len())
        {
            self.anchor = None;
            self.cursor = None;
        }
    }

    fn ensure_widths(&mut self, ui: &Ui, g: &GridCtx) {
        let n = g.rs.ncols();
        if self.widths.len() == n {
            return;
        }
        let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(&theme::mono(g.font_size), '0'));
        let sample = g.rs.nrows().min(300);
        self.widths = (0..n)
            .map(|c| {
                let name = g.rs.columns.get(c).map_or(4, |x| x.name.chars().count());
                let ty = g.type_names.get(c).map_or(0, |t| t.chars().count());
                let mut maxc = (name + 3).max(ty + 2);
                for r in 0..sample {
                    let len = g.rs.get(r, c).map_or(4, |v| v.lines().next().unwrap_or("").chars().take(60).count());
                    maxc = maxc.max(len);
                }
                (maxc as f32 * char_w + 22.0).clamp(64.0, 420.0)
            })
            .collect();
        if let Some(p) = self.pending_prefs.take() {
            for (i, c) in g.rs.columns.iter().enumerate() {
                if let Some(w) = p.widths.get(&c.name) {
                    self.widths[i] = w.max(MIN_W);
                }
                if p.hidden.contains(&c.name) {
                    self.hidden.insert(i);
                }
            }
            self.sort = p.sort.and_then(|(name, asc)| Some((g.rs.col_index(&name)?, asc)));
            self.hide_empty = p.hide_empty;
            self.view_key = None;
        }
    }
}

fn contains_ascii_ci(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > hay.len() {
        return false;
    }
    hay.windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle))
}

/// Compares strings so that "file2" < "file10".
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let o = na
                    .trim_start_matches('0')
                    .len()
                    .cmp(&nb.trim_start_matches('0').len())
                    .then_with(|| na.trim_start_matches('0').cmp(nb.trim_start_matches('0')));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase()).then(x.cmp(&y));
                if o != std::cmp::Ordering::Equal {
                    return o;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

fn display_text(v: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(v.len().min(max_chars + 4));
    for (n, c) in v.chars().enumerate() {
        if n >= max_chars {
            out.push('…');
            break;
        }
        match c {
            '\n' => out.push('↵'),
            '\r' => {}
            '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Formats a timestamptz value in local time or UTC.
pub fn format_ts(v: &str, mode: TsMode) -> Option<String> {
    if mode == TsMode::Raw {
        return None;
    }
    // Server format (DateStyle ISO): 2026-10-22 00:11:51.411675+02 / +05:30
    let s = v.trim();
    // `+02` style offsets need minutes for chrono's %:z
    let b = s.as_bytes();
    let short_offset = b.len() > 3 && matches!(b[b.len() - 3], b'+' | b'-') && b[b.len() - 2..].iter().all(u8::is_ascii_digit);
    let padded = if short_offset { format!("{s}:00") } else { s.to_string() };
    let dt = chrono::DateTime::parse_from_str(&padded, "%Y-%m-%d %H:%M:%S%.f%:z").ok()?;
    Some(match mode {
        TsMode::Local => dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S%.3f %:z").to_string(),
        TsMode::Utc => dt.with_timezone(&chrono::Utc).format("%Y-%m-%d %H:%M:%S%.3fZ").to_string(),
        TsMode::Raw => unreachable!(),
    })
}

/// Preserve bigint/numeric precision when floating-point sort keys tie.
fn decimal_cmp(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    fn parts(v: &str) -> Option<(bool, &str, &str)> {
        let negative = v.starts_with('-');
        let v = v.strip_prefix(['-', '+']).unwrap_or(v);
        let (whole, frac) = v.split_once('.').unwrap_or((v, ""));
        if whole.is_empty() && frac.is_empty() || !whole.bytes().chain(frac.bytes()).all(|c| c.is_ascii_digit()) {
            return None;
        }
        let whole = whole.trim_start_matches('0');
        let frac = frac.trim_end_matches('0');
        Some((negative && !(whole.is_empty() && frac.is_empty()), whole, frac))
    }
    let (an, ai, af) = parts(a)?;
    let (bn, bi, bf) = parts(b)?;
    if an != bn {
        return Some(if an { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater });
    }
    let order = ai.len().cmp(&bi.len()).then_with(|| ai.cmp(bi)).then_with(|| af.cmp(bf));
    Some(if an { order.reverse() } else { order })
}

fn cell_job(text: &str, font: &FontId, color: Color32, width: f32, italics: bool) -> LayoutJob {
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat { font_id: font.clone(), color, italics, ..Default::default() });
    job.wrap = TextWrapping { max_width: width.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    job
}

/// Pretty-prints JSON (first `max_lines` lines) for previews.
pub fn json_preview(v: &str, max_lines: usize) -> String {
    let pretty = serde_json::from_str::<serde_json::Value>(v).ok().and_then(|j| serde_json::to_string_pretty(&j).ok()).unwrap_or_else(|| v.to_string());
    let mut lines: Vec<&str> = pretty.lines().take(max_lines + 1).collect();
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        lines.push("  …");
    }
    lines.join("\n")
}

/// Draws the grid; returns an optional action for the owner to handle.
pub fn show(ui: &mut Ui, id: Id, st: &mut GridState, mut g: GridCtx) -> Option<GridAction> {
    let t0 = std::time::Instant::now();
    let pal = theme::p();
    st.ensure_widths(ui, &g);
    st.ensure_view(&g);
    // A data grid has hard boundaries. Elastic trackpad overscroll can spring
    // backwards while new pages extend the content, making rows appear to flip.
    ui.style_mut().spacing.scroll.kinetic.rubber_band = false;
    st.hover_fk = None;
    let font = theme::mono(g.font_size);
    let row_h = (g.font_size * 1.9).round();
    let ncols = st.dcols.len();
    let nrows = st.view.len();
    let digits = nrows.max(1).to_string().len().max(2);
    let gutter_w = digits as f32 * g.font_size * 0.62 + 22.0;
    let mut xs = Vec::with_capacity(ncols + 1);
    let mut x = gutter_w;
    for &sc in &st.dcols {
        xs.push(x);
        x += st.widths.get(sc).copied().unwrap_or(120.0);
    }
    xs.push(x);
    let total_w = x + 40.0;
    let total_h = HEADER_H + nrows as f32 * row_h + row_h;
    let mut action = None;
    let editable = g.edits.is_some();
    let focus_id = id.with("focus");
    let search = st.filter.trim().to_lowercase();
    let is_tstz: Vec<bool> = g.type_names.iter().map(|t| t == "timestamptz" || t == "timestamp with time zone").collect();

    // ---- keyboard ---------------------------------------------------------------
    let has_focus = ui.memory(|m| m.has_focus(focus_id));
    if has_focus && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::F)) {
        st.open_finder();
    }
    if has_focus && st.editing.is_none() && st.finder.is_none() && ncols > 0 && nrows > 0 {
        let page = ((ui.available_height() - HEADER_H) / row_h).max(1.0) as usize;
        let mv = |dr: isize, dc: isize, extend: bool, st: &mut GridState| {
            let cur = st.cursor.unwrap_or(Cell { row: 0, col: 0 });
            let row = (cur.row as isize + dr).clamp(0, nrows as isize - 1) as usize;
            let col = (cur.col as isize + dc).clamp(0, ncols as isize - 1) as usize;
            st.cursor = Some(Cell { row, col });
            if !extend {
                st.anchor = st.cursor;
            }
            st.scroll_to_cursor = true;
        };
        let events = ui.input(|i| i.events.clone());
        for e in &events {
            if let egui::Event::Key { key, pressed: true, modifiers, .. } = e {
                let ext = modifiers.shift;
                let jump = modifiers.command || modifiers.alt;
                match key {
                    Key::ArrowDown => mv(if jump { nrows as isize } else { 1 }, 0, ext, st),
                    Key::ArrowUp => mv(if jump { -(nrows as isize) } else { -1 }, 0, ext, st),
                    Key::ArrowRight => mv(0, if jump { ncols as isize } else { 1 }, ext, st),
                    Key::ArrowLeft => mv(0, if jump { -(ncols as isize) } else { -1 }, ext, st),
                    Key::PageDown => mv(page as isize, 0, ext, st),
                    Key::PageUp => mv(-(page as isize), 0, ext, st),
                    Key::Home if modifiers.command => mv(-(nrows as isize), -(ncols as isize), ext, st),
                    Key::End if modifiers.command => mv(nrows as isize, ncols as isize, ext, st),
                    Key::Home => mv(0, -(ncols as isize), ext, st),
                    Key::End => mv(0, ncols as isize, ext, st),
                    Key::Tab => mv(0, if modifiers.shift { -1 } else { 1 }, false, st),
                    Key::A if modifiers.command => st.select_all(),
                    Key::Enter | Key::F2 => {
                        if let Some((src, sc)) = st.cursor_source() {
                            let cur = cell_value(&g, src, sc).map(str::to_string);
                            if editable && !cur.as_deref().is_some_and(|v| v.contains('\n') || v.len() > 2000) {
                                st.editing = Some((src, sc, cur.unwrap_or_default(), true));
                            } else {
                                action = Some(GridAction::ViewValue(src, sc));
                            }
                        }
                    }
                    Key::Delete | Key::Backspace if editable => {
                        if let Some((rows, cols)) = st.selection_rows_cols()
                            && let Some(e) = g.edits.as_deref_mut()
                        {
                            for &r in &rows {
                                for &c in &cols {
                                    set_cell(e, g.rs, r, c, Some(String::new()));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let egui::Event::Copy = e
                && let Some((rows, cols)) = st.selection_rows_cols()
            {
                let headers = ui.input(|i| i.modifiers.shift);
                let txt = export::format(g.rs, &rows, &cols, g.cats, Format::Tsv, headers, g.table_name);
                ui.ctx().copy_text(txt.trim_end_matches('\n').to_string());
            }
            if editable
                && let egui::Event::Text(t) = e
                && let Some((src, sc)) = st.cursor_source()
                && !ui.input(|i| i.modifiers.command)
            {
                st.editing = Some((src, sc, t.clone(), false));
            }
        }
    }

    // ---- header area (sticky) -----------------------------------------------------
    let (header_rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    let scroll_id = id.with("scroll");

    // ---- body ---------------------------------------------------------------------
    let mut sa = egui::ScrollArea::both().id_salt(scroll_id).auto_shrink([false, false]);
    if st.scroll_to_cursor
        && let Some(c) = st.cursor
    {
        let prev = egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(egui::IdSalt::new(scroll_id)));
        let off = prev.map(|s| s.clamped_offset()).unwrap_or_default();
        let view_h = ui.available_height();
        let view_w = ui.available_width();
        let y0 = c.row as f32 * row_h;
        let mut oy = off.y;
        if y0 < oy {
            oy = y0;
        } else if y0 + row_h > oy + view_h - row_h {
            oy = y0 + 2.0 * row_h - view_h;
        }
        let mut ox = off.x;
        if c.col < ncols {
            let (cx0, cx1) = (xs[c.col] - gutter_w, xs[c.col + 1]);
            if cx0 < ox {
                ox = cx0;
            } else if cx1 > ox + view_w {
                ox = cx1 - view_w;
            }
        }
        sa = sa.scroll_offset(vec2(ox.max(0.0), oy.max(0.0)));
        st.scroll_to_cursor = false;
    }
    let now = ui.input(|i| i.time);
    let mut painted_offset_x = 0.0;
    sa.show_viewport(ui, |ui, viewport| {
        painted_offset_x = viewport.min.x;
        ui.set_min_size(vec2(total_w, total_h - HEADER_H));
        let origin = ui.min_rect().min;
        let content = Rect::from_min_size(origin, vec2(total_w.max(ui.available_width()), (total_h - HEADER_H).max(ui.available_height())));
        let resp = ui.interact(content, focus_id, Sense::click_and_drag());
        let painter = ui.painter();

        let first = (viewport.min.y / row_h).floor().max(0.0) as usize;
        let last = ((viewport.max.y / row_h).ceil() as usize + 1).min(nrows);
        st.near_end = last + 20 >= nrows;
        let vx0 = viewport.min.x + gutter_w;
        let vx1 = viewport.max.x;
        let col_first = xs.partition_point(|&x| x <= vx0).saturating_sub(1).min(ncols);
        let col_last = xs.partition_point(|&x| x < vx1).min(ncols);
        let sel = st.selection();
        let deleted = g.edits.as_ref().map(|e| &e.deleted);
        let hover_pos = ui.input(|i| i.pointer.hover_pos()).filter(|p| ui.clip_rect().contains(*p) && ui.ctx().layer_id_at(*p) == Some(ui.layer_id()));
        let mut hovered_cell: Option<(usize, usize, Rect)> = None;

        for vr in first..last {
            let src = st.view[vr];
            let y = origin.y + vr as f32 * row_h;
            if vr % 2 == 1 {
                painter.rect_filled(Rect::from_min_max(pos2(origin.x + viewport.min.x, y), pos2(origin.x + viewport.max.x, y + row_h)), 0.0, pal.row_alt);
            }
            let is_new = src >= g.rs.nrows();
            let is_deleted = deleted.is_some_and(|d| d.contains(&src));
            for c in col_first..col_last {
                let sc = st.dcols[c];
                let x0 = origin.x + xs[c];
                let x1 = origin.x + xs[c + 1];
                let rect = Rect::from_min_max(pos2(x0, y), pos2(x1, y + row_h));
                let in_sel = sel.is_some_and(|(r0, r1, c0, c1)| vr >= r0 && vr <= r1 && c >= c0 && c <= c1);
                let modified = g.edits.as_ref().is_some_and(|e| e.cells.contains_key(&(src, sc)));
                let val = cell_value(&g, src, sc);
                if in_sel {
                    painter.rect_filled(rect, 0.0, pal.sel);
                } else if !search.is_empty()
                    && let Some(v) = val
                    && (if search.is_ascii() { contains_ascii_ci(v.as_bytes(), search.as_bytes()) } else { v.to_lowercase().contains(&search) })
                {
                    // local search hit
                    painter.rect_filled(rect.shrink(1.0), 3.0, pal.warn.gamma_multiply(0.22));
                }
                if modified || is_new {
                    painter.rect_filled(rect, 0.0, pal.warn.gamma_multiply(0.14));
                }
                if is_deleted {
                    painter.rect_filled(rect, 0.0, pal.danger.gamma_multiply(0.14));
                }
                let cat = g.cats.get(sc).copied().unwrap_or(Cat::Other);
                let mut badge: Option<String> = None;
                let (text, color, italics) = match val {
                    None => ("NULL".to_string(), pal.null, true),
                    Some(v) => {
                        let color = match cat {
                            Cat::Number => pal.number,
                            Cat::Bool => pal.bool_c,
                            Cat::Json => pal.json,
                            Cat::DateTime => pal.string,
                            Cat::Uuid | Cat::Network => pal.qident,
                            _ => pal.text,
                        };
                        let shown = match cat {
                            Cat::Bool => match v {
                                "t" => "true".into(),
                                "f" => "false".into(),
                                _ => display_text(v, g.max_chars),
                            },
                            Cat::DateTime if is_tstz.get(sc) == Some(&true) => format_ts(v, g.ts_mode).unwrap_or_else(|| display_text(v, g.max_chars)),
                            _ => display_text(v, g.max_chars),
                        };
                        if cat == Cat::Json && v.len() > JSON_BADGE_BYTES {
                            let kind = if v.starts_with('[') { "[…]" } else { "{…}" };
                            badge = Some(format!("{kind} {}", crate::util::bytes(v.len())));
                        }
                        (shown, color, false)
                    }
                };
                let badge_w = badge.as_ref().map_or(0.0, |b| b.len() as f32 * g.font_size * 0.5 + 14.0);
                let w = rect.width() - 14.0 - badge_w;
                let galley = ui.ctx().fonts_mut(|f| f.layout_job(cell_job(&text, &font, color, w, italics)));
                let tx = if cat.right_aligned() && val.is_some() { rect.right() - 7.0 - galley.size().x } else { rect.left() + 7.0 };
                let ty = rect.center().y - galley.size().y / 2.0;
                painter.galley(pos2(tx, ty), galley, color);
                if let Some(b) = badge
                    && w > 20.0
                {
                    let bf = FontId::proportional(g.font_size * 0.78);
                    let bg = ui.ctx().fonts_mut(|f| f.layout_no_wrap(b, bf, pal.dim));
                    let br =
                        Rect::from_min_size(pos2(rect.right() - bg.size().x - 12.0, rect.center().y - bg.size().y / 2.0 - 1.0), bg.size() + vec2(8.0, 2.0));
                    painter.rect_filled(br, 4.0, pal.bg3);
                    painter.galley(br.min + vec2(4.0, 1.0), bg, pal.dim);
                }
                if is_deleted {
                    painter.hline(rect.x_range(), rect.center().y, Stroke::new(1.0, pal.danger));
                }
                if let Some(p) = hover_pos
                    && rect.contains(p)
                {
                    hovered_cell = Some((src, sc, rect));
                    if val.is_some() && g.fk_cols.contains(&sc) {
                        let cmd = ui.input(|i| i.modifiers.command);
                        painter.text(
                            rect.right_center() - vec2(8.0, 0.0),
                            Align2::RIGHT_CENTER,
                            icon::EXTERNAL,
                            FontId::proportional(11.0),
                            if cmd { pal.accent } else { pal.faint },
                        );
                        if cmd {
                            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                        }
                    }
                }
                painter.vline(x1 - 0.5, rect.y_range(), Stroke::new(1.0, pal.border.gamma_multiply(0.45)));
            }
            painter.hline(origin.x + viewport.min.x..=origin.x + xs[col_last.min(ncols)], y + row_h - 0.5, Stroke::new(1.0, pal.border.gamma_multiply(0.3)));
        }

        // hover: FK peek signal and JSON preview
        match hovered_cell {
            Some((src, sc, rect)) if st.editing.is_none() => {
                let since = match st.hover_since {
                    Some((cell, t)) if cell == (src, sc) => t,
                    _ => {
                        st.hover_since = Some(((src, sc), now));
                        now
                    }
                };
                let dwell = now - since;
                if dwell > 0.35 {
                    if g.fk_cols.contains(&sc) && cell_value(&g, src, sc).is_some() {
                        st.hover_fk = Some((src, sc, rect.left_bottom()));
                    } else if g.cats.get(sc) == Some(&Cat::Json)
                        && let Some(v) = cell_value(&g, src, sc)
                        && v.len() > 40
                    {
                        let preview = json_preview(v, 24);
                        egui::Area::new(id.with("json_peek"))
                            .order(egui::Order::Tooltip)
                            .fixed_pos(rect.left_bottom() + vec2(0.0, 2.0))
                            .constrain(true)
                            .interactable(false)
                            .show(ui.ctx(), |ui| {
                                egui::Frame::popup(ui.style()).fill(pal.bg2).show(ui, |ui| {
                                    ui.set_max_width(520.0);
                                    ui.label(egui::RichText::new(preview).font(theme::mono(11.5)).color(pal.json));
                                    ui.label(
                                        egui::RichText::new(format!("{} · double-click or Enter for the JSON tree", crate::util::bytes(v.len())))
                                            .small()
                                            .color(pal.faint),
                                    );
                                });
                            });
                    }
                } else {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
                }
            }
            _ => st.hover_since = None,
        }

        // cursor outline
        if let Some(c) = st.cursor
            && c.row >= first
            && c.row < last
            && c.col < ncols
        {
            let y = origin.y + c.row as f32 * row_h;
            let r = Rect::from_min_max(pos2(origin.x + xs[c.col], y), pos2(origin.x + xs[c.col + 1], y + row_h)).shrink(0.5);
            painter.rect_stroke(r, 2.0, Stroke::new(1.5, if has_focus { pal.accent } else { pal.dim }), egui::StrokeKind::Inside);
        }

        // sticky row-number gutter
        let gx0 = origin.x + viewport.min.x;
        let gutter = Rect::from_min_max(pos2(gx0, origin.y + viewport.min.y), pos2(gx0 + gutter_w, origin.y + viewport.max.y));
        painter.rect_filled(gutter, 0.0, pal.bg1);
        let gfont = theme::mono(g.font_size * 0.88);
        for vr in first..last {
            let y = origin.y + vr as f32 * row_h;
            let src = st.view[vr];
            let in_sel = sel.is_some_and(|(r0, r1, _, _)| vr >= r0 && vr <= r1);
            let label = if src >= g.rs.nrows() { "+".to_string() } else { (src + 1).to_string() };
            painter.text(pos2(gx0 + gutter_w - 9.0, y + row_h / 2.0), Align2::RIGHT_CENTER, label, gfont.clone(), if in_sel { pal.accent } else { pal.faint });
        }
        painter.vline(gx0 + gutter_w - 0.5, gutter.y_range(), Stroke::new(1.0, pal.border));

        // ---- pointer interaction ----
        let to_cell = |p: egui::Pos2| -> Option<Cell> {
            let lx = p.x - origin.x;
            let ly = p.y - origin.y;
            if ly < 0.0 {
                return None;
            }
            let row = (ly / row_h) as usize;
            if row >= nrows {
                return None;
            }
            if lx - viewport.min.x < gutter_w {
                return Some(Cell { row, col: usize::MAX });
            }
            let col = xs.partition_point(|&x| x <= lx).checked_sub(1)?;
            (col < ncols).then_some(Cell { row, col })
        };
        if resp.clicked() || resp.drag_started() || resp.secondary_clicked() {
            resp.request_focus();
        }
        if let Some(pos) = resp.interact_pointer_pos()
            && let Some(cell) = to_cell(pos)
        {
            let shift = ui.input(|i| i.modifiers.shift);
            let src_col = (cell.col != usize::MAX).then(|| st.dcols[cell.col]);
            if resp.clicked() && src_col.is_some_and(|sc| g.fk_cols.contains(&sc)) && ui.input(|i| i.modifiers.command) {
                let src = st.view[cell.row];
                if cell_value(&g, src, src_col.unwrap()).is_some() {
                    action = Some(GridAction::FollowFk(src, src_col.unwrap()));
                }
            } else if resp.drag_started() || resp.clicked() {
                if cell.col == usize::MAX {
                    if !shift || st.anchor.is_none() {
                        st.anchor = Some(Cell { row: cell.row, col: 0 });
                    }
                    st.cursor = Some(Cell { row: cell.row, col: ncols.saturating_sub(1) });
                } else if shift && st.anchor.is_some() {
                    st.cursor = Some(cell);
                } else {
                    st.anchor = Some(cell);
                    st.cursor = Some(cell);
                }
                st.selecting = resp.drag_started();
                if resp.clicked()
                    && let Some(sc) = src_col
                {
                    if let Some((prev, t)) = st.last_click
                        && prev == cell
                        && now - t < 0.4
                    {
                        let src = st.view[cell.row];
                        let v = cell_value(&g, src, sc);
                        let big = v.is_some_and(|v| v.contains('\n') || v.len() > 2000) || g.cats.get(sc) == Some(&Cat::Json);
                        if editable && !big {
                            st.editing = Some((src, sc, v.unwrap_or("").to_string(), true));
                        } else {
                            action = Some(GridAction::ViewValue(src, sc));
                        }
                    }
                    st.last_click = Some((cell, now));
                }
            } else if resp.dragged() && st.selecting && cell.col != usize::MAX {
                st.cursor = Some(cell);
            }
        }
        if resp.drag_stopped() {
            st.selecting = false;
        }
        if resp.secondary_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
            && let Some(cell) = to_cell(pos)
            && cell.col != usize::MAX
        {
            let inside = st.selection().is_some_and(|(r0, r1, c0, c1)| cell.row >= r0 && cell.row <= r1 && cell.col >= c0 && cell.col <= c1);
            if !inside {
                st.anchor = Some(cell);
                st.cursor = Some(cell);
            }
        }

        // ---- inline editor ----
        if let Some((src, sc, buf, select_all)) = st.editing.as_mut() {
            let dc = st.dcols.iter().position(|c| c == sc);
            if let (Some(vr), Some(dc)) = (st.view.iter().position(|r| r == src), dc) {
                let y = origin.y + vr as f32 * row_h;
                let rect = Rect::from_min_max(pos2(origin.x + xs[dc], y), pos2(origin.x + xs[dc + 1], y + row_h));
                let edit_id = id.with("cell_edit");
                let te = egui::TextEdit::singleline(buf).id(edit_id).font(font.clone()).frame(egui::Frame::NONE).margin(egui::Margin::symmetric(6, 3));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                child.painter().rect(rect, 2.0, pal.bg2, Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
                let r = child.add_sized(rect.size(), te);
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                    if *select_all {
                        let mut s = egui::TextEdit::load_state(ui.ctx(), edit_id).unwrap_or_default();
                        s.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::end_of_str(buf))));
                        s.store(ui.ctx(), edit_id);
                        *select_all = false;
                    }
                }
                let (enter, esc, tab) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape), i.key_pressed(Key::Tab)));
                if esc {
                    st.editing = None;
                    resp.request_focus();
                } else if enter || tab || r.lost_focus() {
                    let (src, sc, val) = (*src, *sc, buf.clone());
                    if let Some(e) = g.edits.as_deref_mut() {
                        let orig = cell_value_raw(g.rs, e, src, sc).map(str::to_string);
                        if orig.as_deref() != Some(val.as_str()) {
                            set_cell(e, g.rs, src, sc, Some(val));
                        }
                    }
                    st.editing = None;
                    if tab && let Some(c) = st.cursor {
                        st.cursor = Some(Cell { row: c.row, col: (c.col + 1).min(ncols.saturating_sub(1)) });
                        st.anchor = st.cursor;
                    }
                    ui.memory_mut(|m| m.request_focus(focus_id));
                }
            } else {
                st.editing = None;
            }
        }

        // ---- context menu ----
        resp.context_menu(|ui| {
            ui.set_min_width(240.0);
            let Some((rows, cols)) = st.selection_rows_cols() else { return };
            let n = rows.len() * cols.len();
            ui.label(
                egui::RichText::new(format!("{} cell{} · {} row{}", n, if n == 1 { "" } else { "s" }, rows.len(), if rows.len() == 1 { "" } else { "s" }))
                    .small()
                    .color(pal.faint),
            );
            if ui.add(egui::Button::new(format!("{}  Copy", icon::COPY)).shortcut_text("⌘C")).clicked() {
                ui.ctx().copy_text(export::format(g.rs, &rows, &cols, g.cats, Format::Tsv, false, g.table_name).trim_end().to_string());
                ui.close();
            }
            ui.menu_button(format!("{}  Copy as", icon::CLIPBOARD), |ui| {
                ui.set_min_width(180.0);
                let all_cols: Vec<usize> = st.dcols.clone();
                for (label, f, whole_rows) in [
                    ("TSV with headers", Format::Tsv, false),
                    ("CSV", Format::Csv, false),
                    ("JSON", Format::Json, true),
                    ("Markdown table", Format::Markdown, false),
                    ("SQL INSERT", Format::Insert, true),
                    ("SQL UPDATE", Format::Update, true),
                    ("SQL WHERE", Format::Where, false),
                    ("TypeScript", Format::TypeScript, true),
                    ("IN (…) list", Format::InList, false),
                ] {
                    if w::menu_item(ui, "", label).clicked() {
                        // record-shaped formats use whole rows when a single cell is selected
                        let c = if whole_rows && cols.len() == 1 { &all_cols } else { &cols };
                        ui.ctx().copy_text(export::format_with_pk(g.rs, &rows, c, g.cats, f, true, g.table_name, g.pk_cols));
                        ui.close();
                    }
                }
            });
            if cols.len() == 1 && w::menu_item(ui, icon::COLUMNS, "Copy column name").clicked() {
                ui.ctx().copy_text(g.rs.columns[cols[0]].name.clone());
                ui.close();
            }
            ui.separator();
            if let Some((src, col)) = st.cursor_source() {
                let v = cell_value(&g, src, col).map(str::to_string);
                if w::menu_item(ui, icon::EYE, "View value").clicked() {
                    action = Some(GridAction::ViewValue(src, col));
                    ui.close();
                }
                if g.fk_cols.contains(&col) && v.is_some() && w::menu_item_shortcut(ui, icon::EXTERNAL, "Go to referenced row", "⌘-click").clicked() {
                    action = Some(GridAction::FollowFk(src, col));
                    ui.close();
                }
                ui.menu_button(format!("{}  Filter", icon::FILTER), |ui| {
                    ui.set_min_width(200.0);
                    match &v {
                        Some(val) => {
                            if w::menu_item(ui, icon::FILTER, "Filter by this value").clicked() {
                                action = Some(GridAction::Filter(col, FilterOp::Eq(val.clone())));
                                ui.close();
                            }
                            if w::menu_item(ui, icon::X, "Exclude this value").clicked() {
                                action = Some(GridAction::Filter(col, FilterOp::Ne(val.clone())));
                                ui.close();
                            }
                        }
                        None => {
                            if w::menu_item(ui, icon::CIRCLE, "Show only NULLs").clicked() {
                                action = Some(GridAction::Filter(col, FilterOp::IsNull));
                                ui.close();
                            }
                        }
                    }
                    if w::menu_item(ui, icon::CIRCLE, "Show NULLs").clicked() {
                        action = Some(GridAction::Filter(col, FilterOp::IsNull));
                        ui.close();
                    }
                    if w::menu_item(ui, icon::EYE_OFF, "Hide NULLs").clicked() {
                        action = Some(GridAction::Filter(col, FilterOp::NotNull));
                        ui.close();
                    }
                    ui.separator();
                    if w::menu_item(ui, icon::CODE, "Copy as WHERE").clicked() {
                        ui.ctx().copy_text(export::format(g.rs, &rows, &cols, g.cats, Format::Where, false, g.table_name));
                        ui.close();
                    }
                });
            }
            if rows.len() >= 2 && rows.len() <= 6 && w::menu_item(ui, icon::COLUMNS, &format!("Compare {} rows", rows.len())).clicked() {
                action = Some(GridAction::CompareRows(rows.clone()));
                ui.close();
            }
            if editable {
                ui.separator();
                if w::menu_item(ui, icon::ERASER, "Set NULL").clicked() {
                    if let Some(e) = g.edits.as_deref_mut() {
                        for &r in &rows {
                            for &c in &cols {
                                set_cell(e, g.rs, r, c, None);
                            }
                        }
                    }
                    ui.close();
                }
                if let Some((src, _)) = st.cursor_source()
                    && w::menu_item(ui, icon::COPY, "Duplicate row").clicked()
                {
                    action = Some(GridAction::DuplicateRow(src));
                    ui.close();
                }
                let label = if rows.len() == 1 { "Delete row".to_string() } else { format!("Delete {} rows", rows.len()) };
                if w::menu_item_danger(ui, icon::TRASH, &label).clicked() {
                    action = Some(GridAction::DeleteRows(rows.clone()));
                    ui.close();
                }
            }
        });
    });
    // ScrollArea applies this frame's wheel delta after painting its contents.
    // Use the offset actually painted, so the header cannot lead the body.
    let header_offset_x = painted_offset_x;

    // ---- header painting & interaction --------------------------------------------
    let painter = ui.painter_at(header_rect);
    painter.rect_filled(header_rect, 0.0, pal.bg1);
    painter.hline(header_rect.x_range(), header_rect.bottom() - 0.5, Stroke::new(1.0, pal.border));
    let hx = header_rect.left() - header_offset_x;
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let mut new_sort = None;
    let mut hide: Option<usize> = None;
    for c in 0..ncols {
        let sc = st.dcols[c];
        let x0 = hx + xs[c];
        let x1 = hx + xs[c + 1];
        if x1 < header_rect.left() + gutter_w || x0 > header_rect.right() {
            continue;
        }
        let rect = Rect::from_min_max(pos2(x0, header_rect.top()), pos2(x1, header_rect.bottom()));
        let col = &g.rs.columns[sc];
        let is_pk = g.pk_cols.contains(&sc);
        let is_fk = g.fk_cols.contains(&sc);
        let sorted = st.sort.filter(|s| s.0 == sc);
        let filtered = st.col_filters.iter().any(|f| f.column == col.name);
        let hovered = pointer.is_some_and(|p| rect.contains(p) && header_rect.contains(p));
        let cursor_col = st.cursor.is_some_and(|cu| cu.col == c);
        if hovered {
            painter.rect_filled(rect, 0.0, pal.bg3);
        } else if cursor_col {
            painter.rect_filled(rect, 0.0, pal.accent_soft.gamma_multiply(0.5));
        }
        let mut name_x = rect.left() + 8.0;
        if is_pk || is_fk {
            painter.text(
                pos2(name_x, rect.top() + 13.0),
                Align2::LEFT_CENTER,
                if is_pk { icon::KEY } else { icon::LINK },
                FontId::proportional(11.0),
                if is_pk { pal.warn } else { pal.accent },
            );
            name_x += 16.0;
        }
        let w = rect.width() - (name_x - rect.left()) - 22.0;
        let name_g = ui.ctx().fonts_mut(|f| f.layout_job(cell_job(&col.name, &theme::bold(g.font_size), pal.text, w, false)));
        painter.galley(pos2(name_x, rect.top() + 6.0), name_g, pal.text);
        let ty = g.type_names.get(sc).map(String::as_str).unwrap_or("");
        let ty_g = ui.ctx().fonts_mut(|f| f.layout_job(cell_job(ty, &FontId::proportional(g.font_size * 0.82), pal.faint, rect.width() - 16.0, false)));
        painter.galley(pos2(rect.left() + 8.0, rect.top() + 22.0), ty_g, pal.faint);
        if let Some((_, asc)) = sorted {
            painter.text(
                pos2(rect.right() - 10.0, rect.center().y),
                Align2::CENTER_CENTER,
                if asc { icon::ARROW_UP } else { icon::ARROW_DOWN },
                FontId::proportional(12.0),
                pal.accent,
            );
        } else if filtered {
            painter.text(pos2(rect.right() - 10.0, rect.center().y), Align2::CENTER_CENTER, icon::FILTER, FontId::proportional(11.0), pal.warn);
        }
        painter.vline(x1 - 0.5, rect.y_range(), Stroke::new(1.0, pal.border.gamma_multiply(0.6)));

        // resize handle
        let handle = Rect::from_min_max(pos2(x1 - 4.0, rect.top()), pos2(x1 + 4.0, rect.bottom()));
        let hr = ui.interact(handle.intersect(header_rect), id.with(("resize", sc)), Sense::click_and_drag());
        if hr.hovered() || hr.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeColumn);
        }
        if hr.drag_started() {
            st.drag_resize = Some((sc, st.widths[sc], hr.interact_pointer_pos().map_or(0.0, |p| p.x)));
        }
        if hr.dragged()
            && let Some((dc, w0, x_start)) = st.drag_resize
            && dc == sc
            && let Some(p) = hr.interact_pointer_pos()
        {
            st.widths[sc] = (w0 + p.x - x_start).max(MIN_W);
        }
        if hr.drag_stopped() {
            st.drag_resize = None;
        }
        if hr.double_clicked() {
            let char_w = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0'));
            let mut maxc = col.name.chars().count() + 3;
            for &r in st.view.iter().take(2000) {
                maxc = maxc.max(cell_value(&g, r, sc).map_or(4, |v| v.lines().next().unwrap_or("").chars().count().min(200)));
            }
            st.widths[sc] = (maxc as f32 * char_w + 22.0).clamp(MIN_W, 900.0);
        }
        let body = Rect::from_min_max(rect.min, pos2(x1 - 4.0, rect.max.y)).intersect(header_rect);
        let br = ui.interact(body, id.with(("hdr", sc)), Sense::click());
        if br.clicked() {
            if ui.input(|i| i.modifiers.command || i.modifiers.shift) {
                if nrows > 0 {
                    st.anchor = Some(Cell { row: 0, col: c });
                    st.cursor = Some(Cell { row: nrows - 1, col: c });
                }
            } else {
                new_sort = Some(match sorted {
                    None => Some((sc, true)),
                    Some((_, true)) => Some((sc, false)),
                    Some((_, false)) => None,
                });
            }
        }
        br.on_hover_text(format!(
            "{}\n{}{}{}\n\nClick: sort · ⌘-click: select column · drag edge: resize · double-click edge: fit",
            col.name,
            ty,
            if is_pk { " · primary key" } else { "" },
            if is_fk { " · foreign key (⌘-click a value to follow)" } else { "" }
        ))
        .context_menu(|ui| {
            if w::menu_item(ui, icon::SORT_ASC, "Sort ascending").clicked() {
                new_sort = Some(Some((sc, true)));
                ui.close();
            }
            if w::menu_item(ui, icon::SORT_DESC, "Sort descending").clicked() {
                new_sort = Some(Some((sc, false)));
                ui.close();
            }
            if st.sort.is_some() && w::menu_item(ui, icon::X, "Clear sort").clicked() {
                new_sort = Some(None);
                ui.close();
            }
            ui.separator();
            if w::menu_item(ui, icon::EYE_OFF, "Hide column").clicked() {
                hide = Some(sc);
                ui.close();
            }
            if w::menu_item(ui, icon::COPY, "Copy column name").clicked() {
                ui.ctx().copy_text(col.name.clone());
                ui.close();
            }
            if w::menu_item(ui, icon::COLUMNS, "Select column").clicked() {
                if nrows > 0 {
                    st.anchor = Some(Cell { row: 0, col: c });
                    st.cursor = Some(Cell { row: nrows - 1, col: c });
                }
                ui.close();
            }
        });
    }
    // gutter corner
    let corner = Rect::from_min_size(header_rect.min, vec2(gutter_w, HEADER_H));
    painter.rect_filled(corner, 0.0, pal.bg1);
    painter.text(corner.center(), Align2::CENTER_CENTER, "#", theme::mono(g.font_size * 0.9), pal.faint);
    painter.vline(corner.right() - 0.5, corner.y_range(), Stroke::new(1.0, pal.border));
    let cr = ui.interact(corner, id.with("corner"), Sense::click()).on_hover_text("Select all (⌘A)");
    if cr.clicked() {
        st.select_all();
        ui.memory_mut(|m| m.request_focus(focus_id));
    }
    if let Some(s) = new_sort {
        st.sort = s;
    }
    if let Some(h) = hide {
        st.hidden.insert(h);
    }

    // ---- column finder (⌘F) -----------------------------------------------------------
    if let Some((query, sel)) = &mut st.finder {
        let mut close = false;
        let mut jump: Option<usize> = None;
        let q = query.to_lowercase();
        let matches: Vec<usize> = (0..g.rs.ncols()).filter(|&c| q.is_empty() || g.rs.columns[c].name.to_lowercase().contains(&q)).take(30).collect();
        egui::Area::new(id.with("finder")).order(egui::Order::Foreground).fixed_pos(header_rect.right_top() + vec2(-300.0, HEADER_H + 4.0)).show(
            ui.ctx(),
            |ui| {
                egui::Frame::popup(ui.style()).fill(pal.bg1).show(ui, |ui| {
                    ui.set_width(280.0);
                    let r = ui.add(egui::TextEdit::singleline(query).hint_text(format!("{}  Jump to column…", icon::SEARCH)).desired_width(f32::INFINITY));
                    r.request_focus();
                    if r.changed() {
                        *sel = 0;
                    }
                    let (up, down, enter, esc) = ui.input_mut(|i| {
                        (
                            i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                            i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                            i.consume_key(egui::Modifiers::NONE, Key::Enter),
                            i.consume_key(egui::Modifiers::NONE, Key::Escape),
                        )
                    });
                    if down && !matches.is_empty() {
                        *sel = (*sel + 1) % matches.len();
                    }
                    if up && !matches.is_empty() {
                        *sel = (*sel + matches.len() - 1) % matches.len();
                    }
                    if esc {
                        close = true;
                    }
                    if enter {
                        jump = matches.get(*sel).copied();
                    }
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        for (i, &c) in matches.iter().enumerate() {
                            let hidden = st.hidden.contains(&c);
                            let label = format!("{}{}", g.rs.columns[c].name, if hidden { "  (hidden)" } else { "" });
                            if ui.selectable_label(i == *sel, label).clicked() {
                                jump = Some(c);
                            }
                        }
                    });
                });
            },
        );
        if let Some(sc) = jump {
            st.hidden.remove(&sc);
            if let Some(dc) = (0..g.rs.ncols()).filter(|c| !st.hidden.contains(c)).position(|c| c == sc) {
                // keep the current row, highlight the found column
                let row = st.cursor.map_or(0, |c| c.row);
                st.cursor = Some(Cell { row, col: dc });
                st.anchor = st.cursor;
                st.scroll_to_cursor = true;
            }
            close = true;
        }
        if close {
            st.finder = None;
            ui.memory_mut(|m| m.request_focus(focus_id));
        }
    }
    st.render_ms = t0.elapsed().as_secs_f32() * 1000.0;
    action
}

fn cell_value<'a>(g: &'a GridCtx, src: usize, col: usize) -> Option<&'a str> {
    if let Some(e) = g.edits.as_deref() {
        return cell_value_raw(g.rs, e, src, col);
    }
    g.rs.get(src, col)
}

fn cell_value_raw<'a>(rs: &'a ResultSet, e: &'a Edits, src: usize, col: usize) -> Option<&'a str> {
    if src >= rs.nrows() {
        return e.inserted.get(src - rs.nrows())?.get(col)?.as_deref();
    }
    match e.cells.get(&(src, col)) {
        Some(v) => v.as_deref(),
        None => rs.get(src, col),
    }
}

fn set_cell(e: &mut Edits, rs: &ResultSet, src: usize, col: usize, v: Option<String>) {
    e.revision = e.revision.wrapping_add(1);
    if src >= rs.nrows() {
        if let Some(row) = e.inserted.get_mut(src - rs.nrows())
            && let Some(cell) = row.get_mut(col)
        {
            *cell = v;
        }
        return;
    }
    if rs.get(src, col).map(str::to_string) == v {
        e.cells.remove(&(src, col));
    } else {
        e.cells.insert((src, col), v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(values: &[Option<&str>]) -> ResultSet {
        let mut rs = ResultSet::new(vec![crate::db::result::Column::text("value")]);
        for &v in values {
            rs.push_text_row(&[v]);
        }
        rs
    }

    fn ctx<'a>(rs: &'a ResultSet, edits: Option<&'a mut Edits>) -> GridCtx<'a> {
        GridCtx { rs, edits, cats: &[], type_names: &[], font_size: 12.0, max_chars: 100, table_name: "", pk_cols: &[], fk_cols: &[], ts_mode: TsMode::Raw }
    }

    #[test]
    fn equal_sized_replacement_invalidates_filter_and_empty_columns() {
        let mut st = GridState { filter: "yes".into(), hide_empty: true, ..Default::default() };
        let a = data(&[Some("yes"), None]);
        st.ensure_view(&ctx(&a, None));
        assert_eq!(st.view_rows(), &[0]);
        let b = data(&[Some("no!"), None]);
        st.ensure_view(&ctx(&b, None));
        assert!(st.view_rows().is_empty());
        let empty = data(&[None, None]);
        st.ensure_view(&ctx(&empty, None));
        assert!(st.display_cols().is_empty());
        st.ensure_view(&ctx(&a, None));
        assert_eq!(st.display_cols(), &[0]);
        assert_eq!(st.view_rows(), &[0]);
    }

    #[test]
    fn search_sort_and_hide_empty_follow_edits() {
        let rs = data(&[Some("beta"), Some("alpha")]);
        let mut edits = Edits::default();
        let mut st = GridState { sort: Some((0, true)), ..Default::default() };
        st.ensure_view(&ctx(&rs, Some(&mut edits)));
        assert_eq!(st.view_rows(), &[1, 0]);
        set_cell(&mut edits, &rs, 0, 0, Some("aardvark".into()));
        st.ensure_view(&ctx(&rs, Some(&mut edits)));
        assert_eq!(st.view_rows(), &[0, 1]);
        st.filter = "aardvark".into();
        st.ensure_view(&ctx(&rs, Some(&mut edits)));
        assert_eq!(st.view_rows(), &[0]);
        set_cell(&mut edits, &rs, 0, 0, None);
        set_cell(&mut edits, &rs, 1, 0, None);
        st.hide_empty = true;
        st.ensure_view(&ctx(&rs, Some(&mut edits)));
        assert!(st.display_cols().is_empty());
        assert!(st.view_rows().is_empty());
        st.ensure_view(&ctx(&rs, None));
        assert_eq!(st.display_cols(), &[0]);
    }

    /// Release-mode client-side benchmark; no server or GPU required.
    #[test]
    #[ignore]
    fn bench_empty_filter_million() {
        let mut rs = ResultSet::new(vec![crate::db::result::Column::text("value")]);
        for _ in 0..1_000_000 {
            rs.push_text_row(&[Some("hello")]);
        }
        let mut st = GridState { filter: "absent".into(), ..Default::default() };
        let start = std::time::Instant::now();
        st.ensure_view(&ctx(&rs, None));
        let first = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            st.ensure_view(&ctx(&rs, None));
        }
        println!(
            "TRUNQ_BENCH empty_filter rows={} first_ms={:.3} cached_1000_ms={:.3}",
            rs.nrows(),
            first.as_secs_f64() * 1000.0,
            start.elapsed().as_secs_f64() * 1000.0
        );
        assert!(st.view_rows().is_empty());
    }

    #[test]
    fn aggregate_cache_tracks_selection_and_edits() {
        let rs = data(&[Some("2"), Some("4"), Some("NaN")]);
        let mut st = GridState::default();
        st.ensure_view(&ctx(&rs, None));
        st.select_all();
        let cats = [Cat::Number];
        let stats = st.selection_stats(&rs, &cats, None).unwrap();
        assert!(stats.contains("Σ 6 · avg 3 · min 2 · max 4"));
        assert_eq!(st.selection_stats(&rs, &cats, None).as_ref(), Some(&stats));
        let mut edits = Edits::default();
        set_cell(&mut edits, &rs, 0, 0, Some("8".into()));
        assert!(st.selection_stats(&rs, &cats, Some(&edits)).unwrap().contains("Σ 12 · avg 6"));
        st.focus_row(1);
        assert!(st.selection_stats(&rs, &cats, Some(&edits)).unwrap().contains("Σ 4"));
    }

    #[test]
    fn numeric_sort_preserves_bigint_precision_and_orders_nan() {
        let rs = data(&[Some("9007199254740993"), Some("9007199254740992"), Some("NaN"), Some("-9007199254740993"), Some("-9007199254740992")]);
        let mut st = GridState { sort: Some((0, true)), ..Default::default() };
        let mut context = ctx(&rs, None);
        context.cats = &[Cat::Number];
        st.ensure_view(&context);
        assert_eq!(st.view_rows(), &[3, 4, 1, 0, 2]);
        assert_eq!(decimal_cmp("-0.00100", "-0.00099"), Some(std::cmp::Ordering::Less));
        assert_eq!(decimal_cmp("0.100", "0.1"), Some(std::cmp::Ordering::Equal));
        assert_eq!(decimal_cmp("-0", "0"), Some(std::cmp::Ordering::Equal));
    }

    #[test]
    fn scrolling_loaded_pages_does_not_reverse_direction() {
        let mut rs = data(&vec![Some("row"); 5000]);
        let mut grid = GridState::default();
        let context = egui::Context::default();
        let mut offsets = Vec::new();
        for frame in 0..340 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0))),
                time: Some(frame as f64 / 60.0),
                events: vec![
                    egui::Event::PointerMoved(pos2(400.0, 300.0)),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: vec2(0.0, if frame < 250 { -800.0 } else { -800.0 * 0.9_f32.powi(frame - 250) }),
                        phase: egui::TouchPhase::Move,
                        source: if frame < 250 { egui::MouseWheelSource::Trackpad } else { egui::MouseWheelSource::Momentum },
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            let mut output = context.run_ui(input, |ui| {
                let id = Id::unique("scroll-regression");
                let scroll_id = ui.make_persistent_id(egui::IdSalt::new(id.with("scroll")));
                show(ui, id, &mut grid, ctx(&rs, None));
                let state = egui::scroll_area::State::load(ui.ctx(), scroll_id).unwrap();
                offsets.push(state.unclamped_offset().y);
            });
            output.textures_delta.clear();
            if grid.near_end && rs.nrows() < 8000 {
                for _ in 0..1000 {
                    rs.push_text_row(&[Some("row")]);
                }
            }
        }
        for pair in offsets.windows(2) {
            assert!(pair[1] >= pair[0] - 1.0, "scroll reversed: {pair:?}");
        }
        assert!(offsets.last().unwrap() > &100_000.0);
    }

    #[test]
    fn natural() {
        assert_eq!(natural_cmp("file2", "file10"), std::cmp::Ordering::Less);
        assert_eq!(natural_cmp("a", "B"), std::cmp::Ordering::Less);
        assert!(contains_ascii_ci(b"Hello World", b"o w"));
    }

    #[test]
    fn timestamps() {
        assert_eq!(format_ts("2026-10-22 00:11:51.411675+02", TsMode::Utc).as_deref(), Some("2026-10-21 22:11:51.411Z"));
        assert_eq!(format_ts("2026-10-22 00:11:51+05:30", TsMode::Utc).as_deref(), Some("2026-10-21 18:41:51.000Z"));
        assert!(format_ts("2026-10-22 00:11:51+02", TsMode::Raw).is_none());
    }

    #[test]
    fn filter_sql() {
        let f = ColFilter { column: "status".into(), op: FilterOp::Ne("paid".into()) };
        assert_eq!(f.sql(), "status IS DISTINCT FROM 'paid'");
        assert_eq!(ColFilter { column: "x".into(), op: FilterOp::IsNull }.sql(), "x IS NULL");
    }
}
