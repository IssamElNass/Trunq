//! pg_cron inventory. Every mutation is a SQL preview, never a background action.

use super::snapshot::Snapshot;
use crate::action::Action;
use crate::config::Profile;
use crate::db::{meta::quote_literal, result::ResultSet};
use crate::ui::{
    editor::{self, EditorState},
    theme::{self, icon},
    widgets as w,
};
use egui::{RichText, Ui};

pub const DETECT_SQL: &str = r#"
WITH ns AS (SELECT oid FROM pg_namespace WHERE nspname='cron'),
jobs AS (SELECT c.oid FROM pg_class c JOIN ns ON ns.oid=c.relnamespace WHERE c.relname='job'),
runs AS (SELECT c.oid FROM pg_class c JOIN ns ON ns.oid=c.relnamespace WHERE c.relname='job_run_details')
SELECT COALESCE(e.extversion,''), COALESCE((SELECT setting FROM pg_settings WHERE name='cron.database_name'),'not visible to this role'),
       COALESCE((SELECT setting FROM pg_settings WHERE name='cron.timezone'),'server default (GMT unless configured)'),
       COALESCE(has_schema_privilege((SELECT oid FROM ns),'USAGE') AND has_table_privilege((SELECT oid FROM jobs),'SELECT'),false),
       EXISTS (SELECT 1 FROM pg_proc p JOIN ns ON ns.oid=p.pronamespace
               WHERE p.proname='alter_job' AND p.pronargs=6 AND has_function_privilege(p.oid,'EXECUTE')),
       COALESCE(has_schema_privilege((SELECT oid FROM ns),'USAGE') AND has_table_privilege((SELECT oid FROM runs),'SELECT'),false)
FROM (VALUES (1)) AS seed(n) LEFT JOIN pg_extension e ON e.extname='pg_cron';
"#;

pub const JOBS_SQL: &str = r#"
SELECT jobid, COALESCE(to_jsonb(j)->>'jobname',jobid::text), schedule, database, username,
       COALESCE((to_jsonb(j)->>'active')::boolean,true), command, nodename, nodeport
FROM cron.job j ORDER BY jobid;
"#;

pub const HISTORY_SQL: &str = r#"
SELECT jobid,runid,status,return_message,start_time,end_time
FROM cron.job_run_details ORDER BY start_time DESC NULLS LAST,runid DESC LIMIT 200;
"#;

/// Only changed fields are passed, leaving target database and run-as role intact.
pub fn alter_sql(id: i64, schedule: Option<&str>, command: Option<&str>, active: Option<bool>) -> String {
    let mut args = vec![id.to_string()];
    if let Some(s) = schedule {
        args.push(format!("schedule := {}", quote_literal(s)));
    }
    if let Some(s) = command {
        args.push(format!("command := {}", quote_literal(s)));
    }
    if let Some(active) = active {
        args.push(format!("active := {active}"));
    }
    format!("-- Review before running. This changes a scheduled database job.\nSELECT cron.alter_job(\n    {}\n);", args.join(",\n    "))
}

struct Draft {
    row: usize,
    id: i64,
    schedule: String,
    command: EditorState,
    active: bool,
}

impl Draft {
    fn new(rs: &ResultSet, row: usize, profile: u64, db: &str) -> Option<Self> {
        let id = rs.s(row, 0).parse().ok()?;
        let mut command = EditorState::new(egui::Id::unique(("cron_command", profile, db, id)), rs.s(row, 6).into());
        command.request_focus = false;
        Some(Self { row, id, schedule: rs.s(row, 2).into(), command, active: rs.s(row, 5) == "t" })
    }
}

pub struct CronTab {
    pub title: String,
    pub profile_id: u64,
    pub database: String,
    snapshot: Snapshot,
    metadata: Option<ResultSet>,
    data: Option<Vec<ResultSet>>,
    draft: Option<Draft>,
    search: String,
}

impl CronTab {
    pub fn new(profile_id: u64, database: &str) -> Self {
        Self {
            title: format!("Cron jobs · {database}"),
            profile_id,
            database: database.into(),
            snapshot: Snapshot::default(),
            metadata: None,
            data: None,
            draft: None,
            search: String::new(),
        }
    }

