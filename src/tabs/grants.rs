//! Searchable ACL inventory, membership routes and reviewable permission SQL.

use super::snapshot::Snapshot;
use crate::action::Action;
use crate::config::Profile;
use crate::db::grants::{self, Inventory, Route};
use crate::db::session::Promise;
use crate::ui::{
    theme::{self, icon},
    widgets as w,
};
use egui::{RichText, Ui};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Objects,
    Defaults,
    Memberships,
}

#[derive(PartialEq, Eq)]
struct FilterKey {
    search: String,
    role: Option<usize>,
    schema: Option<usize>,
    kind: Option<usize>,
    inherited: bool,
    public: bool,
    grantable: bool,
    system: bool,
    view: View,
}

pub struct GrantsTab {
    pub title: String,
    pub profile_id: u64,
    pub database: String,
    snapshot: Snapshot,
    data: Option<Arc<Inventory>>,
    preparing: Option<Promise<Result<Inventory, String>>>,
    filter_key: Option<FilterKey>,
    filtered_rows: Vec<usize>,
    route_role: Option<Option<usize>>,
    route_cache: HashMap<String, Route>,
    view: View,
    search: String,
    role: Option<usize>,
    schema: Option<usize>,
    kind: Option<usize>,
    inherited: bool,
    public_only: bool,
    grantable_only: bool,
    system: bool,
    selected: Option<usize>,
}

impl GrantsTab {
    pub fn new(profile_id: u64, database: &str, search: String) -> Self {
        Self {
            title: format!("Grants · {database}"),
            profile_id,
            database: database.into(),
            snapshot: Snapshot::default(),
            data: None,
            preparing: None,
            filter_key: None,
            filtered_rows: Vec::new(),
            route_role: None,
            route_cache: HashMap::new(),
            view: View::Objects,
            search,
            role: None,
            schema: None,
            kind: None,
            inherited: true,
            public_only: false,
            grantable_only: false,
            system: false,
            selected: None,
        }
    }

    pub fn poll(&mut self, profile: Option<&Profile>, password: Option<String>, ctx: &egui::Context) {
        if let Some(sets) = self.snapshot.poll(profile, &self.database, password, ctx, grants::SQL) {
            let prepared = Promise::new();
            self.preparing = Some(prepared.clone());
            let repaint = ctx.clone();
            std::thread::spawn(move || {
                prepared.set(Inventory::parse(&sets));
                repaint.request_repaint();
            });
        }
        if let Some(result) = self.preparing.as_ref().and_then(Promise::take) {
            self.preparing = None;
            match result {
                Ok(data) => {
                    self.data = Some(Arc::new(data));
                    self.selected = None;
                    self.role = None;
                    self.schema = None;
                    self.kind = None;
                    self.filter_key = None;
                    self.route_role = None;
                }
                Err(e) => self.snapshot.error = Some(e),
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, profile: Option<&Profile>, password: Option<String>, actions: &mut Vec<Action>) {
        self.poll(profile, password, ui.ctx());
        egui::ScrollArea::vertical().id_salt("grants_page").auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::NONE.inner_margin(egui::Margin::same(20)).show(ui, |ui| self.content(ui, actions));
        });
    }

    /// Scripted visual checks use the same state as interactive controls.
    pub fn dev(&mut self, arg: &str) {
        let (key, value) = arg.split_once('=').unwrap_or(("view", arg));
        match key {
            "view" => {
                self.view = match value {
                    "defaults" => View::Defaults,
                    "memberships" => View::Memberships,
                    _ => View::Objects,
                }
            }
            "search" => self.search = value.into(),
            "role" => self.role = self.data.as_ref().and_then(|d| d.roles.iter().position(|r| r.0 == value)),
            "row" => self.selected = value.parse().ok(),
            _ => {}
        }
    }

