//! Table / view structure: table health, columns, indexes, constraints,
//! references, triggers, partitions, grants and DDL.

use egui::{RichText, Ui, vec2};

use crate::action::Action;
use crate::db::meta;
use crate::db::result::ResultSet;
use crate::db::session::Promise;
use crate::ui::editor;
use crate::ui::theme::{self, icon};
use crate::ui::widgets as w;

#[derive(Clone, Copy, PartialEq)]
pub enum Section {
    Health,
    Columns,
    Indexes,
    Constraints,
    Ddl,
}

pub struct StructureTab {
    pub profile_id: u64,
    pub database: String,
    pub title: String,
    pub section: Section,
    promise: Option<Promise<Result<Vec<ResultSet>, String>>>,
    data: Option<Result<Vec<ResultSet>, String>>,
    ddl: String,
}

fn ago(ts: &str) -> (String, bool) {
    if ts.is_empty() {
        return ("never".into(), true);
    }
    let parsed = chrono::DateTime::parse_from_str(&format!("{ts}:00"), "%Y-%m-%d %H:%M:%S%.f%:z")
        .or_else(|_| chrono::DateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S%.f%:z"));
    match parsed {
        Ok(t) => {
            let secs = (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds().max(0);
            let text = if secs < 3600 {
                format!("{} min ago", secs / 60)
            } else if secs < 86_400 * 2 {
                format!("{} h ago", secs / 3600)
            } else {
                format!("{} days ago", secs / 86_400)
            };
            (text, secs > 86_400 * 30)
        }
        Err(_) => (ts.to_string(), false),
    }
}

impl StructureTab {
    pub fn new(profile_id: u64, database: &str, title: &str, section: Section, promise: Promise<Result<Vec<ResultSet>, String>>) -> StructureTab {
        StructureTab { profile_id, database: database.into(), title: title.into(), section, promise: Some(promise), data: None, ddl: String::new() }
    }

    pub fn ui(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let pal = theme::p();
        if let Some(p) = &self.promise
            && let Some(r) = p.take()
        {
            self.promise = None;
            if let Ok(rs) = &r {
                self.ddl = meta::build_table_ddl(rs);
            }
            self.data = Some(r);
        }
        egui::Frame::NONE.inner_margin(egui::Margin { left: 20, right: 20, top: 16, bottom: 12 }).show(ui, |ui| {
            let subtitle = format!("Structure and health · {}", self.database);
            let ddl = self.ddl.clone();
            w::page_header(ui, icon::TABLE_PROPS, pal.accent, &self.title, &subtitle, |ui| {
                if !ddl.is_empty() && w::button(ui, w::Kind::Secondary, icon::FILE_CODE, "Open DDL in editor").clicked() {
                    actions.push(Action::NewQuery { profile: self.profile_id, db: self.database.clone(), sql: ddl.clone(), run: false });
                }
            });
            ui.add_space(14.0);
            w::segmented(
                ui,
                &mut self.section,
                &[
                    (Section::Health, icon::ACTIVITY, "Health"),
                    (Section::Columns, icon::COLUMNS, "Columns"),
                    (Section::Indexes, icon::LIST_TREE, "Indexes"),
                    (Section::Constraints, icon::LINK, "Constraints and references"),
                    (Section::Ddl, icon::CODE, "DDL"),
                ],
            );
        });
        let r = ui.cursor();
        ui.painter().hline(r.x_range(), r.top(), egui::Stroke::new(1.0, pal.border));
        let rs = match &self.data {
            None => {
                ui.vertical_centered(|ui| {
                    ui.add_space(30.0);
                    ui.spinner();
                });
                return;
            }
            Some(Err(e)) => {
                egui::Frame::NONE.inner_margin(egui::Margin::same(20)).show(ui, |ui| w::callout(ui, pal.danger, icon::CIRCLE_X, e));
                return;
            }
            Some(Ok(rs)) => rs,
        };
        let (cols, cons, idx, trg, info, parts, refs, grants) = (&rs[0], &rs[1], &rs[2], &rs[3], &rs[4], &rs[5], &rs[6], &rs[7]);
        if info.nrows() == 0 {
            ui.label("Object not found");
            return;
        }
        let qname = format!("{}.{}", meta::quote_ident(info.s(0, 20)), meta::quote_ident(info.s(0, 21)));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin { left: 18, right: 18, top: 8, bottom: 20 }).show(ui, |ui| match self.section {
                Section::Health => {
                    let stat = |ui: &mut Ui, label: &str, value: String, warn: bool| {
                        egui::Frame::NONE
                            .fill(pal.bg1)
                            .stroke(egui::Stroke::new(1.0, if warn { pal.warn.gamma_multiply(0.6) } else { pal.border }))
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::same(12))
                            .show(ui, |ui| {
                                ui.set_min_width(170.0);
                                ui.label(RichText::new(label).small().color(pal.faint));
                                ui.label(RichText::new(value).font(theme::bold(17.0)).color(if warn { pal.warn } else { pal.text }));
                            });
                    };
                    let live: i64 = info.s(0, 7).parse().unwrap_or(0);
                    let dead: i64 = info.s(0, 8).parse().unwrap_or(0);
                    let dead_pct = if live + dead > 0 { dead as f64 * 100.0 / (live + dead) as f64 } else { 0.0 };
                    let seq: i64 = info.s(0, 13).parse().unwrap_or(0);
                    let idx_scans: i64 = info.s(0, 14).parse().unwrap_or(0);
                    ui.horizontal_wrapped(|ui| {
                        stat(ui, "Total size", info.s(0, 3).to_string(), false);
                        stat(ui, "Table · Indexes", format!("{} · {}", info.s(0, 4), info.s(0, 5)), false);
                        stat(ui, "Estimated rows", crate::util::compact_num(info.s(0, 6).parse().unwrap_or(0)), false);
                        stat(ui, "Dead tuples", format!("{} ({dead_pct:.1}%)", crate::util::compact_num(dead)), dead_pct > 20.0 && dead > 1000);
                    });
                    ui.add_space(8.0);
                    ui.horizontal_wrapped(|ui| {
                        let (v, old) = ago(info.get(0, 9).max(info.get(0, 10)).unwrap_or(""));
                        stat(ui, "Last vacuum", v, old);
                        let (a, old) = ago(info.get(0, 11).max(info.get(0, 12)).unwrap_or(""));
                        stat(ui, "Last analyze", a, old);
                        stat(ui, "Sequential scans", crate::util::compact_num(seq), seq > 1000 && seq > idx_scans * 10 && live > 10_000);
                        stat(ui, "Index scans", crate::util::compact_num(idx_scans), false);
                    });
                    ui.add_space(10.0);
                    let mut tips = Vec::new();
                    if dead_pct > 20.0 && dead > 1000 {
                        tips.push(format!("Many dead rows: run VACUUM (ANALYZE) {qname};"));
                    }
                    if info.get(0, 11).is_none() && info.get(0, 12).is_none() && live > 1000 {
                        tips.push(format!("Never analyzed: the planner is guessing. Run ANALYZE {qname};"));
                    }
                    if seq > 1000 && seq > idx_scans * 10 && live > 10_000 {
                        tips.push("Mostly sequential scans on a large table: an index may be missing. EXPLAIN your slow queries.".into());
                    }
                    let unused: Vec<&str> =
                        idx.rows().filter(|&r| idx.s(r, 5) == "0" && idx.s(r, 3) != "t" && idx.s(r, 2) != "t").map(|r| idx.s(r, 0)).collect();
                    if !unused.is_empty() {
                        tips.push(format!("Unused indexes (no scans since stats reset): {}", unused.join(", ")));
                    }
                    for t in tips {
                        ui.label(RichText::new(format!("{}  {t}", icon::WARN)).color(pal.warn));
                    }
                    ui.add_space(6.0);
                    egui::Grid::new("health_meta").num_columns(2).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
                        for (k, v) in [
                            ("Owner", info.s(0, 1)),
                            ("Comment", info.s(0, 2)),
                            ("Persistence", if info.s(0, 15) == "u" { "unlogged" } else { "logged" }),
                            ("Partition key", info.s(0, 16)),
                            ("Partition of", info.s(0, 18)),
                            ("Storage options", info.s(0, 19)),
                            ("Tablespace", info.s(0, 23)),
                        ] {
                            if !v.is_empty() {
                                ui.label(RichText::new(k).color(pal.dim));
                                ui.label(v);
                                ui.end_row();
                            }
                        }
                    });
                }
                Section::Columns => {
                    egui::Grid::new("struct_cols").striped(true).spacing(vec2(16.0, 5.0)).show(ui, |ui| {
                        for h in ["#", "Column", "Type", "Nullable", "Default / generated", "Comment"] {
                            ui.label(RichText::new(h).small().strong().color(pal.faint));
                        }
                        ui.end_row();
                        for r in cols.rows() {
                            ui.label(RichText::new(cols.s(r, 0)).color(pal.faint));
                            ui.label(RichText::new(cols.s(r, 1)).font(theme::mono(12.5)).strong());
                            ui.label(RichText::new(cols.s(r, 2)).font(theme::mono(12.0)).color(pal.kw));
                            ui.label(if cols.s(r, 3) == "t" {
                                RichText::new("NOT NULL").small().color(pal.dim)
                            } else {
                                RichText::new("null").small().color(pal.faint)
                            });
                            let def = match (cols.s(r, 5), cols.s(r, 6)) {
                                ("a", _) => "identity (always)".to_string(),
                                ("d", _) => "identity (by default)".to_string(),
                                (_, "s") => format!("generated: {}", cols.s(r, 4)),
                                _ => cols.s(r, 4).to_string(),
                            };
                            ui.label(RichText::new(def).font(theme::mono(11.5)).color(pal.dim));
                            ui.label(RichText::new(cols.s(r, 7)).small().color(pal.dim));
                            ui.end_row();
                        }
                    });
                }
                Section::Indexes => {
                    for r in idx.rows() {
                        egui::Frame::NONE
                            .fill(pal.bg1)
                            .stroke(egui::Stroke::new(1.0, pal.border))
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::same(10))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(idx.s(r, 0)).strong());
                                    if idx.s(r, 3) == "t" {
                                        ui.label(RichText::new("primary").small().color(pal.warn));
                                    } else if idx.s(r, 2) == "t" {
                                        ui.label(RichText::new("unique").small().color(pal.accent));
                                    }
                                    if idx.s(r, 7) != "t" {
                                        ui.label(RichText::new("INVALID").small().color(pal.danger));
                                    }
                                    ui.label(RichText::new(format!("{} · {} scans", idx.s(r, 4), idx.s(r, 5))).small().color(if idx.s(r, 5) == "0" {
                                        pal.warn
                                    } else {
                                        pal.faint
                                    }));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if w::small_button(ui, w::Kind::Ghost, icon::COPY, "Copy DDL").clicked() {
                                            ui.ctx().copy_text(format!("{};", idx.s(r, 1)));
                                        }
                                    });
                                });
                                editor::code_view(ui, idx.s(r, 1), 12.0);
                            });
                        ui.add_space(6.0);
                    }
                    if idx.nrows() == 0 {
                        ui.label(RichText::new("No indexes").color(pal.faint));
                    }
                }
                Section::Constraints => {
                    let section = |ui: &mut Ui, title: &str| {
                        ui.add_space(8.0);
                        ui.label(RichText::new(title).small().strong().color(pal.faint));
                    };
                    section(ui, "CONSTRAINTS");
                    for r in cons.rows() {
                        let kind = match cons.s(r, 1) {
                            "p" => "PRIMARY KEY",
                            "f" => "FOREIGN KEY",
                            "u" => "UNIQUE",
                            "c" => "CHECK",
                            "x" => "EXCLUDE",
                            "n" => "NOT NULL",
                            _ => "",
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(kind).small().color(pal.kw));
                            ui.label(RichText::new(cons.s(r, 0)).strong());
                            ui.label(RichText::new(cons.s(r, 2)).font(theme::mono(12.0)).color(pal.dim));
                        });
                    }
                    section(ui, "REFERENCED BY");
                    for r in refs.rows() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon::LINK).color(pal.accent));
                            ui.label(RichText::new(refs.s(r, 1)).strong());
                            ui.label(RichText::new(format!("{} · {}", refs.s(r, 0), refs.s(r, 2))).font(theme::mono(12.0)).color(pal.dim));
                        });
                    }
                    if refs.nrows() == 0 {
                        ui.label(RichText::new("No other table references this one").small().color(pal.faint));
                    }
                    if trg.nrows() > 0 {
                        section(ui, "TRIGGERS");
                        for r in trg.rows() {
                            ui.label(RichText::new(trg.s(r, 1)).font(theme::mono(12.0)));
                        }
                    }
                    if parts.nrows() > 0 {
                        section(ui, "PARTITIONS");
                        for r in parts.rows() {
                            ui.label(RichText::new(format!("{}  {}", parts.s(r, 0), parts.s(r, 1))).font(theme::mono(12.0)));
                        }
                    }
                    if grants.nrows() > 0 {
                        section(ui, "PRIVILEGES");
                        for r in grants.rows() {
                            ui.label(RichText::new(format!("{}: {}", grants.s(r, 0), grants.s(r, 1))).font(theme::mono(12.0)));
                        }
                    }
                }
                Section::Ddl => {
                    ui.horizontal(|ui| {
                        if w::button(ui, w::Kind::Secondary, icon::COPY, "Copy").clicked() {
                            ui.ctx().copy_text(self.ddl.clone());
                        }
                        if w::button(ui, w::Kind::Secondary, icon::FILE_CODE, "Open in editor").clicked() {
                            actions.push(Action::NewQuery { profile: self.profile_id, db: self.database.clone(), sql: self.ddl.clone(), run: false });
                        }
                    });
                    ui.add_space(6.0);
                    editor::code_view(ui, &self.ddl, 12.5);
                }
            });
        });
    }
}
