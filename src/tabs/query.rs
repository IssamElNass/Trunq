//! Query console tab. Also powers the table data view (`table` mode): the
//! editor is replaced by a filter bar and the grid becomes editable.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Align, Layout, RichText, Ui, vec2};

use crate::action::{Action, ToastKind};
use crate::config::{HistoryEntry, Profile, Settings};
use crate::db::meta::{Catalog, quote_ident, quote_literal};
use crate::db::result::ResultSet;
use crate::db::session::{Event, Promise, Session};
use crate::db::types::Cat;
use crate::db::wire::TxStatus;
use crate::export::{self, Format};
use crate::sql::{format, lexer};
use crate::ui::editor::{self, EditorState};
use crate::ui::explain::PlanView;
use crate::ui::grid::{self, Edits, GridAction, GridState};
use crate::ui::theme::{self, icon};
use crate::ui::widgets as w;
use crate::util;

mod results;

/// Close idle connections after this long (no transaction, no open cursor).
const IDLE_DISCONNECT: Duration = Duration::from_secs(15 * 60);

pub struct TabCtx<'a> {
    pub settings: &'a Settings,
    pub profiles: &'a [Profile],
    pub catalog: Option<Arc<Catalog>>,
    pub actions: &'a mut Vec<Action>,
    pub history: &'a mut crate::config::History,
    pub passwords: &'a HashMap<u64, String>,
    /// Handle on the server's browser connection, used for live monitoring.
    pub monitor: Option<crate::db::session::QueryHandle>,
    /// Metadata connection of this tab's database (FK peek, row lookups).
    pub meta: Option<crate::db::session::QueryHandle>,
    pub table_prefs: &'a HashMap<String, crate::config::TablePrefs>,
    pub snippets: &'a [crate::config::Snippet],
}

pub struct Pane {
    pub title: String,
    pub sql: String,
    pub rs: ResultSet,
    pub cats: Vec<Cat>,
    pub type_names: Vec<String>,
    pub grid: GridState,
    pub more: bool,
    pub fetching: bool,
    pub tag: Option<String>,
    pub elapsed: Duration,
    pub plan: Option<PlanView>,
    pub show_plan_raw: bool,
    /// Per column: referenced (schema, table, column) for single-column FKs.
    pub fks: Vec<Option<(String, String, String)>>,
    pub chart: Option<crate::ui::chart::ChartState>,
    /// Kept when the query is re-run.
    pub pinned: bool,
    /// Row card view instead of the grid.
    pub card: bool,
    /// Server time until the first rows arrived, and the rest of the fetch.
    pub exec_ms: Option<f32>,
    pub fetch_ms: Option<f32>,
    /// Bound parameters of the statement (for "export entire result").
    pub params: Vec<Option<String>>,
}

#[derive(Clone, Copy, PartialEq)]
enum MsgKind {
    Info,
    Ok,
    Error,
    Notice,
}

struct Msg {
    kind: MsgKind,
    text: String,
    at: chrono::DateTime<chrono::Local>,
}

struct Running {
    id: u64,
    started: Instant,
    /// statement text and its byte offset in the editor
    stmts: Vec<(String, usize)>,
    current: usize,
    explain: Option<bool>,
    fetch: bool,
    rows_total: u64,
    ok: bool,
    /// Dry run: (used a savepoint instead of BEGIN)
    dry: Option<bool>,
    /// When the current statement started / delivered its first result.
    stmt_started: Instant,
    first_result: Option<Instant>,
    params: Vec<Vec<Option<String>>>,
}

pub struct TableMode {
    pub schema: String,
    pub name: String,
    pub filter: String,
    pub order: String,
    pub pk: Vec<String>,
    pub edits: Edits,
    pub save: Option<Promise<Result<Vec<ResultSet>, String>>>,
    pub preview_sql: Option<String>,
    pub count: Option<Promise<crate::db::session::QueryResult>>,
    pub exact_count: Option<String>,
}

/// Statements awaiting `:name` values: (statements, explain mode, parameter names).
type ParamsPrompt = (Vec<(String, usize)>, Option<bool>, Vec<String>);

struct Confirm {
    stmts: Vec<(String, usize)>,
    params: Vec<Vec<Option<String>>>,
    explain: Option<bool>,
    danger: Option<String>,
    lints: Vec<crate::sql::lint::Lint>,
}

pub struct Viewer {
    title: String,
    value: Option<String>,
    pretty: bool,
    tree: Option<crate::ui::json_tree::JsonTree>,
    /// Show the JSON tree (when the value is JSON) instead of text.
    as_tree: bool,
}

/// A running "export the entire result" job.
struct ExportJob {
    session: Session,
    promise: Promise<Result<u64, String>>,
    progress: Arc<std::sync::atomic::AtomicU64>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    path: PathBuf,
}

pub struct QueryTab {
    pub uid: u64,
    pub title: String,
    pub profile_id: Option<u64>,
    pub database: String,
    pub session: Option<Session>,
    pub editor: EditorState,
    pub panes: Vec<Pane>,
    active_pane: usize,
    messages: Vec<Msg>,
    run: Option<Running>,
    pub file: Option<PathBuf>,
    pub saved_text: String,
    split: f32,
    viewer: Option<Viewer>,
    confirm: Option<Confirm>,
    params_prompt: Option<ParamsPrompt>,
    pub param_values: HashMap<String, String>,
    password_input: String,
    pub autocommit: bool,
    pub table: Option<TableMode>,
    last_activity: Instant,
    idle_closed: bool,
    pending_run: bool,
    monitor: super::monitor::Monitor,
    tx_since: Option<Instant>,
    pub ts_mode: grid::TsMode,
    /// (pane, source rows, differences only)
    compare: Option<(usize, Vec<usize>, bool)>,
    side_by_side: bool,
    fk_cache: HashMap<String, Result<ResultSet, String>>,
    fk_pending: HashMap<String, Promise<crate::db::session::QueryResult>>,
    export_job: Option<ExportJob>,
    renaming: Option<(usize, String)>,
    prefs_applied: bool,
    prefs_checked: Instant,
    prefs_last: Option<crate::config::TablePrefs>,
    snippet_prompt: Option<String>,
}