    fn content(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let p = theme::p();
        w::page_header(ui, icon::KEY, p.accent, &format!("Grants · {}", self.database), "Explore privileges, their source and role membership paths.", |ui| {
            if w::button(ui, w::Kind::Secondary, icon::REFRESH, "Refresh").clicked() {
                self.snapshot.refresh();
                self.preparing = None;
                self.data = None;
                self.selected = None;
            }
        });
        ui.add_space(12.0);
        if let Some(error) = &self.snapshot.error {
            w::callout(ui, p.danger, icon::CIRCLE_X, error);
            return;
        }
        if self.snapshot.loading() || self.preparing.is_some() {
            w::empty_state(ui, icon::LOADER, "Loading grants", "Reading database ACLs and role memberships…");
            ui.spinner();
            return;
        }
        let Some(data) = &self.data else { return };
        ui.horizontal_wrapped(|ui| {
            w::stat(ui, &data.user_object_count.to_string(), "user objects with privileges", p.accent);
            w::stat(ui, &data.roles.len().to_string(), "roles", p.text);
            w::stat(ui, &data.public_count.to_string(), "PUBLIC privileges", p.warn);
            w::stat(ui, &data.memberships.len().to_string(), "memberships", p.dim);
        });
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            w::segmented(
                ui,
                &mut self.view,
                &[
                    (View::Objects, icon::TABLE, "Object grants"),
                    (View::Defaults, icon::PLUS, "Default privileges"),
                    (View::Memberships, icon::USERS, "Memberships"),
                ],
            );
        });
        ui.add_space(10.0);
        let role_options: Vec<_> = std::iter::once((None, "All roles")).chain(data.roles.iter().enumerate().map(|(i, r)| (Some(i), r.0.as_str()))).collect();
        ui.horizontal_wrapped(|ui| {
            w::input(ui, &mut self.search, "Search object, role, grantor or privilege", Some(icon::SEARCH), ui.available_width().min(330.0));
            ui.push_id("grants_role", |ui| {
                w::dropdown(ui, &mut self.role, &role_options, 180.0);
            });
            if w::small_button(ui, w::Kind::Ghost, icon::X, "Clear filters").clicked() {
                self.search.clear();
                self.role = None;
                self.schema = None;
                self.kind = None;
                self.public_only = false;
                self.grantable_only = false;
                self.inherited = true;
                self.system = false;
            }
        });
        let role = self.role.and_then(|i| data.roles.get(i));
        if self.route_role != Some(self.role) {
            self.route_cache = role.map(|r| data.routes(&r.0)).unwrap_or_default();
            self.route_role = Some(self.role);
        }
        let routes = &self.route_cache;
        if let Some(r) = role
            && (r.1 || r.2)
        {
            w::callout(
                ui,
                p.warn,
                icon::SHIELD,
                if r.1 {
                    "This role is a superuser. Its access extends beyond the ACL rows shown here."
                } else {
                    "This role bypasses row-level security. ACL privileges still apply."
                },
            );
        }
        let search = self.search.to_lowercase();
        if self.view == View::Memberships {
            ui.add_space(8.0);
            w::hint(ui, "Direct memberships, including INHERIT, SET ROLE and ADMIN options. Select a role to follow its reachable memberships.");
            let memberships: Vec<_> = data
                .memberships
                .iter()
                .filter(|m| role.is_none() || routes.contains_key(&m.member) || role.is_some_and(|r| r.0 == m.parent))
                .filter(|m| [&m.member, &m.parent, &m.grantor].iter().any(|s| s.to_lowercase().contains(&search)))
                .collect();
            if memberships.is_empty() {
                w::empty_state(ui, icon::USERS, "No memberships match", "Change the role or search filter.");
            }
            for m in memberships {
                w::card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        w::pill(ui, &m.member, p.accent);
                        ui.label("→");
                        w::pill(ui, &m.parent, p.qident);
                        w::badge(ui, if m.inherit { "INHERIT" } else { "NO INHERIT" }, if m.inherit { p.success } else { p.warn });
                        w::badge(ui, if m.set_role { "SET ROLE" } else { "NO SET ROLE" }, p.dim);
                        if m.admin {
                            w::badge(ui, "ADMIN", p.warn);
                        }
                    });
                    w::hint(ui, &format!("Granted by {}", m.grantor));
                });
                ui.add_space(6.0);
            }
            return;
        }
        let schemas = if self.system { &data.schemas } else { &data.user_schemas };
        let kinds = &data.kinds;
        ui.horizontal_wrapped(|ui| {
            let opts: Vec<_> = std::iter::once((None, "All schemas"))
                .chain(schemas.iter().enumerate().map(|(i, s)| (Some(i), if s.is_empty() { "No schema / global" } else { s.as_str() })))
                .collect();
            ui.push_id("grants_schema", |ui| {
                w::dropdown(ui, &mut self.schema, &opts, 180.0);
            });
            let opts: Vec<_> = std::iter::once((None, "All object types")).chain(kinds.iter().enumerate().map(|(i, s)| (Some(i), s.as_str()))).collect();
            ui.push_id("grants_kind", |ui| {
                w::dropdown(ui, &mut self.kind, &opts, 180.0);
            });
            w::checkbox(ui, &mut self.inherited, "Include inherited & PUBLIC");
            w::checkbox(ui, &mut self.public_only, "PUBLIC only");
            w::checkbox(ui, &mut self.grantable_only, "Grant option");
            if w::checkbox(ui, &mut self.system, "System objects").changed() {
                self.schema = None;
            }
        });
        let key = FilterKey {
            search: search.clone(),
            role: self.role,
            schema: self.schema,
            kind: self.kind,
            inherited: self.inherited,
            public: self.public_only,
            grantable: self.grantable_only,
            system: self.system,
            view: self.view,
        };
        if self.filter_key.as_ref() != Some(&key) {
            self.filtered_rows = data
                .grants
                .iter()
                .enumerate()
                .filter(|(i, g)| {
                    g.future == (self.view == View::Defaults)
                        && (self.system || !g.is_system())
                        && self.schema.and_then(|i| schemas.get(i)).is_none_or(|s| **s == g.schema)
                        && self.kind.and_then(|i| kinds.get(i)).is_none_or(|s| **s == g.kind)
                        && (!self.public_only || g.public)
                        && (!self.grantable_only || g.grantable)
                        && role.is_none_or(|r| (!g.public && g.grantee == r.0) || (self.inherited && (g.public || routes.contains_key(&g.grantee))))
                        && (search.is_empty() || data.search_text[*i].contains(&search))
                })
                .map(|(i, _)| i)
                .collect();
            self.filter_key = Some(key);
            if !self.filtered_rows.iter().any(|i| Some(*i) == self.selected) {
                self.selected = self.filtered_rows.first().copied();
            }
        }
        let rows = &self.filtered_rows;
        ui.horizontal_wrapped(|ui| {
            w::hint(ui, &format!("{} privileges · select a row to trace access", rows.len()));
            if !rows.is_empty() && w::small_button(ui, w::Kind::Ghost, icon::COPY, "Copy filtered CSV").clicked() {
                let mut csv = "type,schema,object,owner,grantor,grantee,privilege,grant_option,source,rls\r\n".to_string();
                for i in rows {
                    let g = &data.grants[*i];
                    let object = g.object_label();
                    let values = [
                        g.kind.as_str(),
                        &g.schema,
                        &object,
                        &g.owner,
                        &g.grantor,
                        &g.grantee,
                        &g.privilege,
                        if g.grantable { "true" } else { "false" },
                        g.source(),
                        if g.rls { "true" } else { "false" },
                    ];
                    csv.push_str(&values.iter().map(|v| format!("\"{}\"", v.replace('"', "\"\""))).collect::<Vec<_>>().join(","));
                    csv.push_str("\r\n");
                }
                ui.ctx().copy_text(csv);
            }
        });
        if self.view == View::Defaults {
            w::callout(
                ui,
                p.accent,
                icon::INFO,
                "Default privileges apply to future objects created by the listed owner. They do not change existing objects; global and per-schema defaults combine.",
            );
        }
        if rows.is_empty() {
            w::empty_state(ui, icon::SEARCH, "No privileges match", "Clear filters or include system objects to widen the inventory.");
            return;
        }
        ui.add_space(8.0);
        let table_width = ui.available_width().max(900.0);
        let table_height = (ui.ctx().content_rect().height() * 0.23).clamp(120.0, 240.0);
        egui::ScrollArea::horizontal().id_salt("grants_horizontal").show(ui, |ui| {
            ui.set_width(table_width);
            egui_extras::TableBuilder::new(ui)
                .id_salt("grants_table")
                .striped(true)
                .resizable(true)
                .sense(egui::Sense::click())
                .max_scroll_height(table_height)
                .column(egui_extras::Column::remainder().at_least(200.0).resizable(false).clip(true))
                .columns(egui_extras::Column::initial(125.0).at_least(70.0), 3)
                .column(egui_extras::Column::initial(150.0))
                .column(egui_extras::Column::initial(130.0))
                .header(28.0, |mut header| {
                    for text in ["Object", "Type", "Grantee", "Privilege", "Grantor", "Source"] {
                        header.col(|ui| {
                            ui.label(RichText::new(text).small().strong().color(p.dim));
                        });
                    }
                })
                .body(|body| {
                    body.rows(32.0, rows.len(), |mut row| {
                        let index = rows[row.index()];
                        let g = &data.grants[index];
                        row.set_selected(self.selected == Some(index));
                        let object = if g.future {
                            format!("{} · future {}", if g.schema.is_empty() { "Global" } else { &g.schema }, g.kind.to_lowercase())
                        } else {
                            g.object_label()
                        };
                        for text in
                            [&object, &g.kind, &g.grantee, &format!("{}{}", g.privilege, if g.grantable { " + grant" } else { "" }), &g.grantor, g.source()]
                        {
                            row.col(|ui| {
                                ui.add(egui::Label::new(text).truncate()).on_hover_text(text);
                            });
                        }
                        if row.response().clicked() {
                            self.selected = Some(index);
                        }
                    })
                });
        });
        if let Some(g) = self.selected.and_then(|i| data.grants.get(i)) {
            ui.add_space(14.0);
            w::card(ui, |ui| {
                w::section(ui, "Access trace");
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if g.public {
                        w::pill(ui, role.map(|r| r.0.as_str()).unwrap_or("Every role"), p.accent);
                        ui.label("→");
                    }
                    if !g.public
                        && let Some(route) = routes.get(&g.grantee)
                    {
                        for name in &route.roles {
                            w::pill(ui, name, p.accent);
                            ui.label("→");
                        }
                    } else {
                        w::pill(ui, &g.grantee, p.accent);
                        ui.label("→");
                    }
                    w::pill(ui, &g.privilege, p.success);
                    ui.label("→");
                    ui.label(
                        RichText::new(if g.future {
                            format!("Future {} in {}", g.kind, if g.schema.is_empty() { "all schemas" } else { &g.schema })
                        } else {
                            g.object_label()
                        })
                        .monospace(),
                    );
                });
                if let Some(route) = routes.get(&g.grantee)
                    && !route.inherited
                {
                    w::callout(
                        ui,
                        p.warn,
                        icon::WARN,
                        "Membership path only: NO INHERIT stops automatic access. Check SET ROLE options in Memberships before using this role.",
                    );
                }
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Owner: {}", g.owner));
                    ui.separator();
                    ui.label(format!("Grantor: {}", g.grantor));
                    w::badge(ui, g.source(), p.dim);
                    if g.grantable {
                        w::badge(ui, "WITH GRANT OPTION", p.warn);
                    }
                    if g.rls {
                        w::badge(ui, "RLS enabled", p.warn);
                    }
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if w::small_button(ui,w::Kind::Secondary,icon::COPY,"Copy GRANT SQL").clicked() {ui.ctx().copy_text(g.sql(false));}
                    if w::small_button(ui,w::Kind::Secondary,icon::FILE_CODE,"Review REVOKE SQL").clicked() {
                        actions.push(Action::NewQuery {profile:self.profile_id,db:self.database.clone(),sql:format!("-- Review before running. REVOKE uses RESTRICT (no CASCADE).\n-- Runs as your current SQL role; original grantor: {}\n-- Other grants, ownership or memberships may retain access.\n\n{}",g.grantor.replace(['\n','\r']," "),g.sql(true)),run:false});
                    }
                    if g.rls && w::small_button(ui,w::Kind::Secondary,icon::LOCK,"Inspect RLS policies").clicked() {
                        let table=&g.name;
                        actions.push(Action::NewQuery {profile:self.profile_id,db:self.database.clone(),sql:grants::policies_sql(&g.schema,table),run:true});
                    }
                });
                w::hint(
                    ui,
                    "ACL snapshot, not a change history or a complete authorization check. Schema USAGE, database CONNECT, RLS, ownership and superuser rights also affect access. SQL previews run as your current role.",
                );
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::result::{Column, ResultSet};

    /// Measures real inspector frames with 100k privileges, without a GPU/server.
    #[test]
    #[ignore]
    fn bench_grants_virtualized_frames() {
        let mut rs = ResultSet::new((0..14).map(|i| Column::text(&i.to_string())).collect());
        for i in 0..100_000 {
            let name = format!("table_{i}");
            let identity = format!("shop.{name}");
            rs.push_text_row(&[
                Some("Table"),
                Some("shop"),
                Some(&name),
                Some(&identity),
                Some("owner"),
                Some("owner"),
                Some("readers"),
                Some("SELECT"),
                Some("f"),
                Some("f"),
                Some("f"),
                Some("f"),
                Some("f"),
                Some(""),
            ]);
        }
        let prepare = std::time::Instant::now();
        let data = Inventory::parse(&[rs, ResultSet::default(), ResultSet::default()]).unwrap();
        let prepare_ms = prepare.elapsed().as_secs_f64() * 1000.0;
        let mut tab = GrantsTab::new(1, "benchmark", String::new());
        tab.data = Some(Arc::new(data));
        tab.snapshot = Snapshot::ready_fixture();
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, crate::config::ThemeChoice::Dark);
        let render = |tab: &mut GrantsTab| {
            let mut output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0))), ..Default::default() },
                |ui| tab.content(ui, &mut Vec::new()),
            );
            output.textures_delta.clear();
        };
        render(&mut tab);
        assert_eq!(tab.filtered_rows.len(), 100_000);
        let start = std::time::Instant::now();
        for _ in 0..100 {
            render(&mut tab);
        }
        let cached = start.elapsed().as_secs_f64() * 1000.0 / 100.0;
        tab.search = "no matches here".into();
        render(&mut tab);
        assert!(tab.filtered_rows.is_empty());
        let start = std::time::Instant::now();
        for _ in 0..100 {
            render(&mut tab);
        }
        println!(
            "TRUNQ_BENCH grants rows=100000 prepare_ms={prepare_ms:.3} cached_frame_mean_ms={cached:.3} empty_cached_frame_mean_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0 / 100.0
        );
    }
}
