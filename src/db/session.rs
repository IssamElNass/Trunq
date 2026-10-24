//! A `Session` is one live connection owned by a UI component (a query tab,
//! the object browser of a database, the dashboard...). The connection runs
//! as an async actor on the shared tokio runtime; the UI talks to it through
//! a command channel and polls events each frame, so the UI never blocks.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::mpsc as tmpsc;

use super::result::{Column, ResultSet};
use super::wire::{CancelToken, DbError, Notice, Notification, PgConn, StepOutcome, TxStatus};
use crate::config::Profile;

static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();

pub fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().thread_name("trunq-io").build().expect("tokio runtime"))
}

/// Every connection Trunq has open, for the "Connections" panel and the
/// status bar counter. Entries are removed by an RAII guard inside the
/// connection actor, so a connection can never outlive its registry entry.
pub struct LiveConn {
    pub key: u64,
    pub profile: String,
    pub database: String,
    pub purpose: String,
    pub pid: i32,
    pub since: Instant,
    pub busy: Arc<AtomicBool>,
    kill: Arc<tokio::sync::Notify>,
}

impl LiveConn {
    /// Force-closes the connection (its owner sees a "disconnected" state).
    pub fn kill(&self) {
        self.kill.notify_one();
    }
}

pub static LIVE: LazyLock<Mutex<Vec<LiveConn>>> = LazyLock::new(Default::default);
static NEXT_KEY: AtomicU64 = AtomicU64::new(1);

pub fn live_count() -> usize {
    LIVE.lock().len()
}

/// Closes every connection; used on application exit.
pub fn kill_all() {
    for c in LIVE.lock().iter() {
        c.kill();
    }
}

struct LiveGuard(u64);

impl Drop for LiveGuard {
    fn drop(&mut self) {
        LIVE.lock().retain(|c| c.key != self.0);
    }
}

/// A value filled in asynchronously; poll it from the UI with [`Promise::ready`].
pub struct Promise<T>(Arc<Mutex<Option<T>>>);

impl<T> Clone for Promise<T> {
    fn clone(&self) -> Self {
        Promise(self.0.clone())
    }
}

impl<T> Default for Promise<T> {
    fn default() -> Self {
        Promise(Arc::new(Mutex::new(None)))
    }
}

impl<T> Promise<T> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self, v: T) {
        *self.0.lock() = Some(v);
    }
    pub fn is_ready(&self) -> bool {
        self.0.lock().is_some()
    }
    pub fn take(&self) -> Option<T> {
        self.0.lock().take()
    }
}

pub type QueryResult = Result<ResultSet, String>;

pub enum Cmd {
    /// Run statements in order (stops at the first error).
    /// Each statement has its own bound parameters (`$1`, `$2`, … as text).
    Run { id: u64, statements: Vec<(String, Vec<Option<String>>)>, page_size: usize },
    /// Fetch more rows from the open cursor; `rows == 0` fetches everything.
    FetchMore { id: u64, rows: usize },
    /// Run one parameterised statement to completion.
    Query { sql: String, params: Vec<Option<String>>, reply: Promise<QueryResult> },
    /// Run SQL through the simple protocol (multiple statements allowed).
    Batch { sql: String, reply: Promise<Result<Vec<ResultSet>, String>> },
    /// Re-runs `sql` and streams every row to a file (export the entire result).
    Export {
        sql: String,
        params: Vec<Option<String>>,
        path: std::path::PathBuf,
        format: crate::export::Format,
        cats: Vec<crate::db::types::Cat>,
        table: String,
        progress: Arc<AtomicU64>,
        cancel: Arc<AtomicBool>,
        reply: Promise<Result<u64, String>>,
    },
    /// Atomically: run `setup` (e.g. CREATE TABLE), stream `path` into the
    /// COPY statement, commit. Everything is rolled back on any error.
    Import {
        setup: String,
        copy_sql: String,
        path: std::path::PathBuf,
        progress: Arc<AtomicU64>,
        cancel: Arc<AtomicBool>,
        reply: Promise<Result<String, String>>,
    },
}