static NEXT_UID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl QueryTab {
    pub fn new(profile_id: Option<u64>, database: &str, sql: String, editor_font: f32) -> QueryTab {
        let uid = NEXT_UID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut editor = EditorState::new(egui::Id::unique(("editor", uid)), sql.clone());
        editor.font_size = editor_font;
        QueryTab {
            uid,
            title: format!("Query {uid}"),
            profile_id,
            database: database.to_string(),
            session: None,
            editor,
            panes: Vec::new(),
            active_pane: 0,
            messages: Vec::new(),
            run: None,
            file: None,
            saved_text: sql,
            split: 0.45,
            viewer: None,
            confirm: None,
            params_prompt: None,
            param_values: HashMap::new(),
            password_input: String::new(),
            autocommit: true,
            table: None,
            last_activity: Instant::now(),
            idle_closed: false,
            pending_run: false,
            monitor: Default::default(),
            tx_since: None,
            ts_mode: grid::TsMode::Raw,
            compare: None,
            side_by_side: false,
            fk_cache: HashMap::new(),
            fk_pending: HashMap::new(),
            export_job: None,
            renaming: None,
            prefs_applied: false,
            prefs_checked: Instant::now(),
            prefs_last: None,
            snippet_prompt: None,
        }
    }

    pub fn new_table(profile_id: u64, database: &str, schema: &str, name: &str, pk: Vec<String>, settings: &Settings) -> QueryTab {
        let mut t = QueryTab::new(Some(profile_id), database, String::new(), settings.editor_font_size);
        t.title = name.to_string();
        t.table = Some(TableMode {
            schema: schema.to_string(),
            name: name.to_string(),
            filter: String::new(),
            order: String::new(),
            pk,
            edits: Edits::default(),
            save: None,
            preview_sql: None,
            count: None,
            exact_count: None,
        });
        t.pending_run = true;
        t
    }

    pub fn is_dirty(&self) -> bool {
        self.table.is_none() && self.file.is_some() && self.editor.text != self.saved_text
    }

    pub fn dismiss_dialogs(&mut self) {
        self.confirm = None;
        self.params_prompt = None;
    }

    pub fn is_running(&self) -> bool {
        self.run.is_some()
    }

    pub fn has_unsaved_edits(&self) -> bool {
        self.table.as_ref().is_some_and(|t| !t.edits.is_empty())
    }

    fn profile<'a>(&self, ctx: &'a TabCtx) -> Option<&'a Profile> {
        ctx.profiles.iter().find(|p| Some(p.id) == self.profile_id)
    }

    fn ensure_session(&mut self, ctx: &TabCtx, ui: &Ui) -> bool {
        let Some(profile) = self.profile(ctx).cloned() else {
            self.msg(MsgKind::Error, "No connection selected. Pick one in the toolbar.".into());
            return false;
        };
        let need_new = match &self.session {
            None => true,
            Some(s) => !s.is_alive() || s.profile_id != profile.id || s.database != self.database,
        };
        if need_new {
            let purpose = if self.table.is_some() { format!("Data: {}", self.title) } else { self.title.clone() };
            self.session = Some(Session::connect(&profile, &self.database, &purpose, ctx.passwords.get(&profile.id).cloned(), ui.ctx()));
        }
        true
    }

    fn msg(&mut self, kind: MsgKind, text: String) {
        self.messages.push(Msg { kind, text, at: chrono::Local::now() });
        if self.messages.len() > 500 {
            self.messages.drain(..100);
        }
    }

    /// Drops idle connections so that a forgotten tab never pins a backend.
    pub fn maintenance(&mut self) {
        if let Some(s) = &self.session
            && !s.is_busy()
            && self.run.is_none()
            && s.tx_status == TxStatus::Idle
            && self.panes.iter().all(|p| !p.more)
            && self.last_activity.elapsed() > IDLE_DISCONNECT
        {
            self.session = None;
            self.idle_closed = true;
        }
    }

    // ---- running ----------------------------------------------------------------

    /// Statements to run: the selection, the whole script, or the
    /// statement under the cursor. Each with its byte offset in the editor.
    fn collect_statements(&self, ui: &Ui, all: bool) -> Vec<(String, usize)> {
        let text = &self.editor.text;
        let state = egui::TextEdit::load_state(ui.ctx(), self.editor.id.with("text"));
        let (sel, cursor) = match state.and_then(|s| s.cursor.char_range()) {
            Some(r) => {
                let [a, b] = r.sorted_cursors();
                let ab = text.char_indices().nth(a.index.0).map_or(text.len(), |x| x.0);
                let bb = text.char_indices().nth(b.index.0).map_or(text.len(), |x| x.0);
                let cur = text.char_indices().nth(r.primary.index.0).map_or(text.len(), |x| x.0);
                ((ab != bb).then_some(ab..bb), cur)
            }
            None => (None, text.len()),
        };
        let mut stmts: Vec<(String, usize)> = Vec::new();
        if let Some(sel) = sel {
            let part = &text[sel.clone()];
            for s in lexer::split_statements(part) {
                stmts.push((s.text(part).to_string(), sel.start + s.range.start));
            }
        } else if all {
            for s in lexer::split_statements(text) {
                stmts.push((s.text(text).to_string(), s.range.start));
            }
        } else if let Some(s) = lexer::statement_at(text, cursor) {
            stmts.push((s.text(text).to_string(), s.range.start));
        }
        stmts
    }

    pub fn run_current(&mut self, ctx: &mut TabCtx, ui: &Ui, all: bool, explain: Option<bool>) {
        if self.run.is_some() {
            return;
        }
        let mut stmts = self.collect_statements(ui, all);
        if stmts.is_empty() {
            self.msg(MsgKind::Info, "Nothing to run".into());
            return;
        }
        if let Some(exp) = explain {
            let (s, off) = stmts[0].clone();
            let prefix = if exp { "EXPLAIN (ANALYZE, BUFFERS, VERBOSE, FORMAT JSON) " } else { "EXPLAIN (VERBOSE, FORMAT JSON) " };
            stmts = vec![(format!("{prefix}{s}"), off.saturating_sub(prefix.len()))];
        }
        let mut names: Vec<String> = Vec::new();
        for (s, _) in &stmts {
            for (n, _) in lexer::find_params(s) {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
        }
        if !names.is_empty() {
            self.params_prompt = Some((stmts, explain, names));
            return;
        }
        self.run_checked(ctx, ui, stmts, Vec::new(), explain);
    }

    /// Safety checks (dangerous statements, migration linter), then run.
    fn run_checked(&mut self, ctx: &mut TabCtx, ui: &Ui, stmts: Vec<(String, usize)>, params: Vec<Vec<Option<String>>>, explain: Option<bool>) {
        if ctx.settings.confirm_destructive && explain != Some(false) {
            let danger = dangerous(&stmts);
            let lints: Vec<_> = stmts.iter().flat_map(|(s, _)| crate::sql::lint::lint(s)).collect();
            if danger.is_some() || !lints.is_empty() {
                self.confirm = Some(Confirm { stmts, params, explain, danger, lints });
                return;
            }
        }
        self.start_run_params(ctx, ui, stmts, params, explain);
    }

    /// Executes the DML statement under the cursor inside a transaction (or
    /// savepoint), shows the affected rows, and always rolls back.
    pub fn dry_run(&mut self, ctx: &mut TabCtx, ui: &Ui) {
        if self.run.is_some() {
            return;
        }
        let stmts = self.collect_statements(ui, false);
        let Some((stmt, off)) = stmts.into_iter().next() else { return };
        let words: Vec<String> = lexer::tokenize(&stmt)
            .into_iter()
            .filter(|t| !matches!(t.kind, lexer::Tok::Whitespace | lexer::Tok::Comment))
            .map(|t| stmt[t.range].to_ascii_lowercase())
            .collect();
        let first = words.iter().find(|w| *w != "with").map(String::as_str).unwrap_or("");
        let is_dml = ["insert", "update", "delete", "merge"].contains(&first)
            || (words.first().is_some_and(|w| w == "with") && words.iter().any(|w| ["insert", "update", "delete"].contains(&w.as_str())));
        if !is_dml {
            self.msg(MsgKind::Info, "Dry run works for INSERT, UPDATE, DELETE and MERGE statements.".into());
            self.active_pane = usize::MAX;
            return;
        }
        let mut body = stmt.clone();
        if first != "merge" && !words.iter().any(|w| w == "returning") {
            body.push_str("\nRETURNING *");
        }
        let in_tx = self.session.as_ref().is_some_and(|s| s.tx_status != TxStatus::Idle);
        let (open, close) = if in_tx { ("SAVEPOINT trunq_dry_run", "ROLLBACK TO SAVEPOINT trunq_dry_run") } else { ("BEGIN", "ROLLBACK") };
        let plan = vec![(open.to_string(), usize::MAX), (body, off), (close.to_string(), usize::MAX)];
        self.start_run(ctx, ui, plan, None);
        if let Some(r) = &mut self.run {
            r.dry = Some(in_tx);
        }
        self.msg(MsgKind::Info, "Dry run: executing inside a transaction that will be rolled back…".into());
    }

    fn start_run(&mut self, ctx: &mut TabCtx, ui: &Ui, stmts: Vec<(String, usize)>, explain: Option<bool>) {
        self.start_run_params(ctx, ui, stmts, Vec::new(), explain);
    }

    fn start_run_params(&mut self, ctx: &mut TabCtx, ui: &Ui, mut stmts: Vec<(String, usize)>, mut params: Vec<Vec<Option<String>>>, explain: Option<bool>) {
        if !self.ensure_session(ctx, ui) {
            return;
        }
        params.resize(stmts.len(), Vec::new());
        let session = self.session.as_mut().unwrap();
        if !self.autocommit && session.tx_status == TxStatus::Idle && !lexer::split_statements(&stmts[0].0).is_empty() {
            let first = stmts[0].0.trim_start().to_ascii_lowercase();
            if !(first.starts_with("begin") || first.starts_with("start transaction")) {
                stmts.insert(0, ("BEGIN".into(), usize::MAX));
                params.insert(0, Vec::new());
            }
        }
        let id = session.run_params(stmts.iter().map(|s| s.0.clone()).zip(params.iter().cloned()).collect(), ctx.settings.page_size);
        // pinned results survive a new run (their cursor is gone though)
        self.panes.retain(|p| p.pinned);
        for p in &mut self.panes {
            p.more = false;
            p.fetching = false;
        }
        self.active_pane = 0;
        self.viewer = None;
        self.editor.error = None;
        self.run = Some(Running {
            id,
            started: Instant::now(),
            stmts,
            current: 0,
            explain,
            fetch: false,
            rows_total: 0,
            ok: true,
            dry: None,
            stmt_started: Instant::now(),
            first_result: None,
            params,
        });
        self.last_activity = Instant::now();
        self.idle_closed = false;
    }

    fn run_table_query(&mut self, ctx: &mut TabCtx, ui: &Ui) {
        let Some(t) = &self.table else { return };
        let mut sql = format!("SELECT * FROM {}.{}", quote_ident(&t.schema), quote_ident(&t.name));
        if !t.filter.trim().is_empty() {
            sql.push_str(&format!("\nWHERE {}", t.filter.trim()));
        }
        if !t.order.trim().is_empty() {
            sql.push_str(&format!("\nORDER BY {}", t.order.trim()));
        } else if !t.pk.is_empty() {
            sql.push_str(&format!("\nORDER BY {}", t.pk.iter().map(|c| quote_ident(c)).collect::<Vec<_>>().join(", ")));
        }
        self.editor.text = sql.clone();
        if let Some(t) = &mut self.table {
            t.edits = Edits::default();
            t.exact_count = None;
        }
        self.start_run(ctx, ui, vec![(sql, 0)], None);
    }

    fn fetch_more(&mut self, ctx: &TabCtx, all: bool) {
        if self.run.is_some() {
            return;
        }
        let Some(session) = &self.session else { return };
        let Some(pane) = self.panes.last_mut() else { return };
        if !pane.more || pane.fetching {
            return;
        }
        pane.fetching = true;
        let id = 1_000_000 + self.uid * 1000 + pane.rs.nrows() as u64 % 1000;
        session.fetch_more(id, if all { 0 } else { ctx.settings.page_size.max(1000) });
        self.run = Some(Running {
            id,
            started: Instant::now(),
            stmts: vec![],
            current: 0,
            explain: None,
            fetch: true,
            rows_total: 0,
            ok: true,
            dry: None,
            stmt_started: Instant::now(),
            first_result: None,
            params: Vec::new(),
        });
        self.last_activity = Instant::now();
    }

    fn cancel(&mut self) {
        if let Some(s) = &self.session {
            s.cancel();
            self.msg(MsgKind::Info, "Cancel requested".into());
        }
    }

    /// Processes session events. Call every frame.
    pub fn poll(&mut self, ctx: &mut TabCtx) {
        let Some(session) = self.session.as_mut() else { return };
        let events = session.poll();
        if events.is_empty() {
            return;
        }
        for e in events {
            // Ignore stragglers from a previous (cancelled / superseded) run.
            let event_run = match &e {
                Event::StmtStart { id, .. }
                | Event::Columns { id, .. }
                | Event::Rows { id, .. }
                | Event::StmtDone { id, .. }
                | Event::StmtError { id, .. }
                | Event::RunFinished { id, .. } => Some(*id),
                _ => None,
            };
            if let Some(id) = event_run
                && self.run.as_ref().is_none_or(|r| r.id != id)
            {
                continue;
            }
            match e {
                Event::Connected { server_version, .. } => {
                    let pid = self.session.as_ref().map_or(0, |s| s.pid);
                    self.msg(MsgKind::Info, format!("Connected to {} (PostgreSQL {server_version}, backend pid {pid})", self.database));
                }
                Event::ConnectFailed { error, needs_password } => {
                    self.msg(MsgKind::Error, format!("Connection failed: {error}"));
                    if needs_password {
                        self.msg(MsgKind::Info, "Enter the password above to connect.".into());
                    }
                    self.run = None;
                    self.active_pane = usize::MAX;
                    self.pending_run = false;
                }
                Event::StmtStart { index, .. } => {
                    if let Some(r) = &mut self.run {
                        r.current = index;
                        r.stmt_started = Instant::now();
                        r.first_result = None;
                    }
                }
                Event::Columns { index, columns, elapsed: first_rows, .. } => {
                    let sql = self.run.as_ref().and_then(|r| r.stmts.get(index)).map(|s| s.0.clone()).unwrap_or_default();
                    let cat = ctx.catalog.as_deref();
                    let cats = columns
                        .iter()
                        .map(|c| cat.map_or_else(|| crate::db::types::builtin(c.type_oid).map_or(Cat::Other, |b| b.1), |k| k.type_cat(c.type_oid)))
                        .collect();
                    let type_names = columns
                        .iter()
                        .map(|c| {
                            cat.map_or_else(
                                || crate::db::types::builtin(c.type_oid).map_or_else(|| format!("oid {}", c.type_oid), |b| b.0.to_string()),
                                |k| k.type_name(c.type_oid),
                            )
                        })
                        .collect();
                    let explain = self.run.as_ref().and_then(|r| r.explain);
                    let n = self.panes.len() + 1;
                    let dry = self.run.as_ref().is_some_and(|r| r.dry.is_some());
                    let title = if explain.is_some() {
                        "Plan".to_string()
                    } else if dry {
                        "Dry run · rolled back".into()
                    } else if self.table.is_some() {
                        "Data".into()
                    } else if let Some(name) = result_name(&sql) {
                        name
                    } else {
                        format!("Result {n}")
                    };
                    let exec_ms = Some(first_rows.as_secs_f32() * 1000.0);
                    if let Some(r) = &mut self.run {
                        r.first_result = Some(Instant::now());
                    }
                    // reuse the remembered layout for table views
                    let mut grid = GridState::default();
                    if let Some(prefs) = self.prefs_last.clone() {
                        grid.apply_prefs(prefs.grid);
                    }
                    self.panes.push(Pane {
                        title,
                        sql,
                        rs: ResultSet::new(columns),
                        cats,
                        type_names,
                        grid,
                        more: false,
                        fetching: false,
                        tag: None,
                        elapsed: Duration::ZERO,
                        plan: None,
                        show_plan_raw: false,
                        fks: Vec::new(),
                        chart: None,
                        pinned: false,
                        card: self.prefs_last.as_ref().is_some_and(|p| p.card),
                        exec_ms,
                        fetch_ms: None,
                        params: self.run.as_ref().and_then(|r| r.params.get(index).cloned()).unwrap_or_default(),
                    });
                    // focus the first new result (pinned ones stay where they are)
                    if self.panes.iter().filter(|p| !p.pinned).count() == 1 {
                        self.active_pane = self.panes.len() - 1;
                    }
                }
                Event::Rows { index, rows, .. } => {
                    let n = rows.nrows() as u64;
                    let _ = index; // rows always belong to the most recent result
                    if let Some(p) = self.panes.last_mut() {
                        p.rs.append(rows);
                    }
                    if let Some(r) = &mut self.run {
                        r.rows_total += n;
                    }
                }
                Event::StmtDone { index, tag, more, elapsed, .. } => {
                    let fetch = self.run.as_ref().is_some_and(|r| r.fetch);
                    let stmt_has_pane =
                        self.panes.last().is_some_and(|p| p.sql == self.run.as_ref().and_then(|r| r.stmts.get(index)).map(|s| s.0.as_str()).unwrap_or(""));
                    if (fetch || (stmt_has_pane && index != usize::MAX))
                        && let Some(p) = self.panes.last_mut()
                    {
                        p.more = more;
                        p.fetching = false;
                        if !fetch {
                            p.elapsed = elapsed;
                            p.fetch_ms = p.exec_ms.map(|e| (elapsed.as_secs_f32() * 1000.0 - e).max(0.0));
                        } else {
                            p.fetch_ms = Some(p.fetch_ms.unwrap_or(0.0) + elapsed.as_secs_f32() * 1000.0);
                        }
                        if tag.is_some() {
                            p.tag = tag.clone();
                        }
                        if self.run.as_ref().and_then(|r| r.explain).is_some() {
                            p.plan = PlanView::from_result(&p.rs);
                        }
                    }
                    if !fetch {
                        let stmt = self.run.as_ref().and_then(|r| r.stmts.get(index)).map(|s| s.0.clone()).unwrap_or_default();
                        let short: String = stmt.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(90).collect();
                        let t = tag.clone().unwrap_or_else(|| if more { "rows fetched (more available)".into() } else { "OK".into() });
                        self.msg(MsgKind::Ok, format!("{t}  ·  {}  ·  {short}", util::duration(elapsed)));
                    }
                }
                Event::StmtError { index, error, .. } => {
                    if let Some(r) = &mut self.run {
                        r.ok = false;
                    }
                    if let Some(p) = self.panes.last_mut() {
                        p.fetching = false;
                    }
                    let mut text = error.to_string();
                    if !error.code.is_empty() {
                        text = format!("[{}] {text}", error.code);
                    }
                    // locate error in editor
                    if let (Some(pos), Some(r)) = (error.position, &self.run)
                        && let Some((stmt, off)) = r.stmts.get(index)
                        && *off != usize::MAX
                        && self.table.is_none()
                    {
                        let byte_in_stmt = stmt.char_indices().nth(pos.saturating_sub(1)).map_or(stmt.len(), |x| x.0);
                        let start = off + byte_in_stmt;
                        let end = start
                            + self.editor.text[start.min(self.editor.text.len())..]
                                .find(|c: char| c.is_whitespace() || c == ',' || c == ')' || c == ';')
                                .unwrap_or(1)
                                .max(1);
                        if start <= self.editor.text.len() {
                            self.editor.error = Some((start..end.min(self.editor.text.len()), error.message.clone()));
                            let line = self.editor.text[..start].matches('\n').count() + 1;
                            text = format!("{text}\nat line {line}");
                        }
                    }
                    self.msg(MsgKind::Error, text);
                    if self.panes.iter().all(|p| p.rs.is_empty()) {
                        self.active_pane = usize::MAX;
                    }
                }
                Event::RunFinished { elapsed, .. } => {
                    if let Some(r) = self.run.as_ref()
                        && let Some(savepoint) = r.dry
                    {
                        // Make sure nothing survives a dry run, even after an error.
                        let tx = self.session.as_ref().map(|s| s.tx_status).unwrap_or(TxStatus::Idle);
                        let needs = if savepoint { !r.ok } else { tx != TxStatus::Idle };
                        if needs && let Some(s) = self.session.as_mut() {
                            let undo = if savepoint { "ROLLBACK TO SAVEPOINT trunq_dry_run" } else { "ROLLBACK" };
                            s.run(vec![undo.to_string()], 0);
                        }
                        let affected = self.panes.last().and_then(|p| p.tag.clone()).unwrap_or_default();
                        if r.ok {
                            self.msg(
                                MsgKind::Ok,
                                format!("Dry run finished: {affected}, everything was rolled back, nothing changed. (Sequences may still have advanced.)"),
                            );
                        } else {
                            self.msg(MsgKind::Info, "Dry run failed, rolled back.".into());
                        }
                    }
                    if let Some(r) = self.run.take()
                        && !r.fetch
                    {
                        let sql = r.stmts.iter().filter(|s| s.1 != usize::MAX).map(|s| s.0.as_str()).collect::<Vec<_>>().join(";\n");
                        if self.table.is_none() {
                            let profile = self.profile(ctx).map(|p| p.display_name()).unwrap_or_default();
                            ctx.history.push(HistoryEntry {
                                sql,
                                profile,
                                database: self.database.clone(),
                                at: chrono::Utc::now().timestamp(),
                                ms: elapsed.as_millis() as u64,
                                ok: r.ok,
                                rows: Some(r.rows_total),
                            });
                        }
                        if r.stmts.len() > 1 {
                            self.msg(MsgKind::Info, format!("Script finished in {}", util::duration(elapsed)));
                        }
                        if self.panes.is_empty() && self.active_pane != usize::MAX {
                            self.active_pane = usize::MAX;
                        }
                    }
                }
                Event::Notices(ns) => {
                    for n in ns {
                        let mut text = format!("{}: {}", n.severity, n.message);
                        if let Some(d) = n.detail {
                            text.push_str(&format!("\nDETAIL: {d}"));
                        }
                        if let Some(h) = n.hint {
                            text.push_str(&format!("\nHINT: {h}"));
                        }
                        self.msg(MsgKind::Notice, text);
                    }
                }
                Event::Notifications(ns) => {
                    for n in ns {
                        self.msg(MsgKind::Notice, format!("NOTIFY {} (pid {}): {}", n.channel, n.pid, n.payload));
                    }
                }
                Event::Tx(_) => {}
                Event::Disconnected(reason) => {
                    self.msg(MsgKind::Error, format!("Disconnected: {reason}"));
                    self.run = None;
                    for p in &mut self.panes {
                        p.more = false;
                        p.fetching = false;
                    }
                }
            }
        }
    }

    // ---- UI ------------------------------------------------------------------------

    /// Fills in information that needs the catalog once it has loaded.
    fn apply_catalog(&mut self, cat: &Catalog) {
        for p in &mut self.panes {
            if p.fks.len() != p.rs.ncols() {
                // RowDescription tells us the source table/column of each result column.
                p.fks =
                    p.rs.columns
                        .iter()
                        .map(|c| {
                            let rel = cat.relation_by_oid(c.table_oid)?;
                            let col = rel.columns.iter().find(|x| x.attnum == c.column_id)?;
                            let fk = rel.fk_for(&col.name)?;
                            let target = cat.relation_by_oid(fk.ref_oid)?;
                            Some((target.schema.clone(), target.name.clone(), fk.ref_columns.first()?.clone()))
                        })
                        .collect();
            }
            if p.type_names.iter().any(|t| t.starts_with("oid ")) {
                p.type_names = p.rs.columns.iter().map(|c| cat.type_name(c.type_oid)).collect();
                p.cats = p.rs.columns.iter().map(|c| cat.type_cat(c.type_oid)).collect();
            }
        }
        if let Some(t) = &mut self.table
            && t.pk.is_empty()
            && let Some(rel) = cat.find_relation(Some(&t.schema), &t.name)
        {
            t.pk = rel.pk_columns().iter().map(|s| s.to_string()).collect();
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, ctx: &mut TabCtx) {
        let pal = theme::p();
        self.poll(ctx);
        if let Some(cat) = ctx.catalog.clone() {
            self.apply_catalog(&cat);
        }
        // live monitoring of long statements & forgotten transactions
        match (&self.run, &self.session) {
            (Some(r), Some(s)) if !r.fetch => self.monitor.tick(ctx.monitor.as_ref(), s.pid, r.started.elapsed()),
            _ => self.monitor.reset(),
        }
        match self.session.as_ref().map(|s| s.tx_status) {
            Some(TxStatus::InTransaction | TxStatus::Failed) => {
                self.tx_since.get_or_insert_with(Instant::now);
            }
            _ => self.tx_since = None,
        }
        self.apply_table_prefs(ctx);
        if self.pending_run && self.table.is_some() {
            self.pending_run = false;
            self.run_table_query(ctx, ui);
        }
        if let Some(t) = &mut self.table {
            if let Some(p) = &t.save
                && let Some(r) = p.take()
            {
                t.save = None;
                match r {
                    Ok(_) => {
                        ctx.actions.push(Action::Toast(ToastKind::Success, format!("Saved changes to {}", t.name)));
                        self.pending_run = true;
                    }
                    Err(e) => {
                        ctx.actions.push(Action::Toast(ToastKind::Error, "Saving failed, no changes were applied".into()));
                        self.messages.push(Msg { kind: MsgKind::Error, text: e, at: chrono::Local::now() });
                        self.active_pane = usize::MAX;
                    }
                }
            }
            if let Some(p) = &t.count
                && let Some(r) = p.take()
            {
                t.count = None;
                t.exact_count = Some(match r {
                    Ok(rs) => util::thousands(rs.s(0, 0).parse().unwrap_or(0)),
                    Err(e) => format!("error: {e}"),
                });
            }
        }

        // keyboard shortcuts (tab-level)
        let dry = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::ALT, egui::Key::Enter));
        if dry && self.table.is_none() {
            self.dry_run(ctx, ui);
        }
        let (run, run_all, explain, explain_an, fmt, cancel, save_file) = ui.input_mut(|i| {
            use egui::{Key, Modifiers};
            (
                i.consume_key(Modifiers::COMMAND, Key::Enter),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Enter),
                i.consume_key(Modifiers::COMMAND, Key::E),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::E),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F),
                i.consume_key(Modifiers::NONE, Key::Escape) && self.run.is_some(),
                i.consume_key(Modifiers::COMMAND, Key::S),
            )
        });
        if self.table.is_none() {
            if run_all {
                self.run_current(ctx, ui, true, None);
            } else if run {
                self.run_current(ctx, ui, false, None);
            } else if explain_an {
                self.run_current(ctx, ui, false, Some(true));
            } else if explain {
                self.run_current(ctx, ui, false, Some(false));
            }
            if fmt {
                self.format_sql(ctx.settings.uppercase_keywords);
            }
            if save_file {
                self.save_file(false);
            }
        } else if run || run_all {
            self.pending_run = true;
        }
        if cancel {
            self.cancel();
        }

        // color strip for the connection (e.g. red = production)
        if let Some(c) = self.profile(ctx).and_then(|p| p.color.or(p.environment.color())) {
            let r = ui.available_rect_before_wrap();
            ui.painter().hline(r.x_range(), r.top() + 1.0, egui::Stroke::new(2.0, theme::rgb(c)));
        }

        self.toolbar(ui, ctx);

        if let Some(s) = &self.session
            && s.needs_password
            && s.failed.is_some()
        {
            self.password_bar(ui, ctx);
        }

        if self.run.is_some() {
            self.monitor.ui(ui, ctx.monitor.as_ref());
        } else if let Some(since) = self.tx_since
            && let Some(commit) = super::monitor::open_tx_banner(ui, since)
        {
            let sql = if commit { "COMMIT" } else { "ROLLBACK" };
            self.start_run(ctx, ui, vec![(sql.into(), usize::MAX)], None);
        }

        let avail = ui.available_height();
        if self.table.is_some() {
            self.filter_bar(ui, ctx);
            self.results(ui, ctx);
        } else {
            let editor_h = (avail * self.split).clamp(40.0, (avail - 80.0).max(40.0));
            let catalog = ctx.catalog.clone();
            ui.allocate_ui(vec2(ui.available_width(), editor_h), |ui| {
                egui::Frame::NONE.fill(pal.bg0).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    editor::sql_editor(ui, &mut self.editor, catalog.as_deref(), ctx.settings.auto_complete, ctx.settings.uppercase_keywords, false, true);
                });
            });
            // splitter
            let (sr, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), egui::Sense::drag());
            ui.painter().hline(sr.x_range(), sr.center().y, egui::Stroke::new(1.0, if resp.hovered() || resp.dragged() { pal.accent } else { pal.border }));
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
            if resp.dragged() {
                self.split = ((editor_h + resp.drag_delta().y) / avail).clamp(0.12, 0.88);
            }
            self.results(ui, ctx);
        }

        // save-as-snippet prompt
        if let Some(mut name) = self.snippet_prompt.take() {
            let mut keep = true;
            egui::Modal::new(egui::Id::unique(("snippet", self.uid))).frame(w::modal_frame()).show(ui.ctx(), |ui| {
                ui.set_width(460.0);
                w::dialog_header(ui, icon::STAR, pal.accent, "Save snippet", "Reusable query, available from the command palette (⌘K)");
                let r = w::input(ui, &mut name, "Snippet name", Some(icon::TYPE), 0.0);
                r.request_focus();
                ui.add_space(8.0);
                w::hint(ui, ":name placeholders are asked for every time you run the snippet and sent as bound parameters.");
                w::dialog_footer(ui, |ui| {
                    let save = w::button(ui, w::Kind::Primary, icon::SAVE, "Save snippet").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if save && !name.trim().is_empty() {
                        let sql = self.editor.text.trim().to_string();
                        ctx.actions.push(Action::SaveSnippet { name: name.trim().to_string(), sql });
                        keep = false;
                    }
                    if w::button(ui, w::Kind::Ghost, "", "Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        keep = false;
                    }
                });
            });
            if keep {
                self.snippet_prompt = Some(name);
            }
        }

        // :name parameter prompt
        if let Some((stmts, explain, names)) = self.params_prompt.take() {
            let mut keep = true;
            let mut go = false;
            egui::Modal::new(egui::Id::unique(("params", self.uid))).frame(w::modal_frame()).show(ui.ctx(), |ui| {
                ui.set_width(480.0);
                w::dialog_header(ui, icon::BRACES, pal.accent, "Query parameters", "Sent as bound parameters, never pasted into the SQL");
                for (i, n) in names.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(130.0, w::H), egui::Sense::hover());
                        ui.painter().text(egui::pos2(r.left(), r.center().y), egui::Align2::LEFT_CENTER, format!(":{n}"), theme::mono(13.0), pal.param);
                        let v = self.param_values.entry(n.clone()).or_default();
                        let r = w::code_input(ui, v, "value or NULL", None, 0.0);
                        if i == 0 && !ui.memory(|m| m.focused().is_some()) {
                            r.request_focus();
                        }
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && i + 1 == names.len() {
                            go = true;
                        }
                    });
                    ui.add_space(6.0);
                }
                w::hint(ui, "PostgreSQL infers each type from the query. Type NULL for a null value.");
                w::dialog_footer(ui, |ui| {
                    if w::button(ui, w::Kind::Primary, icon::PLAY, "Run").clicked() {
                        go = true;
                    }
                    if w::button(ui, w::Kind::Ghost, "", "Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        keep = false;
                    }
                });
            });
            if go {
                // Bind as real parameters ($1, $2, …): values never become SQL text.
                let mut bound_stmts = Vec::new();
                let mut bound_params = Vec::new();
                for (sql, off) in stmts {
                    let (rewritten, order) = lexer::bind_params(&sql);
                    let values = order
                        .iter()
                        .map(|n| {
                            let v = self.param_values.get(n).cloned().unwrap_or_default();
                            if v.trim().eq_ignore_ascii_case("null") { None } else { Some(v) }
                        })
                        .collect();
                    bound_stmts.push((rewritten, off));
                    bound_params.push(values);
                }
                self.run_checked(ctx, ui, bound_stmts, bound_params, explain);
            } else if keep {
                self.params_prompt = Some((stmts, explain, names));
            }
        }

        // confirmation for dangerous / lock-heavy statements
        if let Some(c) = self.confirm.take() {
            let mut keep = true;
            let mut go = false;
            let mut dry = false;
            let mut fix: Option<(usize, String)> = None;
            egui::Modal::new(egui::Id::unique(("confirm", self.uid))).frame(w::modal_frame()).show(ui.ctx(), |ui| {
                ui.set_width(580.0);
                let (title, color) = if c.danger.is_some() { ("Are you sure?", pal.danger) } else { ("This may lock busy tables", pal.warn) };
                w::dialog_header(ui, icon::WARN, color, title, "Review before running");
                if let Some(d) = &c.danger {
                    w::callout(ui, pal.danger, icon::CIRCLE_X, d);
                    ui.add_space(8.0);
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for l in &c.lints {
                        let tip = l.title.starts_with("Tip");
                        egui::Frame::NONE
                            .fill(pal.bg2)
                            .stroke(egui::Stroke::new(1.0, pal.border))
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(format!("{}  {}", if tip { icon::INFO } else { icon::WARN }, l.title)).strong().color(if tip {
                                    pal.accent
                                } else {
                                    pal.warn
                                }));
                                ui.add_space(2.0);
                                ui.label(RichText::new(&l.why).color(pal.dim));
                                ui.add_space(4.0);
                                ui.label(RichText::new(format!("{}  {}", icon::WAND, l.fix)).color(pal.text));
                            });
                        ui.add_space(6.0);
                    }
                });
                // quick fix: add CONCURRENTLY
                for (i, (s, off)) in c.stmts.iter().enumerate() {
                    if let Some(fixed) = crate::sql::lint::add_concurrently(s)
                        && *off != usize::MAX
                        && w::button(ui, w::Kind::Secondary, icon::WAND, &format!("Add CONCURRENTLY to statement {}", i + 1)).clicked()
                    {
                        fix = Some((i, fixed));
                    }
                }
                w::dialog_footer(ui, |ui| {
                    if w::button(ui, w::Kind::Danger, icon::PLAY, "Run anyway").clicked() {
                        go = true;
                    }
                    let first = c.stmts.first().map(|s| s.0.trim_start().to_ascii_lowercase()).unwrap_or_default();
                    if c.stmts.len() == 1
                        && ["update", "delete", "insert", "merge"].iter().any(|k| first.starts_with(k))
                        && w::button(ui, w::Kind::Primary, icon::SHIELD, "Dry run instead").clicked()
                    {
                        dry = true;
                    }
                    if w::button(ui, w::Kind::Ghost, "", "Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        keep = false;
                    }
                });
            });
            if let Some((i, fixed)) = fix {
                let (orig, off) = &c.stmts[i];
                self.editor.replace_range(*off..*off + orig.len(), &fixed);
                self.msg(MsgKind::Info, "Added CONCURRENTLY, review and run again.".into());
            } else if go {
                self.start_run_params(ctx, ui, c.stmts, c.params, c.explain);
            } else if dry {
                let stmts = c.stmts;
                if let Some((s, off)) = stmts.into_iter().next() {
                    let in_tx = self.session.as_ref().is_some_and(|s| s.tx_status != TxStatus::Idle);
                    let (open, close) = if in_tx { ("SAVEPOINT trunq_dry_run", "ROLLBACK TO SAVEPOINT trunq_dry_run") } else { ("BEGIN", "ROLLBACK") };
                    let body = if s.to_ascii_lowercase().contains("returning") || s.trim_start().to_ascii_lowercase().starts_with("merge") {
                        s
                    } else {
                        format!("{s}\nRETURNING *")
                    };
                    self.start_run(ctx, ui, vec![(open.into(), usize::MAX), (body, off), (close.into(), usize::MAX)], None);
                    if let Some(r) = &mut self.run {
                        r.dry = Some(in_tx);
                    }
                }
            } else if keep {
                self.confirm = Some(c);
            }
        }
    }

    fn password_bar(&mut self, ui: &mut Ui, ctx: &mut TabCtx) {
        let pal = theme::p();
        egui::Frame::NONE.fill(pal.warn.gamma_multiply(0.10)).inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::KEY).size(15.0).color(pal.warn));
                ui.label(RichText::new("Password required for this connection").color(pal.text));
                let r = w::password_input(ui, &mut self.password_input, "Password", 240.0);
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (w::button(ui, w::Kind::Primary, icon::LOG_IN, "Connect").clicked() || enter)
                    && !self.password_input.is_empty()
                    && let Some(pid) = self.profile_id
                {
                    ctx.actions.push(Action::SetPassword(pid, std::mem::take(&mut self.password_input)));
                    self.session = None;
                    if self.table.is_some() {
                        self.pending_run = true;
                    }
                }
            });
        });
    }

    fn toolbar(&mut self, ui: &mut Ui, ctx: &mut TabCtx) {
        let pal = theme::p();
        let compact = ui.available_width() < 900.0;
        egui::Frame::NONE.inner_margin(egui::Margin { left: 12, right: 12, top: 8, bottom: 8 }).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let running = self.run.is_some();
                if self.table.is_none() {
                    if ui
                        .add_enabled_ui(!running, |ui| w::button(ui, w::Kind::Primary, icon::PLAY, "Run"))
                        .inner
                        .on_hover_text("Run the selection, or the statement under the cursor (⌘⏎)")
                        .clicked()
                    {
                        self.run_current(ctx, ui, false, None);
                    }
                    if ui
                        .add_enabled_ui(!running, |ui| w::button(ui, w::Kind::Secondary, icon::PLAY_CIRCLE, "Run all"))
                        .inner
                        .on_hover_text("Run the whole script (⇧⌘⏎)")
                        .clicked()
                    {
                        self.run_current(ctx, ui, true, None);
                    }
                } else {
                    if ui
                        .add_enabled_ui(!running, |ui| w::button(ui, w::Kind::Secondary, icon::REFRESH, "Refresh"))
                        .inner
                        .on_hover_text("Reload data (⌘⏎)")
                        .clicked()
                    {
                        self.pending_run = true;
                    }
                    // Query templates for this table
                    if let (Some(t), Some(pid)) = (&self.table, self.profile_id) {
                        let q = format!("{}.{}", quote_ident(&t.schema), quote_ident(&t.name));
                        let cols: Vec<String> = ctx
                            .catalog
                            .as_deref()
                            .and_then(|c| c.find_relation(Some(&t.schema), &t.name))
                            .map(|r| r.columns.iter().map(|c| quote_ident(&c.name)).collect())
                            .unwrap_or_default();
                        let pk: Vec<String> = t.pk.iter().map(|c| quote_ident(c)).collect();
                        let where_pk = if pk.is_empty() {
                            String::new()
                        } else {
                            pk.iter().map(|c| format!("{c} = :{}", c.trim_matches('"'))).collect::<Vec<_>>().join(" AND ")
                        };
                        let filter = if t.filter.trim().is_empty() { String::new() } else { format!("\nWHERE {}", t.filter.trim()) };
                        let db = self.database.clone();
                        w::menu_button(ui, w::Kind::Secondary, icon::WAND, "Templates", |ui| {
                            let list = [
                                (
                                    icon::LIST,
                                    "SELECT",
                                    format!("SELECT {}\nFROM {q}{filter}\nLIMIT 100;", if cols.is_empty() { "*".into() } else { cols.join(",\n       ") }),
                                ),
                                (
                                    icon::PLUS,
                                    "INSERT",
                                    format!(
                                        "INSERT INTO {q} ({})\nVALUES ({});",
                                        cols.join(", "),
                                        cols.iter().map(|c| format!(":{}", c.trim_matches('"'))).collect::<Vec<_>>().join(", ")
                                    ),
                                ),
                                (
                                    icon::SQUARE_PEN,
                                    "UPDATE",
                                    format!(
                                        "UPDATE {q}\nSET {}\nWHERE {where_pk};",
                                        cols.iter()
                                            .filter(|c| !pk.contains(c))
                                            .map(|c| format!("{c} = :{}", c.trim_matches('"')))
                                            .collect::<Vec<_>>()
                                            .join(",\n    ")
                                    ),
                                ),
                                (icon::TRASH, "DELETE", format!("DELETE FROM {q}\nWHERE {where_pk};")),
                                (icon::WORKFLOW, "EXPLAIN ANALYZE", format!("EXPLAIN (ANALYZE, BUFFERS)\nSELECT * FROM {q}{filter}\nLIMIT 100;")),
                                (icon::SIGMA, "COUNT", format!("SELECT count(*) FROM {q}{filter};")),
                            ];
                            for (ico, label, sql) in list {
                                if w::menu_item(ui, ico, label).clicked() {
                                    ctx.actions.push(Action::NewQuery { profile: pid, db: db.clone(), sql, run: false });
                                }
                            }
                        });
                    }
                }
                if running && w::button(ui, w::Kind::Danger, icon::STOP, "Cancel").on_hover_text("Cancel the running query (Esc)").clicked() {
                    self.cancel();
                }
                if self.table.is_none() {
                    w::menu_button(ui, w::Kind::Secondary, icon::WORKFLOW, "Explain", |ui| {
                        if w::menu_item_shortcut(ui, icon::WORKFLOW, "Explain", "⌘E").clicked() {
                            self.run_current(ctx, ui, false, Some(false));
                        }
                        if w::menu_item_shortcut(ui, icon::ZAP, "Explain Analyze (executes)", "⇧⌘E").clicked() {
                            self.run_current(ctx, ui, false, Some(true));
                        }
                    });
                    if ui
                        .add_enabled_ui(!running, |ui| w::button(ui, w::Kind::Secondary, icon::SHIELD, "Dry run"))
                        .inner
                        .on_hover_text("Run the INSERT / UPDATE / DELETE under the cursor in a transaction, show the affected rows, then roll back (⌥⌘⏎)")
                        .clicked()
                    {
                        self.dry_run(ctx, ui);
                    }
                    divider(ui);
                    if w::icon_button(ui, icon::ALIGN_LEFT, "Format SQL (⇧⌘F)").clicked() {
                        self.format_sql(ctx.settings.uppercase_keywords);
                    }
                    let resp = w::icon_button(ui, icon::ELLIPSIS, "More");
                    egui::Popup::menu(&resp).show(|ui| {
                        ui.set_min_width(240.0);
                        if w::menu_item(ui, icon::FOLDER_OPEN, "Open file…").clicked() {
                            self.open_file();
                        }
                        if w::menu_item_shortcut(ui, icon::SAVE, "Save", "⌘S").clicked() {
                            self.save_file(false);
                        }
                        if w::menu_item(ui, icon::SAVE, "Save as…").clicked() {
                            self.save_file(true);
                        }
                        ui.separator();
                        if w::menu_item(ui, icon::BOOK, "Save as snippet…").clicked() {
                            self.snippet_prompt = Some(self.title.clone());
                        }
                        ui.menu_button(format!("{}   Insert snippet", icon::SCROLL), |ui| {
                            ui.set_min_width(240.0);
                            if ctx.snippets.is_empty() {
                                ui.label(
                                    RichText::new("No snippets yet. Use “Save as snippet”.\n:name placeholders become bound parameters.")
                                        .small()
                                        .color(pal.faint),
                                );
                            }
                            for sn in ctx.snippets {
                                if w::menu_item(ui, icon::SCROLL, &sn.name).on_hover_text(&sn.sql).clicked() {
                                    let end = self.editor.text.len();
                                    let sep = if self.editor.text.trim().is_empty() { "" } else { "\n\n" };
                                    self.editor.replace_range(end..end, &format!("{sep}{}", sn.sql));
                                }
                            }
                        });
                        ui.separator();
                        if w::menu_item(ui, icon::HISTORY, "Query history").clicked() {
                            ctx.actions.push(Action::ShowHistory);
                        }
                    });
                }
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                // connection identity
                if let Some(p) = self.profile(ctx)
                    && let Some(c) = p.environment.color()
                {
                    ui.add_space(2.0);
                    w::badge(ui, p.environment.label(), theme::rgb(c));
                }
                let current = self.profile(ctx).map(|p| p.display_name()).unwrap_or_else(|| "Choose connection".into());
                let mut new_profile = None;
                if self.table.is_none() {
                    let resp = w::menu_button(ui, w::Kind::Secondary, icon::SERVER, &current, |ui| {
                        for p in ctx.profiles {
                            let r = w::menu_check(ui, Some(p.id) == self.profile_id, &p.display_name());
                            if r.clicked() {
                                new_profile = Some(p.clone());
                            }
                        }
                    });
                    resp.on_hover_text("Connection for this tab");
                    let mut db = self.database.clone();
                    let r = w::input(ui, &mut db, "database", Some(icon::DATABASE), if compact { 100.0 } else { 150.0 });
                    if r.changed() {
                        self.database = db;
                    }
                    if r.lost_focus() {
                        self.session = None;
                    }
                } else {
                    ui.label(RichText::new(format!("{}  {current}", icon::SERVER)).color(pal.text));
                    ui.label(RichText::new(format!("{}  {}", icon::DATABASE, self.database)).color(pal.dim));
                }
                if let Some(p) = new_profile {
                    self.profile_id = Some(p.id);
                    self.database = p.database.clone();
                    self.session = None;
                }

                // These controls wrap individually instead of overlapping the connection.
                {
                    if let Some(s) = &self.session {
                        if s.failed.is_some() {
                            w::pill(ui, &format!("{}  Disconnected", icon::WIFI_OFF), pal.danger).on_hover_text(s.failed.clone().unwrap_or_default());
                        } else if !s.connected {
                            w::pill(ui, "Connecting…", pal.dim);
                            ui.spinner();
                        } else {
                            match s.tx_status {
                                TxStatus::InTransaction | TxStatus::Failed => {
                                    let failed = s.tx_status == TxStatus::Failed;
                                    if w::button(ui, w::Kind::Secondary, icon::UNDO, "Rollback").clicked() {
                                        self.start_run(ctx, ui, vec![("ROLLBACK".into(), usize::MAX)], None);
                                    }
                                    if !failed && w::button(ui, w::Kind::Primary, icon::CHECK, "Commit").clicked() {
                                        self.start_run(ctx, ui, vec![("COMMIT".into(), usize::MAX)], None);
                                    }
                                    let (txt, col) = if failed { ("Transaction failed", pal.danger) } else { ("In transaction", pal.warn) };
                                    w::pill(ui, &format!("{}  {txt}", icon::HOURGLASS), col);
                                }
                                TxStatus::Idle => {
                                    w::pill(ui, &format!("{}  Connected", icon::CIRCLE_CHECK), pal.success)
                                        .on_hover_text(format!("PostgreSQL {} · backend pid {}", s.server_version, s.pid));
                                }
                            }
                        }
                    } else if self.idle_closed {
                        w::pill(ui, "Idle connection closed · reconnects on run", pal.faint);
                    }
                    if self.table.is_none() {
                        let mut manual = !self.autocommit;
                        if w::segmented(ui, &mut manual, &[(false, "", if compact { "Auto" } else { "Auto-commit" }), (true, "", "Manual")]) {
                            self.autocommit = !manual;
                        }
                    }
                    if let Some(r) = &self.run {
                        ui.label(RichText::new(format!("{}  {}", icon::TIMER, util::duration(r.started.elapsed()))).font(theme::mono(12.5)).color(pal.dim));
                        ui.spinner();
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    }
                }
            });
        });
        ui.painter().hline(ui.available_rect_before_wrap().x_range(), ui.cursor().top(), egui::Stroke::new(1.0, pal.border));
    }

    fn filter_bar(&mut self, ui: &mut Ui, ctx: &mut TabCtx) {
        let pal = theme::p();
        let mut run = false;
        let mut save = false;
        let mut discard = false;
        let mut add_row = false;
        let mut count = false;
        if let Some(t) = &mut self.table {
            egui::Frame::NONE.fill(pal.bg1).inner_margin(egui::Margin { left: 12, right: 12, top: 8, bottom: 8 }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    keyword_label(ui, "WHERE");
                    let wide = (ui.available_width() - 420.0).max(220.0);
                    let r = w::code_input(ui, &mut t.filter, "status = 'paid' AND total > 100", None, wide);
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        run = true;
                    }
                    ui.add_space(6.0);
                    keyword_label(ui, "ORDER BY");
                    let r = w::code_input(ui, &mut t.order, "placed_at DESC", None, 170.0);
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        run = true;
                    }
                    if w::button(ui, w::Kind::Secondary, icon::FILTER, "Apply").clicked() {
                        run = true;
                    }
                    if (!t.filter.is_empty() || !t.order.is_empty()) && w::icon_button(ui, icon::X, "Clear filter and order").clicked() {
                        t.filter.clear();
                        t.order.clear();
                        run = true;
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let editable = !t.pk.is_empty();
                    if editable {
                        if w::button(ui, w::Kind::Secondary, icon::PLUS, "Add row").clicked() {
                            add_row = true;
                        }
                        let n = t.edits.count();
                        if n > 0 {
                            if w::button(ui, w::Kind::Primary, icon::SAVE, &format!("Save {n} change{}", if n == 1 { "" } else { "s" })).clicked() {
                                save = true;
                            }
                            if w::button(ui, w::Kind::Ghost, icon::UNDO, "Discard").clicked() {
                                discard = true;
                            }
                            if w::button(ui, w::Kind::Ghost, icon::CODE, "Preview SQL").clicked() {
                                t.preview_sql = Some(edits_sql(t, self.panes.first()));
                            }
                        }
                        if t.save.is_some() {
                            ui.spinner();
                        }
                    } else {
                        w::pill(ui, &format!("{}  Read-only: this table has no primary key", icon::LOCK), pal.dim);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| match &t.exact_count {
                        Some(c) => {
                            w::pill(ui, &format!("{c} rows in total"), pal.accent);
                        }
                        None if t.count.is_some() => {
                            ui.spinner();
                        }
                        None => {
                            if w::button(ui, w::Kind::Ghost, icon::SIGMA, "Count rows").clicked() {
                                count = true;
                            }
                        }
                    });
                });
            });
            ui.painter().hline(ui.available_rect_before_wrap().x_range(), ui.cursor().top(), egui::Stroke::new(1.0, pal.border));
        }
        if run {
            self.pending_run = true;
        }
        if discard && let Some(t) = &mut self.table {
            t.edits = Edits::default();
        }
        if add_row && let (Some(t), Some(p)) = (&mut self.table, self.panes.first()) {
            t.edits.inserted.push(vec![None; p.rs.ncols()]);
        }
        if count {
            self.ensure_session(ctx, ui);
            if let (Some(t), Some(s)) = (&mut self.table, &self.session) {
                let mut sql = format!("SELECT count(*) FROM {}.{}", quote_ident(&t.schema), quote_ident(&t.name));
                if !t.filter.trim().is_empty() {
                    sql.push_str(&format!(" WHERE {}", t.filter));
                }
                t.count = Some(s.query(sql, vec![]));
            }
        }
        if save {
            self.ensure_session(ctx, ui);
            if let (Some(t), Some(s)) = (&mut self.table, &self.session) {
                let sql = edits_sql(t, self.panes.first());
                t.save = Some(s.batch(format!("BEGIN;\n{sql}\nCOMMIT;")));
            }
        }
        // SQL preview
        if let Some(t) = &mut self.table
            && let Some(sql) = t.preview_sql.clone()
        {
            let mut close = false;
            egui::Modal::new(egui::Id::unique(("preview", self.uid))).frame(w::modal_frame()).show(ui.ctx(), |ui| {
                ui.set_width(680.0);
                w::dialog_header(ui, icon::CODE, pal.accent, "Pending changes", "These statements run in one transaction when you press Save.");
                egui::Frame::NONE.fill(pal.bg0).corner_radius(8.0).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    egui::ScrollArea::both().max_height(360.0).show(ui, |ui| editor::code_view(ui, &sql, 12.5));
                });
                w::dialog_footer(ui, |ui| {
                    if w::button(ui, w::Kind::Primary, icon::CHECK, "Done").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                    if w::button(ui, w::Kind::Secondary, icon::COPY, "Copy").clicked() {
                        ui.ctx().copy_text(sql.clone());
                    }
                });
            });
            if close {
                t.preview_sql = None;
            }
        }
    }

    fn messages_view(&mut self, ui: &mut Ui) {
        let pal = theme::p();
        egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            ui.add_space(6.0);
            if self.messages.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Run a query with ⌘⏎, results appear here").color(pal.faint));
                });
            }
            for m in &self.messages {
                ui.horizontal_top(|ui| {
                    ui.add_space(8.0);
                    let (ico, col) = match m.kind {
                        MsgKind::Info => (icon::INFO, pal.dim),
                        MsgKind::Ok => (icon::CIRCLE_CHECK, pal.success),
                        MsgKind::Error => (icon::CIRCLE_X, pal.danger),
                        MsgKind::Notice => (icon::BELL, pal.warn),
                    };
                    ui.label(RichText::new(m.at.format("%H:%M:%S").to_string()).font(theme::mono(11.5)).color(pal.faint));
                    ui.label(RichText::new(ico).color(col));
                    let text = RichText::new(&m.text).font(theme::mono(12.5)).color(if m.kind == MsgKind::Error { pal.danger } else { pal.text });
                    ui.add(egui::Label::new(text).selectable(true).wrap());
                });
            }
            ui.add_space(8.0);
        });
    }

    fn format_sql(&mut self, upper: bool) {
        let formatted = format::format_sql(&self.editor.text, upper);
        let end = self.editor.text.len();
        self.editor.replace_range(0..end, formatted.trim_end());
    }

    pub fn open_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new().add_filter("SQL", &["sql", "txt", "psql"]).pick_file()
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            self.editor.set_text(text.clone());
            self.saved_text = text;
            self.title = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            self.file = Some(path);
        }
    }

    pub fn save_file(&mut self, as_new: bool) {
        let path = match (&self.file, as_new) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new().add_filter("SQL", &["sql"]).set_file_name(format!("{}.sql", self.title)).save_file(),
        };
        if let Some(path) = path
            && std::fs::write(&path, &self.editor.text).is_ok()
        {
            self.saved_text = self.editor.text.clone();
            self.title = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            self.file = Some(path);
        }
    }
}