    pub fn poll(&mut self, profile: Option<&Profile>, password: Option<String>, ctx: &egui::Context) {
        let sql = match &self.metadata {
            None => DETECT_SQL.to_string(),
            Some(meta) if !meta.s(0, 0).is_empty() && meta.s(0, 3) == "t" => format!("{JOBS_SQL}{}", if meta.s(0, 5) == "t" { HISTORY_SQL } else { "" }),
            Some(_) => return,
        };
        if self.data.is_some() {
            return;
        }
        if let Some(mut sets) = self.snapshot.poll(profile, &self.database, password, ctx, &sql) {
            if self.metadata.is_none() {
                if sets.len() == 1 && sets[0].nrows() == 1 {
                    self.metadata = Some(sets.remove(0));
                    self.snapshot.refresh();
                    ctx.request_repaint();
                } else {
                    self.snapshot.error = Some("Incomplete pg_cron metadata. Refresh to retry.".into());
                }
            } else if sets.is_empty() {
                self.snapshot.error = Some("Incomplete pg_cron jobs. Refresh to retry.".into());
            } else {
                self.data = Some(sets);
                self.draft = None;
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, profile: Option<&Profile>, password: Option<String>, actions: &mut Vec<Action>) {
        self.poll(profile, password, ui.ctx());
        egui::ScrollArea::vertical().id_salt("cron_page").auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin::same(20)).show(ui, |ui| self.content(ui, profile.is_some_and(|p| p.read_only), actions));
        });
    }

    fn content(&mut self, ui: &mut Ui, readonly: bool, actions: &mut Vec<Action>) {
        let p = theme::p();
        w::page_header(
            ui,
            icon::TIMER,
            p.accent,
            &format!("Cron jobs · {}", self.database),
            "Inspect pg_cron schedules and review changes before they run.",
            |ui| {
                if w::button(ui, w::Kind::Secondary, icon::REFRESH, "Refresh").clicked() {
                    self.snapshot.refresh();
                    self.metadata = None;
                    self.data = None;
                    self.draft = None;
                }
            },
        );
        ui.add_space(12.0);
        if let Some(error) = &self.snapshot.error {
            w::callout(ui, p.danger, icon::CIRCLE_X, error);
            return;
        }
        let Some(meta) = &self.metadata else {
            ui.spinner();
            return;
        };
        if meta.s(0, 0).is_empty() || meta.s(0, 3) != "t" {
            w::empty_state(
                ui,
                icon::TIMER,
                if meta.s(0, 0).is_empty() { "pg_cron is not installed in this database" } else { "No permission to read cron.job" },
                &format!("The server's cron.database_name is {}. Connect to that database to inspect its jobs.", meta.s(0, 1)),
            );
            w::hint(ui, "Trunq does not install extensions or change server configuration automatically.");
            if w::button(ui, w::Kind::Secondary, icon::PUZZLE, "Open extensions").clicked() {
                actions.push(Action::OpenExtensions { profile: self.profile_id, db: self.database.clone(), selected: None });
            }
            return;
        }
        let Some(sets) = &self.data else {
            ui.spinner();
            return;
        };
        let rs = &sets[0];
        ui.horizontal_wrapped(|ui| {
            w::stat(ui, &rs.nrows().to_string(), "visible jobs", p.accent);
            w::stat(ui, &rs.rows().filter(|&r| rs.s(r, 5) == "t").count().to_string(), "active", p.success);
            w::pill(ui, &format!("Schedule timezone: {}", meta.s(0, 2)), p.dim);
        });
        ui.add_space(10.0);
        w::input(ui, &mut self.search, "Search job, schedule, database, role or command", Some(icon::SEARCH), ui.available_width().min(440.0));
        ui.add_space(8.0);
        let search = self.search.to_lowercase();
        let rows: Vec<_> = rs.rows().filter(|&r| [1, 2, 3, 4, 6].iter().any(|&c| rs.s(r, c).to_lowercase().contains(&search))).collect();
        if !rows.iter().any(|r| self.draft.as_ref().is_some_and(|d| d.row == *r)) {
            self.draft = rows.first().and_then(|&r| Draft::new(rs, r, self.profile_id, &self.database));
        }
        if rows.is_empty() {
            w::empty_state(
                ui,
                icon::TIMER,
                "No jobs match",
                "Only jobs visible to your PostgreSQL role are shown. Trunq does not bypass pg_cron's row security.",
            );
            return;
        }
        let table_width = ui.available_width().max(710.0);
        egui::ScrollArea::horizontal().id_salt("cron_horizontal").show(ui, |ui| {
            ui.set_width(table_width);
            egui_extras::TableBuilder::new(ui)
                .id_salt("cron_jobs")
                .striped(true)
                .resizable(true)
                .sense(egui::Sense::click())
                .max_scroll_height(180.0)
                .column(egui_extras::Column::remainder().at_least(180.0).resizable(false).clip(true))
                .columns(egui_extras::Column::initial(130.0), 3)
                .column(egui_extras::Column::initial(90.0))
                .header(28.0, |mut h| {
                    for text in ["Job", "Schedule", "Database", "Run as", "Status"] {
                        h.col(|ui| {
                            ui.label(RichText::new(text).small().strong().color(p.dim));
                        });
                    }
                })
                .body(|body| {
                    body.rows(32.0, rows.len(), |mut row| {
                        let r = rows[row.index()];
                        row.set_selected(self.draft.as_ref().is_some_and(|d| d.row == r));
                        for c in [1, 2, 3, 4] {
                            row.col(|ui| {
                                ui.add(egui::Label::new(rs.s(r, c)).truncate()).on_hover_text(rs.s(r, c));
                            });
                        }
                        row.col(|ui| {
                            w::badge(ui, if rs.s(r, 5) == "t" { "Active" } else { "Paused" }, if rs.s(r, 5) == "t" { p.success } else { p.dim });
                        });
                        if row.response().clicked() {
                            self.draft = Draft::new(rs, r, self.profile_id, &self.database);
                        }
                    })
                });
        });
        let Some(draft) = &mut self.draft else {
            return;
        };
        ui.add_space(12.0);
        w::card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(rs.s(draft.row, 1)).font(theme::bold(16.0)));
                w::pill(ui, &format!("Job {}", draft.id), p.dim);
            });
            ui.label(format!("Database: {} · role: {} · host: {}:{}", rs.s(draft.row, 3), rs.s(draft.row, 4), rs.s(draft.row, 7), rs.s(draft.row, 8)));
            ui.add_space(8.0);
            let can_edit = meta.s(0, 4) == "t" && !readonly;
            ui.add_enabled_ui(can_edit, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Schedule");
                    w::code_input(ui, &mut draft.schedule, "0 2 * * *", None, 240.0);
                    w::checkbox(ui, &mut draft.active, "Active");
                });
                w::hint(
                    ui,
                    "Use pg_cron syntax (five cron fields, supported intervals or last-day $). The server validates the schedule when you execute the preview.",
                );
                ui.add_space(6.0);
                egui::Frame::NONE.fill(p.bg0).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(w::RADIUS).inner_margin(egui::Margin::same(8)).show(
                    ui,
                    |ui| {
                        ui.set_height(110.0);
                        editor::sql_editor(ui, &mut draft.command, None, false, true, !can_edit, false);
                    },
                );
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    let original_active = rs.s(draft.row, 5) == "t";
                    if w::small_button(
                        ui,
                        w::Kind::Secondary,
                        if original_active { icon::HOURGLASS } else { icon::PLAY },
                        if original_active { "Review pause SQL" } else { "Review enable SQL" },
                    )
                    .clicked()
                    {
                        actions.push(Action::NewQuery {
                            profile: self.profile_id,
                            db: self.database.clone(),
                            sql: alter_sql(draft.id, None, None, Some(!original_active)),
                            run: false,
                        });
                    }
                    let schedule = (draft.schedule != rs.s(draft.row, 2)).then_some(draft.schedule.as_str());
                    let command = (draft.command.text != rs.s(draft.row, 6)).then_some(draft.command.text.as_str());
                    let active = (draft.active != original_active).then_some(draft.active);
                    let valid = !draft.schedule.trim().is_empty() && !draft.command.text.trim().is_empty();
                    ui.add_enabled_ui(valid && (schedule.is_some() || command.is_some() || active.is_some()), |ui| {
                        if w::small_button(ui, w::Kind::Primary, icon::FILE_CODE, "Review changes SQL").clicked() {
                            actions.push(Action::NewQuery {
                                profile: self.profile_id,
                                db: self.database.clone(),
                                sql: alter_sql(draft.id, schedule, command, active),
                                run: false,
                            });
                        }
                    });
                });
            });
            if !can_edit {
                w::hint(
                    ui,
                    if readonly {
                        "This connection is read-only. Cron edits are disabled."
                    } else {
                        "This pg_cron version does not expose cron.alter_job. Upgrade the extension to edit jobs here."
                    },
                );
            }
            w::hint(
                ui,
                "Changes only take effect after you execute the generated SQL. Active jobs execute their command on schedule; pausing does not stop an already running job. Refresh after changes.",
            );
        });
        if let Some(history) = sets.get(1) {
            let recent: Vec<_> = history.rows().filter(|&r| history.s(r, 0) == draft.id.to_string()).collect();
            ui.add_space(10.0);
            w::disclosure(ui, "cron_history", icon::HISTORY, "Recent runs (latest 200 visible server runs)", &recent.len().to_string(), false, |ui| {
                if recent.is_empty() {
                    w::hint(ui, "No recent runs for this job. Logging may be disabled or the job may not have run yet.");
                }
                for r in recent {
                    ui.horizontal_wrapped(|ui| {
                        w::badge(ui, history.s(r, 2), if history.s(r, 2) == "succeeded" { p.success } else { p.warn });
                        ui.label(history.s(r, 4));
                        ui.label(history.s(r, 3));
                    });
                }
            });
        } else {
            w::hint(ui, "Run history is unavailable or your role cannot read cron.job_run_details.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_quotes_sql_and_only_touches_requested_fields() {
        let sql = alter_sql(42, None, None, Some(false));
        assert!(sql.contains("42,\n    active := false"));
        assert!(!sql.contains("command :="));
        assert!(!sql.contains("database :="));
        let sql = alter_sql(42, Some("0 2 * * *"), Some("SELECT 'it\\works'; -- \"quoted\""), None);
        assert!(sql.contains("command := E'SELECT ''it\\\\works''; -- \"quoted\"'"));
        assert!(!sql.contains("active :="));
    }
}
