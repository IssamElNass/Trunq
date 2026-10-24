//! Live insight into a running statement, polled over the server's existing
//! object-browser connection (no extra connection is opened):
//!
//! * which sessions block it (and one-click cancel / terminate of the blocker),
//! * what it is waiting on,
//! * progress of CREATE INDEX / VACUUM / ANALYZE / COPY / CLUSTER.

use std::time::{Duration, Instant};

use egui::{RichText, Ui};
use serde_json::Value;

use crate::db::session::{Promise, QueryHandle, QueryResult};
use crate::ui::theme::{self, icon};
use crate::ui::widgets as w;
use crate::util;

const POLL_EVERY: Duration = Duration::from_millis(1000);
/// Only start watching statements that run longer than this.
pub const WATCH_AFTER: Duration = Duration::from_millis(1500);

#[derive(Clone, Debug)]
pub struct Blocker {
    pub pid: i32,
    pub user: String,
    pub app: String,
    pub state: String,
    pub query: String,
    pub xact_secs: i64,
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub command: String,
    pub phase: String,
    pub fraction: Option<f32>,
    pub detail: String,
}

#[derive(Default)]
pub struct Monitor {
    last_poll: Option<Instant>,
    pending: Option<Promise<QueryResult>>,
    action: Option<Promise<QueryResult>>,
    pub wait: Option<String>,
    pub blockers: Vec<Blocker>,
    pub progress: Option<Progress>,
    /// Server too old for some progress views: fall back to the basic query.
    basic: bool,
}

const FULL_SQL: &str = r#"
SELECT a.wait_event_type, a.wait_event, a.state,
  (SELECT json_agg(json_build_object('pid', b.pid, 'user', b.usename, 'app', b.application_name, 'state', b.state,
                                     'query', left(b.query, 400), 'xact', extract(epoch FROM now() - b.xact_start)::bigint))
     FROM pg_stat_activity b WHERE b.pid = ANY (pg_blocking_pids(a.pid))),
  (SELECT json_build_object('cmd', p.command, 'phase', p.phase, 'done', p.blocks_done, 'total', p.blocks_total,
                            'tdone', p.tuples_done, 'ttotal', p.tuples_total)
     FROM pg_stat_progress_create_index p WHERE p.pid = a.pid),
  (SELECT json_build_object('cmd', 'VACUUM', 'phase', p.phase, 'done', p.heap_blks_scanned, 'total', p.heap_blks_total)
     FROM pg_stat_progress_vacuum p WHERE p.pid = a.pid),
  (SELECT json_build_object('cmd', 'ANALYZE', 'phase', p.phase, 'done', p.sample_blks_scanned, 'total', p.sample_blks_total)
     FROM pg_stat_progress_analyze p WHERE p.pid = a.pid),
  (SELECT json_build_object('cmd', p.command, 'phase', p.type, 'done', p.bytes_processed, 'total', p.bytes_total, 'tdone', p.tuples_processed)
     FROM pg_stat_progress_copy p WHERE p.pid = a.pid),
  (SELECT json_build_object('cmd', p.command, 'phase', p.phase, 'done', p.heap_blks_scanned, 'total', p.heap_blks_total)
     FROM pg_stat_progress_cluster p WHERE p.pid = a.pid)
FROM pg_stat_activity a WHERE a.pid = $1::int"#;

const BASIC_SQL: &str = r#"
SELECT a.wait_event_type, a.wait_event, a.state,
  (SELECT json_agg(json_build_object('pid', b.pid, 'user', b.usename, 'app', b.application_name, 'state', b.state,
                                     'query', left(b.query, 400), 'xact', extract(epoch FROM now() - b.xact_start)::bigint))
     FROM pg_stat_activity b WHERE b.pid = ANY (pg_blocking_pids(a.pid)))
FROM pg_stat_activity a WHERE a.pid = $1::int"#;

impl Monitor {
    pub fn reset(&mut self) {
        let basic = self.basic;
        *self = Monitor { basic, ..Default::default() };
    }

    pub fn is_interesting(&self) -> bool {
        !self.blockers.is_empty() || self.progress.is_some() || self.wait.is_some()
    }

    /// Call every frame while a statement runs on backend `pid`.
    pub fn tick(&mut self, handle: Option<&QueryHandle>, pid: i32, running_for: Duration) {
        if let Some(p) = &self.pending
            && let Some(r) = p.take()
        {
            self.pending = None;
            match r {
                Ok(rs) if rs.nrows() > 0 => self.parse(&rs),
                Ok(_) => {}
                Err(e) => {
                    if !self.basic && (e.contains("does not exist") || e.contains("column")) {
                        self.basic = true;
                    }
                }
            }
        }
        if let Some(a) = &self.action
            && a.take().is_some()
        {
            self.action = None;
            self.last_poll = None;
        }
        let (Some(h), true) = (handle, pid > 0 && running_for >= WATCH_AFTER) else { return };
        if self.pending.is_some() || self.last_poll.is_some_and(|t| t.elapsed() < POLL_EVERY) {
            return;
        }
        self.last_poll = Some(Instant::now());
        self.pending = h.query(if self.basic { BASIC_SQL } else { FULL_SQL }, vec![Some(pid.to_string())]);
    }

