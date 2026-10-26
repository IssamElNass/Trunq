//! Grouped ACL inventory with readable role routes and reviewable permission SQL.

use super::snapshot::Snapshot;
use crate::action::Action;
use crate::config::Profile;
use crate::db::grants::{self, Grant, Inventory, Route};
use crate::db::session::Promise;
use crate::ui::{
    theme::{self, icon},
    widgets as w,
};
use egui::{RichText, Ui, vec2};
use std::collections::{BTreeSet, HashMap};
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

/// A visible object/recipient pair; every original grant stays individually
/// selectable, including grants of the same privilege from different grantors.
struct GrantGroup {
    rows: Vec<usize>,
    privileges: Vec<String>,
}

#[derive(Default)]
struct Summary {
    objects: usize,
    recipients: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    Direct,
    Inherited,
    Public,
    MembershipOnly,
    RoleGrant,
}

impl Access {
    fn label(self) -> &'static str {
        match self {
            Self::Direct => "Direct",
            Self::Inherited => "Inherited",
            Self::Public => "PUBLIC",
            Self::MembershipOnly => "Not inherited",
            Self::RoleGrant => "Role grant",
        }
    }
    fn color(self) -> egui::Color32 {
        let p = theme::p();
        match self {
            Self::Direct => p.success,
            Self::Inherited => p.accent,
            Self::Public | Self::MembershipOnly => p.warn,
            Self::RoleGrant => p.dim,
        }
    }
}

fn access(grant: &Grant, role: Option<&str>, routes: &HashMap<String, Route>) -> Access {
    if grant.public {
        Access::Public
    } else if role == Some(grant.grantee.as_str()) {
        Access::Direct
    } else if let Some(route) = role.and_then(|_| routes.get(&grant.grantee)) {
        if route.inherited { Access::Inherited } else { Access::MembershipOnly }
    } else {
        Access::RoleGrant
    }
}

fn privilege_description(grant: &Grant) -> &'static str {
    match grant.privilege.as_str() {
        "SELECT" if grant.kind == "Sequence" => "Read the sequence value (currval)",
        "SELECT" => "Read rows or values",
        "INSERT" => "Add new rows",
        "UPDATE" if grant.kind == "Sequence" => "Advance or set the sequence value",
        "UPDATE" => "Change existing rows or values",
        "DELETE" => "Remove rows",
        "TRUNCATE" => "Empty the entire table",
        "REFERENCES" => "Create foreign keys referencing this object",
        "TRIGGER" => "Create triggers on this table",
        "EXECUTE" => "Call this function or procedure",
        "CONNECT" => "Connect to this database",
        "TEMPORARY" => "Create temporary tables in this database",
        "CREATE" if grant.kind == "Database" => "Create schemas and eligible database objects",
        "CREATE" if grant.kind == "Tablespace" => "Create objects in this tablespace",
        "CREATE" => "Create objects in this schema",
        "USAGE" if grant.kind == "Schema" => "Look up objects in this schema",
        "USAGE" if grant.kind == "Sequence" => "Use the sequence (nextval and currval)",
        "USAGE" if grant.kind == "Type" => "Use this type when defining database objects",
        "USAGE" => "Use this object; its own permission rules still apply",
        "MAINTAIN" => "Perform supported table maintenance operations",
        _ => "PostgreSQL object privilege",
    }
}

fn privilege_rank(value: &str) -> usize {
    ["SELECT", "INSERT", "UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER", "USAGE", "CREATE", "EXECUTE", "CONNECT", "TEMPORARY", "MAINTAIN"]
        .iter()
        .position(|p| *p == value)
        .unwrap_or(usize::MAX)
}