#[derive(Debug)]
pub enum Event {
    Connected {
        pid: i32,
        server_version: String,
    },
    ConnectFailed {
        error: String,
        needs_password: bool,
    },
    StmtStart {
        id: u64,
        index: usize,
    },
    /// `elapsed`: time until the server returned the first rows.
    Columns {
        id: u64,
        index: usize,
        columns: Vec<Column>,
        elapsed: Duration,
    },
    Rows {
        id: u64,
        index: usize,
        rows: ResultSet,
    },
    StmtDone {
        id: u64,
        index: usize,
        tag: Option<String>,
        more: bool,
        elapsed: Duration,
    },
    StmtError {
        id: u64,
        index: usize,
        error: DbError,
    },
    RunFinished {
        id: u64,
        elapsed: Duration,
    },
    Notices(Vec<Notice>),
    Notifications(Vec<Notification>),
    Tx(TxStatus),
    Disconnected(String),
}

pub struct Session {
    tx: tmpsc::UnboundedSender<Cmd>,
    rx: mpsc::Receiver<Event>,
    cancel: Arc<Mutex<Option<CancelToken>>>,
    busy: Arc<AtomicBool>,
    pub connected: bool,
    pub failed: Option<String>,
    pub needs_password: bool,
    pub pid: i32,
    pub server_version: String,
    pub tx_status: TxStatus,
    pub database: String,
    pub profile_id: u64,
    next_id: u64,
}

/// A non-owning handle for sending ad-hoc queries over an existing session.
/// It holds a *weak* sender, so it never keeps a connection alive on its own.
#[derive(Clone)]
pub struct QueryHandle(tmpsc::WeakUnboundedSender<Cmd>);

impl QueryHandle {
    pub fn query(&self, sql: impl Into<String>, params: Vec<Option<String>>) -> Option<Promise<QueryResult>> {
        let tx = self.0.upgrade()?;
        let reply = Promise::new();
        tx.send(Cmd::Query { sql: sql.into(), params, reply: reply.clone() }).ok()?;
        Some(reply)
    }
}

impl Drop for Session {
    /// Dropping the handle closes the command channel, which ends the actor and
    /// terminates the connection. A running statement is cancelled first so
    /// the server does not keep working for nobody.
    fn drop(&mut self) {
        if self.is_busy() {
            self.cancel();
        }
    }
}

