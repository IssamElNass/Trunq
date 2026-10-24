//! One-click database health check: finds the problems that cause real
//! incidents (ID overflow, wraparound, WAL-retaining slots, missing FK
//! indexes, bloat, forgotten transactions...) and generates the fix SQL.

use std::time::Instant;

use egui::{RichText, Ui, vec2};

use crate::action::Action;
use crate::config::Profile;
use crate::db::meta::{quote_ident, quote_literal};
use crate::db::result::ResultSet;
use crate::db::session::{Promise, QueryResult, Session};
use crate::ui::theme::{self, icon};
use crate::ui::widgets as w;

#[derive(Clone, Copy, PartialEq)]
enum Severity {
    Critical,
    Warning,
    Info,
}

struct Check {
    title: &'static str,
    why: &'static str,
    severity: Severity,
    sql: &'static str,
    fix: Option<fn(&ResultSet) -> String>,
}

fn checks() -> Vec<Check> {
    vec![
        Check {
            title: "Integer columns running out of IDs",
            why: "serial / int columns whose sequence has used more than half of the integer range. When it reaches 2,147,483,647 every INSERT fails.",
            severity: Severity::Critical,
            sql: r#"SELECT n.nspname AS schema, c.relname AS "table", a.attname AS "column", format_type(a.atttypid, a.atttypmod) AS type,
       s.last_value, round(100.0 * s.last_value / CASE a.atttypid WHEN 21 THEN 32767 ELSE 2147483647 END, 1) AS pct_used
FROM pg_attribute a
JOIN pg_class c ON c.oid = a.attrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_depend d ON d.refobjid = c.oid AND d.refobjsubid = a.attnum AND d.classid = 'pg_class'::regclass AND d.deptype IN ('a', 'i')
JOIN pg_class sc ON sc.oid = d.objid AND sc.relkind = 'S'
JOIN pg_namespace sn ON sn.oid = sc.relnamespace
JOIN pg_sequences s ON s.schemaname = sn.nspname AND s.sequencename = sc.relname
WHERE a.atttypid IN (21, 23) AND s.last_value IS NOT NULL
  AND s.last_value > 0.5 * CASE a.atttypid WHEN 21 THEN 32767 ELSE 2147483647 END
ORDER BY pct_used DESC"#,
            fix: Some(|rs| {
                rs.rows()
                    .map(|r| format!("-- {}% used. Rewrites the table: run in a maintenance window or use a new column + backfill.\nALTER TABLE {}.{} ALTER COLUMN {} TYPE bigint;", rs.s(r, 5), quote_ident(rs.s(r, 0)), quote_ident(rs.s(r, 1)), quote_ident(rs.s(r, 2))))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            }),
        },
        Check {
            title: "Transaction ID wraparound",
            why: "Databases whose oldest unfrozen transaction is over 500 million transactions old. At ~2 billion PostgreSQL stops accepting writes.",
            severity: Severity::Critical,
            sql: "SELECT datname AS database, age(datfrozenxid) AS xid_age, round(100.0 * age(datfrozenxid) / 2000000000, 1) AS pct_to_shutdown FROM pg_database WHERE age(datfrozenxid) > 500000000 ORDER BY 2 DESC",
            fix: Some(|rs| rs.rows().map(|r| format!("-- connect to {} and run:\nVACUUM (FREEZE, VERBOSE);", rs.s(r, 0))).collect::<Vec<_>>().join("\n\n")),
        },
        Check {
            title: "Inactive replication slots retaining WAL",
            why: "An inactive slot keeps every WAL file since it stopped, the disk eventually fills up and the server goes down.",
            severity: Severity::Critical,
            sql: "SELECT slot_name, slot_type, database, active, pg_size_pretty(pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)) AS retained_wal FROM pg_replication_slots WHERE NOT active",
            fix: Some(|rs| {
                rs.rows()
                    .map(|r| format!("-- Only if this slot is no longer needed!\nSELECT pg_drop_replication_slot({});", quote_literal(rs.s(r, 0))))
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
        },
        Check {
            title: "Long-running or idle-in-transaction sessions",
            why: "Transactions open for more than 5 minutes hold locks and prevent VACUUM from cleaning up dead rows.",
            severity: Severity::Warning,
            sql: "SELECT pid, usename AS \"user\", application_name AS app, state, now() - xact_start AS open_for, left(query, 120) AS last_query FROM pg_stat_activity WHERE xact_start < now() - interval '5 minutes' AND pid <> pg_backend_pid() ORDER BY xact_start",
            fix: Some(|rs| {
                rs.rows().map(|r| format!("SELECT pg_terminate_backend({});  -- {} · {}", rs.s(r, 0), rs.s(r, 1), rs.s(r, 2))).collect::<Vec<_>>().join("\n")
            }),
        },
        Check {
            title: "Foreign keys without an index",
            why: "Deleting / updating a referenced row scans the whole referencing table, and joins on the key are slow.",
            severity: Severity::Warning,
            sql: r#"SELECT c.conrelid::regclass::text AS "table", c.conname AS constraint_name,
       (SELECT string_agg(quote_ident(a.attname), ', ' ORDER BY x.n) FROM unnest(c.conkey) WITH ORDINALITY x(attnum, n)
          JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = x.attnum) AS columns,
       c.confrelid::regclass::text AS references
FROM pg_constraint c
WHERE c.contype = 'f'
  AND NOT EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = c.conrelid
                  AND (i.indkey::int2[])[0:cardinality(c.conkey) - 1] @> c.conkey)
ORDER BY 1"#,
            fix: Some(|rs| rs.rows().map(|r| format!("CREATE INDEX CONCURRENTLY ON {} ({});", rs.s(r, 0), rs.s(r, 2))).collect::<Vec<_>>().join("\n")),
        },
        Check {
            title: "Invalid indexes",
            why: "Left behind by a failed CREATE INDEX CONCURRENTLY: they slow down writes and are never used by queries.",
            severity: Severity::Warning,
            sql: "SELECT i.indexrelid::regclass::text AS index, i.indrelid::regclass::text AS \"table\", pg_size_pretty(pg_relation_size(i.indexrelid)) AS size FROM pg_index i WHERE NOT i.indisvalid",
            fix: Some(|rs| rs.rows().map(|r| format!("REINDEX INDEX CONCURRENTLY {};", rs.s(r, 0))).collect::<Vec<_>>().join("\n")),
        },
        Check {
            title: "Duplicate indexes",
            why: "Indexes with exactly the same definition: pure overhead on every write and wasted disk.",
            severity: Severity::Warning,
            sql: r#"SELECT (array_agg(idx ORDER BY idx))[1] AS keep, (array_agg(idx ORDER BY idx))[2] AS duplicate, pg_size_pretty(sum(pg_relation_size(idx::regclass))::bigint) AS total_size
FROM (SELECT indexrelid::regclass::text AS idx,
             indrelid::text || E'\n' || indclass::text || E'\n' || indkey::text || E'\n' || coalesce(indexprs::text, '') || E'\n' || coalesce(indpred::text, '') AS k
      FROM pg_index) s
GROUP BY k HAVING count(*) > 1"#,
            fix: Some(|rs| {
                rs.rows().map(|r| format!("DROP INDEX CONCURRENTLY {};  -- duplicate of {}", rs.s(r, 1), rs.s(r, 0))).collect::<Vec<_>>().join("\n")
            }),
        },
        Check {
            title: "Unused indexes",
            why: "Never used since statistics were last reset, yet maintained on every write. (Check replicas before dropping!)",
            severity: Severity::Info,
            sql: r#"SELECT s.schemaname || '.' || s.indexrelname AS index, s.schemaname || '.' || s.relname AS "table", pg_size_pretty(pg_relation_size(s.indexrelid)) AS size, s.idx_scan AS scans
FROM pg_stat_user_indexes s JOIN pg_index i ON i.indexrelid = s.indexrelid
WHERE s.idx_scan = 0 AND NOT i.indisunique AND NOT i.indisprimary AND pg_relation_size(s.indexrelid) > 65536
ORDER BY pg_relation_size(s.indexrelid) DESC LIMIT 50"#,
            fix: Some(|rs| rs.rows().map(|r| format!("DROP INDEX CONCURRENTLY {};  -- {}", rs.s(r, 0), rs.s(r, 2))).collect::<Vec<_>>().join("\n")),
        },
        Check {
            title: "Tables with many dead rows",
            why: "More than 20% dead tuples: queries read garbage, the table and its indexes bloat.",
            severity: Severity::Warning,
            sql: "SELECT schemaname || '.' || relname AS \"table\", n_live_tup AS live, n_dead_tup AS dead, round(100.0 * n_dead_tup / greatest(n_live_tup, 1), 1) AS dead_pct, greatest(last_vacuum, last_autovacuum) AS last_vacuum FROM pg_stat_user_tables WHERE n_dead_tup > 1000 AND n_dead_tup > 0.2 * n_live_tup ORDER BY n_dead_tup DESC LIMIT 50",
            fix: Some(|rs| rs.rows().map(|r| format!("VACUUM (ANALYZE, VERBOSE) {};", rs.s(r, 0))).collect::<Vec<_>>().join("\n")),
        },
        Check {
            title: "Tables never analyzed",
            why: "Without statistics the planner guesses row counts and picks bad plans.",
            severity: Severity::Info,
            sql: "SELECT schemaname || '.' || relname AS \"table\", n_live_tup AS rows FROM pg_stat_user_tables WHERE last_analyze IS NULL AND last_autoanalyze IS NULL AND n_live_tup > 1000 ORDER BY n_live_tup DESC LIMIT 50",
            fix: Some(|rs| rs.rows().map(|r| format!("ANALYZE {};", rs.s(r, 0))).collect::<Vec<_>>().join("\n")),
        },
        Check {
            title: "Tables without a primary key",
            why: "Rows can't be uniquely addressed: logical replication, many tools and safe row editing need a primary key.",
            severity: Severity::Info,
            sql: "SELECT n.nspname || '.' || c.relname AS \"table\", c.reltuples::bigint AS est_rows FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p') AND NOT c.relispartition AND n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname !~ '^pg_toast' AND NOT EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = c.oid AND i.indisprimary) ORDER BY 1",
            fix: None,
        },
        Check {
            title: "Low cache hit ratio",
            why: "Less than 99% of reads come from memory, the working set may not fit in shared_buffers / RAM.",
            severity: Severity::Info,
            sql: "SELECT datname AS database, round(100.0 * blks_hit / nullif(blks_hit + blks_read, 0), 2) AS hit_pct, blks_read AS disk_reads FROM pg_stat_database WHERE datname = current_database() AND blks_hit + blks_read > 10000 AND 100.0 * blks_hit / nullif(blks_hit + blks_read, 0) < 99",
            fix: None,
        },
    ]
}

struct Outcome {
    promise: Option<Promise<QueryResult>>,
    result: Option<Result<ResultSet, String>>,
    open: bool,
}

pub struct HealthTab {
    pub title: String,
    pub profile_id: u64,
    pub database: String,
    session: Option<Session>,
    checks: Vec<Check>,
    outcomes: Vec<Outcome>,
    started: Option<Instant>,
}

impl HealthTab {
    pub fn new(profile_id: u64, database: &str) -> HealthTab {
        HealthTab {
            title: format!("Health · {database}"),
            profile_id,
            database: database.to_string(),
            session: None,
            checks: checks(),
            outcomes: Vec::new(),
            started: None,
        }
    }

    fn start(&mut self, profile: &Profile, password: Option<String>, ctx: &egui::Context) {
        let s = Session::connect(profile, &self.database, "Health check", password, ctx);
        self.outcomes = self.checks.iter().map(|c| Outcome { promise: Some(s.query(c.sql, vec![])), result: None, open: false }).collect();
        self.session = Some(s);
        self.started = Some(Instant::now());
    }

    pub fn ui(&mut self, ui: &mut Ui, profile: Option<&Profile>, password: Option<String>, actions: &mut Vec<Action>) {
        let pal = theme::p();
        // Run once when the tab opens (and again only via "Run again").
        if self.started.is_none()
            && let Some(p) = profile
        {
            self.start(p, password.clone(), ui.ctx());
        }
        if let Some(s) = &mut self.session {
            for e in s.poll() {
                if let crate::db::session::Event::ConnectFailed { error, .. } = e {
                    for o in &mut self.outcomes {
                        o.promise = None;
                        o.result = Some(Err(error.clone()));
                    }
                }
            }
        }
        let mut pending = 0;
        for o in &mut self.outcomes {
            if let Some(p) = &o.promise {
                match p.take() {
                    Some(r) => {
                        o.open = matches!(&r, Ok(rs) if rs.nrows() > 0);
                        o.result = Some(r);
                        o.promise = None;
                    }
                    None => pending += 1,
                }
            }
        }
        if pending == 0 && self.session.is_some() {
            // Done: release the connection right away.
            self.session = None;
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        let issues = |sev: Severity| {
            self.checks.iter().zip(&self.outcomes).filter(|(c, o)| c.severity == sev && matches!(&o.result, Some(Ok(rs)) if rs.nrows() > 0)).count()
        };
        let (crit, warn, info) = (issues(Severity::Critical), issues(Severity::Warning), issues(Severity::Info));

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 24, top: 18, bottom: 24 }).show(ui, |ui| {
                ui.set_max_width(1100.0);
                let title = format!("Health check · {}", self.database);
                let mut rerun = false;
                w::page_header(
                    ui,
                    icon::SHIELD,
                    pal.accent,
                    &title,
                    "Finds problems before they become incidents. Every fix is generated as SQL for you to review, nothing runs automatically.",
                    |ui| {
                        if pending == 0 && w::button(ui, w::Kind::Secondary, icon::REFRESH, "Run again").clicked() {
                            rerun = true;
                        }
                    },
                );
                if rerun && let Some(p) = profile {
                    self.start(p, password.clone(), ui.ctx());
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let chip = |ui: &mut Ui, n: usize, label: &str, c: egui::Color32| {
                        w::stat(ui, &n.to_string(), label, if n == 0 && c != pal.success { pal.dim } else { c });
                    };
                    if pending > 0 {
                        ui.spinner();
                        ui.label(format!("Running {} checks…", pending));
                    } else {
                        chip(ui, crit, "critical", pal.danger);
                        chip(ui, warn, "warnings", pal.warn);
                        chip(ui, info, "suggestions", pal.accent);
                        let ok = self.outcomes.iter().filter(|o| matches!(&o.result, Some(Ok(rs)) if rs.nrows() == 0)).count();
                        chip(ui, ok, "passed", pal.success);
                    }
                });
                ui.add_space(14.0);
                for (check, out) in self.checks.iter().zip(self.outcomes.iter_mut()) {
                    let (ico, color, status) = match &out.result {
                        None => (icon::LOADER, pal.faint, "running…".to_string()),
                        Some(Err(e)) => (icon::CIRCLE_HELP, pal.faint, format!("could not run: {}", e.lines().next().unwrap_or(""))),
                        Some(Ok(rs)) if rs.nrows() == 0 => (icon::CIRCLE_CHECK, pal.success, "OK".into()),
                        Some(Ok(rs)) => {
                            let c = match check.severity {
                                Severity::Critical => pal.danger,
                                Severity::Warning => pal.warn,
                                Severity::Info => pal.accent,
                            };
                            (icon::WARN, c, format!("{} found", rs.nrows()))
                        }
                    };
                    let accent_bar = matches!(&out.result, Some(Ok(rs)) if rs.nrows() > 0);
                    let card = egui::Frame::NONE
                        .fill(pal.bg1)
                        .stroke(egui::Stroke::new(1.0, pal.border))
                        .corner_radius(10.0)
                        .inner_margin(egui::Margin { left: 18, right: 14, top: 12, bottom: 12 })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let header = ui.horizontal(|ui| {
                                ui.label(RichText::new(ico).size(16.0).color(color));
                                ui.label(RichText::new(check.title).font(theme::bold(14.0)));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    w::pill(ui, &status, color);
                                });
                            });
                            if header.response.interact(egui::Sense::click()).clicked() {
                                out.open = !out.open;
                            }
                            ui.label(RichText::new(check.why).color(pal.dim));
                            if let Some(Ok(rs)) = &out.result
                                && rs.nrows() > 0
                            {
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    let (ico, label) = if out.open { (icon::CHEVRON_UP, "Hide details") } else { (icon::CHEVRON_DOWN, "Show details") };
                                    if w::small_button(ui, w::Kind::Secondary, ico, label).clicked() {
                                        out.open = !out.open;
                                    }
                                    if let Some(fix) = check.fix
                                        && w::small_button(ui, w::Kind::Primary, icon::WAND, "Open fix SQL").clicked()
                                    {
                                        actions.push(Action::NewQuery {
                                            profile: self.profile_id,
                                            db: self.database.clone(),
                                            sql: format!("-- {}\n-- Review before running!\n\n{}\n", check.title, fix(rs)),
                                            run: false,
                                        });
                                    }
                                });
                                if out.open {
                                    ui.add_space(6.0);
                                    egui::ScrollArea::horizontal().id_salt(check.title).show(ui, |ui| {
                                        egui::Grid::new(check.title).striped(true).spacing(vec2(18.0, 4.0)).show(ui, |ui| {
                                            for c in rs.columns.iter() {
                                                ui.label(RichText::new(&c.name).small().strong().color(pal.dim));
                                            }
                                            ui.end_row();
                                            for r in rs.rows().take(200) {
                                                for c in 0..rs.ncols() {
                                                    let v: String = rs.get(r, c).unwrap_or("NULL").chars().take(120).collect();
                                                    ui.label(RichText::new(v).font(theme::mono(12.0)));
                                                }
                                                ui.end_row();
                                            }
                                        });
                                    });
                                }
                            }
                        });
                    if accent_bar {
                        let r = card.response.rect;
                        let bar = egui::Rect::from_min_max(egui::pos2(r.left() + 1.0, r.top() + 10.0), egui::pos2(r.left() + 4.0, r.bottom() - 10.0));
                        ui.painter().rect_filled(bar, 2.0, color);
                    }
                    ui.add_space(8.0);
                }
            });
        });
    }
}