fn group_rows(data: &Inventory, rows: &[usize]) -> (Vec<GrantGroup>, Summary) {
    let mut groups: Vec<GrantGroup> = Vec::new();
    let mut by_object_role = HashMap::new();
    let mut objects = BTreeSet::new();
    let mut recipients = BTreeSet::new();
    for &index in rows {
        let g = &data.grants[index];
        let object = (&g.kind, &g.schema, &g.identity, &g.column, &g.owner, g.future);
        objects.insert(object);
        recipients.insert(&g.grantee);
        let key = (object, &g.grantee);
        let group = *by_object_role.entry(key).or_insert_with(|| {
            groups.push(GrantGroup { rows: Vec::new(), privileges: Vec::new() });
            groups.len() - 1
        });
        groups[group].rows.push(index);
    }
    for group in &mut groups {
        group.rows.sort_by(|a, b| {
            let a = &data.grants[*a];
            let b = &data.grants[*b];
            (privilege_rank(&a.privilege), &a.privilege, &a.grantor).cmp(&(privilege_rank(&b.privilege), &b.privilege, &b.grantor))
        });
        for &index in &group.rows {
            let privilege = &data.grants[index].privilege;
            if group.privileges.last() != Some(privilege) {
                group.privileges.push(privilege.clone());
            }
        }
    }
    (groups, Summary { objects: objects.len(), recipients: recipients.len() })
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
    groups: Vec<GrantGroup>,
    summary: Summary,
    filtered_memberships: Vec<usize>,
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
    selected_group: Option<usize>,
    selected_membership: Option<usize>,
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
            groups: Vec::new(),
            summary: Summary::default(),
            filtered_memberships: Vec::new(),
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
            selected_group: None,
            selected_membership: None,
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
                    self.selected_group = None;
                    self.selected_membership = None;
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
            "row" => {
                self.selected = value.parse().ok();
                self.filter_key = None;
            }
            "group" => {
                self.selected_group = value.parse::<usize>().ok().filter(|i| *i < self.groups.len());
                self.selected = self.selected_group.and_then(|i| self.groups[i].rows.first().copied());
            }
            _ => {}
        }
    }

    fn reset_filters(&mut self) {
        self.search.clear();
        self.role = None;
        self.schema = None;
        self.kind = None;
        self.public_only = false;
        self.grantable_only = false;
        self.inherited = true;
        self.system = false;
    }

    fn filters(&mut self, ui: &mut Ui, data: &Inventory) {
        let p = theme::p();
        let role_options: Vec<_> = std::iter::once((None, "All roles")).chain(data.roles.iter().enumerate().map(|(i, r)| (Some(i), r.0.as_str()))).collect();
        let advanced = usize::from(self.schema.is_some())
            + usize::from(self.kind.is_some())
            + usize::from(self.role.is_some() && !self.inherited)
            + usize::from(self.public_only)
            + usize::from(self.grantable_only)
            + usize::from(self.system);
        egui::Frame::NONE.fill(p.bg1).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(8).inner_margin(12).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                ui.label(RichText::new("Inspect role").small().color(p.dim));
                ui.push_id("grants_role", |ui| {
                    w::dropdown(ui, &mut self.role, &role_options, 200.0);
                });
                w::input(ui, &mut self.search, "Find an object, role or permission", Some(icon::SEARCH), ui.available_width().clamp(140.0, 300.0));
                if self.view != View::Memberships {
                    let title = if advanced == 0 { "Filters".into() } else { format!("Filters · {advanced}") };
                    w::menu_button(ui, w::Kind::Secondary, icon::FILTER, &title, |ui| {
                        ui.set_min_width(260.0);
                        w::section(ui, "Scope");
                        let schemas = if self.system { &data.schemas } else { &data.user_schemas };
                        let options: Vec<_> = std::iter::once((None, "All schemas"))
                            .chain(schemas.iter().enumerate().map(|(i, s)| (Some(i), if s.is_empty() { "No schema / global" } else { s.as_str() })))
                            .collect();
                        ui.push_id("grants_schema", |ui| {
                            w::dropdown(ui, &mut self.schema, &options, 260.0);
                        });
                        let options: Vec<_> =
                            std::iter::once((None, "All object types")).chain(data.kinds.iter().enumerate().map(|(i, s)| (Some(i), s.as_str()))).collect();
                        ui.push_id("grants_kind", |ui| {
                            w::dropdown(ui, &mut self.kind, &options, 260.0);
                        });
                        ui.add_space(8.0);
                        ui.add_enabled_ui(self.role.is_some(), |ui| {
                            w::checkbox(ui, &mut self.inherited, "Include memberships and PUBLIC")
                                .on_hover_text("Show reachable role grants too. NO INHERIT paths are marked Not inherited, not automatic access.");
                        })
                        .response
                        .on_disabled_hover_text("Choose a role to trace its memberships and PUBLIC access.");
                        w::checkbox(ui, &mut self.public_only, "PUBLIC grants only");
                        w::checkbox(ui, &mut self.grantable_only, "Can grant to other roles");
                        if w::checkbox(ui, &mut self.system, "Include system objects").changed() {
                            self.schema = None;
                        }
                    });
                }
                if (advanced > 0 || self.role.is_some() || !self.search.is_empty()) && w::small_button(ui, w::Kind::Ghost, icon::X, "Reset").clicked() {
                    self.reset_filters();
                }
            });
            if self.view != View::Memberships && advanced > 0 {
                let schemas = if self.system { &data.schemas } else { &data.user_schemas };
                let mut labels = Vec::new();
                if let Some(schema) = self.schema.and_then(|i| schemas.get(i)) {
                    labels.push(format!("Schema: {}", if schema.is_empty() { "global" } else { schema }));
                }
                if let Some(kind) = self.kind.and_then(|i| data.kinds.get(i)) {
                    labels.push(format!("Type: {kind}"));
                }
                if self.role.is_some() && !self.inherited {
                    labels.push("Direct grants only".into());
                }
                if self.public_only {
                    labels.push("PUBLIC only".into());
                }
                if self.grantable_only {
                    labels.push("Grant option only".into());
                }
                if self.system {
                    labels.push("System objects included".into());
                }
                let text = labels.join("  ·  ");
                ui.add(egui::Label::new(RichText::new(&text).small().color(p.dim)).truncate()).on_hover_text(text);
            }
        });
    }

    fn prepare_view(&mut self, data: &Inventory) {
        let role = self.role.and_then(|i| data.roles.get(i));
        if self.route_role != Some(self.role) {
            self.route_cache = role.map(|r| data.routes(&r.0)).unwrap_or_default();
            self.route_role = Some(self.role);
        }
        let search = self.search.to_lowercase();
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
        if self.filter_key.as_ref() == Some(&key) {
            return;
        }
        let schemas = if self.system { &data.schemas } else { &data.user_schemas };
        self.filtered_rows = data
            .grants
            .iter()
            .enumerate()
            .filter(|(i, g)| {
                g.future == (self.view == View::Defaults)
                    && (self.system || !g.is_system())
                    && self.schema.and_then(|i| schemas.get(i)).is_none_or(|s| **s == g.schema)
                    && self.kind.and_then(|i| data.kinds.get(i)).is_none_or(|s| **s == g.kind)
                    && (!self.public_only || g.public)
                    && (!self.grantable_only || g.grantable)
                    && role.is_none_or(|r| (!g.public && g.grantee == r.0) || (self.inherited && (g.public || self.route_cache.contains_key(&g.grantee))))
                    && (search.is_empty() || data.search_text[*i].contains(&search))
            })
            .map(|(i, _)| i)
            .collect();
        (self.groups, self.summary) = group_rows(data, &self.filtered_rows);
        self.selected_group = self.selected.and_then(|selected| self.groups.iter().position(|group| group.rows.contains(&selected)));
        if self.selected_group.is_none() {
            self.selected_group = (!self.groups.is_empty()).then_some(0);
            self.selected = self.groups.first().and_then(|g| g.rows.first().copied());
        }
        self.filtered_memberships = data
            .memberships
            .iter()
            .enumerate()
            .filter(|(_, m)| role.is_none() || self.route_cache.contains_key(&m.member) || role.is_some_and(|r| r.0 == m.parent))
            .filter(|(_, m)| [&m.member, &m.parent, &m.grantor].iter().any(|s| s.to_lowercase().contains(&search)))
            .map(|(i, _)| i)
            .collect();
        if !self.filtered_memberships.iter().any(|i| Some(*i) == self.selected_membership) {
            self.selected_membership = self.filtered_memberships.first().copied();
        }
        self.filter_key = Some(key);
    }

    fn content(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let p = theme::p();
        w::page_header(ui, icon::KEY, p.accent, "Grants & access", &self.database, |ui| {
            if w::button(ui, w::Kind::Secondary, icon::REFRESH, "Refresh").clicked() {
                self.snapshot.refresh();
                self.preparing = None;
                self.data = None;
                self.selected = None;
            }
        });
        ui.add_space(16.0);
        if let Some(error) = &self.snapshot.error {
            w::callout(ui, p.danger, icon::CIRCLE_X, error);
            return;
        }
        if self.snapshot.loading() || self.preparing.is_some() {
            w::empty_state(ui, icon::LOADER, "Loading permissions", "Reading object grants and role memberships…");
            ui.spinner();
            return;
        }
        let Some(data) = self.data.clone() else { return };
        ui.horizontal_wrapped(|ui| {
            w::segmented(
                ui,
                &mut self.view,
                &[
                    (View::Objects, icon::TABLE, "Object access"),
                    (View::Defaults, icon::PLUS, "Default privileges"),
                    (View::Memberships, icon::USERS, "Memberships"),
                ],
            );
        });
        ui.add_space(12.0);
        self.filters(ui, &data);
        self.prepare_view(&data);
        ui.add_space(12.0);
        let role = self.role.and_then(|i| data.roles.get(i));
        if let Some(role) = role {
            if role.1 || role.2 {
                w::callout(
                    ui,
                    p.warn,
                    icon::SHIELD,
                    if role.1 {
                        "Superuser: access extends beyond the grants listed here."
                    } else {
                        "This role bypasses row security; object privileges still apply."
                    },
                );
            }
        } else {
            w::hint(ui, "All roles are shown. Choose a role above to trace its direct and inherited permissions.");
        }
        if self.view == View::Defaults {
            w::callout(ui, p.accent, icon::INFO, "These rules apply only to future objects created by the listed owner. They do not change existing objects.");
        }
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            let text = if self.view == View::Memberships {
                counted(self.filtered_memberships.len(), "membership")
            } else {
                format!(
                    "{}   ·   {}   ·   {}",
                    counted(self.summary.objects, "object"),
                    counted(self.summary.recipients, "recipient"),
                    counted(self.filtered_rows.len(), "grant record")
                )
            };
            ui.label(RichText::new(text).small().color(p.dim)).on_hover_text(format!(
                "Counts reflect the current filters. Entire snapshot: {} user objects and {} PUBLIC grant records.",
                data.user_object_count, data.public_count
            ));
            if self.view != View::Memberships && !self.filtered_rows.is_empty() && w::small_button(ui, w::Kind::Ghost, icon::COPY, "Export CSV").clicked() {
                ui.ctx().copy_text(self.filtered_csv(&data));
            }
        });
        ui.add_space(8.0);
        let empty = if self.view == View::Memberships { self.filtered_memberships.is_empty() } else { self.groups.is_empty() };
        if empty {
            w::empty_state(ui, icon::SEARCH, "No permissions match", "Try another role or reset the filters.");
            return;
        }
        let wide = ui.available_width() >= 980.0;
        let height = (ui.ctx().content_rect().bottom() - ui.cursor().top() - 40.0).max(220.0);
        if wide {
            let details_width = 330.0;
            let list_width = ui.available_width() - details_width - 16.0;
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(list_width, height), egui::Layout::top_down(egui::Align::Min), |ui| self.list(ui, &data, height));
                ui.add_space(8.0);
                ui.allocate_ui_with_layout(vec2(details_width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("grants_details")
                        .max_height(height)
                        .auto_shrink([false, true])
                        .show(ui, |ui| self.details(ui, &data, actions));
                });
            });
        } else {
            self.list(ui, &data, height.clamp(160.0, 300.0));
            ui.add_space(12.0);
            self.details(ui, &data, actions);
        }
    }

    fn list(&mut self, ui: &mut Ui, data: &Inventory, height: f32) {
        if self.view == View::Memberships {
            self.membership_list(ui, data, height);
            return;
        }
        let p = theme::p();
        let role = self.role.and_then(|i| data.roles.get(i)).map(|r| r.0.as_str());
        let compact = ui.available_width() < 700.0;
        let widths = if compact { [0.38, 0.31, 0.31, 0.0] } else { [0.32, 0.24, 0.26, 0.18] };
        let headings: &[&str] =
            if compact { &["Object / scope", "Granted to / access", "Permissions"] } else { &["Object / scope", "Granted to", "Permissions", "Access path"] };
        table_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            let row_width = table_header(ui, headings, widths);
            egui::ScrollArea::vertical()
                .id_salt("grants_grouped_rows")
                .max_height((height - 50.0).max(64.0))
                .auto_shrink([false, true])
                .min_scrolled_height(0.0)
                .show_rows(ui, 64.0, self.groups.len(), |ui, visible| {
                    for group_index in visible {
                        let group = &self.groups[group_index];
                        let g = &data.grants[group.rows[0]];
                        let (rect, response) = ui.allocate_exact_size(vec2(row_width, 64.0), egui::Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::Role::ListBoxOption,
                                ui.is_enabled(),
                                self.selected_group == Some(group_index),
                                format!("{}, {}, {}", g.object_label(), g.grantee, group.privileges.join(", ")),
                            )
                        });
                        paint_row(ui, rect, self.selected_group == Some(group_index), response.hovered());
                        let cells = table_cells(rect, widths);
                        let mut detail = if g.future {
                            format!("{} · created by {}", if g.schema.is_empty() { "All schemas" } else { &g.schema }, g.owner)
                        } else {
                            format!("{}{}", g.kind, if g.schema.is_empty() { String::new() } else { format!(" · {}", g.schema) })
                        };
                        if g.rls {
                            detail = format!("RLS · {detail}");
                        }
                        cell_text(ui, cells[0], &object_title(g), Some((&detail, p.dim)), p.text, true);
                        ui.interact(cells[0], response.id.with("object"), egui::Sense::hover()).on_hover_text(g.object_label());
                        let source = access(g, role, &self.route_cache);
                        let subtitle = if compact {
                            Some((if g.public { "Every role" } else { source.label() }, source.color()))
                        } else if g.public {
                            Some(("Every role", p.dim))
                        } else {
                            None
                        };
                        cell_text(ui, cells[1], &g.grantee, subtitle, if g.public { p.warn } else { p.text }, false);
                        ui.interact(cells[1], response.id.with("recipient"), egui::Sense::hover()).on_hover_text(&g.grantee);
                        permission_badges(ui, cells[2], &group.privileges);
                        ui.interact(cells[2], response.id.with("permissions"), egui::Sense::hover()).on_hover_text(group.privileges.join(" · "));
                        if !compact {
                            paint_badge(ui, cells[3], source.label(), source.color());
                        }
                        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.selected_group = Some(group_index);
                            self.selected = group.rows.first().copied();
                        }
                    }
                });
        });
    }

    fn membership_list(&mut self, ui: &mut Ui, data: &Inventory, height: f32) {
        let p = theme::p();
        let widths = [0.36, 0.36, 0.28, 0.0];
        table_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            let row_width = table_header(ui, &["Member", "Member of role", "Uses permissions"], widths);
            egui::ScrollArea::vertical()
                .id_salt("membership_rows")
                .max_height((height - 50.0).max(64.0))
                .auto_shrink([false, true])
                .min_scrolled_height(0.0)
                .show_rows(ui, 56.0, self.filtered_memberships.len(), |ui, visible| {
                    for row in visible {
                        let index = self.filtered_memberships[row];
                        let m = &data.memberships[index];
                        let (rect, response) = ui.allocate_exact_size(vec2(row_width, 56.0), egui::Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::Role::ListBoxOption,
                                ui.is_enabled(),
                                self.selected_membership == Some(index),
                                format!("{} is a member of {}", m.member, m.parent),
                            )
                        });
                        paint_row(ui, rect, self.selected_membership == Some(index), response.hovered());
                        let cells = table_cells(rect, widths);
                        cell_text(ui, cells[0], &m.member, None, p.text, false);
                        cell_text(ui, cells[1], &m.parent, None, p.text, false);
                        paint_badge(ui, cells[2], if m.inherit { "Automatically" } else { "Not inherited" }, if m.inherit { p.success } else { p.warn });
                        ui.interact(cells[0], response.id.with("member"), egui::Sense::hover()).on_hover_text(&m.member);
                        ui.interact(cells[1], response.id.with("parent"), egui::Sense::hover()).on_hover_text(&m.parent);
                        if response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.selected_membership = Some(index);
                        }
                    }
                });
        });
    }

    fn details(&mut self, ui: &mut Ui, data: &Inventory, actions: &mut Vec<Action>) {
        let p = theme::p();
        if self.view == View::Memberships {
            let Some(m) = self.selected_membership.and_then(|i| data.memberships.get(i)) else {
                return;
            };
            w::card(ui, |ui| {
                w::section(ui, "Role membership");
                ui.add_space(10.0);
                field(ui, "Member", &m.member);
                field(ui, "Member of", &m.parent);
                ui.separator();
                for (label, enabled, explanation) in [
                    (
                        "Inherit permissions",
                        m.inherit,
                        if m.inherit {
                            "The member automatically receives this role's object privileges."
                        } else {
                            "This membership does not pass object privileges automatically."
                        },
                    ),
                    (
                        "Switch to role",
                        m.set_role,
                        if m.set_role {
                            "SET ROLE is allowed on this edge. Every edge of a longer path must allow it too."
                        } else {
                            "This membership does not allow switching to the role."
                        },
                    ),
                    (
                        "Manage membership",
                        m.admin,
                        if m.admin {
                            "The member can grant or revoke membership in this role."
                        } else {
                            "This grant does not allow managing the role's membership."
                        },
                    ),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(label).color(p.dim));
                        w::pill(ui, if enabled { "Yes" } else { "No" }, if enabled { p.success } else { p.faint });
                    });
                    ui.add(egui::Label::new(RichText::new(explanation).small().color(p.dim)).wrap());
                    ui.add_space(10.0);
                }
                ui.separator();
                field(ui, "Granted by", &m.grantor);
                w::hint(ui, "These are the options on this direct membership. A longer route may have different options on each edge.");
            });
            return;
        }
        let Some(group) = self.selected_group.and_then(|i| self.groups.get(i)) else {
            return;
        };
        let Some(g) = self.selected.and_then(|i| data.grants.get(i)) else {
            return;
        };
        let role = self.role.and_then(|i| data.roles.get(i)).map(|r| r.0.as_str());
        let source = access(g, role, &self.route_cache);
        w::card(ui, |ui| {
            w::section(ui, "Permission details");
            ui.add_space(10.0);
            ui.add(egui::Label::new(RichText::new(object_title(g)).font(theme::bold(16.0)).color(p.text)).wrap());
            ui.add(egui::Label::new(RichText::new(g.object_label()).small().monospace().color(p.dim)).wrap());
            ui.add_space(8.0);
            field(ui, "Granted to", if g.public { "PUBLIC · every role" } else { &g.grantee });
            ui.add_space(8.0);
            w::pill(ui, source.label(), source.color());
            match source {
                Access::Direct => {
                    w::hint(ui, "This grant names the selected role directly.");
                }
                Access::Inherited | Access::MembershipOnly => {
                    w::hint(
                        ui,
                        if source == Access::Inherited {
                            "Received through this membership path:"
                        } else {
                            "This path does not inherit permissions automatically:"
                        },
                    );
                    if let Some(route) = self.route_cache.get(&g.grantee) {
                        for (step, name) in route.roles.iter().enumerate() {
                            ui.add(
                                egui::Label::new(RichText::new(format!("{}  {name}", if step == 0 { icon::USER } else { icon::ARROW_RIGHT })).color(p.text))
                                    .truncate(),
                            )
                            .on_hover_text(name);
                        }
                    }
                    if source == Access::MembershipOnly {
                        w::hint(ui, "Check the SET ROLE options in Memberships; this path alone does not prove usable access.");
                    }
                }
                Access::Public => {
                    w::hint(ui, "PUBLIC grants apply to every role, without requiring membership.");
                }
                Access::RoleGrant => {
                    w::hint(ui, "Choose a role above to trace how it receives this grant.");
                }
            }
            ui.add_space(12.0);
            ui.separator();
            w::section(ui, "Permissions · select to inspect");
            ui.add_space(8.0);
            for &index in &group.rows {
                let grant = &data.grants[index];
                let title = if group.rows.iter().filter(|i| data.grants[**i].privilege == grant.privilege).count() > 1 {
                    format!("{} · {}", grant.privilege, grant.grantor)
                } else {
                    grant.privilege.clone()
                };
                let detail = format!("{}{}", privilege_description(grant), if grant.grantable { " · can grant onward" } else { "" });
                let response = w::list_item(ui, false, icon::KEY, &title, &detail);
                if self.selected == Some(index) {
                    ui.painter().rect_stroke(response.rect, 8.0, egui::Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
                    ui.painter().line_segment(
                        [response.rect.left_top() + vec2(1.5, 9.0), response.rect.left_bottom() + vec2(1.5, -9.0)],
                        egui::Stroke::new(3.0, p.accent),
                    );
                }
                if response.clicked() {
                    self.selected = Some(index);
                }
                ui.add_space(4.0);
            }
            ui.add_space(8.0);
            field(ui, if g.future { "Created by" } else { "Owner" }, &g.owner);
            field(ui, "Granted by", &g.grantor);
            field(ui, "Recorded in", g.source());
            if g.grantable {
                ui.add_space(6.0);
                w::pill(ui, "Can grant this permission to others", p.warn);
            }
            if g.rls {
                ui.add_space(6.0);
                w::pill(ui, "Row security policies apply", p.warn);
            }
            ui.add_space(12.0);
            ui.horizontal_wrapped(|ui| {
                if w::small_button(ui,w::Kind::Secondary,icon::COPY,"Copy GRANT SQL").clicked() { ui.ctx().copy_text(g.sql(false)); }
                if w::small_button(ui,w::Kind::Ghost,icon::FILE_CODE,"Review REVOKE").clicked() {
                    actions.push(Action::NewQuery { profile:self.profile_id, db:self.database.clone(), sql:format!("-- Review before running. REVOKE uses RESTRICT (no CASCADE).\n-- Original grantor: {}. Runs as your current SQL role.\n-- Other grants, ownership or memberships may retain access.\n\n{}",g.grantor.replace(['\n','\r']," "),g.sql(true)), run:false });
                }
                if g.rls && w::small_button(ui,w::Kind::Ghost,icon::LOCK,"Inspect row policies").clicked() {
                    actions.push(Action::NewQuery { profile:self.profile_id, db:self.database.clone(), sql:grants::policies_sql(&g.schema,&g.name), run:true });
                }
            });
            ui.add_space(8.0);
            egui::CollapsingHeader::new("How to read this snapshot").id_salt("grants_help").show(ui,|ui| {
                w::hint(ui,"Permissions are grouped for reading; SQL actions target only the selected privilege and recipient. Grant option means it can be passed to others.");
                w::hint(ui,"Object ACL is a stored grant. Built-in ACL is PostgreSQL's initial default for this object. Default privileges are separate rules for future objects.");
                w::hint(ui,"This is not a complete authorization check or change history. Database CONNECT, schema USAGE, row security, ownership, superuser and predefined-role rights also affect access.");
            });
        });
    }

    fn filtered_csv(&self, data: &Inventory) -> String {
        let mut csv = "type,schema,object,owner,grantor,grantee,privilege,grant_option,source,rls\r\n".to_string();
        for &index in &self.filtered_rows {
            let g = &data.grants[index];
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
        csv
    }
}