/// Thin vertical divider between toolbar groups.
fn divider(ui: &mut Ui) {
    let pal = theme::p();
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 22.0), egui::Sense::hover());
    ui.painter().vline(r.center().x, r.y_range(), egui::Stroke::new(1.0, pal.border));
}

/// SQL keyword label in front of an input (WHERE, ORDER BY).
fn keyword_label(ui: &mut Ui, kw: &str) {
    let pal = theme::p();
    ui.label(RichText::new(kw).font(theme::mono(12.0)).strong().color(pal.kw));
}

/// `-- name: Failed payments` above a statement names its result tab.
fn result_name(sql: &str) -> Option<String> {
    for line in sql.lines() {
        let l = line.trim();
        if !l.starts_with("--") {
            break;
        }
        let body = l.trim_start_matches('-').trim();
        if let Some(rest) = body.strip_prefix("name:").or_else(|| body.strip_prefix("@name")) {
            let name = rest.trim();
            if !name.is_empty() {
                return Some(name.chars().take(40).collect());
            }
        }
    }
    None
}

/// Warns about statements that are probably mistakes.
fn dangerous(stmts: &[(String, usize)]) -> Option<String> {
    for (s, _) in stmts {
        let toks: Vec<String> = lexer::tokenize(s)
            .into_iter()
            .filter(|t| !matches!(t.kind, lexer::Tok::Whitespace | lexer::Tok::Comment))
            .map(|t| s[t.range].to_ascii_lowercase())
            .collect();
        let first = toks.first().map(String::as_str).unwrap_or("");
        let has_where = toks.iter().any(|t| t == "where");
        if (first == "delete" || first == "update") && !has_where {
            return Some(format!("This {} has no WHERE clause and will affect every row in the table.", first.to_uppercase()));
        }
        if first == "drop" && toks.get(1).is_some_and(|t| t == "database" || t == "schema" || t == "table") {
            return Some(format!("This will permanently DROP a {}.", toks[1].to_uppercase()));
        }
        if first == "truncate" {
            return Some("TRUNCATE removes all rows from the table.".into());
        }
    }
    None
}