impl Session {
    /// `purpose` shows up in `pg_stat_activity.application_name` and in the
    /// Connections panel, so every backend can be traced back to its owner.
    pub fn connect(profile: &Profile, database: &str, purpose: &str, session_password: Option<String>, ctx: &egui::Context) -> Session {
        let (cmd_tx, mut cmd_rx) = tmpsc::unbounded_channel::<Cmd>();
        let (ev_tx, ev_rx) = mpsc::channel::<Event>();
        let cancel = Arc::new(Mutex::new(None));
        let busy = Arc::new(AtomicBool::new(true));
        let profile_id = profile.id;
        let profile = profile.clone();
        let db = database.to_string();
        let ctx = ctx.clone();
        let cancel2 = cancel.clone();
        let busy2 = busy.clone();
        let purpose = purpose.to_string();
        runtime().spawn(async move {
            let send = |e: Event| {
                let _ = ev_tx.send(e);
                ctx.request_repaint();
            };
            let pw = session_password.clone();
            let prof = profile.clone();
            let dbn = db.clone();
            let params = tokio::task::spawn_blocking(move || prof.connect_params(&dbn, pw.as_deref())).await;
            let mut params = match params {
                Ok(Ok(p)) => p,
                Ok(Err(e)) => {
                    busy2.store(false, Ordering::SeqCst);
                    send(Event::ConnectFailed { error: format!("{e:#}"), needs_password: false });
                    return;
                }
                Err(e) => {
                    busy2.store(false, Ordering::SeqCst);
                    send(Event::ConnectFailed { error: e.to_string(), needs_password: false });
                    return;
                }
            };
            params.application_name = format!("Trunq: {purpose}");
            if purpose.starts_with("Object browser") {
                // Background metadata work must never hang behind another
                // session's locks or run away on a huge catalog.
                let guard = "-c lock_timeout=5s -c statement_timeout=60s -c idle_in_transaction_session_timeout=60s";
                params.options = Some(match params.options.take() {
                    Some(o) => format!("{o} {guard}"),
                    None => guard.to_string(),
                });
            }
            let needs_pw_hint = params.password.is_none();
            let mut conn = match PgConn::connect(&params).await {
                Ok(c) => c,
                Err(e) => {
                    busy2.store(false, Ordering::SeqCst);
                    let msg = format!("{e:#}");
                    let needs_password = msg.contains("no password supplied")
                        || (needs_pw_hint && msg.contains("password authentication failed"))
                        || e.downcast_ref::<DbError>().is_some_and(|d| d.code == "28P01" && !profile.is_azure());
                    send(Event::ConnectFailed { error: msg, needs_password });
                    return;
                }
            };
            *cancel2.lock() = Some(conn.cancel_token());
            let kill = Arc::new(tokio::sync::Notify::new());
            let key = NEXT_KEY.fetch_add(1, Ordering::SeqCst);
            LIVE.lock().push(LiveConn {
                key,
                profile: profile.display_name(),
                database: db.clone(),
                purpose: purpose.clone(),
                pid: conn.backend_pid(),
                since: Instant::now(),
                busy: busy2.clone(),
                kill: kill.clone(),
            });
            let _guard = LiveGuard(key);
            busy2.store(false, Ordering::SeqCst);
            send(Event::Connected { pid: conn.backend_pid(), server_version: conn.server_version() });

            // Remember a manually typed password for the rest of the session.
            if let Some(pw) = &session_password
                && profile.save_password
                && !profile.is_azure()
            {
                profile.store_password(pw);
            }

            loop {
                let cmd = tokio::select! {
                    cmd = cmd_rx.recv() => match cmd {
                        Some(c) => c,
                        None => break, // owner dropped the Session
                    },
                    _ = kill.notified() => {
                        send(Event::Disconnected("connection closed from the Connections panel".into()));
                        break;
                    }
                };
                busy2.store(true, Ordering::SeqCst);
                let res = tokio::select! {
                    r = handle(&mut conn, cmd, &send) => r,
                    _ = kill.notified() => {
                        let _ = conn.cancel_token().cancel().await;
                        busy2.store(false, Ordering::SeqCst);
                        send(Event::Disconnected("connection closed from the Connections panel".into()));
                        return; // dropping `conn` closes the socket
                    }
                };
                if !conn.notices.is_empty() {
                    send(Event::Notices(std::mem::take(&mut conn.notices)));
                }
                if !conn.notifications.is_empty() {
                    send(Event::Notifications(std::mem::take(&mut conn.notifications)));
                }
                send(Event::Tx(conn.tx_status));
                busy2.store(false, Ordering::SeqCst);
                if let Err(e) = res {
                    send(Event::Disconnected(format!("{e:#}")));
                    return;
                }
            }
            conn.terminate().await;
        });
        Session {
            tx: cmd_tx,
            rx: ev_rx,
            cancel,
            busy,
            connected: false,
            failed: None,
            needs_password: false,
            pid: 0,
            server_version: String::new(),
            tx_status: TxStatus::Idle,
            database: database.to_string(),
            profile_id,
            next_id: 1,
        }
    }