fn counted(value: usize, label: &str) -> String {
    format!("{value} {label}{}", if value == 1 { "" } else { "s" })
}

fn table_frame() -> egui::Frame {
    let p = theme::p();
    egui::Frame::NONE.fill(p.bg1).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(10).inner_margin(8)
}

fn table_cells(rect: egui::Rect, widths: [f32; 4]) -> [egui::Rect; 4] {
    let mut left = rect.left();
    widths.map(|fraction| {
        let width = rect.width() * fraction;
        let cell = egui::Rect::from_min_size(egui::pos2(left, rect.top()), vec2(width, rect.height()));
        left += width;
        cell
    })
}

fn table_header(ui: &mut Ui, headings: &[&str], widths: [f32; 4]) -> f32 {
    // The same inset and column geometry is used for headers and virtual rows.
    let width = ui.available_width() - ui.spacing().scroll.allocated_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), egui::Sense::hover());
    let cells = table_cells(rect, widths);
    for (index, title) in headings.iter().enumerate() {
        let g = w::truncated(ui, title, theme::bold(11.5), theme::p().dim, (cells[index].width() - 24.0).max(1.0));
        ui.painter().galley(egui::pos2(cells[index].left() + 12.0, cells[index].center().y - g.size().y / 2.0), g, theme::p().dim);
    }
    ui.painter().hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1.0, theme::p().border));
    width
}

