//! Active sessions: a compact, auto-refreshing view of pg_stat_activity
//! (within the user's privileges) with cancel / terminate. Polls only while
//! visible, over the server's existing browser connection.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use egui::{RichText, Sense, Stroke, Ui, pos2, vec2};

use crate::db::result::ResultSet;
use crate::db::session::{Promise, QueryHandle, QueryResult};
use crate::ui::theme::{self, icon};
use crate::ui::widgets as w;
use crate::util;

const SQL: &str = r#"
SELECT pid, usename, datname, application_name, coalesce(client_addr::text, 'local'), coalesce(state, backend_type),
       wait_event_type, wait_event,
       extract(epoch FROM now() - query_start)::float8,
       extract(epoch FROM now() - xact_start)::float8,
       left(query, 2000), cardinality(pg_blocking_pids(pid)), backend_type
FROM pg_stat_activity
WHERE pid <> pg_backend_pid()
ORDER BY (state = 'idle') NULLS LAST, query_start NULLS LAST"#;

pub struct SessionsTab {
    pub profile_id: u64,
    pub title: String,
    data: Option<ResultSet>,
    error: Option<String>,
    pending: Option<Promise<QueryResult>>,
    action: Option<Promise<QueryResult>>,
    last: Option<Instant>,
    history: VecDeque<usize>,
    pub hide_idle: bool,
    pub show_background: bool,
    pub filter: String,
    paused: bool,
    confirm: Option<(i32, bool)>,
}

impl SessionsTab {
    pub fn new(profile_id: u64) -> SessionsTab {
        SessionsTab {
            profile_id,
            title: "Sessions".into(),
            data: None,
            error: None,
            pending: None,
            action: None,
            last: None,
            history: VecDeque::new(),
            hide_idle: true,
            show_background: false,
            filter: String::new(),
            paused: false,
            confirm: None,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, handle: Option<QueryHandle>, server: &str) {
        let pal = theme::p();
        if let Some(p) = &self.pending
            && let Some(r) = p.take()
        {
            self.pending = None;
            match r {
                Ok(rs) => {
                    let active = rs.rows().filter(|&r| rs.s(r, 5) == "active").count();
                    self.history.push_back(active);
                    if self.history.len() > 90 {
                        self.history.pop_front();
                    }
                    self.data = Some(rs);
                    self.error = None;
                }
                Err(e) => self.error = Some(e),
            }
        }
        if let Some(a) = &self.action
            && a.take().is_some()
        {
            self.action = None;
            self.last = None; // refresh right away
        }
        match &handle {
            Some(h) if !self.paused && self.pending.is_none() && self.last.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) => {
                self.pending = h.query(SQL, vec![]);
                self.last = Some(Instant::now());
            }
            None if self.error.is_none() => self.error = Some("Expand the server in the sidebar to connect".into()),
            _ => {}
        }
        ui.ctx().request_repaint_after(Duration::from_millis(500));

        egui::Frame::NONE.inner_margin(egui::Margin { left: 18, right: 18, top: 14, bottom: 8 }).show(ui, |ui| {
            let title = format!("Active sessions · {server}");
            w::page_header(
                ui,
                icon::ACTIVITY,
                pal.accent,
                &title,
                "Refreshes every 2 seconds while this tab is visible. You only see what your role is allowed to see.",
                |ui| {
                    let (ico, label) = if self.paused { (icon::PLAY, "Resume") } else { (icon::HOURGLASS, "Pause") };
                    if w::button(ui, w::Kind::Secondary, ico, label).clicked() {
                        self.paused = !self.paused;
                    }
                    // sparkline of active sessions
                    let (r, _) = ui.allocate_exact_size(vec2(180.0, 34.0), Sense::hover());
                    let max = self.history.iter().copied().max().unwrap_or(1).max(1) as f32;
                    let n = self.history.len().max(2) as f32;
                    let pts: Vec<egui::Pos2> = self
                        .history
                        .iter()
                        .enumerate()
                        .map(|(i, v)| pos2(r.left() + i as f32 / (n - 1.0) * r.width(), r.bottom() - *v as f32 / max * (r.height() - 4.0)))
                        .collect();
                    ui.painter().hline(r.x_range(), r.bottom(), Stroke::new(1.0, pal.border));
                    if pts.len() >= 2 {
                        ui.painter().add(egui::Shape::line(pts.clone(), Stroke::new(2.0, pal.accent)));
                    }
                    if let Some(last) = pts.last() {
                        ui.painter().circle_filled(*last, 3.0, pal.accent);
                    }
                    ui.label(RichText::new("active").small().color(pal.faint));
                },
            );
            ui.add_space(12.0);
            if let Some(rs) = &self.data {
                let count = |f: &dyn Fn(usize) -> bool| rs.rows().filter(|&r| f(r)).count();
                let active = count(&|r| rs.s(r, 5) == "active");
                let idle_tx = count(&|r| rs.s(r, 5).starts_with("idle in transaction"));
                let waiting = count(&|r| rs.s(r, 6) == "Lock");
                let blocked = count(&|r| rs.s(r, 11).parse::<i32>().unwrap_or(0) > 0);
                let long = count(&|r| rs.s(r, 5) == "active" && rs.s(r, 8).parse::<f64>().unwrap_or(0.0) > 60.0);
                ui.horizontal(|ui| {
                    let chip = |ui: &mut Ui, n: usize, label: &str, c: egui::Color32| {
                        w::stat(ui, &n.to_string(), label, c);
                    };
                    chip(ui, rs.nrows(), "connections", pal.dim);
                    chip(ui, active, "active", pal.accent);
                    chip(ui, idle_tx, "idle in transaction", if idle_tx > 0 { pal.warn } else { pal.dim });
                    chip(ui, waiting, "waiting on locks", if waiting > 0 { pal.warn } else { pal.dim });
                    chip(ui, blocked, "blocked", if blocked > 0 { pal.danger } else { pal.dim });
                    chip(ui, long, "running > 1 min", if long > 0 { pal.warn } else { pal.dim });
                });
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                w::input(ui, &mut self.filter, "Filter by user, app, database or query", Some(icon::SEARCH), 360.0);
                w::checkbox(ui, &mut self.hide_idle, "Hide idle");
                w::checkbox(ui, &mut self.show_background, "Show background processes");
            });
            if let Some(e) = &self.error {
                ui.add_space(8.0);
                w::callout(ui, pal.danger, icon::CIRCLE_X, e);
            }
        });