    /// Drains pending events, updating connection bookkeeping.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(e) = self.rx.try_recv() {
            match &e {
                Event::Connected { pid, server_version } => {
                    self.connected = true;
                    self.pid = *pid;
                    self.server_version = server_version.clone();
                }
                Event::ConnectFailed { error, needs_password } => {
                    self.failed = Some(error.clone());
                    self.needs_password = *needs_password;
                }
                Event::Disconnected(msg) => {
                    self.connected = false;
                    self.failed = Some(msg.clone());
                }
                Event::Tx(s) => self.tx_status = *s,
                _ => {}
            }
            out.push(e);
        }
        out
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    pub fn is_alive(&self) -> bool {
        self.failed.is_none()
    }

    pub fn run(&mut self, statements: Vec<String>, page_size: usize) -> u64 {
        self.run_params(statements.into_iter().map(|s| (s, Vec::new())).collect(), page_size)
    }

    /// Runs statements with bound parameters (never string-interpolated).
    pub fn run_params(&mut self, statements: Vec<(String, Vec<Option<String>>)>, page_size: usize) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let _ = self.tx.send(Cmd::Run { id, statements, page_size });
        id
    }

    pub fn fetch_more(&self, id: u64, rows: usize) {
        let _ = self.tx.send(Cmd::FetchMore { id, rows });
    }

    pub fn query(&self, sql: impl Into<String>, params: Vec<Option<String>>) -> Promise<QueryResult> {
        let reply = Promise::new();
        let _ = self.tx.send(Cmd::Query { sql: sql.into(), params, reply: reply.clone() });
        reply
    }

    pub fn handle(&self) -> QueryHandle {
        QueryHandle(self.tx.downgrade())
    }

    /// Re-runs a query on this session and streams all rows to `path`.
    #[allow(clippy::too_many_arguments)]
    pub fn export(
        &self,
        sql: String,
        params: Vec<Option<String>>,
        path: std::path::PathBuf,
        format: crate::export::Format,
        cats: Vec<crate::db::types::Cat>,
        table: String,
        progress: Arc<AtomicU64>,
        cancel: Arc<AtomicBool>,
    ) -> Promise<Result<u64, String>> {
        let reply = Promise::new();
        let _ = self.tx.send(Cmd::Export { sql, params, path, format, cats, table, progress, cancel, reply: reply.clone() });
        reply
    }

    pub fn import(
        &self,
        setup: String,
        copy_sql: String,
        path: std::path::PathBuf,
        progress: Arc<AtomicU64>,
        cancel: Arc<AtomicBool>,
    ) -> Promise<Result<String, String>> {
        let reply = Promise::new();
        let _ = self.tx.send(Cmd::Import { setup, copy_sql, path, progress, cancel, reply: reply.clone() });
        reply
    }

    pub fn batch(&self, sql: impl Into<String>) -> Promise<Result<Vec<ResultSet>, String>> {
        let reply = Promise::new();
        let _ = self.tx.send(Cmd::Batch { sql: sql.into(), reply: reply.clone() });
        reply
    }

    /// Sends a cancel request for whatever is currently running.
    pub fn cancel(&self) {
        if let Some(tok) = self.cancel.lock().clone() {
            runtime().spawn(async move {
                let _ = tok.cancel().await;
            });
        }
    }
}