fn paint_row(ui: &Ui, rect: egui::Rect, selected: bool, hovered: bool) {
    let p = theme::p();
    let rect = rect.shrink(0.5);
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if selected {
        painter.rect_filled(rect, 7.0, p.bg2);
        painter.rect_stroke(rect, 7.0, egui::Stroke::new(1.0, p.border_strong), egui::StrokeKind::Inside);
        painter.line_segment([rect.left_top() + vec2(2.0, 11.0), rect.left_bottom() + vec2(2.0, -11.0)], egui::Stroke::new(3.0, p.accent));
    } else if hovered {
        painter.rect_filled(rect, 7.0, p.control_hover);
    }
}

/// Lay out the complete text block around the cell's vertical centre, rather
/// than letting a nested layout grow beyond a virtual row's fixed height.
fn cell_text(ui: &Ui, rect: egui::Rect, title: &str, detail: Option<(&str, egui::Color32)>, color: egui::Color32, strong: bool) {
    let font = if strong { theme::bold(13.0) } else { egui::FontId::proportional(13.0) };
    let width = (rect.width() - 24.0).max(1.0);
    let title = w::truncated(ui, title, font, color, width);
    let detail = detail.map(|(text, color)| (w::truncated(ui, text, egui::FontId::proportional(11.0), color, width), color));
    let block_height = title.size().y + detail.as_ref().map_or(0.0, |(g, _)| g.size().y + 4.0);
    let top = rect.center().y - block_height / 2.0;
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let title_height = title.size().y;
    painter.galley(egui::pos2(rect.left() + 12.0, top), title, color);
    if let Some((detail, color)) = detail {
        painter.galley(egui::pos2(rect.left() + 12.0, top + title_height + 4.0), detail, color);
    }
}