    fn parse(&mut self, rs: &crate::db::result::ResultSet) {
        let wt = rs.s(0, 0);
        let we = rs.s(0, 1);
        self.wait = match (wt, we) {
            ("", _) | (_, "") => None,
            ("Lock", e) => Some(format!("waiting for a {e} lock")),
            ("IO", e) => Some(format!("waiting on disk I/O ({e})")),
            ("Client", e) => Some(format!("waiting on the client ({e})")),
            ("IPC", e) => Some(format!("waiting on parallel workers / IPC ({e})")),
            ("LWLock", e) => Some(format!("waiting on internal lock {e}")),
            (t, e) => Some(format!("waiting: {t} / {e}")),
        };
        self.blockers = rs
            .get(0, 3)
            .and_then(|j| serde_json::from_str::<Vec<Value>>(j).ok())
            .unwrap_or_default()
            .iter()
            .map(|b| Blocker {
                pid: b["pid"].as_i64().unwrap_or(0) as i32,
                user: b["user"].as_str().unwrap_or("").to_string(),
                app: b["app"].as_str().unwrap_or("").to_string(),
                state: b["state"].as_str().unwrap_or("").to_string(),
                query: b["query"].as_str().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" "),
                xact_secs: b["xact"].as_i64().unwrap_or(0),
            })
            .collect();
        self.progress = None;
        for c in 4..rs.ncols() {
            let Some(v) = rs.get(0, c).and_then(|j| serde_json::from_str::<Value>(j).ok()) else { continue };
            let done = v["done"].as_f64().unwrap_or(0.0);
            let total = v["total"].as_f64().unwrap_or(0.0);
            let fraction = (total > 0.0).then(|| (done / total).clamp(0.0, 1.0) as f32);
            let mut detail = String::new();
            if let Some(t) = v["tdone"].as_f64() {
                let tt = v["ttotal"].as_f64().unwrap_or(0.0);
                detail = if tt > 0.0 {
                    format!("{} / {} tuples", util::thousands(t as usize), util::thousands(tt as usize))
                } else {
                    format!("{} tuples", util::thousands(t as usize))
                };
            }
            self.progress =
                Some(Progress { command: v["cmd"].as_str().unwrap_or("").to_string(), phase: v["phase"].as_str().unwrap_or("").to_string(), fraction, detail });
            break;
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, handle: Option<&QueryHandle>) {
        let pal = theme::p();
        if !self.is_interesting() {
            return;
        }
        let blocked = !self.blockers.is_empty();
        let tint = if blocked { pal.warn } else { pal.accent };
        egui::Frame::NONE.fill(tint.gamma_multiply(0.10)).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(p) = &self.progress {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  {}", icon::LOADER, p.command)).strong().color(pal.accent));
                    ui.label(RichText::new(&p.phase).color(pal.dim));
                    if let Some(f) = p.fraction {
                        ui.add(egui::ProgressBar::new(f).desired_width(260.0).show_percentage());
                    }
                    if !p.detail.is_empty() {
                        ui.label(RichText::new(&p.detail).small().color(pal.faint));
                    }
                });
            }
            if blocked {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}  Blocked", icon::LOCK)).strong().color(pal.warn));
                    ui.label(RichText::new(self.wait.clone().unwrap_or_default()).color(pal.dim));
                    ui.label(RichText::new("Your statement can't continue until these sessions finish:").color(pal.dim));
                });
                for b in self.blockers.clone() {
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        if let Some(h) = handle {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            if w::small_button(ui, w::Kind::Secondary, icon::STOP, "Cancel its query")
                                .on_hover_text("pg_cancel_backend, stops the blocker's current statement")
                                .clicked()
                            {
                                self.action = h.query("SELECT pg_cancel_backend($1::int)", vec![Some(b.pid.to_string())]);
                            }
                            if w::small_button(ui, w::Kind::Danger, icon::POWER, "Terminate session")
                                .on_hover_text("pg_terminate_backend, disconnects the blocker and rolls back its transaction")
                                .clicked()
                            {
                                self.action = h.query("SELECT pg_terminate_backend($1::int)", vec![Some(b.pid.to_string())]);
                            }
                        }
                        ui.label(RichText::new(format!("pid {}", b.pid)).font(theme::mono(12.0)).color(pal.text));
                        ui.label(RichText::new(format!("{} · {} · {}", b.user, if b.app.is_empty() { "?" } else { &b.app }, b.state)).small().color(pal.dim));
                        if b.xact_secs > 0 {
                            ui.label(
                                RichText::new(format!("transaction open {}", util::duration(Duration::from_secs(b.xact_secs as u64)))).small().color(pal.warn),
                            );
                        }
                        ui.add(egui::Label::new(RichText::new(&b.query).font(theme::mono(11.5)).color(pal.faint)).truncate()).on_hover_text(&b.query);
                    });
                }
            } else if let (Some(wait), None) = (&self.wait, &self.progress) {
                ui.label(RichText::new(format!("{}  {wait}", icon::HOURGLASS)).color(pal.dim));
            }
        });
    }
}

/// Banner for a transaction left open in a tab (it keeps holding locks).
pub fn open_tx_banner(ui: &mut Ui, since: Instant) -> Option<bool> {
    let pal = theme::p();
    let age = since.elapsed();
    if age < Duration::from_secs(60) {
        return None;
    }
    let mut out = None;
    egui::Frame::NONE.fill(pal.warn.gamma_multiply(0.10)).inner_margin(egui::Margin::symmetric(14, 7)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{}  Transaction open for {}", icon::HOURGLASS, util::duration(age))).strong().color(pal.warn));
            ui.label(RichText::new("Locks are held and vacuum is held back until you commit or roll back.").color(pal.dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if w::small_button(ui, w::Kind::Secondary, icon::UNDO, "Rollback").clicked() {
                    out = Some(false);
                }
                if w::small_button(ui, w::Kind::Primary, icon::CHECK, "Commit").clicked() {
                    out = Some(true);
                }
            });
        });
    });
    ui.ctx().request_repaint_after(Duration::from_secs(1));
    out
}