/// Builds UPDATE / INSERT / DELETE statements for pending edits.
fn edits_sql(t: &TableMode, pane: Option<&Pane>) -> String {
    let Some(p) = pane else { return String::new() };
    let table = format!("{}.{}", quote_ident(&t.schema), quote_ident(&t.name));
    let lit = |v: &Option<String>, c: usize| -> String {
        match v {
            None => "NULL".into(),
            Some(s) => {
                let ty = p.type_names.get(c).cloned().unwrap_or_default();
                if ty.is_empty() || ty.starts_with("oid ") { quote_literal(s) } else { format!("{}::{}", quote_literal(s), ty) }
            }
        }
    };
    let pk_idx: Vec<usize> = t.pk.iter().filter_map(|c| p.rs.col_index(c)).collect();
    let where_pk = |r: usize| -> String {
        pk_idx
            .iter()
            .map(|&c| match p.rs.get(r, c) {
                Some(v) => format!("{} = {}", quote_ident(&p.rs.columns[c].name), lit(&Some(v.to_string()), c)),
                None => format!("{} IS NULL", quote_ident(&p.rs.columns[c].name)),
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    };
    let mut out = Vec::new();
    let mut by_row: std::collections::BTreeMap<usize, Vec<(usize, &Option<String>)>> = Default::default();
    for ((r, c), v) in &t.edits.cells {
        if !t.edits.deleted.contains(r) {
            by_row.entry(*r).or_default().push((*c, v));
        }
    }
    for (r, mut cells) in by_row {
        cells.sort_by_key(|x| x.0);
        let sets = cells.iter().map(|(c, v)| format!("{} = {}", quote_ident(&p.rs.columns[*c].name), lit(v, *c))).collect::<Vec<_>>().join(", ");
        out.push(format!("UPDATE {table} SET {sets} WHERE {};", where_pk(r)));
    }
    for row in &t.edits.inserted {
        let cols: Vec<usize> = (0..row.len()).filter(|&c| row[c].is_some()).collect();
        if cols.is_empty() {
            out.push(format!("INSERT INTO {table} DEFAULT VALUES;"));
        } else {
            let names = cols.iter().map(|&c| quote_ident(&p.rs.columns[c].name)).collect::<Vec<_>>().join(", ");
            let vals = cols.iter().map(|&c| lit(&row[c], c)).collect::<Vec<_>>().join(", ");
            out.push(format!("INSERT INTO {table} ({names}) VALUES ({vals});"));
        }
    }
    let mut deleted: Vec<_> = t.edits.deleted.iter().copied().collect();
    deleted.sort_unstable();
    for r in deleted {
        out.push(format!("DELETE FROM {table} WHERE {};", where_pk(r)));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn danger_detection() {
        assert!(dangerous(&[("delete from t".into(), 0)]).is_some());
        assert!(dangerous(&[("delete from t where id = 1".into(), 0)]).is_none());
        assert!(dangerous(&[("update t set a = 1 -- where".into(), 0)]).is_some());
        assert!(dangerous(&[("select 1".into(), 0)]).is_none());
    }
}