fn paint_badge(ui: &Ui, rect: egui::Rect, text: &str, color: egui::Color32) {
    let text = w::truncated(ui, text, egui::FontId::proportional(11.0), color, (rect.width() - 38.0).max(1.0));
    let badge = egui::Rect::from_min_size(egui::pos2(rect.left() + 12.0, rect.center().y - 11.0), vec2(text.size().x + 14.0, 22.0));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(badge, 5.0, color.gamma_multiply(0.13));
    painter.galley(egui::pos2(badge.left() + 7.0, badge.center().y - text.size().y / 2.0), text, color);
}

fn permission_badges(ui: &Ui, rect: egui::Rect, privileges: &[String]) {
    let p = theme::p();
    let mut left = rect.left();
    for (i, privilege) in privileges.iter().enumerate() {
        let color = if privilege == "SELECT" || privilege == "CONNECT" { p.accent } else { p.text };
        let width = ui.fonts_mut(|fonts| fonts.layout_no_wrap(privilege.clone(), egui::FontId::proportional(11.0), color).size().x) + 14.0;
        let more_width = if i + 1 < privileges.len() { 40.0 } else { 0.0 };
        let remaining = rect.right() - left - 24.0;
        let cell = egui::Rect::from_min_max(egui::pos2(left, rect.top()), rect.right_bottom());
        if width + more_width > remaining {
            paint_badge(ui, cell, &format!("+{}", privileges.len() - i), p.dim);
            break;
        }
        paint_badge(ui, cell, privilege, color);
        left += width + 6.0;
    }
}