        let Some(rs) = &self.data else {
            ui.vertical_centered(|ui| ui.spinner());
            return;
        };
        let f = self.filter.to_lowercase();
        let rows: Vec<usize> = rs
            .rows()
            .filter(|&r| !self.hide_idle || rs.s(r, 5) != "idle")
            .filter(|&r| self.show_background || rs.s(r, 12) == "client backend")
            .filter(|&r| f.is_empty() || [1, 2, 3, 10].iter().any(|&c| rs.s(r, c).to_lowercase().contains(&f)))
            .collect();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(18, 0)).show(ui, |ui| {
                egui::Grid::new("sessions_grid").striped(true).spacing(vec2(14.0, 6.0)).show(ui, |ui| {
                    for h in ["", "PID", "User", "Database", "Application", "Running", "Waiting on", "", "Query"] {
                        ui.label(RichText::new(h).small().strong().color(pal.faint));
                    }
                    ui.end_row();
                    for &r in &rows {
                        let state = rs.s(r, 5);
                        let blocked = rs.s(r, 11).parse::<i32>().unwrap_or(0) > 0;
                        let color = match state {
                            "active" if blocked => pal.danger,
                            "active" => pal.success,
                            s if s.starts_with("idle in transaction") => pal.warn,
                            _ => pal.faint,
                        };
                        let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 4.0, color);
                        let pid: i32 = rs.s(r, 0).parse().unwrap_or(0);
                        ui.label(RichText::new(pid.to_string()).font(theme::mono(12.0)));
                        ui.label(rs.s(r, 1));
                        ui.label(RichText::new(rs.s(r, 2)).color(pal.dim));
                        let app = rs.s(r, 3);
                        let mine = app.starts_with("Trunq");
                        ui.label(RichText::new(if app.is_empty() { rs.s(r, 12) } else { app }).color(if mine { pal.accent } else { pal.text }))
                            .on_hover_text(format!("{}\nclient {}{}", state, rs.s(r, 4), if mine { "\nThis is one of your Trunq connections" } else { "" }));
                        let secs = if state.starts_with("idle in transaction") { rs.s(r, 9) } else { rs.s(r, 8) };
                        let secs: f64 = secs.parse().unwrap_or(0.0);
                        let dur = if secs > 0.0 { util::duration(Duration::from_secs_f64(secs)) } else { "·".into() };
                        ui.label(RichText::new(dur).font(theme::mono(12.0)).color(if secs > 60.0 && state != "idle" { pal.warn } else { pal.dim }));
                        let wait = match (rs.s(r, 6), rs.s(r, 7)) {
                            ("", _) => String::new(),
                            (t, e) => format!("{t}: {e}"),
                        };
                        ui.label(RichText::new(if blocked { format!("{wait} (blocked)") } else { wait }).small().color(if blocked {
                            pal.danger
                        } else {
                            pal.dim
                        }));
                        // background workers can't be cancelled / terminated
                        let client = rs.s(r, 12) == "client backend";
                        ui.scope(|ui| {
                            if !client {
                                ui.set_invisible();
                            }
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                if state != "idle"
                                    && w::small_button(ui, w::Kind::Secondary, icon::STOP, "Cancel")
                                        .on_hover_text("pg_cancel_backend: stop the current statement")
                                        .clicked()
                                {
                                    self.confirm = Some((pid, false));
                                }
                                if w::small_button(ui, w::Kind::Danger, icon::POWER, "Terminate")
                                    .on_hover_text("pg_terminate_backend: disconnect the session")
                                    .clicked()
                                {
                                    self.confirm = Some((pid, true));
                                }
                            });
                        });
                        let q = rs.s(r, 10);
                        let short: String = q.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(110).collect();
                        let resp = ui.add(egui::Label::new(RichText::new(short).font(theme::mono(11.5)).color(pal.text)).sense(Sense::click()));
                        let q_full = q.to_string();
                        resp.on_hover_text(q).context_menu(|ui| {
                            if w::menu_item(ui, icon::COPY, "Copy query").clicked() {
                                ui.ctx().copy_text(q_full.clone());
                                ui.close();
                            }
                        });
                        ui.end_row();
                    }
                });
                if rows.is_empty() {
                    ui.add_space(20.0);
                    ui.label(RichText::new("No sessions match").color(pal.faint));
                }
            });
        });

        if let Some((pid, terminate)) = self.confirm {
            let mut decided = None;
            let modal = egui::Modal::new(egui::Id::unique(("sess_confirm", pid))).frame(w::modal_frame()).show(ui.ctx(), |ui| {
                ui.set_width(420.0);
                let title = if terminate { format!("Terminate session {pid}?") } else { format!("Cancel the query of {pid}?") };
                w::dialog_header(ui, if terminate { icon::POWER } else { icon::STOP }, pal.danger, &title, "");
                ui.label(
                    RichText::new(if terminate {
                        "The session is disconnected and its open transaction is rolled back."
                    } else {
                        "The running statement is stopped; the session stays connected."
                    })
                    .color(pal.text),
                );
                w::dialog_footer(ui, |ui| {
                    if w::button(ui, w::Kind::Danger, if terminate { icon::POWER } else { icon::STOP }, if terminate { "Terminate" } else { "Cancel query" })
                        .clicked()
                    {
                        decided = Some(true);
                    }
                    if w::button(ui, w::Kind::Ghost, "", "Keep it").clicked() {
                        decided = Some(false);
                    }
                });
            });
            if modal.should_close() && decided.is_none() {
                decided = Some(false);
            }
            if let Some(yes) = decided {
                if yes && let Some(h) = &handle {
                    let f = if terminate { "pg_terminate_backend" } else { "pg_cancel_backend" };
                    self.action = h.query(format!("SELECT {f}($1::int)"), vec![Some(pid.to_string())]);
                }
                self.confirm = None;
            }
        }
    }
}