async fn handle(conn: &mut PgConn, cmd: Cmd, send: &impl Fn(Event)) -> anyhow::Result<()> {
    match cmd {
        Cmd::Run { id, statements, page_size } => {
            let started = Instant::now();
            let n = statements.len();
            for (index, (sql, params)) in statements.into_iter().enumerate() {
                let is_last = index + 1 == n;
                send(Event::StmtStart { id, index });
                let t0 = Instant::now();
                let (cols, res) = conn.start(&sql, &params, page_size).await?;
                let has_columns = cols.is_some();
                if let Some(cols) = cols {
                    send(Event::Columns { id, index, columns: cols, elapsed: t0.elapsed() });
                }
                match res {
                    Ok((rows, outcome)) => {
                        // COPY TO STDOUT supplies its columns while reading rows.
                        if !has_columns && !rows.columns.is_empty() {
                            send(Event::Columns { id, index, columns: rows.columns.as_ref().clone(), elapsed: t0.elapsed() });
                        }
                        if !rows.is_empty() {
                            send(Event::Rows { id, index, rows });
                        }
                        let (tag, more) = match outcome {
                            StepOutcome::Done { tag } => (Some(tag), false),
                            StepOutcome::Empty => (None, false),
                            StepOutcome::Suspended => (None, true),
                        };
                        if more && !is_last {
                            conn.close_portal().await?;
                        }
                        send(Event::StmtDone { id, index, tag, more: more && is_last, elapsed: t0.elapsed() });
                    }
                    Err(error) => {
                        send(Event::StmtError { id, index, error });
                        break;
                    }
                }
                if !conn.notices.is_empty() {
                    send(Event::Notices(std::mem::take(&mut conn.notices)));
                }
            }
            send(Event::RunFinished { id, elapsed: started.elapsed() });
        }
        Cmd::FetchMore { id, rows } => {
            if !conn.portal_open() {
                return Ok(());
            }
            let t0 = Instant::now();
            // Fetch in chunks so the UI can show progress on "fetch all".
            let chunk = if rows == 0 { 50_000 } else { rows };
            let mut remaining = rows;
            loop {
                let want = if rows == 0 { chunk } else { remaining.min(chunk) };
                match conn.fetch(0, want).await? {
                    Ok((rs, outcome)) => {
                        let got = rs.nrows();
                        if got > 0 {
                            send(Event::Rows { id, index: usize::MAX, rows: rs });
                        }
                        match outcome {
                            StepOutcome::Suspended => {
                                if rows != 0 {
                                    remaining = remaining.saturating_sub(got);
                                    if remaining == 0 {
                                        send(Event::StmtDone { id, index: usize::MAX, tag: None, more: true, elapsed: t0.elapsed() });
                                        break;
                                    }
                                }
                            }
                            StepOutcome::Done { tag } => {
                                send(Event::StmtDone { id, index: usize::MAX, tag: Some(tag), more: false, elapsed: t0.elapsed() });
                                break;
                            }
                            StepOutcome::Empty => {
                                send(Event::StmtDone { id, index: usize::MAX, tag: None, more: false, elapsed: t0.elapsed() });
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        send(Event::StmtError { id, index: usize::MAX, error });
                        break;
                    }
                }
            }
        }
        Cmd::Query { sql, params, reply } => {
            let r = conn.query_params(&sql, &params).await;
            match r {
                Ok(rs) => reply.set(Ok(rs)),
                Err(e) => {
                    let msg = format!("{e:#}");
                    let fatal = e.downcast_ref::<DbError>().is_none();
                    reply.set(Err(msg));
                    if fatal {
                        return Err(e);
                    }
                }
            }
        }
        Cmd::Export { sql, params, path, format, cats, table, progress, cancel, reply } => {
            let result: anyhow::Result<u64> = async {
                let mut w = crate::export::StreamWriter::create(&path, format, &table)?;
                let (cols, first) = conn.start(&sql, &params, 10_000).await?;
                let header = ResultSet::new(cols.unwrap_or_default());
                let (mut rs, mut outcome) = first.map_err(anyhow::Error::from)?;
                loop {
                    rs.columns = header.columns.clone();
                    let rows: Vec<usize> = (0..rs.nrows()).collect();
                    w.write(&rs, &rows, &cats)?;
                    progress.store(w.rows(), Ordering::Relaxed);
                    if !matches!(outcome, StepOutcome::Suspended) {
                        break;
                    }
                    if cancel.load(Ordering::Relaxed) {
                        conn.close_portal().await?;
                        anyhow::bail!("export cancelled");
                    }
                    let (next, o) = conn.fetch(header.ncols(), 10_000).await?.map_err(anyhow::Error::from)?;
                    rs = next;
                    outcome = o;
                }
                Ok(w.finish(Some(&header), &cats)?)
            }
            .await;
            match result {
                Ok(n) => reply.set(Ok(n)),
                Err(e) => {
                    let _ = std::fs::remove_file(&path);
                    reply.set(Err(format!("{e:#}")));
                }
            }
        }
        Cmd::Import { setup, copy_sql, path, progress, cancel, reply } => {
            let result: anyhow::Result<String> = async {
                conn.simple_query("BEGIN").await?;
                if !setup.trim().is_empty() {
                    conn.simple_query(&setup).await?;
                }
                let file = tokio::fs::File::open(&path).await?;
                let tag = conn.copy_in(&copy_sql, tokio::io::BufReader::new(file), &progress, &cancel).await?;
                conn.simple_query("COMMIT").await?;
                Ok(tag)
            }
            .await;
            match result {
                Ok(tag) => reply.set(Ok(tag)),
                Err(e) => {
                    let _ = conn.simple_query("ROLLBACK").await;
                    let fatal = e.downcast_ref::<DbError>().is_none() && !format!("{e}").contains("cancelled") && !format!("{e:#}").contains("could not read");
                    reply.set(Err(format!("{e:#}")));
                    if fatal && conn.tx_status == TxStatus::Failed {
                        return Err(e);
                    }
                }
            }
        }
        Cmd::Batch { sql, reply } => match conn.simple_query(&sql).await {
            Ok(rs) => reply.set(Ok(rs)),
            Err(e) => {
                let fatal = e.downcast_ref::<DbError>().is_none();
                reply.set(Err(format!("{e:#}")));
                if fatal {
                    return Err(e);
                }
            }
        },
    }
    Ok(())
}