fn object_title(g: &Grant) -> String {
    if g.future {
        format!("Future {}s", g.kind.to_lowercase())
    } else if g.kind == "Column" {
        format!("{} · {}", g.name, g.column)
    } else if matches!(g.kind.as_str(), "Function" | "Procedure") {
        g.identity.clone()
    } else {
        g.name.clone()
    }
}

fn field(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).small().color(theme::p().dim));
        ui.add(egui::Label::new(RichText::new(value).small().color(theme::p().text)).truncate()).on_hover_text(value);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::result::{Column, ResultSet};

    fn grant(privilege: &str, grantee: &str) -> Grant {
        Grant {
            kind: "Table".into(),
            schema: "shop".into(),
            name: "orders".into(),
            identity: "shop.orders".into(),
            owner: "owner".into(),
            grantor: "owner".into(),
            grantee: grantee.into(),
            privilege: privilege.into(),
            grantable: false,
            builtin: false,
            rls: false,
            future: false,
            public: false,
            column: String::new(),
        }
    }

    fn inventory(grants: &[Grant]) -> Inventory {
        let mut rows = ResultSet::new((0..14).map(|i| Column::text(&i.to_string())).collect());
        let boolean = |value| if value { "t" } else { "f" };
        for g in grants {
            rows.push_text_row(&[
                Some(&g.kind),
                Some(&g.schema),
                Some(&g.name),
                Some(&g.identity),
                Some(&g.owner),
                Some(&g.grantor),
                Some(&g.grantee),
                Some(&g.privilege),
                Some(boolean(g.grantable)),
                Some(boolean(g.builtin)),
                Some(boolean(g.rls)),
                Some(boolean(g.future)),
                Some(boolean(g.public)),
                Some(&g.column),
            ]);
        }
        let mut roles = ResultSet::with_ncols(3);
        for role in ["analyst", "readers", "blocked", "other"] {
            roles.push_text_row(&[Some(role), Some("f"), Some("f")]);
        }
        let mut memberships = ResultSet::with_ncols(6);
        memberships.push_text_row(&[Some("analyst"), Some("readers"), Some("owner"), Some("f"), Some("t"), Some("t")]);
        memberships.push_text_row(&[Some("analyst"), Some("blocked"), Some("owner"), Some("f"), Some("f"), Some("f")]);
        Inventory::parse(&[rows, roles, memberships]).unwrap()
    }

    #[test]
    fn grouping_keeps_column_scopes_default_creators_and_grantors_distinct() {
        let select = grant("SELECT", "readers");
        let update = grant("UPDATE", "readers");
        let duplicate = Grant { grantor: "second_owner".into(), grantable: true, ..select.clone() };
        let other_role = grant("SELECT", "other");
        let column = Grant { kind: "Column".into(), column: "email".into(), ..select.clone() };
        let another_column = Grant { column: "phone".into(), ..column.clone() };
        let future = Grant { future: true, identity: String::new(), ..select.clone() };
        let another_owner = Grant { owner: "second_owner".into(), ..future.clone() };
        let data = inventory(&[select, update, duplicate, other_role, column, another_column, future, another_owner]);
        let (groups, summary) = group_rows(&data, &(0..8).collect::<Vec<_>>());
        assert_eq!(groups.len(), 6);
        assert_eq!(groups[0].rows.len(), 3, "same privilege from two grantors must remain inspectable");
        assert_eq!(groups[0].privileges, ["SELECT", "UPDATE"]);
        assert_eq!(summary.objects, 5);
        assert_eq!(summary.recipients, 2);
        assert_eq!(groups.iter().map(|g| g.rows.len()).sum::<usize>(), 8);
    }

    #[test]
    fn access_labels_do_not_call_noinherit_or_public_role_names_inherited() {
        let data = inventory(&[]);
        let routes = data.routes("analyst");
        assert_eq!(access(&grant("SELECT", "analyst"), Some("analyst"), &routes), Access::Direct);
        assert_eq!(access(&grant("SELECT", "readers"), Some("analyst"), &routes), Access::Inherited);
        assert_eq!(access(&grant("SELECT", "blocked"), Some("analyst"), &routes), Access::MembershipOnly);
        assert_eq!(access(&grant("SELECT", "readers"), None, &routes), Access::RoleGrant);
        let public = Grant { public: true, ..grant("SELECT", "PUBLIC") };
        assert_eq!(access(&public, Some("analyst"), &routes), Access::Public);
        assert_eq!(access(&grant("SELECT", "PUBLIC"), Some("analyst"), &routes), Access::RoleGrant);
    }

    #[test]
    fn filtering_and_csv_preserve_the_exact_matching_grants() {
        let data = inventory(&[
            grant("SELECT", "readers"),
            grant("UPDATE", "readers"),
            grant("SELECT", "blocked"),
            grant("SELECT", "analyst"),
            Grant { public: true, ..grant("SELECT", "PUBLIC") },
            Grant { future: true, ..grant("SELECT", "readers") },
        ]);
        let mut tab = GrantsTab::new(1, "test", String::new());
        tab.role = Some(0);
        tab.prepare_view(&data);
        assert_eq!(tab.filtered_rows.len(), 5);
        assert_eq!(tab.groups.len(), 4);
        tab.search = "UPDATE".into();
        tab.prepare_view(&data);
        assert_eq!(tab.filtered_rows, [1]);
        assert_eq!(tab.groups[0].privileges, ["UPDATE"]);
        assert_eq!(tab.selected, Some(1));
        assert_eq!(tab.filtered_csv(&data).lines().count(), 2);
        assert!(tab.filtered_csv(&data).contains("\"UPDATE\""));
        tab.search.clear();
        tab.inherited = false;
        tab.prepare_view(&data);
        assert_eq!(tab.filtered_rows, [3]);
        tab.inherited = true;
        tab.view = View::Defaults;
        tab.prepare_view(&data);
        assert_eq!(tab.filtered_rows, [5]);
        assert_eq!(tab.selected, Some(5));
    }

    #[test]
    fn grouped_permissions_render_without_horizontal_page_overflow() {
        let mut long = grant("SELECT", "a_very_long_group_name_for_customer_reporting");
        long.name = "a_very_long_object_name_for_customer_reporting".into();
        long.identity = format!("shop.{}", long.name);
        let data = Arc::new(inventory(&[long, grant("INSERT", "readers"), grant("UPDATE", "readers")]));
        for view in [View::Objects, View::Defaults, View::Memberships] {
            for width in [480.0, 1200.0] {
                let mut tab = GrantsTab::new(1, "sample", String::new());
                tab.data = Some(data.clone());
                tab.snapshot = Snapshot::ready_fixture();
                tab.view = view;
                let context = egui::Context::default();
                theme::install_fonts(&context);
                let mut output = context.run_ui(
                    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 900.0))), ..Default::default() },
                    |ui| {
                        tab.content(ui, &mut Vec::new());
                        assert!(ui.min_rect().right() <= width + 1.0, "page expanded at {width}");
                    },
                );
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn table_text_and_badges_are_centered_and_inside_their_cells() {
        let data = inventory(&[grant("SELECT", "readers")]);
        let mut tab = GrantsTab::new(1, "sample", String::new());
        tab.prepare_view(&data);
        let context = egui::Context::default();
        theme::install_fonts(&context);
        let mut output = context
            .run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 350.0))), ..Default::default() }, |ui| {
                tab.list(ui, &data, 250.0)
            });
        let bounds = |value: &str| {
            let (clip, text) = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == value => Some((shape.clip_rect, text)),
                    _ => None,
                })
                .expect("table cell text must be visible");
            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
            assert!(clip.contains_rect(rect), "{value} is clipped: {rect:?} in {clip:?}");
            rect
        };
        let object = bounds("orders").union(bounds("Table · shop"));
        let recipient = bounds("readers");
        let permission = bounds("SELECT");
        assert!((object.center().y - recipient.center().y).abs() <= 0.5);
        assert!((permission.center().y - recipient.center().y).abs() <= 0.5);
        output.textures_delta.clear();
    }

    #[test]
    fn clicking_a_group_selects_its_original_grant_for_sql_actions() {
        let data =
            inventory(&[grant("SELECT", "readers"), Grant { name: "invoices".into(), identity: "shop.invoices".into(), ..grant("UPDATE", "invoice editors") }]);
        let mut tab = GrantsTab::new(1, "sample", String::new());
        tab.prepare_view(&data);
        let context = egui::Context::default();
        let input =
            |events| egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 350.0))), events, ..Default::default() };
        let mut output = context.run_ui(input(Vec::new()), |ui| tab.list(ui, &data, 250.0));
        let target = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "invoices" => Some(text.pos + text.galley.size() / 2.0),
                _ => None,
            })
            .unwrap();
        output.textures_delta.clear();
        for pressed in [true, false] {
            let mut output = context.run_ui(
                input(vec![
                    egui::Event::PointerMoved(target),
                    egui::Event::PointerButton { pos: target, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE },
                ]),
                |ui| tab.list(ui, &data, 250.0),
            );
            output.textures_delta.clear();
        }
        assert_eq!(tab.selected_group, Some(1));
        assert_eq!(tab.selected, Some(1));
        assert_eq!(data.grants[tab.selected.unwrap()].sql(true), "REVOKE UPDATE ON TABLE shop.invoices FROM \"invoice editors\";");
    }

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
