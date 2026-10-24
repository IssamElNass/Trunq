//! Application shell: sidebar object browser, tabs, status bar, dialogs.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Align, Align2, Color32, FontId, Layout, RichText, Sense, Stroke, Ui, pos2, vec2};

use crate::action::{Action, ToastKind};
use crate::config::{Config, History, Profile, SavedTab, ThemeChoice};
use crate::db::azure::{self, SignInPrompt};
use crate::db::meta::{self, Catalog, RelKind};
use crate::db::result::ResultSet;
use crate::db::session::{self, Promise, Session};
use crate::tabs::query::{QueryTab, TabCtx};
use crate::ui::conn_dialog::{ConnDialog, DialogOutcome};
use crate::ui::theme::{self, icon};
use crate::ui::widgets as w;
use crate::util;

type BatchPromise = Promise<Result<Vec<ResultSet>, String>>;

struct DbInfo {
    name: String,
    size: String,
    can_connect: bool,
}

#[derive(Default)]
struct DbNode {
    session: Option<Session>,
    catalog: Option<Arc<Catalog>>,
    loading: Option<BatchPromise>,
    error: Option<String>,
    expanded: bool,
    /// The user explicitly expanded/collapsed this database.
    toggled: bool,
    open: HashSet<String>,
}

struct Server {
    profile_id: u64,
    expanded: bool,
    root: Option<Session>,
    dbs_promise: Option<BatchPromise>,
    dbs: Option<Vec<DbInfo>>,
    nodes: HashMap<String, DbNode>,
    error: Option<String>,
    password_input: String,
    /// Roles and tablespaces (loaded once per connection, separately from the
    /// database list so a restricted server can't break the browser).
    objects_promise: Option<BatchPromise>,
    roles: Option<Vec<meta::RoleInfo>>,
    /// (name, owner, location)
    tablespaces: Vec<(String, String, String)>,
    /// Expanded server-level groups ("roles", "tablespaces", ...).
    open: HashSet<String>,
}

impl Server {
    fn new(profile_id: u64) -> Server {
        Server {
            profile_id,
            expanded: false,
            root: None,
            dbs_promise: None,
            dbs: None,
            nodes: HashMap::new(),
            error: None,
            password_input: String::new(),
            objects_promise: None,
            roles: None,
            tablespaces: Vec::new(),
            open: HashSet::new(),
        }
    }
    fn disconnect(&mut self) {
        self.root = None;
        self.dbs = None;
        self.dbs_promise = None;
        self.nodes.clear();
        self.objects_promise = None;
        self.roles = None;
        self.tablespaces.clear();
        self.error = None;
    }
    fn is_connected(&self) -> bool {
        self.root.as_ref().is_some_and(|s| s.connected && s.is_alive())
    }
}

// Only a handful of tabs exist at a time, so boxing the large variant buys nothing.
#[allow(clippy::large_enum_variant)]
pub enum Tab {
    Query(QueryTab),
    Health(crate::tabs::health::HealthTab),
    Sessions(crate::tabs::sessions::SessionsTab),
    Structure(crate::tabs::structure::StructureTab),
    Erd(crate::tabs::erd::ErdTab),
}

impl Tab {
    fn profile(&self) -> Option<u64> {
        match self {
            Tab::Query(q) => q.profile_id,
            Tab::Health(h) => Some(h.profile_id),
            Tab::Sessions(s) => Some(s.profile_id),
            Tab::Structure(s) => Some(s.profile_id),
            Tab::Erd(e) => Some(e.profile_id),
        }
    }
}

pub struct App {
    cfg: Config,
    history: History,
    servers: Vec<Server>,
    tabs: Vec<Tab>,
    active: usize,
    dialog: Option<ConnDialog>,
    toasts: Vec<(ToastKind, String, Instant)>,
    passwords: HashMap<u64, String>,
    sidebar_filter: String,
    show_connections: bool,
    show_history: bool,
    history_filter: String,
    show_settings: bool,
    confirm_delete: Option<u64>,
    logo: Option<egui::TextureHandle>,
    tree_sel: String,
    pending_ddl: Vec<(u64, BatchPromise)>,
    palette: Option<(String, usize)>,
    import: Option<crate::ui::import::ImportDialog>,
    pending_actions: Vec<Action>,
    /// DDL being fetched for "Copy DDL": (label, result)
    pending_copy: Vec<(String, BatchPromise)>,
    /// Settings changed in a way that only needs a debounced save.
    dirty_config: bool,
    last_text_hash: u64,
    text_changed_at: Option<Instant>,
    last_save: Instant,
    sidebar_open: bool,
    script: Option<crate::devscript::Script>,
    bench_frames: u32,
    bench_reported: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> App {
        theme::install_fonts(&cc.egui_ctx);
        let cfg = Config::load();
        theme::apply(&cc.egui_ctx, cfg.settings.theme);
        cc.egui_ctx.set_zoom_factor(cfg.settings.ui_scale.clamp(0.6, 2.0));
        let open: HashSet<&str> = cfg.tree_open.iter().map(String::as_str).collect();
        let servers = cfg
            .profiles
            .iter()
            .map(|p| {
                let mut s = Server::new(p.id);
                s.expanded = open.contains(format!("s{}", p.id).as_str()) || cfg.tree_open.is_empty();
                let dprefix = format!("d{}/", p.id);
                let oprefix = format!("o{}/", p.id);
                for k in &cfg.tree_open {
                    if let Some(db) = k.strip_prefix(&dprefix) {
                        let n = s.nodes.entry(db.to_string()).or_default();
                        n.expanded = true;
                        n.toggled = true;
                    } else if let Some(rest) = k.strip_prefix(&oprefix)
                        && let Some((db, key)) = rest.split_once('/')
                    {
                        s.nodes.entry(db.to_string()).or_default().open.insert(key.to_string());
                    }
                }
                s
            })
            .collect();
        let mut tabs: Vec<Tab> = cfg
            .tabs
            .iter()
            .map(|t| {
                let mut q = QueryTab::new(t.profile_id, &t.database, t.sql.clone(), cfg.settings.editor_font_size);
                q.title = t.title.clone();
                q.file = t.file.clone();
                if let Some(f) = &t.file {
                    q.saved_text = std::fs::read_to_string(f).unwrap_or_default();
                }
                Tab::Query(q)
            })
            .collect();
        tabs.retain(|t| matches!(t, Tab::Query(_)));
        let active = cfg.active_tab.min(tabs.len().saturating_sub(1));
        let logo = load_logo(&cc.egui_ctx, cfg.settings.theme == ThemeChoice::Dark);
        App {
            cfg,
            history: History::load(),
            servers,
            tabs,
            active,
            dialog: None,
            toasts: Vec::new(),
            passwords: HashMap::new(),
            sidebar_filter: String::new(),
            show_connections: false,
            show_history: false,
            history_filter: String::new(),
            show_settings: false,
            confirm_delete: None,
            logo,
            tree_sel: String::new(),
            pending_ddl: Vec::new(),
            palette: None,
            import: None,
            pending_actions: Vec::new(),
            pending_copy: Vec::new(),
            dirty_config: false,
            last_text_hash: 0,
            text_changed_at: None,
            last_save: Instant::now(),
            sidebar_open: true,
            script: crate::devscript::Script::from_env(),
            bench_frames: 0,
            bench_reported: false,
        }
    }

    fn profile(&self, id: u64) -> Option<&Profile> {
        self.cfg.profiles.iter().find(|p| p.id == id)
    }

    fn catalog(&self, profile: Option<u64>, db: &str) -> Option<Arc<Catalog>> {
        let s = self.servers.iter().find(|s| Some(s.profile_id) == profile)?;
        s.nodes.get(db)?.catalog.clone()
    }

    fn toast(&mut self, kind: ToastKind, msg: impl Into<String>) {
        self.toasts.push((kind, msg.into(), Instant::now()));
    }

    fn save_config(&mut self) {
        if self.script.is_some() || std::env::var_os("TRUNQ_BENCH").is_some() {
            return;
        }
        self.cfg.tabs = self
            .tabs
            .iter()
            .filter_map(|t| match t {
                Tab::Query(q) if q.table.is_none() => Some(SavedTab {
                    title: q.title.clone(),
                    profile_id: q.profile_id,
                    database: q.database.clone(),
                    sql: q.editor.text.clone(),
                    file: q.file.clone(),
                }),
                _ => None,
            })
            .collect();
        self.cfg.active_tab = self.active;
        let mut open = Vec::new();
        for s in &self.servers {
            if s.expanded {
                open.push(format!("s{}", s.profile_id));
            }
            for (db, n) in &s.nodes {
                if n.expanded {
                    open.push(format!("d{}/{db}", s.profile_id));
                }
                for k in &n.open {
                    open.push(format!("o{}/{db}/{k}", s.profile_id));
                }
            }
        }
        open.sort();
        self.cfg.tree_open = open;
        if let Err(e) = self.cfg.save() {
            self.toast(ToastKind::Error, format!("Could not save settings: {e}"));
        }
    }

    fn new_query(&mut self, profile: Option<u64>, db: Option<String>, sql: String) -> usize {
        let profile = profile.or_else(|| self.tabs.get(self.active).and_then(|t| t.profile())).or_else(|| self.cfg.profiles.first().map(|p| p.id));
        let db = db
            .or_else(|| match self.tabs.get(self.active) {
                Some(Tab::Query(q)) if q.profile_id == profile => Some(q.database.clone()),
                _ => None,
            })
            .or_else(|| profile.and_then(|p| self.profile(p)).map(|p| p.database.clone()))
            .unwrap_or_else(|| "postgres".into());
        let mut q = QueryTab::new(profile, &db, sql, self.cfg.settings.editor_font_size);
        let n = self.tabs.iter().filter(|t| matches!(t, Tab::Query(q) if q.table.is_none())).count() + 1;
        q.title = format!("Query {n}");
        self.tabs.push(Tab::Query(q));
        self.active = self.tabs.len() - 1;
        self.active
    }

    fn handle(&mut self, actions: Vec<Action>, ui: &Ui) {
        for a in actions {
            match a {
                Action::NewQuery { profile, db, sql, run } => {
                    let i = self.new_query(Some(profile), Some(db), sql);
                    if run && let Some(Tab::Query(q)) = self.tabs.get_mut(i) {
                        let catalog =
                            self.servers.iter().find(|s| s.profile_id == profile).and_then(|s| s.nodes.get(&q.database)).and_then(|n| n.catalog.clone());
                        let mut actions = Vec::new();
                        let mut ctx = TabCtx {
                            settings: &self.cfg.settings,
                            profiles: &self.cfg.profiles,
                            catalog,
                            actions: &mut actions,
                            history: &mut self.history,
                            passwords: &self.passwords,
                            monitor: None,
                            meta: None,
                            table_prefs: &self.cfg.table_prefs,
                            snippets: &self.cfg.snippets,
                        };
                        q.run_current(&mut ctx, ui, true, None);
                    }
                }
                Action::OpenTable { profile, db, schema, name } => {
                    if let Some(i) = self.tabs.iter().position(|t| matches!(t, Tab::Query(q) if q.profile_id == Some(profile) && q.database == db && q.table.as_ref().is_some_and(|t| t.schema == schema && t.name == name))) {
                        self.active = i;
                        continue;
                    }
                    let pk = self
                        .catalog(Some(profile), &db)
                        .and_then(|c| c.find_relation(Some(&schema), &name).map(|r| r.pk_columns().iter().map(|s| s.to_string()).collect()))
                        .unwrap_or_default();
                    let q = QueryTab::new_table(profile, &db, &schema, &name, pk, &self.cfg.settings);
                    self.tabs.push(Tab::Query(q));
                    self.active = self.tabs.len() - 1;
                }
                Action::OpenTableFiltered { profile, db, schema, name, filter } => {
                    let pk = self
                        .catalog(Some(profile), &db)
                        .and_then(|c| c.find_relation(Some(&schema), &name).map(|r| r.pk_columns().iter().map(|s| s.to_string()).collect()))
                        .unwrap_or_default();
                    let mut q = QueryTab::new_table(profile, &db, &schema, &name, pk, &self.cfg.settings);
                    if let Some(t) = &mut q.table {
                        t.filter = filter;
                    }
                    self.tabs.push(Tab::Query(q));
                    self.active = self.tabs.len() - 1;
                }
                Action::OpenStructure { profile, db, oid, title, ddl } => {
                    let session = self.servers.iter().find(|s| s.profile_id == profile).and_then(|s| {
                        let p = self.cfg.profiles.iter().find(|p| p.id == profile)?;
                        if db == p.database { s.root.as_ref() } else { s.nodes.get(&db)?.session.as_ref() }
                    });
                    if let Some(session) = session {
                        let promise = session.batch(meta::table_details_sql(oid));
                        let section = if ddl { crate::tabs::structure::Section::Ddl } else { crate::tabs::structure::Section::Health };
                        self.tabs.push(Tab::Structure(crate::tabs::structure::StructureTab::new(profile, &db, &title, section, promise)));
                        self.active = self.tabs.len() - 1;
                    }
                }
                Action::OpenErd { profile, db, schema, focus } => {
                    // the diagram is drawn from the catalog: make sure it loads
                    if let Some(s) = self.servers.iter_mut().find(|s| s.profile_id == profile) {
                        s.expanded = true;
                    }
                    self.tabs.push(Tab::Erd(crate::tabs::erd::ErdTab::new(profile, &db, schema, focus)));
                    self.active = self.tabs.len() - 1;
                }
                Action::OpenHealth { profile, db } => {
                    self.tabs.push(Tab::Health(crate::tabs::health::HealthTab::new(profile, &db)));
                    self.active = self.tabs.len() - 1;
                }
                Action::OpenDashboard { profile } => {
                    if let Some(i) = self.tabs.iter().position(|t| matches!(t, Tab::Sessions(s) if s.profile_id == profile)) {
                        self.active = i;
                    } else {
                        self.tabs.push(Tab::Sessions(crate::tabs::sessions::SessionsTab::new(profile)));
                        self.active = self.tabs.len() - 1;
                    }
                    if let Some(s) = self.servers.iter_mut().find(|s| s.profile_id == profile) {
                        s.expanded = true; // make sure the browser connection exists
                    }
                }
                Action::RefreshCatalog { profile, db } => {
                    if let Some(s) = self.servers.iter_mut().find(|s| s.profile_id == profile)
                        && let Some(n) = s.nodes.get_mut(&db)
                    {
                        n.catalog = None;
                        n.loading = None;
                    }
                }
                Action::EditProfile(id) => {
                    self.dialog = Some(ConnDialog::new(self.profile(id).cloned()));
                }
                Action::NewProfile => self.dialog = Some(ConnDialog::new(None)),
                Action::Toast(k, m) => self.toast(k, m),
                Action::SetPassword(id, pw) => {
                    self.passwords.insert(id, pw);
                }
                Action::ShowHistory => self.show_history = true,
                Action::SaveTablePrefs(key, prefs) => {
                    self.cfg.table_prefs.insert(key, prefs);
                    self.dirty_config = true;
                }
                Action::SaveSnippet { name, sql } => {
                    self.cfg.snippets.retain(|s| s.name != name);
                    self.cfg.snippets.push(crate::config::Snippet { name: name.clone(), sql });
                    self.save_config();
                    self.toast(ToastKind::Success, format!("Saved snippet “{name}”"));
                }
                Action::CopyDdl { profile, db, sql, label } => {
                    let session = self.servers.iter().find(|s| s.profile_id == profile).and_then(|s| {
                        let p = self.cfg.profiles.iter().find(|p| p.id == profile)?;
                        if db == p.database { s.root.as_ref() } else { s.nodes.get(&db)?.session.as_ref() }
                    });
                    match session {
                        Some(session) => self.pending_copy.push((label, session.batch(sql))),
                        None => self.toast(ToastKind::Info, "Expand the database in the sidebar first"),
                    }
                }
                Action::ImportCsv { profile, db, schema, path } => {
                    let path = path.or_else(|| rfd::FileDialog::new().add_filter("CSV / TSV", &["csv", "tsv", "txt"]).pick_file());
                    let profile = profile.or_else(|| self.tabs.get(self.active).and_then(|t| t.profile())).or_else(|| self.cfg.profiles.first().map(|p| p.id));
                    if let (Some(path), Some(pid)) = (path, profile) {
                        let db = db.or_else(|| match self.tabs.get(self.active) {
                            Some(Tab::Query(q)) if q.profile_id == Some(pid) => Some(q.database.clone()),
                            _ => None,
                        });
                        let db = db.or_else(|| self.profile(pid).map(|p| p.database.clone())).unwrap_or_default();
                        match crate::ui::import::ImportDialog::open(path, pid, db) {
                            Ok(mut d) => {
                                if let Some(s) = schema {
                                    d.set_schema(s);
                                }
                                self.import = Some(d);
                            }
                            Err(e) => self.toast(ToastKind::Error, format!("Could not read file: {e}")),
                        }
                    } else if profile.is_none() {
                        self.toast(ToastKind::Info, "Add a connection first");
                    }
                }
            }
        }
    }

    // ---- sidebar ---------------------------------------------------------------------

    fn sidebar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let pal = theme::p();
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            w::section(ui, "Explorer");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if w::icon_button(ui, icon::PLUS, "New connection").clicked() {
                    actions.push(Action::NewProfile);
                }
                if w::icon_button(ui, icon::IMPORT, "Import CSV").clicked() {
                    actions.push(Action::ImportCsv { profile: None, db: None, schema: None, path: None });
                }
            });
        });
        ui.add_space(4.0);
        w::input(ui, &mut self.sidebar_filter, "Filter tables, views, functions", Some(icon::SEARCH), 0.0);
        ui.add_space(8.0);
        let filter = self.sidebar_filter.trim().to_lowercase();

        if self.cfg.profiles.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(icon::DATABASE).size(28.0).color(pal.faint));
                ui.add_space(6.0);
                ui.label(RichText::new("No connections yet").color(pal.text));
                ui.label(RichText::new("Add a server to start exploring.").small().color(pal.dim));
                ui.add_space(10.0);
                if w::button(ui, w::Kind::Primary, icon::PLUS, "Add connection").clicked() {
                    actions.push(Action::NewProfile);
                }
            });
            return;
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut groups: Vec<String> = self.cfg.profiles.iter().map(|p| p.group.clone()).collect();
            groups.sort();
            groups.dedup();
            for group in groups {
                if !group.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new(group.to_uppercase()).small().color(pal.faint));
                }
                let ids: Vec<u64> = self.cfg.profiles.iter().filter(|p| p.group == group).map(|p| p.id).collect();
                for pid in ids {
                    self.server_tree(ui, pid, &filter, actions);
                }
            }
            ui.add_space(20.0);
        });
    }

    /// Keeps a server's root connection and its database list up to date.
    fn drive_server(server: &mut Server, profile: &Profile, passwords: &HashMap<u64, String>, ctx: &egui::Context) {
        if server.root.is_none() && server.error.is_none() {
            server.root = Some(Session::connect(profile, &profile.database, "Object browser", passwords.get(&profile.id).cloned(), ctx));
        }
        if let Some(root) = &mut server.root {
            for e in root.poll() {
                if let session::Event::ConnectFailed { error, .. } | session::Event::Disconnected(error) = e {
                    server.error = Some(error);
                }
            }
        }
        if server.is_connected() && server.dbs.is_none() && server.dbs_promise.is_none() {
            server.dbs_promise = server.root.as_ref().map(|s| s.batch(meta::DATABASES_SQL));
        }
        if let Some(p) = &server.dbs_promise
            && let Some(r) = p.take()
        {
            server.dbs_promise = None;
            match r {
                Ok(rs) => {
                    let rs = &rs[0];
                    server.dbs = Some(
                        rs.rows()
                            .map(|r| DbInfo { name: rs.s(r, 0).to_string(), size: rs.s(r, 1).to_string(), can_connect: rs.s(r, 2) == "t" })
                            .filter(|d| !profile.only_default_db || d.name == profile.database)
                            .collect(),
                    );
                }
                Err(e) => server.error = Some(e),
            }
        }
        if server.dbs.is_some() && server.roles.is_none() && server.objects_promise.is_none() {
            server.objects_promise = server.root.as_ref().map(|s| s.batch(meta::SERVER_OBJECTS_SQL));
        }
        if let Some(p) = &server.objects_promise
            && let Some(r) = p.take()
        {
            server.objects_promise = None;
            match r {
                Ok(rs) => {
                    server.roles = Some(rs.first().map(meta::parse_roles).unwrap_or_default());
                    if let Some(t) = rs.get(1) {
                        server.tablespaces = t.rows().map(|r| (t.s(r, 0).to_string(), t.s(r, 1).to_string(), t.s(r, 2).to_string())).collect();
                    }
                }
                // not fatal: the server just shows no roles
                Err(_) => server.roles = Some(Vec::new()),
            }
        }
    }

    /// Loads (or keeps loading) the catalog of one database. The default
    /// database reuses the server's root connection.
    fn drive_node(server: &mut Server, profile: &Profile, db: &str, passwords: &HashMap<u64, String>, ctx: &egui::Context) {
        if !server.is_connected() {
            return;
        }
        let node = server.nodes.entry(db.to_string()).or_default();
        if node.catalog.is_none() && node.loading.is_none() && node.error.is_none() {
            let sql = meta::catalog_sql(profile.show_system_objects);
            if db == profile.database {
                node.loading = server.root.as_ref().map(|s| s.batch(sql));
            } else {
                if node.session.is_none() {
                    node.session = Some(Session::connect(profile, db, &format!("Object browser · {db}"), passwords.get(&profile.id).cloned(), ctx));
                }
                node.loading = node.session.as_ref().map(|s| s.batch(sql));
            }
        }
        if let Some(s) = &mut node.session {
            for e in s.poll() {
                if let session::Event::ConnectFailed { error, .. } | session::Event::Disconnected(error) = e {
                    node.error = Some(error);
                    node.loading = None;
                    node.session = None;
                }
            }
        }
        if let Some(p) = &node.loading
            && let Some(r) = p.take()
        {
            node.loading = None;
            match r {
                Ok(rs) => node.catalog = Some(Arc::new(meta::parse_catalog(rs))),
                Err(e) => node.error = Some(e),
            }
        }
    }

    /// Makes sure the catalog of the active tab's database is loaded, even if
    /// the sidebar is collapsed (autocomplete, type names, primary keys).
    fn drive_active_catalog(&mut self, ctx: &egui::Context) {
        let Some(Tab::Query(q)) = self.tabs.get(self.active) else { return };
        let (Some(pid), db) = (q.profile_id, q.database.clone()) else { return };
        let Some(profile) = self.profile(pid).cloned() else { return };
        let passwords = &self.passwords;
        let Some(server) = self.servers.iter_mut().find(|s| s.profile_id == pid) else { return };
        if server.error.is_some() {
            return;
        }
        Self::drive_server(server, &profile, passwords, ctx);
        Self::drive_node(server, &profile, &db, passwords, ctx);
    }

    fn server_tree(&mut self, ui: &mut Ui, pid: u64, filter: &str, actions: &mut Vec<Action>) {
        let pal = theme::p();
        let Some(profile) = self.profile(pid).cloned() else { return };
        let passwords = self.passwords.clone();
        let tree_sel = &mut self.tree_sel;
        let confirm_delete = &mut self.confirm_delete;
        let Some(server) = self.servers.iter_mut().find(|s| s.profile_id == pid) else { return };

        if server.expanded || server.root.is_some() {
            Self::drive_server(server, &profile, &passwords, ui.ctx());
        }
        let connected = server.is_connected();
        let connecting = server.root.as_ref().is_some_and(|s| !s.connected && s.is_alive());
        let color = profile.color.map(theme::rgb).unwrap_or(pal.accent);
        let status = if server.error.is_some() {
            pal.danger
        } else if connected {
            pal.success
        } else if connecting {
            pal.warn
        } else {
            pal.faint
        };

        // ---- server row ----------------------------------------------------
        let subtitle = if profile.host.starts_with('/') {
            format!("socket · {}", profile.database)
        } else {
            format!("{}:{} · {}", profile.host, profile.port, profile.database)
        };
        let (resp, hover_action) = server_row(ui, &profile.display_name(), &subtitle, color, status, server.expanded, profile.is_azure(), profile.read_only);
        match hover_action {
            Some(0) => actions.push(Action::NewQuery { profile: pid, db: profile.database.clone(), sql: String::new(), run: false }),
            Some(1) => {
                server.disconnect();
            }
            _ => {
                if resp.clicked() {
                    server.expanded = !server.expanded;
                }
            }
        }
        let mut disconnect = false;
        resp.context_menu(|ui| {
            ui.set_min_width(210.0);
            if w::menu_item(ui, icon::FILE_PLUS, "New query").clicked() {
                actions.push(Action::NewQuery { profile: pid, db: profile.database.clone(), sql: String::new(), run: false });
                ui.close();
            }
            if w::menu_item(ui, icon::ACTIVITY, "Server activity").clicked() {
                actions.push(Action::OpenDashboard { profile: pid });
                ui.close();
            }
            if w::menu_item(ui, icon::SHIELD, "Health check").clicked() {
                actions.push(Action::OpenHealth { profile: pid, db: profile.database.clone() });
                ui.close();
            }
            ui.separator();
            if w::menu_item(ui, icon::SQUARE_PEN, "Edit connection…").clicked() {
                actions.push(Action::EditProfile(pid));
                ui.close();
            }
            if w::menu_item(ui, icon::LINK, "Copy connection URL").clicked() {
                ui.ctx().copy_text(format!(
                    "postgresql://{}@{}:{}/{}?sslmode={}",
                    profile.user,
                    profile.host,
                    profile.port,
                    profile.database,
                    profile.ssl_mode.label()
                ));
                ui.close();
            }
            if w::menu_item(ui, icon::REFRESH, "Refresh").clicked() {
                disconnect = true;
                ui.close();
            }
            if connected && w::menu_item(ui, icon::UNPLUG, "Disconnect").clicked() {
                disconnect = true;
                server.expanded = false;
                ui.close();
            }
            ui.separator();
            if w::menu_item_danger(ui, icon::TRASH, "Delete connection").clicked() {
                *confirm_delete = Some(pid);
                ui.close();
            }
        });
        if disconnect {
            server.disconnect();
        }
        if !server.expanded {
            return;
        }
        if let Some(err) = server.error.clone() {
            let needs_pw = server.root.as_ref().is_some_and(|s| s.needs_password);
            egui::Frame::NONE
                .fill(pal.danger.gamma_multiply(0.08))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(8, 6))
                .outer_margin(egui::Margin { left: 22, right: 4, top: 2, bottom: 4 })
                .show(ui, |ui| {
                    ui.label(RichText::new(format!("{} {}", icon::CIRCLE_X, first_line(&err))).small().color(pal.danger)).on_hover_text(&err);
                    ui.horizontal(|ui| {
                        if needs_pw {
                            let r = ui.add(egui::TextEdit::singleline(&mut server.password_input).password(true).hint_text("password").desired_width(130.0));
                            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if (ui.small_button("Connect").clicked() || enter) && !server.password_input.is_empty() {
                                let pw = std::mem::take(&mut server.password_input);
                                actions.push(Action::SetPassword(pid, pw.clone()));
                                server.disconnect();
                                server.root = Some(Session::connect(&profile, &profile.database, "Object browser", Some(pw), ui.ctx()));
                            }
                        } else {
                            if ui.small_button(format!("{}  Retry", icon::REFRESH)).clicked() {
                                server.disconnect();
                            }
                            if ui.small_button(format!("{}  Edit", icon::SQUARE_PEN)).clicked() {
                                actions.push(Action::EditProfile(pid));
                            }
                        }
                    });
                });
            return;
        }
        if connecting || server.dbs.is_none() {
            ui.horizontal(|ui| {
                ui.add_space(30.0);
                ui.spinner();
                ui.label(RichText::new(if profile.is_azure() { "Signing in & connecting…" } else { "Connecting…" }).small().color(pal.dim));
            });
            return;
        }

        // ---- databases -------------------------------------------------------
        let dbs: Vec<(String, String, bool)> =
            server.dbs.as_ref().map(|d| d.iter().map(|d| (d.name.clone(), d.size.clone(), d.can_connect)).collect()).unwrap_or_default();
        let single_db = dbs.len() == 1;
        for (db, size, can_connect) in dbs {
            let dkey = format!("{pid}/{db}");
            let expanded = match server.nodes.get(&db) {
                Some(n) if n.toggled => n.expanded,
                n => n.is_some_and(|n| n.expanded) || single_db || db == profile.database,
            };
            let (r, _) =
                tree_row(ui, 1, Some(expanded), icon::DATABASE, if can_connect { pal.warn } else { pal.faint }, &db, &size, None, *tree_sel == dkey, false);
            let r = r.on_hover_text(if can_connect { format!("Database {db} · {size}") } else { format!("You have no CONNECT privilege on {db}") });
            if r.clicked() && can_connect {
                *tree_sel = dkey.clone();
                let node = server.nodes.entry(db.clone()).or_default();
                node.expanded = !expanded;
                node.toggled = true;
            }
            if r.double_clicked() && can_connect {
                actions.push(Action::NewQuery { profile: pid, db: db.clone(), sql: String::new(), run: false });
            }
            r.context_menu(|ui| {
                if w::menu_item(ui, icon::FILE_PLUS, "New query").clicked() {
                    actions.push(Action::NewQuery { profile: pid, db: db.clone(), sql: String::new(), run: false });
                    ui.close();
                }
                if w::menu_item(ui, icon::ACTIVITY, "Health check").clicked() {
                    actions.push(Action::OpenHealth { profile: pid, db: db.clone() });
                    ui.close();
                }
                if w::menu_item(ui, icon::NETWORK, "ER diagram").clicked() {
                    actions.push(Action::OpenErd { profile: pid, db: db.clone(), schema: None, focus: None });
                    ui.close();
                }
                if w::menu_item(ui, icon::TIMER, "Slow queries (pg_stat_statements)").clicked() {
                    actions.push(Action::NewQuery { profile: pid, db: db.clone(), sql: meta::SLOW_QUERIES_SQL.to_string(), run: false });
                    ui.close();
                }
                if w::menu_item(ui, icon::REFRESH, "Refresh schema").clicked() {
                    actions.push(Action::RefreshCatalog { profile: pid, db: db.clone() });
                    ui.close();
                }
                if w::menu_item(ui, icon::COPY, "Copy name").clicked() {
                    ui.ctx().copy_text(db.clone());
                    ui.close();
                }
            });
            {
                let node = server.nodes.entry(db.clone()).or_default();
                if !node.toggled {
                    node.expanded = expanded;
                }
            }
            let expanded = server.nodes.get(&db).is_some_and(|n| n.expanded);
            if !expanded {
                continue;
            }
            Self::drive_node(server, &profile, &db, &passwords, ui.ctx());
            let node = server.nodes.entry(db.clone()).or_default();
            if let Some(e) = node.error.clone() {
                ui.horizontal(|ui| {
                    ui.add_space(46.0);
                    ui.label(RichText::new(format!("{} {}", icon::CIRCLE_X, first_line(&e))).small().color(pal.danger)).on_hover_text(&e);
                    if ui.small_button("Retry").clicked() {
                        node.error = None;
                        node.session = None;
                    }
                });
                continue;
            }
            let Some(cat) = node.catalog.clone() else {
                ui.horizontal(|ui| {
                    ui.add_space(46.0);
                    ui.spinner();
                    ui.label(RichText::new("Loading schema…").small().color(pal.dim));
                });
                continue;
            };
            let user_schemas = cat.schemas.iter().filter(|s| !s.is_system).count();
            for schema in &cat.schemas {
                let matches = |n: &str| filter.is_empty() || n.to_lowercase().contains(filter);
                let any_match = filter.is_empty() || schema.relations.iter().any(|r| matches(&r.name)) || schema.functions.iter().any(|f| matches(&f.name));
                if !any_match {
                    continue;
                }
                // Hide empty "public" when other schemas exist.
                if schema.name == "public" && schema.relations.is_empty() && schema.functions.is_empty() && user_schemas > 1 && filter.is_empty() {
                    continue;
                }
                let skey = schema.name.clone();
                let open =
                    node.open.contains(&skey) || !filter.is_empty() || (user_schemas == 1 && !schema.is_system && !node.open.contains(&format!("{skey}!")));
                let count = schema.relations.iter().filter(|r| !r.is_partition).count();
                let sel_key = format!("{pid}/{db}/{skey}");
                let (r, _) = tree_row(
                    ui,
                    2,
                    Some(open),
                    icon::FOLDER,
                    if schema.is_system { pal.faint } else { pal.accent },
                    &schema.name,
                    &count.to_string(),
                    None,
                    *tree_sel == sel_key,
                    false,
                );
                let r = r.on_hover_text(format!("schema {} · owner {}", schema.name, schema.owner));
                if r.clicked() {
                    *tree_sel = sel_key;
                    if user_schemas == 1 && !schema.is_system {
                        let k = format!("{skey}!");
                        if !node.open.remove(&k) {
                            node.open.insert(k);
                        }
                    } else if !node.open.remove(&skey) {
                        node.open.insert(skey.clone());
                    }
                }
                r.context_menu(|ui| {
                    if w::menu_item(ui, icon::NETWORK, "ER diagram").clicked() {
                        actions.push(Action::OpenErd { profile: pid, db: db.clone(), schema: Some(schema.name.clone()), focus: None });
                        ui.close();
                    }
                    if w::menu_item(ui, icon::FILE_PLUS, "New query in schema").clicked() {
                        actions.push(Action::NewQuery { profile: pid, db: db.clone(), sql: format!("SET search_path TO {}, public;\n\n", meta::quote_ident(&schema.name)), run: false });
                        ui.close();
                    }
                    if w::menu_item(ui, icon::PLUS, "New table…").clicked() {
                        let s = meta::quote_ident(&schema.name);
                        actions.push(Action::NewQuery { profile: pid, db: db.clone(), sql: format!("CREATE TABLE {s}.new_table (\n    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,\n    name text NOT NULL,\n    created_at timestamptz NOT NULL DEFAULT now()\n);\n"), run: false });
                        ui.close();
                    }
                    if w::menu_item(ui, icon::IMPORT, "Import CSV…").clicked() {
                        actions.push(Action::ImportCsv { profile: Some(pid), db: Some(db.clone()), schema: Some(schema.name.clone()), path: None });
                        ui.close();
                    }
                    if w::menu_item(ui, icon::REFRESH, "Refresh").clicked() {
                        actions.push(Action::RefreshCatalog { profile: pid, db: db.clone() });
                        ui.close();
                    }
                    if w::menu_item(ui, icon::COPY, "Copy name").clicked() {
                        ui.ctx().copy_text(schema.name.clone());
                        ui.close();
                    }
                });
                if !open {
                    continue;
                }
                let groups: [(&str, &str, Vec<RelKind>); 5] = [
                    ("Tables", icon::TABLE, vec![RelKind::Table, RelKind::Partitioned]),
                    ("Views", icon::EYE, vec![RelKind::View]),
                    ("Materialized views", icon::LAYERS, vec![RelKind::MatView]),
                    ("Foreign tables", icon::GLOBE, vec![RelKind::Foreign]),
                    ("Sequences", icon::HASH, vec![RelKind::Sequence]),
                ];
                for (gname, gicon, kinds) in groups {
                    let rels: Vec<_> = schema.relations.iter().filter(|r| kinds.contains(&r.kind) && !r.is_partition && matches(&r.name)).collect();
                    if rels.is_empty() {
                        continue;
                    }
                    // Tables are open by default, other groups closed.
                    let gkey = format!("{}/{gname}", schema.name);
                    let toggled = node.open.contains(&gkey);
                    let gopen = !filter.is_empty() || if gname == "Tables" { !toggled } else { toggled };
                    let (r, _) = tree_row(ui, 3, Some(gopen), gicon, pal.dim, gname, &rels.len().to_string(), None, false, false);
                    if r.clicked() && !node.open.remove(&gkey) {
                        node.open.insert(gkey.clone());
                    }
                    if !gopen {
                        continue;
                    }
                    for rel in rels {
                        let rkey = format!("{}.{}", rel.schema, rel.name);
                        let sel_key = format!("{pid}/{db}/{rkey}");
                        let ropen = node.open.contains(&rkey);
                        let detail = if rel.est_rows >= 0 && rel.kind.has_data() { util::compact_num(rel.est_rows) } else { String::new() };
                        let expandable = !rel.columns.is_empty();
                        let ico_color = match rel.kind {
                            RelKind::View | RelKind::MatView => pal.qident,
                            RelKind::Sequence => pal.number,
                            _ => pal.success,
                        };
                        let (r, chevron) = tree_row(
                            ui,
                            4,
                            expandable.then_some(ropen),
                            rel.kind.icon_glyph(),
                            ico_color,
                            &rel.name,
                            &detail,
                            None,
                            *tree_sel == sel_key,
                            false,
                        );
                        let r = r.on_hover_text(format!(
                            "{} {}{}{}\n\nDouble-click to open data · right-click for more",
                            rel.kind.label(),
                            rel.qualified(),
                            if rel.est_rows >= 0 { format!("\n~{} rows", util::thousands(rel.est_rows as usize)) } else { String::new() },
                            rel.comment.as_ref().map(|c| format!("\n\n{c}")).unwrap_or_default()
                        ));
                        if chevron {
                            if !node.open.remove(&rkey) {
                                node.open.insert(rkey.clone());
                            }
                        } else if r.clicked() {
                            *tree_sel = sel_key;
                        }
                        if r.double_clicked() && rel.kind.has_data() {
                            actions.push(Action::OpenTable { profile: pid, db: db.clone(), schema: rel.schema.clone(), name: rel.name.clone() });
                        }
                        r.context_menu(|ui| relation_menu(ui, pid, &db, rel, actions));
                        if ropen {
                            for c in &rel.columns {
                                let ic = if c.is_pk { icon::KEY } else { icon::COLUMNS };
                                let col = if c.is_pk { pal.warn } else { pal.faint };
                                let label = if c.not_null || c.is_pk { c.name.clone() } else { format!("{}?", c.name) };
                                let (r, _) = tree_row(ui, 5, None, ic, col, &label, &c.data_type, None, false, true);
                                let mut tip = format!("{} {}{}", c.name, c.data_type, if c.not_null { " NOT NULL" } else { "" });
                                if let Some(fk) = rel.fk_for(&c.name)
                                    && let Some(target) = cat.relation_by_oid(fk.ref_oid)
                                {
                                    tip.push_str(&format!("\n→ references {}.{} ({})", target.qualified(), fk.ref_columns.join(", "), fk.name));
                                }
                                if let Some(d) = &c.default {
                                    tip.push_str(&format!("\nDEFAULT {d}"));
                                }
                                if let Some(cm) = &c.comment {
                                    tip.push_str(&format!("\n\n{cm}"));
                                }
                                r.on_hover_text(tip).context_menu(|ui| {
                                    if w::menu_item(ui, icon::COPY, "Copy name").clicked() {
                                        ui.ctx().copy_text(c.name.clone());
                                        ui.close();
                                    }
                                });
                            }
                            relation_details_tree(ui, rel, &mut node.open);
                        }
                    }
                }
                let funcs: Vec<_> = schema.functions.iter().filter(|f| matches(&f.name)).collect();
                if !funcs.is_empty() {
                    let gkey = format!("{}/Functions", schema.name);
                    let gopen = node.open.contains(&gkey) || !filter.is_empty();
                    let (r, _) = tree_row(ui, 3, Some(gopen), icon::FUNCTION, pal.dim, "Functions", &funcs.len().to_string(), None, false, false);
                    if r.clicked() && !node.open.remove(&gkey) {
                        node.open.insert(gkey.clone());
                    }
                    if gopen {
                        for f in funcs {
                            let sel_key = format!("{pid}/{db}/fn{}", f.oid);
                            let kind = match f.kind {
                                'p' => "procedure",
                                'a' => "aggregate",
                                'w' => "window",
                                _ => "function",
                            };
                            let (r, _) = tree_row(ui, 4, None, icon::FUNCTION, pal.number, &f.name, "", None, *tree_sel == sel_key, false);
                            let r = r.on_hover_text(format!("{kind} {}\n→ {}\n\nDouble-click to view the definition", f.signature(), f.result));
                            if r.clicked() {
                                *tree_sel = sel_key;
                            }
                            if r.double_clicked() {
                                actions.push(Action::NewQuery {
                                    profile: pid,
                                    db: db.clone(),
                                    sql: format!("-- definition of {}\n{};", f.signature(), meta::function_ddl_sql(f.oid)),
                                    run: true,
                                });
                            }
                            r.context_menu(|ui| {
                                if w::menu_item(ui, icon::CODE, "Show definition").clicked() {
                                    actions.push(Action::NewQuery {
                                        profile: pid,
                                        db: db.clone(),
                                        sql: format!("{};", meta::function_ddl_sql(f.oid)),
                                        run: true,
                                    });
                                    ui.close();
                                }
                                if w::menu_item(ui, icon::PLAY, "Call template").clicked() {
                                    let call = if f.kind == 'p' {
                                        format!("CALL {}.{}();", meta::quote_ident(&f.schema), meta::quote_ident(&f.name))
                                    } else {
                                        format!("SELECT {}.{}();", meta::quote_ident(&f.schema), meta::quote_ident(&f.name))
                                    };
                                    actions.push(Action::NewQuery { profile: pid, db: db.clone(), sql: format!("-- ({})\n{call}", f.args), run: false });
                                    ui.close();
                                }
                                if w::menu_item(ui, icon::CLIPBOARD, "Copy DDL").clicked() {
                                    actions.push(Action::CopyDdl { profile: pid, db: db.clone(), sql: meta::function_ddl_sql(f.oid), label: f.signature() });
                                    ui.close();
                                }
                                if w::menu_item(ui, icon::COPY, "Copy signature").clicked() {
                                    ui.ctx().copy_text(f.signature());
                                    ui.close();
                                }
                            });
                        }
                    }
                }
                if !schema.types.is_empty() && filter.is_empty() {
                    let gkey = format!("{}/Types", schema.name);
                    let gopen = node.open.contains(&gkey);
                    let (r, _) = tree_row(ui, 3, Some(gopen), icon::TYPE, pal.dim, "Types", &schema.types.len().to_string(), None, false, false);
                    if r.clicked() && !node.open.remove(&gkey) {
                        node.open.insert(gkey.clone());
                    }
                    if gopen {
                        for t in &schema.types {
                            let kind = match t.kind {
                                'e' => "enum",
                                'd' => "domain",
                                'r' => "range",
                                'c' => "composite",
                                _ => "type",
                            };
                            let (r, _) = tree_row(ui, 4, None, icon::TYPE, pal.qident, &t.name, kind, None, false, false);
                            if !t.detail.is_empty() {
                                r.on_hover_text(format!("{kind} {}.{}\n{}", t.schema, t.name, t.detail));
                            }
                        }
                    }
                }
            }
        }
        server_objects_tree(ui, server, &profile, filter);
    }

    // ---- tabs strip --------------------------------------------------------------

    fn tab_strip(&mut self, ui: &mut Ui) {
        let pal = theme::p();
        let mut close = None;
        egui::ScrollArea::horizontal().id_salt("tabstrip").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                for (i, t) in self.tabs.iter().enumerate() {
                    let active = i == self.active;
                    let (title, is_table, dirty, running) = match t {
                        Tab::Query(q) => (q.title.clone(), q.table.is_some(), q.is_dirty() || q.has_unsaved_edits(), q.is_running()),
                        Tab::Health(h) => (h.title.clone(), false, false, false),
                        Tab::Sessions(s) => (s.title.clone(), false, false, false),
                        Tab::Structure(s) => (s.title.clone(), false, false, false),
                        Tab::Erd(e) => (e.title.clone(), false, false, false),
                    };
                    let color = t.profile().and_then(|p| self.profile(p)).and_then(|p| p.color).map(theme::rgb);
                    let ico = match t {
                        Tab::Health(_) => icon::SHIELD,
                        Tab::Sessions(_) => icon::ACTIVITY,
                        Tab::Structure(_) => icon::TABLE_PROPS,
                        Tab::Erd(_) => icon::NETWORK,
                        _ if is_table => icon::TABLE,
                        _ => icon::FILE_CODE,
                    };
                    let w = (ui.fonts_mut(|f| f.layout_no_wrap(title.clone(), FontId::proportional(13.0), Color32::WHITE).size().x) + 64.0).clamp(90.0, 240.0);
                    let (rect, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
                    let p = ui.painter();
                    if active {
                        p.rect_filled(rect, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, pal.bg0);
                        p.hline(rect.x_range(), rect.top() + 1.0, Stroke::new(2.0, color.unwrap_or(pal.accent)));
                    } else if resp.hovered() {
                        p.rect_filled(rect, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, pal.bg2);
                    }
                    let fg = if active { pal.text } else { pal.dim };
                    p.text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, ico, FontId::proportional(13.0), color.unwrap_or(fg));
                    let tg = ui.fonts_mut(|f| {
                        let mut job = egui::text::LayoutJob::single_section(title.clone(), egui::TextFormat::simple(FontId::proportional(13.0), fg));
                        job.wrap = egui::text::TextWrapping::truncate_at_width(w - 62.0);
                        f.layout_job(job)
                    });
                    p.galley(pos2(rect.left() + 32.0, rect.center().y - tg.size().y / 2.0), tg, fg);
                    let cr = egui::Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(18.0, 18.0));
                    let close_hover = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| cr.contains(p));
                    if running {
                        p.circle_filled(cr.center(), 3.5, pal.accent);
                    } else if dirty && !close_hover {
                        p.circle_filled(cr.center(), 3.5, pal.warn);
                    } else if resp.hovered() || active {
                        if close_hover {
                            p.rect_filled(cr, 4.0, pal.bg3);
                        }
                        p.text(cr.center(), Align2::CENTER_CENTER, icon::X, FontId::proportional(12.0), pal.dim);
                    }
                    if resp.clicked() {
                        if close_hover {
                            close = Some(i);
                        } else {
                            self.active = i;
                        }
                    }
                    if resp.middle_clicked() {
                        close = Some(i);
                    }
                    resp.on_hover_text(&title);
                }
                if ui
                    .add(egui::Button::new(RichText::new(icon::PLUS).size(14.0)).frame(false).min_size(vec2(32.0, 32.0)))
                    .on_hover_text("New query (⌘T)")
                    .clicked()
                {
                    self.new_query(None, None, String::new());
                }
            });
        });
        if let Some(i) = close {
            self.close_tab(i);
        }
    }

    fn close_tab(&mut self, i: usize) {
        if i < self.tabs.len() {
            self.tabs.remove(i); // drops its Session → connection closed
            if self.active >= self.tabs.len() {
                self.active = self.tabs.len().saturating_sub(1);
            } else if self.active > i {
                self.active -= 1;
            }
        }
    }

    // ---- welcome ---------------------------------------------------------------------

    fn welcome(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let pal = theme::p();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.12).clamp(24.0, 120.0));
                if let Some(logo) = &self.logo {
                    ui.add(egui::Image::new(logo).fit_to_exact_size(vec2(84.0, 84.0)));
                }
                ui.add_space(14.0);
                ui.label(RichText::new("Welcome to Trunq").font(theme::bold(28.0)));
                ui.add_space(2.0);
                ui.label(RichText::new("A fast, native PostgreSQL client that keeps you out of incidents.").size(15.0).color(pal.dim));
                ui.add_space(26.0);
                ui.horizontal(|ui| {
                    let w_total = 3.0 * 190.0 + 2.0 * 12.0;
                    ui.add_space(((ui.available_width() - w_total) / 2.0).max(0.0));
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let action_card = |ui: &mut Ui, ico: &str, title: &str, sub: &str, primary: bool| -> bool {
                        let (rect, resp) = ui.allocate_exact_size(vec2(190.0, 96.0), Sense::click());
                        let hovered = resp.hovered();
                        let (fill, stroke) = if primary {
                            (if hovered { pal.primary_hover } else { pal.primary }, Color32::TRANSPARENT)
                        } else {
                            (if hovered { pal.control_hover } else { pal.bg1 }, if hovered { pal.border_strong } else { pal.border })
                        };
                        ui.painter().rect(rect, 12.0, fill, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
                        let fg = if primary { Color32::WHITE } else { pal.text };
                        let sub_c = if primary { Color32::from_white_alpha(200) } else { pal.dim };
                        ui.painter().text(
                            rect.left_top() + vec2(16.0, 18.0),
                            Align2::LEFT_TOP,
                            ico,
                            FontId::proportional(20.0),
                            if primary { Color32::WHITE } else { pal.accent },
                        );
                        ui.painter().text(rect.left_top() + vec2(16.0, 50.0), Align2::LEFT_TOP, title, theme::bold(14.0), fg);
                        ui.painter().text(rect.left_top() + vec2(16.0, 70.0), Align2::LEFT_TOP, sub, FontId::proportional(12.0), sub_c);
                        if hovered {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        resp.clicked()
                    };
                    if action_card(ui, icon::PLUG, "New connection", "Server, Azure or SSH", true) {
                        actions.push(Action::NewProfile);
                    }
                    if action_card(ui, icon::FILE_PLUS, "New query", "Open a SQL editor  ⌘T", false) {
                        self.new_query(None, None, String::new());
                    }
                    if action_card(ui, icon::IMPORT, "Import a CSV", "Or drop a file anywhere", false) {
                        actions.push(Action::ImportCsv { profile: None, db: None, schema: None, path: None });
                    }
                });
                ui.add_space(30.0);
                if !self.cfg.profiles.is_empty() {
                    let width = 594.0;
                    ui.allocate_ui(vec2(width, 0.0), |ui| {
                        ui.set_width(width);
                        w::section(ui, "Recent connections");
                        ui.add_space(6.0);
                        for p in self.cfg.profiles.iter().take(6) {
                            let color = p.color.or(p.environment.color()).map(theme::rgb).unwrap_or(pal.accent);
                            let (rect, resp) = ui.allocate_exact_size(vec2(width, 44.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, 8.0, pal.control_hover);
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            let av = egui::Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(28.0, 28.0));
                            ui.painter().rect_filled(av, 7.0, color.gamma_multiply(0.18));
                            ui.painter().text(av.center(), Align2::CENTER_CENTER, icon::SERVER, FontId::proportional(14.0), color);
                            ui.painter().text(pos2(rect.left() + 46.0, rect.top() + 13.0), Align2::LEFT_CENTER, p.display_name(), theme::bold(13.5), pal.text);
                            ui.painter().text(
                                pos2(rect.left() + 46.0, rect.top() + 30.0),
                                Align2::LEFT_CENTER,
                                format!("{}@{}:{} / {}", p.user, p.host, p.port, p.database),
                                FontId::proportional(12.0),
                                pal.faint,
                            );
                            if let Some(env) = p.environment.color() {
                                let g = ui.fonts_mut(|f| f.layout_no_wrap(p.environment.label().to_string(), theme::bold(10.5), Color32::BLACK));
                                let br = egui::Rect::from_min_size(pos2(rect.right() - g.size().x - 22.0, rect.center().y - 9.0), g.size() + vec2(10.0, 4.0));
                                ui.painter().rect_filled(br, 5.0, theme::rgb(env));
                                ui.painter().galley(br.min + vec2(5.0, 2.0), g, Color32::BLACK);
                            } else {
                                ui.painter().text(
                                    pos2(rect.right() - 14.0, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    icon::ARROW_RIGHT,
                                    FontId::proportional(13.0),
                                    pal.faint,
                                );
                            }
                            if resp.clicked() {
                                actions.push(Action::NewQuery { profile: p.id, db: p.database.clone(), sql: String::new(), run: false });
                            }
                        }
                    });
                    ui.add_space(26.0);
                }
                let shortcuts = [
                    ("⌘K", "Search everything"),
                    ("⌘⏎", "Run statement"),
                    ("⌥⌘⏎", "Dry run"),
                    ("⌘E", "Explain"),
                    ("⇧⌘F", "Format SQL"),
                    ("⌘F", "Find column"),
                    ("⌘T", "New query tab"),
                    ("⌘B", "Toggle sidebar"),
                ];
                ui.allocate_ui(vec2(594.0, 0.0), |ui| {
                    w::section(ui, "Keyboard");
                    ui.add_space(6.0);
                    egui::Grid::new("shortcuts").num_columns(4).spacing(vec2(14.0, 8.0)).show(ui, |ui| {
                        for (i, (k, d)) in shortcuts.iter().enumerate() {
                            egui::Frame::NONE
                                .fill(pal.bg2)
                                .stroke(Stroke::new(1.0, pal.border))
                                .corner_radius(5.0)
                                .inner_margin(egui::Margin::symmetric(7, 2))
                                .show(ui, |ui| {
                                    ui.label(RichText::new(*k).font(theme::mono(12.0)).color(pal.text));
                                });
                            ui.label(RichText::new(*d).color(pal.dim));
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                });
                ui.add_space(30.0);
            });
        });
    }

    // ---- overlays ----------------------------------------------------------------------

    fn overlays(&mut self, ctx: &egui::Context) {
        let pal = theme::p();
        // connection dialog
        if let Some(d) = &mut self.dialog
            && let Some(outcome) = d.show(ctx)
        {
            if let DialogOutcome::Save(p, pw, ssh_secret) = outcome {
                if let Some(secret) = &ssh_secret {
                    p.store_ssh_secret(secret);
                }
                if let Some(pw) = &pw {
                    if p.save_password {
                        p.store_password(pw);
                    } else {
                        p.forget_password();
                    }
                    self.passwords.insert(p.id, pw.clone());
                }
                if let Some(existing) = self.cfg.profiles.iter_mut().find(|x| x.id == p.id) {
                    *existing = p.clone();
                    if let Some(s) = self.servers.iter_mut().find(|s| s.profile_id == p.id) {
                        s.disconnect();
                    }
                } else {
                    self.cfg.profiles.push(p.clone());
                    let mut s = Server::new(p.id);
                    s.expanded = true;
                    self.servers.push(s);
                }
                self.save_config();
                self.toast(ToastKind::Success, format!("Saved connection {}", p.display_name()));
            }
            self.dialog = None;
        }

        // CSV import
        if let Some(d) = &mut self.import {
            let catalog = self.servers.iter().find(|s| s.profile_id == d.profile_id).and_then(|s| s.nodes.get(&d.database)).and_then(|n| n.catalog.clone());
            match d.show(ctx, &self.cfg.profiles, catalog.as_deref(), &self.passwords) {
                Some(crate::ui::import::ImportOutcome::Close) => self.import = None,
                Some(crate::ui::import::ImportOutcome::Imported(pid, db, schema, table)) => {
                    self.toast(ToastKind::Success, format!("Imported into {schema}.{table}"));
                    if let Some(s) = self.servers.iter_mut().find(|s| s.profile_id == pid)
                        && let Some(n) = s.nodes.get_mut(&db)
                    {
                        n.catalog = None;
                        n.loading = None;
                    }
                }
                None => {}
            }
        }

        // drag & drop: CSV → import, SQL → open
        let (dropped, hovering) = ctx.input(|i| (i.raw.dropped_files.clone(), !i.raw.hovered_files.is_empty()));
        if hovering {
            egui::Area::new(egui::Id::unique("drop_overlay")).order(egui::Order::Foreground).fixed_pos(pos2(0.0, 0.0)).show(ctx, |ui| {
                let r = ctx.content_rect();
                ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(140));
                ui.painter().rect_stroke(r.shrink(16.0), 14.0, Stroke::new(2.0, pal.accent), egui::StrokeKind::Inside);
                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    format!("{}  Drop a CSV to import it · a .sql file to open it", icon::IMPORT),
                    FontId::proportional(20.0),
                    Color32::WHITE,
                );
            });
        }
        for f in dropped {
            let path = f.path().to_path_buf();
            if path.as_os_str().is_empty() {
                continue;
            }
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            if ext == "sql" || ext == "psql" {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    let i = self.new_query(None, None, text.clone());
                    if let Some(Tab::Query(q)) = self.tabs.get_mut(i) {
                        q.title = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                        q.saved_text = text;
                        q.file = Some(path);
                    }
                }
            } else {
                self.pending_actions.push(Action::ImportCsv { profile: None, db: None, schema: None, path: Some(path) });
            }
        }

        // Entra sign-in prompt
        let prompt = azure::PROMPT.lock().clone();
        if let Some(p) = prompt {
            egui::Modal::new(egui::Id::unique("azure_prompt")).frame(w::modal_frame()).show(ctx, |ui| {
                ui.set_width(460.0);
                w::dialog_header(ui, icon::CLOUD, pal.accent, "Sign in with Microsoft", "Microsoft Entra ID authentication for Azure Database for PostgreSQL");
                match &p {
                    SignInPrompt::Browser { url } => {
                        w::callout(ui, pal.accent, icon::GLOBE, "Complete the sign-in in your browser. Trunq continues automatically when it is done.");
                        ui.add_space(10.0);
                        if w::button(ui, w::Kind::Secondary, icon::EXTERNAL, "Open the sign-in page again").clicked() {
                            let _ = open::that_detached(url);
                        }
                    }
                    SignInPrompt::DeviceCode { url, code } => {
                        ui.label(RichText::new(format!("Open {url} and enter this code:")).color(pal.dim));
                        ui.add_space(8.0);
                        egui::Frame::NONE
                            .fill(pal.input)
                            .stroke(Stroke::new(1.0, pal.border))
                            .corner_radius(10.0)
                            .inner_margin(egui::Margin::symmetric(16, 12))
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(code).font(theme::mono(28.0)).color(pal.accent));
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if w::button(ui, w::Kind::Secondary, icon::COPY, "Copy").clicked() {
                                            ui.ctx().copy_text(code.clone());
                                        }
                                    });
                                });
                            });
                        ui.add_space(10.0);
                        if w::button(ui, w::Kind::Primary, icon::EXTERNAL, "Copy code and open sign-in page").clicked() {
                            ui.ctx().copy_text(code.clone());
                            let _ = open::that_detached(url);
                        }
                    }
                }
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Waiting for sign-in…").color(pal.dim));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::button(ui, w::Kind::Ghost, "", "Cancel").clicked() {
                            azure::CANCEL_SIGN_IN.store(true, std::sync::atomic::Ordering::SeqCst);
                        }
                    });
                });
            });
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        // delete confirmation
        if let Some(pid) = self.confirm_delete {
            let name = self.profile(pid).map(|p| p.display_name()).unwrap_or_default();
            let mut decided = None;
            let modal = egui::Modal::new(egui::Id::unique("confirm_delete")).frame(w::modal_frame()).show(ctx, |ui| {
                ui.set_width(420.0);
                w::dialog_header(ui, icon::TRASH, pal.danger, "Delete connection?", "This cannot be undone");
                ui.label(RichText::new(format!("“{name}” will be removed together with its saved password and SSH secret.")).color(pal.text));
                w::dialog_footer(ui, |ui| {
                    if w::button(ui, w::Kind::Danger, icon::TRASH, "Delete connection").clicked() {
                        decided = Some(true);
                    }
                    if w::button(ui, w::Kind::Ghost, "", "Cancel").clicked() {
                        decided = Some(false);
                    }
                });
            });
            if modal.should_close() && decided.is_none() {
                decided = Some(false);
            }
            if let Some(yes) = decided {
                if yes {
                    if let Some(p) = self.profile(pid) {
                        p.forget_password();
                    }
                    self.cfg.profiles.retain(|p| p.id != pid);
                    self.servers.retain(|s| s.profile_id != pid);
                    self.save_config();
                }
                self.confirm_delete = None;
            }
        }

        // live connections
        if self.show_connections {
            let mut close = false;
            let modal = egui::Modal::new(egui::Id::unique("connections")).frame(w::modal_frame()).show(ctx, |ui| {
                ui.set_width(700.0);
                close |= w::dialog_title(
                    ui,
                    icon::NETWORK,
                    pal.accent,
                    "Open connections",
                    "Every connection Trunq holds. Closing a tab or disconnecting a server closes its connections.",
                );
                let live = session::LIVE.lock();
                if live.is_empty() {
                    w::callout(ui, pal.success, icon::CIRCLE_CHECK, "No open connections.");
                } else {
                    egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show(ui, |ui| {
                        for c in live.iter() {
                            let busy = c.busy.load(std::sync::atomic::Ordering::Relaxed);
                            egui::Frame::NONE
                                .fill(pal.bg2)
                                .stroke(Stroke::new(1.0, pal.border))
                                .corner_radius(8.0)
                                .inner_margin(egui::Margin::symmetric(12, 8))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                                        ui.painter().circle_filled(r.center(), 4.0, if busy { pal.warn } else { pal.success });
                                        ui.vertical(|ui| {
                                            ui.label(RichText::new(format!("{} · {}", c.profile, c.database)).color(pal.text).strong());
                                            ui.label(
                                                RichText::new(format!(
                                                    "PID {} · open {}{}",
                                                    c.pid,
                                                    util::duration(c.since.elapsed()),
                                                    if busy { " · running a query" } else { "" }
                                                ))
                                                .size(12.0)
                                                .color(pal.faint),
                                            );
                                        });
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            if w::small_button(ui, w::Kind::Secondary, icon::X, "Close").clicked() {
                                                c.kill();
                                            }
                                            ui.add_space(6.0);
                                            w::pill(ui, &c.purpose, pal.accent);
                                        });
                                    });
                                });
                            ui.add_space(6.0);
                        }
                    });
                }
                let any = !live.is_empty();
                drop(live);
                w::dialog_footer(ui, |ui| {
                    if w::button(ui, w::Kind::Primary, "", "Done").clicked() {
                        close = true;
                    }
                    if any && w::button(ui, w::Kind::Danger, icon::UNPLUG, "Close all").clicked() {
                        session::kill_all();
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        w::hint(ui, "Idle query tabs disconnect after 15 minutes.");
                    });
                });
            });
            if close || modal.should_close() {
                self.show_connections = false;
            }
        }

        // history
        if self.show_history {
            let mut close = false;
            let mut chosen = None;
            let modal = egui::Modal::new(egui::Id::unique("history")).frame(w::modal_frame()).show(ctx, |ui| {
                ui.set_width(760.0);
                close |= w::dialog_title(
                    ui,
                    icon::HISTORY,
                    pal.accent,
                    "Query history",
                    "Every statement you ran, newest first. Click one to open it in a new tab.",
                );
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    w::input(ui, &mut self.history_filter, "Search history", Some(icon::SEARCH), ui.available_width() - 96.0);
                    if w::button(ui, w::Kind::Danger, icon::TRASH, "Clear").clicked() {
                        self.history.clear();
                    }
                });
                ui.add_space(10.0);
                let f = self.history_filter.to_lowercase();
                let items: Vec<_> = self.history.entries.iter().rev().filter(|e| f.is_empty() || e.sql.to_lowercase().contains(&f)).take(500).collect();
                let max_h = (ui.ctx().content_rect().height() - 300.0).clamp(200.0, 560.0);
                egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                    if items.is_empty() {
                        w::callout(ui, pal.faint, icon::INFO, if f.is_empty() { "No queries yet." } else { "Nothing matches your search." });
                    }
                    for e in items {
                        let when = chrono::DateTime::from_timestamp(e.at, 0)
                            .map(|d| d.with_timezone(&chrono::Local).format("%b %d %H:%M").to_string())
                            .unwrap_or_default();
                        let preview: String = e.sql.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(220).collect();
                        let resp = ui
                            .scope_builder(egui::UiBuilder::new().sense(Sense::click()), |ui| {
                                let hovered = ui.response().hovered();
                                egui::Frame::NONE
                                    .fill(if hovered { pal.control_hover } else { pal.bg2 })
                                    .stroke(Stroke::new(1.0, if hovered { pal.border_strong } else { pal.border }))
                                    .corner_radius(8.0)
                                    .inner_margin(egui::Margin::symmetric(12, 9))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(if e.ok { icon::CIRCLE_CHECK } else { icon::CIRCLE_X }).color(if e.ok {
                                                pal.success
                                            } else {
                                                pal.danger
                                            }));
                                            ui.label(RichText::new(format!("{} · {}", e.profile, e.database)).size(12.0).color(pal.dim));
                                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                                ui.label(RichText::new(format!("{when}  ·  {} ms", e.ms)).size(12.0).color(pal.faint));
                                            });
                                        });
                                        ui.add(egui::Label::new(RichText::new(preview).font(theme::mono(12.5)).color(pal.text)).truncate());
                                    });
                            })
                            .response;
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            chosen = Some(e.sql.clone());
                        }
                        ui.add_space(6.0);
                    }
                });
            });
            if let Some(sql) = chosen {
                self.new_query(None, None, sql);
                close = true;
            }
            if close || modal.should_close() {
                self.show_history = false;
            }
        }

        // settings
        if self.show_settings {
            let mut close = false;
            let mut changed = false;
            let modal = egui::Modal::new(egui::Id::unique("settings")).frame(w::modal_frame()).show(ctx, |ui| {
                ui.set_width(560.0);
                close |= w::dialog_title(ui, icon::SETTINGS, pal.accent, "Settings", "Saved automatically");
                let s = &mut self.cfg.settings;
                w::section(ui, "Appearance");
                ui.add_space(6.0);
                w::form_row(ui, "Theme", |ui| {
                    changed |= w::segmented(ui, &mut s.theme, &[(ThemeChoice::Dark, icon::MOON, "Dark"), (ThemeChoice::Light, icon::SUN, "Light")]);
                });
                w::form_row(ui, "Interface zoom", |ui| {
                    changed |=
                        ui.add(egui::Slider::new(&mut s.ui_scale, 0.75..=1.6).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0))).changed();
                });
                w::form_row(ui, "Editor font", |ui| {
                    changed |= ui.add(egui::Slider::new(&mut s.editor_font_size, 10.0..=22.0).step_by(0.5).suffix(" px")).changed();
                });
                w::form_row(ui, "Grid font", |ui| {
                    changed |= ui.add(egui::Slider::new(&mut s.grid_font_size, 10.0..=18.0).step_by(0.5).suffix(" px")).changed();
                });
                ui.add_space(10.0);
                w::section(ui, "Editor and results");
                ui.add_space(6.0);
                w::form_row(ui, "Rows per fetch", |ui| {
                    changed |= w::number(ui, "page_size", &mut s.page_size, 100, 100_000, 120.0).changed();
                    ui.label(RichText::new("more rows load as you scroll").color(pal.faint));
                });
                w::form_row(ui, "Autocomplete", |ui| {
                    changed |= w::checkbox(ui, &mut s.auto_complete, "Suggest while typing").changed();
                });
                w::form_row(ui, "Keywords", |ui| {
                    changed |= w::checkbox(ui, &mut s.uppercase_keywords, "UPPERCASE completions and formatting").changed();
                });
                ui.add_space(10.0);
                w::section(ui, "Safety");
                ui.add_space(6.0);
                w::form_row(ui, "Confirm", |ui| {
                    changed |= w::checkbox(ui, &mut s.confirm_destructive, "UPDATE/DELETE without WHERE, DROP, TRUNCATE").changed();
                });
                w::dialog_footer(ui, |ui| {
                    if w::button(ui, w::Kind::Primary, "", "Done").clicked() {
                        close = true;
                    }
                });
            });
            if changed {
                theme::apply(ctx, self.cfg.settings.theme);
                self.logo = load_logo(ctx, self.cfg.settings.theme == ThemeChoice::Dark);
                ctx.set_zoom_factor(self.cfg.settings.ui_scale);
                for t in &mut self.tabs {
                    if let Tab::Query(q) = t {
                        q.editor.font_size = self.cfg.settings.editor_font_size;
                    }
                }
                self.save_config();
            }
            if close || modal.should_close() {
                self.show_settings = false;
            }
        }

        // toasts
        self.toasts.retain(|t| t.2.elapsed() < Duration::from_secs(if t.0 == ToastKind::Error { 8 } else { 4 }));
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        egui::Area::new(egui::Id::unique("toasts")).anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -40.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            for (kind, msg, _) in &self.toasts {
                let (ic, c) = match kind {
                    ToastKind::Info => (icon::INFO, pal.accent),
                    ToastKind::Success => (icon::CIRCLE_CHECK, pal.success),
                    ToastKind::Error => (icon::CIRCLE_X, pal.danger),
                };
                egui::Frame::popup(ui.style()).fill(pal.bg2).stroke(Stroke::new(1.0, c.gamma_multiply(0.5))).show(ui, |ui| {
                    ui.set_max_width(380.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(ic).color(c));
                        ui.add(egui::Label::new(msg).wrap());
                    });
                });
                ui.add_space(6.0);
            }
        });
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.script.is_some() {
            // egui 0.37 skips UI passes for occluded windows. Automation must
            // keep drawing even when another application covers the client.
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let pal = theme::p();
        let ctx = ui.ctx().clone();
        let mut actions: Vec<Action> = Vec::new();
        if std::env::var("TRUNQ_BENCH").as_deref() == Ok("startup") && !self.bench_reported {
            // Reported on the second frame, i.e. after the first one was presented.
            if self.bench_frames == 1 {
                let ms = crate::STARTED.get().map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
                println!("TRUNQ_BENCH startup_to_first_frame_ms={ms:.1}");
                self.bench_reported = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            self.bench_frames += 1;
            ctx.request_repaint();
        }

        // global shortcuts
        let (new_tab, close_tab, next_tab, prev_tab, toggle_sidebar) = ctx.input_mut(|i| {
            use egui::{Key, Modifiers};
            (
                i.consume_key(Modifiers::COMMAND, Key::T),
                i.consume_key(Modifiers::COMMAND, Key::W),
                i.consume_key(Modifiers::CTRL, Key::Tab) || i.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::ArrowRight),
                i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) || i.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::ArrowLeft),
                i.consume_key(Modifiers::COMMAND, Key::B),
            )
        });
        if new_tab {
            self.new_query(None, None, String::new());
        }
        if close_tab && !self.tabs.is_empty() {
            self.close_tab(self.active);
        }
        if !self.tabs.is_empty() {
            if next_tab {
                self.active = (self.active + 1) % self.tabs.len();
            }
            if prev_tab {
                self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
            }
        }
        if toggle_sidebar {
            self.sidebar_open = !self.sidebar_open;
        }

        // top bar (on macOS the traffic lights live inside it)
        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let inset = 12;
        egui::Panel::top("topbar")
            .exact_size(44.0)
            .frame(egui::Frame::NONE.fill(pal.bg1).inner_margin(egui::Margin { left: inset, right: 10, top: 0, bottom: 0 }))
            .show(ui, |ui| {
                let bar = ui.max_rect();
                let drag = ui.interact(bar, egui::Id::unique("topbar_drag"), Sense::click_and_drag());
                if drag.drag_started_by(egui::PointerButton::Primary) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
                ui.painter().hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, pal.border));
                ui.horizontal_centered(|ui| {
                    if cfg!(target_os = "macos") && !fullscreen {
                        window_controls(ui);
                        ui.add_space(14.0);
                    }
                    let icon_btn = |ui: &mut Ui, ic: &str, tip: &str, active: bool| crate::ui::widgets::icon_toggle(ui, active, ic, tip).clicked();
                    if icon_btn(ui, icon::PANEL_LEFT, "Toggle sidebar (⌘B)", self.sidebar_open) {
                        self.sidebar_open = !self.sidebar_open;
                    }
                    ui.add_space(4.0);
                    if let Some(logo) = &self.logo {
                        ui.add(egui::Image::new(logo).fit_to_exact_size(vec2(20.0, 20.0)));
                    }
                    ui.label(RichText::new("Trunq").font(theme::bold(14.5)));
                    // centered search pill
                    let w = (ui.available_width() * 0.42).clamp(220.0, 520.0);
                    let left = bar.center().x - w / 2.0;
                    let cur = ui.cursor().left();
                    if left > cur {
                        ui.add_space(left - cur);
                    }
                    let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
                    let hovered = resp.hovered();
                    ui.painter().rect(
                        r,
                        7.0,
                        if hovered { pal.bg3 } else { pal.bg2 },
                        Stroke::new(1.0, if hovered { pal.accent.gamma_multiply(0.5) } else { pal.border }),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().text(
                        r.left_center() + vec2(12.0, 0.0),
                        Align2::LEFT_CENTER,
                        format!("{}   Search tables, functions, commands…", icon::SEARCH),
                        FontId::proportional(13.0),
                        pal.faint,
                    );
                    ui.painter().text(r.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, "⌘K", theme::mono(11.5), pal.faint);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.palette = Some((String::new(), 0));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_btn(ui, icon::SETTINGS, "Settings", self.show_settings) {
                            self.show_settings = !self.show_settings;
                        }
                        let theme_icon = if pal.dark { icon::SUN } else { icon::MOON };
                        if icon_btn(ui, theme_icon, "Toggle light / dark", false) {
                            self.cfg.settings.theme = if pal.dark { ThemeChoice::Light } else { ThemeChoice::Dark };
                            theme::apply(&ctx, self.cfg.settings.theme);
                            self.logo = load_logo(&ctx, self.cfg.settings.theme == ThemeChoice::Dark);
                            self.save_config();
                        }
                        if icon_btn(ui, icon::HISTORY, "Query history", self.show_history) {
                            self.show_history = !self.show_history;
                        }
                        if icon_btn(ui, icon::PLUG, "New connection", false) {
                            actions.push(Action::NewProfile);
                        }
                        if icon_btn(ui, icon::FILE_PLUS, "New query (⌘T)", false) {
                            self.new_query(None, None, String::new());
                        }
                    });
                });
            });

        // status bar
        egui::Panel::bottom("statusbar").frame(egui::Frame::NONE.fill(pal.bg1).inner_margin(egui::Margin::symmetric(10, 3))).show(ui, |ui| {
            ui.horizontal(|ui| {
                let current = self.tabs.get(self.active).and_then(|t| match t {
                    Tab::Query(q) => Some((q.profile_id?, q.database.clone(), q.session.as_ref())),
                    other => Some((other.profile()?, String::new(), None)),
                });
                if let Some((pid, db, session)) = current
                    && let Some(p) = self.profile(pid)
                {
                    // Connection identity: environment · user@server · database
                    if let Some(env) = p.environment.color() {
                        egui::Frame::NONE.fill(theme::rgb(env)).corner_radius(4.0).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
                            ui.label(RichText::new(p.environment.label()).small().strong().color(Color32::BLACK));
                        });
                    }
                    let c = p.color.map(theme::rgb).unwrap_or(pal.accent);
                    let user = session.filter(|s| s.connected).map(|_| p.user.clone()).filter(|u| !u.is_empty()).unwrap_or_else(|| p.user.clone());
                    ui.label(RichText::new(format!("{}  {}", icon::SERVER, p.display_name())).small().color(c))
                        .on_hover_text(format!("{}@{}:{}", user, p.host, p.port));
                    ui.label(
                        RichText::new(format!("{}  {}", icon::USER, if user.is_empty() { "(signed-in account)".into() } else { user })).small().color(pal.dim),
                    );
                    if !db.is_empty() {
                        ui.label(RichText::new(format!("{}  {}", icon::DATABASE, db)).small().color(pal.dim));
                    }
                    let q_session = session;
                    if p.read_only {
                        ui.label(RichText::new(format!("{}  read-only", icon::LOCK)).small().color(pal.warn));
                    }
                    if p.is_azure() {
                        ui.label(RichText::new(format!("{}  Entra ID", icon::CLOUD)).small().color(pal.dim));
                    }
                    if let Some(s) = q_session
                        && s.connected
                    {
                        ui.label(RichText::new(format!("PostgreSQL {}", s.server_version.split_whitespace().next().unwrap_or(""))).small().color(pal.faint));
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let n = session::live_count();
                    if ui
                        .add(
                            egui::Button::new(RichText::new(format!("{}  {n} connection{}", icon::NETWORK, if n == 1 { "" } else { "s" })).small())
                                .frame(false),
                        )
                        .on_hover_text("Show open connections")
                        .clicked()
                    {
                        self.show_connections = !self.show_connections;
                    }
                });
            });
        });

        // sidebar
        if self.sidebar_open {
            egui::Panel::left("sidebar")
                .resizable(true)
                .default_size(270.0)
                .size_range(180.0..=600.0)
                .frame(egui::Frame::NONE.fill(pal.bg1).inner_margin(egui::Margin { left: 8, right: 6, top: 2, bottom: 4 }))
                .show(ui, |ui| self.sidebar(ui, &mut actions));
        }

        // main area
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(pal.bg0)).show(ui, |ui| {
            if self.tabs.is_empty() {
                self.welcome(ui, &mut actions);
                return;
            }
            egui::Frame::NONE.fill(pal.bg1).show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.tab_strip(ui);
            });
            let active = self.active.min(self.tabs.len() - 1);
            let catalog = match &self.tabs[active] {
                Tab::Query(q) => self.catalog(q.profile_id, &q.database),
                Tab::Erd(e) => self.catalog(Some(e.profile_id), &e.database),
                Tab::Health(_) | Tab::Sessions(_) | Tab::Structure(_) => None,
            };
            // Background polling for inactive tabs (so results keep streaming in).
            for (i, t) in self.tabs.iter_mut().enumerate() {
                let Tab::Query(q) = t else { continue };
                q.maintenance();
                if i != active {
                    let cat = None;
                    let mut ctx = TabCtx {
                        settings: &self.cfg.settings,
                        profiles: &self.cfg.profiles,
                        catalog: cat,
                        actions: &mut actions,
                        history: &mut self.history,
                        passwords: &self.passwords,
                        monitor: None,
                        meta: None,
                        table_prefs: &self.cfg.table_prefs,
                        snippets: &self.cfg.snippets,
                    };
                    q.poll(&mut ctx);
                }
            }
            let q = match &mut self.tabs[active] {
                Tab::Query(q) => q,
                Tab::Health(h) => {
                    let profile = self.cfg.profiles.iter().find(|p| p.id == h.profile_id);
                    let pw = self.passwords.get(&h.profile_id).cloned();
                    h.ui(ui, profile, pw, &mut actions);
                    return;
                }
                Tab::Sessions(st) => {
                    let handle =
                        self.servers.iter().find(|s| s.profile_id == st.profile_id).and_then(|s| s.root.as_ref()).filter(|r| r.connected).map(|r| r.handle());
                    let name = self.cfg.profiles.iter().find(|p| p.id == st.profile_id).map(|p| p.display_name()).unwrap_or_default();
                    st.ui(ui, handle, &name);
                    return;
                }
                Tab::Structure(st) => {
                    st.ui(ui, &mut actions);
                    return;
                }
                Tab::Erd(e) => {
                    e.ui(ui, catalog, &mut actions);
                    return;
                }
            };
            let monitor =
                self.servers.iter().find(|s| Some(s.profile_id) == q.profile_id).and_then(|s| s.root.as_ref()).filter(|r| r.connected).map(|r| r.handle());
            let meta = self.servers.iter().find(|s| Some(s.profile_id) == q.profile_id).and_then(|s| {
                let default_db = self.cfg.profiles.iter().find(|p| p.id == s.profile_id)?.database.clone();
                let session = if q.database == default_db { s.root.as_ref() } else { s.nodes.get(&q.database)?.session.as_ref() };
                session.filter(|x| x.connected).map(|x| x.handle())
            });
            let mut ctx = TabCtx {
                settings: &self.cfg.settings,
                profiles: &self.cfg.profiles,
                catalog,
                actions: &mut actions,
                history: &mut self.history,
                passwords: &self.passwords,
                monitor,
                meta,
                table_prefs: &self.cfg.table_prefs,
                snippets: &self.cfg.snippets,
            };
            q.ui(ui, &mut ctx);
        });

        self.drive_active_catalog(&ctx);
        self.pending_ddl.retain(|(uid, p)| {
            let Some(r) = p.take() else { return true };
            if let Some(Tab::Query(q)) = self.tabs.iter_mut().find(|t| matches!(t, Tab::Query(q) if q.uid == *uid)) {
                let text = match r {
                    Ok(rs) => meta::build_table_ddl(&rs),
                    Err(e) => format!("-- could not load DDL: {e}"),
                };
                q.editor.set_text(text.clone());
                q.saved_text = text;
            }
            false
        });
        self.command_palette(&ctx, &mut actions);
        self.overlays(&ctx);
        self.run_script(&ctx, ui, &mut actions);
        actions.append(&mut self.pending_actions);
        self.handle(actions, ui);

        // Auto-save scratchpad: persist the workspace shortly after typing stops.
        let text_hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for t in &self.tabs {
                if let Tab::Query(q) = t {
                    q.editor.text.hash(&mut h);
                }
            }
            h.finish()
        };
        if text_hash != self.last_text_hash {
            self.last_text_hash = text_hash;
            self.text_changed_at = Some(Instant::now());
        }
        let typed_recently = self.text_changed_at.is_some_and(|t| t.elapsed() < Duration::from_millis(1500));
        if (self.text_changed_at.is_some() || self.dirty_config) && !typed_recently {
            self.text_changed_at = None;
            self.dirty_config = false;
            self.last_save = Instant::now();
            self.save_config();
        } else if self.text_changed_at.is_some() {
            ctx.request_repaint_after(Duration::from_millis(1600));
        }
        // "Copy DDL" results
        let mut copied = Vec::new();
        self.pending_copy.retain(|(label, p)| {
            let Some(r) = p.take() else { return true };
            copied.push((label.clone(), r));
            false
        });
        for (label, r) in copied {
            match r {
                Ok(rs) => {
                    let text = if rs.len() > 1 {
                        meta::build_table_ddl(&rs)
                    } else {
                        rs.first().map(|r| r.rows().map(|i| r.s(i, 0).to_string()).collect::<Vec<_>>().join("\n")).unwrap_or_default()
                    };
                    ctx.copy_text(text);
                    self.toast(ToastKind::Success, format!("Copied DDL of {label}"));
                }
                Err(e) => self.toast(ToastKind::Error, format!("Could not get DDL: {e}")),
            }
        }
        // keep spinners alive while anything is connecting/running
        if self.servers.iter().any(|s| s.root.as_ref().is_some_and(|r| r.is_busy()) || s.dbs_promise.is_some() || s.nodes.values().any(|n| n.loading.is_some()))
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn on_exit(&mut self) {
        if self.script.is_some() || std::env::var("TRUNQ_BENCH").is_ok() {
            // scripted runs must not overwrite the user's workspace
            self.tabs.clear();
            self.servers.clear();
            session::kill_all();
            std::thread::sleep(Duration::from_millis(150));
            return;
        }
        self.save_config();
        // Close every connection cleanly before the process exits.
        self.tabs.clear();
        self.servers.clear();
        session::kill_all();
        std::thread::sleep(Duration::from_millis(150));
        session::runtime().block_on(crate::db::ssh::close_all());
    }
}

#[derive(Clone)]
enum PaletteCmd {
    Act(Action),
    NewQuery,
    Settings,
    Theme,
    History,
    Connections,
    Sidebar,
    FormatSql,
    SwitchConnection(u64),
    /// Open SQL in a new tab (and run it, which prompts for :params)
    OpenSql(String, bool),
}

impl App {
    fn command_palette(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::P)) {
            self.palette = if self.palette.is_some() { None } else { Some((String::new(), 0)) };
        }
        let Some((mut query, mut selected)) = self.palette.take() else { return };
        let pal = theme::p();
        // gather candidates
        let mut items: Vec<(&'static str, Color32, String, String, PaletteCmd)> = vec![
            (icon::FILE_PLUS, pal.accent, "New query".into(), "⌘T".into(), PaletteCmd::NewQuery),
            (icon::PLUG, pal.accent, "New connection".into(), String::new(), PaletteCmd::Act(Action::NewProfile)),
            (icon::HISTORY, pal.dim, "Query history".into(), String::new(), PaletteCmd::History),
            (
                icon::IMPORT,
                pal.dim,
                "Import CSV…".into(),
                "or drop a file".into(),
                PaletteCmd::Act(Action::ImportCsv { profile: None, db: None, schema: None, path: None }),
            ),
            (icon::NETWORK, pal.dim, "Open connections".into(), String::new(), PaletteCmd::Connections),
            (icon::SETTINGS, pal.dim, "Settings".into(), String::new(), PaletteCmd::Settings),
            (icon::SUN, pal.dim, "Toggle light / dark theme".into(), String::new(), PaletteCmd::Theme),
            (icon::PANEL_LEFT, pal.dim, "Toggle sidebar".into(), "⌘B".into(), PaletteCmd::Sidebar),
        ];
        if matches!(self.tabs.get(self.active), Some(Tab::Query(q)) if q.table.is_none()) {
            items.push((icon::ALIGN_LEFT, pal.dim, "Format SQL".into(), "⇧⌘F".into(), PaletteCmd::FormatSql));
            for p in &self.cfg.profiles {
                items.push((icon::PLUG, pal.dim, format!("Switch this tab to {}", p.display_name()), "connection".into(), PaletteCmd::SwitchConnection(p.id)));
            }
        }
        for sn in &self.cfg.snippets {
            items.push((icon::BOOK, pal.accent, format!("Snippet: {}", sn.name), "run with parameters".into(), PaletteCmd::OpenSql(sn.sql.clone(), true)));
        }
        for h in self.history.entries.iter().rev().take(15) {
            let one_line: String = h.sql.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
            items.push((icon::HISTORY, pal.faint, one_line, format!("recent · {}", h.database), PaletteCmd::OpenSql(h.sql.clone(), false)));
        }
        for p in &self.cfg.profiles {
            items.push((
                icon::SERVER,
                p.color.map(theme::rgb).unwrap_or(pal.accent),
                format!("Query {}", p.display_name()),
                format!("{}:{}", p.host, p.port),
                PaletteCmd::Act(Action::NewQuery { profile: p.id, db: p.database.clone(), sql: String::new(), run: false }),
            ));
            items.push((
                icon::ACTIVITY,
                pal.dim,
                format!("Server activity · {}", p.display_name()),
                String::new(),
                PaletteCmd::Act(Action::OpenDashboard { profile: p.id }),
            ));
            items.push((
                icon::SHIELD,
                pal.dim,
                format!("Health check · {}", p.display_name()),
                p.database.clone(),
                PaletteCmd::Act(Action::OpenHealth { profile: p.id, db: p.database.clone() }),
            ));
        }
        for s in &self.servers {
            for (db, n) in &s.nodes {
                items.push((
                    icon::NETWORK,
                    pal.accent,
                    format!("ER diagram · {db}"),
                    "relationships".into(),
                    PaletteCmd::Act(Action::OpenErd { profile: s.profile_id, db: db.clone(), schema: None, focus: None }),
                ));

                let Some(cat) = &n.catalog else { continue };
                for r in cat.relations() {
                    if r.kind.has_data() {
                        items.push((
                            r.kind.icon_glyph(),
                            pal.success,
                            format!("{}.{}", r.schema, r.name),
                            format!("{} · {db}", r.kind.label()),
                            PaletteCmd::Act(Action::OpenTable { profile: s.profile_id, db: db.clone(), schema: r.schema.clone(), name: r.name.clone() }),
                        ));
                    }
                }
                for f in cat.functions() {
                    items.push((
                        icon::FUNCTION,
                        pal.number,
                        format!("{}.{}()", f.schema, f.name),
                        format!("function · {db}"),
                        PaletteCmd::Act(Action::NewQuery {
                            profile: s.profile_id,
                            db: db.clone(),
                            sql: format!("SELECT pg_get_functiondef({}::regprocedure);", meta::quote_literal(&f.signature())),
                            run: true,
                        }),
                    ));
                }
            }
        }
        let q = query.trim().to_lowercase();
        let mut scored: Vec<(i32, usize)> = items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| {
                if q.is_empty() {
                    return (i < 40).then_some((0, i));
                }
                let l = it.2.to_lowercase();
                let name = l.rsplit('.').next().unwrap_or(&l);
                let s = if name == q {
                    1000
                } else if name.starts_with(&q) {
                    800
                } else if l.contains(&q) {
                    500
                } else {
                    let mut chars = l.chars();
                    if q.chars().filter(|c| !c.is_whitespace()).all(|c| chars.any(|x| x == c)) { 100 } else { return None }
                };
                Some((s - l.len() as i32 / 4, i))
            })
            .collect();
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        scored.truncate(60);
        let n = scored.len();
        let (up, down, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
            )
        });
        if n > 0 {
            if down {
                selected = (selected + 1) % n;
            }
            if up {
                selected = (selected + n - 1) % n;
            }
        }
        selected = selected.min(n.saturating_sub(1));
        let mut chosen = enter.then_some(selected).filter(|_| n > 0);
        let mut close = esc;
        let modal = egui::Modal::new(egui::Id::unique("palette"))
            .frame(w::modal_frame().inner_margin(egui::Margin::ZERO))
            .area(egui::Modal::default_area(egui::Id::unique("palette_area")).anchor(Align2::CENTER_TOP, vec2(0.0, 90.0)))
            .show(ctx, |ui| {
                ui.set_width(660.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                // search row
                let (row, _) = ui.allocate_exact_size(vec2(660.0, 54.0), Sense::hover());
                ui.painter().text(pos2(row.left() + 22.0, row.center().y), Align2::CENTER_CENTER, icon::SEARCH, FontId::proportional(17.0), pal.accent);
                let inner = egui::Rect::from_min_max(pos2(row.left() + 44.0, row.center().y - 9.0), pos2(row.right() - 70.0, row.center().y + 13.0));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
                let r = child.add(
                    egui::TextEdit::singleline(&mut query)
                        .hint_text(RichText::new("Search tables, functions, connections and commands").color(pal.faint))
                        .font(FontId::proportional(16.0))
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .desired_width(inner.width()),
                );
                r.request_focus();
                if r.changed() {
                    selected = 0;
                }
                let cap = egui::Rect::from_center_size(pos2(row.right() - 36.0, row.center().y), vec2(36.0, 22.0));
                ui.painter().rect(cap, 5.0, pal.bg2, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
                ui.painter().text(cap.center(), Align2::CENTER_CENTER, "esc", FontId::proportional(11.0), pal.dim);
                ui.painter().hline(row.x_range(), row.bottom(), Stroke::new(1.0, pal.border));

                egui::Frame::NONE.inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(400.0).auto_shrink([false, true]).show(ui, |ui| {
                        for (row, (_, idx)) in scored.iter().enumerate() {
                            let (ic, color, label, detail, _) = &items[*idx];
                            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                            let active = row == selected;
                            if active {
                                ui.painter().rect_filled(rect, 8.0, pal.accent_soft);
                                if up || down {
                                    resp.scroll_to_me(None);
                                }
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, 8.0, pal.control_hover);
                            }
                            let tile = egui::Rect::from_center_size(pos2(rect.left() + 22.0, rect.center().y), vec2(28.0, 28.0));
                            ui.painter().rect_filled(tile, 7.0, if active { pal.bg1 } else { pal.bg2 });
                            ui.painter().text(tile.center(), Align2::CENTER_CENTER, *ic, FontId::proportional(14.0), *color);
                            if active {
                                ui.painter().text(
                                    pos2(rect.right() - 12.0, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    "↵",
                                    FontId::proportional(13.0),
                                    pal.accent,
                                );
                            }
                            let dr = ui.painter().text(
                                pos2(rect.right() - if active { 32.0 } else { 12.0 }, rect.center().y),
                                Align2::RIGHT_CENTER,
                                detail,
                                FontId::proportional(12.0),
                                pal.faint,
                            );
                            let g = w::truncated(ui, label, FontId::proportional(14.0), pal.text, dr.left() - rect.left() - 46.0 - 16.0);
                            ui.painter().galley(pos2(rect.left() + 46.0, rect.center().y - g.size().y / 2.0), g, pal.text);
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                chosen = Some(row);
                            }
                        }
                        if scored.is_empty() {
                            ui.add_space(18.0);
                            ui.vertical_centered(|ui| ui.label(RichText::new("No matches").color(pal.faint)));
                            ui.add_space(18.0);
                        }
                    });
                });
                // footer hints
                let (foot, _) = ui.allocate_exact_size(vec2(660.0, 36.0), Sense::hover());
                ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, pal.border));
                let mut x = foot.left() + 16.0;
                for (k, t) in [("↑↓", "navigate"), ("↵", "open"), ("esc", "close")] {
                    let g = ui.fonts_mut(|f| f.layout_no_wrap(k.to_string(), FontId::proportional(11.0), pal.dim));
                    let cap = egui::Rect::from_min_size(pos2(x, foot.center().y - 10.0), vec2(g.size().x + 12.0, 20.0));
                    ui.painter().rect(cap, 5.0, pal.bg2, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
                    ui.painter().galley(cap.center() - g.size() / 2.0, g, pal.dim);
                    let tr = ui.painter().text(pos2(cap.right() + 6.0, foot.center().y), Align2::LEFT_CENTER, t, FontId::proportional(12.0), pal.faint);
                    x = tr.right() + 16.0;
                }
                ui.painter().text(
                    pos2(foot.right() - 16.0, foot.center().y),
                    Align2::RIGHT_CENTER,
                    format!("{} results", scored.len()),
                    FontId::proportional(12.0),
                    pal.faint,
                );
            });
        if modal.should_close() {
            close = true;
        }
        if let Some(row) = chosen
            && let Some((_, idx)) = scored.get(row)
        {
            match items[*idx].4.clone() {
                PaletteCmd::Act(a) => actions.push(a),
                PaletteCmd::NewQuery => {
                    self.new_query(None, None, String::new());
                }
                PaletteCmd::Settings => self.show_settings = true,
                PaletteCmd::Theme => {
                    self.cfg.settings.theme = if pal.dark { ThemeChoice::Light } else { ThemeChoice::Dark };
                    theme::apply(ctx, self.cfg.settings.theme);
                }
                PaletteCmd::History => self.show_history = true,
                PaletteCmd::Connections => self.show_connections = true,
                PaletteCmd::Sidebar => self.sidebar_open = !self.sidebar_open,
                PaletteCmd::FormatSql => {
                    if let Some(Tab::Query(q)) = self.tabs.get_mut(self.active) {
                        let formatted = crate::sql::format::format_sql(&q.editor.text, self.cfg.settings.uppercase_keywords);
                        let end = q.editor.text.len();
                        q.editor.replace_range(0..end, formatted.trim_end());
                    }
                }
                PaletteCmd::SwitchConnection(pid) => {
                    let db = self.profile(pid).map(|p| p.database.clone()).unwrap_or_default();
                    if let Some(Tab::Query(q)) = self.tabs.get_mut(self.active)
                        && q.table.is_none()
                    {
                        q.profile_id = Some(pid);
                        q.database = db;
                        q.session = None;
                    }
                }
                PaletteCmd::OpenSql(sql, run) => {
                    let profile = self.tabs.get(self.active).and_then(|t| t.profile()).or_else(|| self.cfg.profiles.first().map(|p| p.id));
                    if let Some(pid) = profile {
                        let db = match self.tabs.get(self.active) {
                            Some(Tab::Query(q)) => q.database.clone(),
                            _ => self.profile(pid).map(|p| p.database.clone()).unwrap_or_default(),
                        };
                        actions.push(Action::NewQuery { profile: pid, db, sql, run });
                    }
                }
            }
            close = true;
        }
        if !close {
            self.palette = Some((query, selected));
        }
    }

    fn run_script(&mut self, ctx: &egui::Context, ui: &Ui, actions: &mut Vec<Action>) {
        use crate::devscript::Step;
        let Some(script) = &mut self.script else { return };
        if script.waiting_for_screenshot() {
            // Keep submitting frames until the GPU readback callback fires.
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let Some(step) = script.next() else { return };
        let profile = self.cfg.profiles.first().map(|p| (p.id, p.database.clone()));
        match step {
            Step::Wait(_) => {}
            Step::Shot(path) => crate::devscript::screenshot(ctx, path),
            Step::Run | Step::RunAll | Step::Explain(_) => {
                let all = matches!(step, Step::RunAll);
                let explain = if let Step::Explain(a) = step { Some(a) } else { None };
                if let Some(Tab::Query(q)) = self.tabs.get_mut(self.active) {
                    let catalog =
                        self.servers.iter().find(|s| Some(s.profile_id) == q.profile_id).and_then(|s| s.nodes.get(&q.database)).and_then(|n| n.catalog.clone());
                    let mut tctx = TabCtx {
                        settings: &self.cfg.settings,
                        profiles: &self.cfg.profiles,
                        catalog,
                        actions,
                        history: &mut self.history,
                        passwords: &self.passwords,
                        monitor: None,
                        meta: None,
                        table_prefs: &self.cfg.table_prefs,
                        snippets: &self.cfg.snippets,
                    };
                    q.run_current(&mut tctx, ui, all, explain);
                }
            }
            Step::OpenTable(schema, name) => {
                if let Some((pid, db)) = profile {
                    actions.push(Action::OpenTable { profile: pid, db, schema, name });
                }
            }
            Step::Sql(sql) => {
                if let Some(Tab::Query(q)) = self.tabs.get_mut(self.active) {
                    q.editor.set_text(sql);
                }
            }
            Step::DryRun => {
                if let Some(Tab::Query(q)) = self.tabs.get_mut(self.active) {
                    let mut tctx = TabCtx {
                        settings: &self.cfg.settings,
                        profiles: &self.cfg.profiles,
                        catalog: None,
                        actions,
                        history: &mut self.history,
                        passwords: &self.passwords,
                        monitor: None,
                        meta: None,
                        table_prefs: &self.cfg.table_prefs,
                        snippets: &self.cfg.snippets,
                    };
                    q.dry_run(&mut tctx, ui);
                }
            }
            Step::Tab(i) => self.active = i.min(self.tabs.len().saturating_sub(1)),
            Step::Theme(light) => {
                self.cfg.settings.theme = if light { ThemeChoice::Light } else { ThemeChoice::Dark };
                theme::apply(ctx, self.cfg.settings.theme);
            }
            Step::NewConnection => self.dialog = Some(ConnDialog::new(None)),
            Step::CloseDialogs => {
                self.dialog = None;
                self.palette = None;
                self.import = None;
                self.show_connections = false;
                self.show_settings = false;
                self.show_history = false;
                if let Some(Tab::Query(q)) = self.tabs.get_mut(self.active) {
                    q.dismiss_dialogs();
                }
            }
            Step::Connections => self.show_connections = true,
            Step::Settings => self.show_settings = true,
            Step::History => self.show_history = true,
            Step::Palette(q) => self.palette = Some((q, 0)),
            Step::Import(path) => actions.push(Action::ImportCsv { profile: None, db: None, schema: None, path: Some(path.into()) }),
            Step::ImportRun => {
                if let Some(d) = &mut self.import {
                    d.start(ctx, &self.cfg.profiles, &self.passwords);
                }
            }
            Step::TabDev(cmd, arg) => {
                if let Some(Tab::Query(q)) = self.tabs.get_mut(self.active) {
                    q.dev(&cmd, &arg);
                }
            }
            Step::Sessions => {
                if let Some((pid, _)) = profile {
                    actions.push(Action::OpenDashboard { profile: pid });
                }
            }
            Step::Structure(name) => {
                if let Some((pid, db)) = profile.clone()
                    && let Some(cat) = self.catalog(Some(pid), &db)
                {
                    let (schema, table) = name.split_once('.').unwrap_or(("public", &name));
                    if let Some(rel) = cat.find_relation(Some(schema), table) {
                        actions.push(Action::OpenStructure { profile: pid, db, oid: rel.oid, title: rel.name.clone(), ddl: false });
                    }
                }
            }
            Step::Erd(schema) => {
                if let Some((pid, db)) = profile {
                    actions.push(Action::OpenErd { profile: pid, db, schema: (!schema.is_empty()).then_some(schema), focus: None });
                }
            }
            Step::Resize(w, h) => ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(w, h))),
            Step::Chart => {
                if let Some(Tab::Query(q)) = self.tabs.get_mut(self.active)
                    && let Some(p) = q.panes.first_mut()
                {
                    p.chart = Some(Default::default());
                }
            }
            Step::Health => {
                if let Some((pid, db)) = profile {
                    actions.push(Action::OpenHealth { profile: pid, db });
                }
            }
            Step::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
        if self.script.as_ref().is_some_and(|s| s.done()) {
            eprintln!("TRUNQ_SCRIPT: done");
        }
    }
}

/// macOS-style close / minimise / zoom buttons, drawn by Trunq so they sit
/// on the same line as the toolbar. Option-click on zoom toggles full screen.
fn window_controls(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
    let d = 12.0;
    let gap = 8.0;
    let (group, _) = ui.allocate_exact_size(vec2(d * 3.0 + gap * 2.0, d), Sense::hover());
    let group_hover = ui.rect_contains_pointer(group.expand(4.0));
    let colors = [
        (Color32::from_rgb(0xFF, 0x5F, 0x57), Color32::from_rgb(0xE0, 0x44, 0x3E)),
        (Color32::from_rgb(0xFE, 0xBC, 0x2E), Color32::from_rgb(0xDE, 0xA1, 0x23)),
        (Color32::from_rgb(0x28, 0xC8, 0x40), Color32::from_rgb(0x1D, 0xAD, 0x2B)),
    ];
    let glyphs = ["×", "−", "+"];
    for i in 0..3 {
        let c = egui::pos2(group.left() + d / 2.0 + i as f32 * (d + gap), group.center().y);
        let r = egui::Rect::from_center_size(c, vec2(d, d));
        let resp = ui.interact(r.expand(2.0), egui::Id::unique(("wctl", i)), Sense::click());
        let (fill, edge) = if focused || group_hover { colors[i] } else { (theme::p().bg3, theme::p().border_strong) };
        ui.painter().circle(c, d / 2.0, fill, Stroke::new(0.5, edge));
        if group_hover {
            ui.painter().text(c + vec2(0.0, -0.5), Align2::CENTER_CENTER, glyphs[i], FontId::proportional(11.0), Color32::from_black_alpha(150));
        }
        if resp.clicked() {
            match i {
                0 => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                1 => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                _ => {
                    if ui.input(|i| i.modifiers.alt) {
                        let fs = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
                    } else {
                        let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                    }
                }
            }
        }
    }
}

/// Loads the logo cropped to its visible bounds. On dark backgrounds the
/// navy parts are recoloured light so the mark stays visible.
fn load_logo(ctx: &egui::Context, dark: bool) -> Option<egui::TextureHandle> {
    // Pre-cropped 128px mark (decoding the full-size artwork cost ~100 ms at startup).
    let mut img = image::load_from_memory(include_bytes!("../assets/logo_128.png")).ok()?.to_rgba8();
    if dark {
        for p in img.pixels_mut() {
            let [r, g, b, a] = p.0;
            let max = r.max(g).max(b) as i32;
            let min = r.min(g).min(b) as i32;
            // navy: dark & low saturation
            if a > 0 && max < 90 && max - min < 50 {
                *p = image::Rgba([228, 235, 243, a]);
            }
        }
    }
    let size = [img.width() as usize, img.height() as usize];
    let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture("logo", ci, egui::TextureOptions::LINEAR))
}

fn first_line(s: &str) -> String {
    let l = s.lines().next().unwrap_or("");
    if l.chars().count() > 80 { format!("{}…", l.chars().take(80).collect::<String>()) } else { l.to_string() }
}

/// A collapsible metadata group below a relation or server.
fn object_group(ui: &mut Ui, open: &mut HashSet<String>, key: String, depth: usize, ico: &str, label: &str, count: usize) -> bool {
    if count == 0 {
        return false;
    }
    let expanded = open.contains(&key);
    let (r, _) = tree_row(ui, depth, Some(expanded), ico, theme::p().dim, label, &count.to_string(), None, false, false);
    if r.clicked() && !open.remove(&key) {
        open.insert(key);
    }
    expanded
}

fn relation_details_tree(ui: &mut Ui, rel: &meta::Relation, open: &mut HashSet<String>) {
    let pal = theme::p();
    let key = |group| format!("{}.{} / {group}", rel.schema, rel.name);
    if object_group(ui, open, key("indexes"), 5, icon::HASH, "Indexes", rel.indexes.len()) {
        for i in &rel.indexes {
            let detail = format!("{}{}{}", i.method, if i.unique { " · unique" } else { "" }, if i.valid { "" } else { " · INVALID" });
            let (r, _) = tree_row(
                ui,
                6,
                None,
                if i.primary { icon::KEY } else { icon::HASH },
                if i.valid { pal.dim } else { pal.danger },
                &i.name,
                &detail,
                None,
                false,
                false,
            );
            r.on_hover_text(format!("{} ({}){}", i.name, i.columns.join(", "), if i.partial { " · partial index" } else { "" }));
        }
    }
    if object_group(ui, open, key("constraints"), 5, icon::SHIELD, "Constraints", rel.constraints.len()) {
        for c in &rel.constraints {
            let (r, _) = tree_row(ui, 6, None, icon::SHIELD, if c.validated { pal.dim } else { pal.warn }, &c.name, c.kind_label(), None, false, false);
            r.on_hover_text(format!(
                "{} ({}){}\n{}",
                c.kind_label(),
                c.columns.join(", "),
                if c.validated { "" } else { " · NOT VALID" },
                c.definition.as_deref().unwrap_or("")
            ));
        }
    }
    if object_group(ui, open, key("triggers"), 5, icon::ZAP, "Triggers", rel.triggers.len()) {
        for t in &rel.triggers {
            let (r, _) = tree_row(ui, 6, None, icon::ZAP, pal.dim, &t.name, if t.enabled { "enabled" } else { "disabled" }, None, false, false);
            r.on_hover_text(format!("{}\n{}", t.when, t.function));
        }
    }
    if rel.rls {
        let (r, _) = tree_row(ui, 5, None, icon::LOCK, pal.warn, "Row security enabled", "RLS", None, false, false);
        r.on_hover_text("Row-level security policies can restrict the rows visible to your database role.");
    }
    if object_group(ui, open, key("policies"), 5, icon::LOCK, "Policies", rel.policies.len()) {
        for p in &rel.policies {
            let (r, _) = tree_row(ui, 6, None, icon::LOCK, pal.dim, &p.name, &p.command, None, false, false);
            r.on_hover_text(format!("{} · roles: {}", if p.permissive { "permissive" } else { "restrictive" }, p.roles.join(", ")));
        }
    }
}

fn server_objects_tree(ui: &mut Ui, server: &mut Server, profile: &Profile, filter: &str) {
    let pal = theme::p();
    let roles: Vec<_> = server
        .roles
        .iter()
        .flatten()
        .filter(|r| (profile.show_system_objects || !r.is_system()) && (filter.is_empty() || r.name.to_lowercase().contains(filter)))
        .collect();
    if object_group(ui, &mut server.open, "roles".into(), 1, icon::USERS, "Roles", roles.len()) {
        for role in roles {
            let detail = if role.is_current {
                "current role"
            } else if role.login {
                "login"
            } else {
                "group"
            };
            let (r, _) = tree_row(ui, 2, None, icon::USER, if role.is_current { pal.accent } else { pal.dim }, &role.name, detail, None, false, false);
            r.on_hover_text(format!("{}\nMember of: {}\n{}", role.attributes().join(" · "), role.member_of.join(", "), role.comment.as_deref().unwrap_or("")))
                .context_menu(|ui| {
                    if w::menu_item(ui, icon::CLIPBOARD, "Copy role DDL").clicked() {
                        ui.ctx().copy_text(role.ddl());
                        ui.close();
                    }
                });
        }
    }
    let spaces: Vec<_> = server.tablespaces.iter().filter(|(name, _, _)| filter.is_empty() || name.to_lowercase().contains(filter)).collect();
    if object_group(ui, &mut server.open, "tablespaces".into(), 1, icon::DATABASE, "Tablespaces", spaces.len()) {
        for (name, owner, location) in spaces {
            let (r, _) = tree_row(ui, 2, None, icon::DATABASE, pal.dim, name, owner, None, false, false);
            r.on_hover_text(format!("Owner: {owner}\n{location}"));
        }
    }
}

/// Context menu for tables / views in the object tree.
fn relation_menu(ui: &mut Ui, pid: u64, db: &str, rel: &meta::Relation, actions: &mut Vec<Action>) {
    ui.set_min_width(220.0);
    let db = db.to_string();
    if rel.kind.has_data() && w::menu_item(ui, icon::TABLE, "Open data").clicked() {
        actions.push(Action::OpenTable { profile: pid, db: db.clone(), schema: rel.schema.clone(), name: rel.name.clone() });
        ui.close();
    }
    if rel.kind.has_data() && w::menu_item(ui, icon::PLAY, "Query (SELECT … LIMIT 100)").clicked() {
        actions.push(Action::NewQuery { profile: pid, db: db.clone(), sql: format!("SELECT *\nFROM {}\nLIMIT 100;", rel.qualified()), run: true });
        ui.close();
    }
    if w::menu_item(ui, icon::CODE, "Show DDL").clicked() {
        if rel.kind == RelKind::Sequence {
            actions.push(Action::NewQuery { profile: pid, db: db.clone(), sql: format!("{};", meta::sequence_ddl_sql(&rel.schema, &rel.name)), run: true });
        } else {
            actions.push(Action::OpenStructure { profile: pid, db: db.clone(), oid: rel.oid, title: rel.name.clone(), ddl: true });
        }
        ui.close();
    }
    if w::menu_item(ui, icon::TABLE_PROPS, "Structure & health").clicked() && rel.kind != RelKind::Sequence {
        actions.push(Action::OpenStructure { profile: pid, db: db.clone(), oid: rel.oid, title: rel.name.clone(), ddl: false });
        ui.close();
    }
    if rel.kind.is_table_like() && w::menu_item(ui, icon::NETWORK, "ER diagram around this table").clicked() {
        actions.push(Action::OpenErd { profile: pid, db: db.clone(), schema: None, focus: Some((rel.oid, rel.name.clone())) });
        ui.close();
    }
    if w::menu_item(ui, icon::CLIPBOARD, "Copy DDL").clicked() {
        let sql = if rel.kind == RelKind::Sequence { meta::sequence_ddl_sql(&rel.schema, &rel.name) } else { meta::table_details_sql(rel.oid) };
        actions.push(Action::CopyDdl { profile: pid, db: db.clone(), sql, label: rel.qualified() });
        ui.close();
    }
    ui.menu_button(format!("{}  Generate SQL", icon::WAND), |ui| {
        let cols: Vec<String> = rel.columns.iter().map(|c| meta::quote_ident(&c.name)).collect();
        let q = rel.qualified();
        let pk = rel.pk_columns();
        let where_pk = if pk.is_empty() { String::new() } else { pk.iter().map(|c| format!("{} = ?", meta::quote_ident(c))).collect::<Vec<_>>().join(" AND ") };
        let gens = [
            ("SELECT", format!("SELECT {}\nFROM {q}\nWHERE ;", cols.join(",\n       "))),
            ("INSERT", format!("INSERT INTO {q} ({})\nVALUES ({});", cols.join(", "), cols.iter().map(|_| "?").collect::<Vec<_>>().join(", "))),
            ("UPDATE", format!("UPDATE {q}\nSET {}\nWHERE {where_pk};", cols.iter().map(|c| format!("{c} = ?")).collect::<Vec<_>>().join(",\n    "))),
            ("DELETE", format!("DELETE FROM {q}\nWHERE {where_pk};")),
            ("COUNT", format!("SELECT count(*) FROM {q};")),
            ("EXPLAIN ANALYZE", format!("EXPLAIN (ANALYZE, BUFFERS)\nSELECT * FROM {q}\nLIMIT 100;")),
        ];
        for (label, sql) in gens {
            if w::menu_item(ui, icon::CODE, label).clicked() {
                actions.push(Action::NewQuery { profile: pid, db: db.clone(), sql, run: false });
                ui.close();
            }
        }
    });
    ui.separator();
    if rel.kind == RelKind::MatView && w::menu_item(ui, icon::REFRESH, "Refresh materialized view").clicked() {
        actions.push(Action::NewQuery { profile: pid, db: db.clone(), sql: format!("REFRESH MATERIALIZED VIEW {};", rel.qualified()), run: false });
        ui.close();
    }
    if rel.kind.is_table_like() {
        ui.menu_button(format!("{}  Maintenance", icon::SLIDERS), |ui| {
            for (label, sql) in [
                ("VACUUM (ANALYZE)", format!("VACUUM (ANALYZE, VERBOSE) {};", rel.qualified())),
                ("ANALYZE", format!("ANALYZE VERBOSE {};", rel.qualified())),
                ("REINDEX", format!("REINDEX TABLE CONCURRENTLY {};", rel.qualified())),
                ("Exact row count", format!("SELECT count(*) FROM {};", rel.qualified())),
                ("Size on disk", format!("SELECT pg_size_pretty(pg_total_relation_size('{0}')) AS total,\n       pg_size_pretty(pg_relation_size('{0}')) AS table,\n       pg_size_pretty(pg_indexes_size('{0}')) AS indexes;", rel.qualified().replace('\'', "''"))),
            ] {
                if w::menu_item(ui, icon::PLAY, label).clicked() {
                    actions.push(Action::NewQuery { profile: pid, db: db.clone(), sql, run: false });
                    ui.close();
                }
            }
        });
    }
    if w::menu_item(ui, icon::COPY, "Copy name").clicked() {
        ui.ctx().copy_text(rel.qualified());
        ui.close();
    }
    ui.separator();
    let kw = match rel.kind {
        RelKind::View => "VIEW",
        RelKind::MatView => "MATERIALIZED VIEW",
        RelKind::Sequence => "SEQUENCE",
        RelKind::Foreign => "FOREIGN TABLE",
        _ => "TABLE",
    };
    if rel.kind.is_table_like() && ui.button(RichText::new(format!("{}  Truncate…", icon::ERASER)).color(theme::p().warn)).clicked() {
        actions.push(Action::NewQuery {
            profile: pid,
            db: db.clone(),
            sql: format!("-- Removes ALL rows. Review, then run with ⌘⏎\nTRUNCATE TABLE {};", rel.qualified()),
            run: false,
        });
        ui.close();
    }
    if ui.button(RichText::new(format!("{}  Drop…", icon::TRASH)).color(theme::p().danger)).clicked() {
        actions.push(Action::NewQuery {
            profile: pid,
            db: db.clone(),
            sql: format!("-- Review carefully, then run with ⌘⏎\nDROP {kw} {};", rel.qualified()),
            run: false,
        });
        ui.close();
    }
}

/// A server (connection profile) row: avatar, name, host subtitle, status.
/// Returns the response and which hover action (0 = new query, 1 = reconnect) was clicked.
#[allow(clippy::too_many_arguments)]
fn server_row(
    ui: &mut Ui,
    name: &str,
    subtitle: &str,
    color: Color32,
    status: Color32,
    expanded: bool,
    azure: bool,
    read_only: bool,
) -> (egui::Response, Option<usize>) {
    let pal = theme::p();
    let h = 42.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let hovered = resp.hovered() || ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p));
    let p = ui.painter();
    if hovered {
        p.rect_filled(rect, 7.0, pal.bg3);
    }
    p.text(
        pos2(rect.left() + 9.0, rect.center().y),
        Align2::CENTER_CENTER,
        if expanded { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT },
        FontId::proportional(11.0),
        pal.faint,
    );
    let av = egui::Rect::from_center_size(pos2(rect.left() + 32.0, rect.center().y), vec2(26.0, 26.0));
    p.rect_filled(av, 7.0, color.gamma_multiply(0.18));
    p.rect_stroke(av, 7.0, Stroke::new(1.0, color.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    let initial: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
    p.text(av.center(), Align2::CENTER_CENTER, initial, theme::bold(13.0), color);
    p.circle_filled(av.right_bottom() - vec2(1.0, 1.0), 4.5, pal.bg1);
    p.circle_filled(av.right_bottom() - vec2(1.0, 1.0), 3.2, status);
    let x = av.right() + 10.0;
    let right_pad = if hovered { 58.0 } else { 6.0 };
    let max_w = (rect.right() - x - right_pad).max(10.0);
    let mut badges = String::new();
    if azure {
        badges.push_str(icon::CLOUD);
        badges.push(' ');
    }
    if read_only {
        badges.push_str(icon::LOCK);
    }
    let name_g = ui.fonts_mut(|f| {
        let mut j = egui::text::LayoutJob::single_section(name.to_string(), egui::TextFormat::simple(theme::bold(13.5), pal.text));
        j.wrap = egui::text::TextWrapping::truncate_at_width(max_w - if badges.is_empty() { 0.0 } else { 30.0 });
        f.layout_job(j)
    });
    let name_w = name_g.size().x;
    p.galley(pos2(x, rect.top() + 5.0), name_g, pal.text);
    if !badges.is_empty() {
        p.text(pos2(x + name_w + 6.0, rect.top() + 13.0), Align2::LEFT_CENTER, badges, FontId::proportional(11.0), pal.dim);
    }
    let sub_g = ui.fonts_mut(|f| {
        let mut j = egui::text::LayoutJob::single_section(subtitle.to_string(), egui::TextFormat::simple(FontId::proportional(11.5), pal.faint));
        j.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
        f.layout_job(j)
    });
    p.galley(pos2(x, rect.top() + 23.0), sub_g, pal.faint);
    let mut action = None;
    if hovered {
        for (i, (ic, tip)) in [(icon::FILE_PLUS, "New query"), (icon::REFRESH, "Reconnect / refresh")].iter().enumerate() {
            let r = egui::Rect::from_center_size(pos2(rect.right() - 16.0 - (1 - i) as f32 * 26.0, rect.center().y), vec2(24.0, 24.0));
            let br = ui.interact(r, resp.id.with(("act", i)), Sense::click());
            if br.hovered() {
                ui.painter().rect_filled(r, 5.0, pal.bg2);
            }
            ui.painter().text(r.center(), Align2::CENTER_CENTER, *ic, FontId::proportional(13.0), if br.hovered() { pal.accent } else { pal.dim });
            if br.on_hover_text(*tip).clicked() {
                action = Some(i);
            }
        }
    }
    (resp, action)
}

/// One row of the object tree. Returns the row response and whether the
/// expand chevron itself was clicked.
#[allow(clippy::too_many_arguments)]
fn tree_row(
    ui: &mut Ui,
    depth: usize,
    expanded: Option<bool>,
    ico: &str,
    ico_color: Color32,
    label: &str,
    detail: &str,
    status: Option<Color32>,
    selected: bool,
    mono_detail: bool,
) -> (egui::Response, bool) {
    let pal = theme::p();
    let h = 25.0;
    let indent = 14.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 6.0, pal.accent_soft);
    } else if resp.hovered() {
        p.rect_filled(rect, 6.0, pal.bg3);
    }
    // indent guides
    for d in 2..=depth {
        let gx = rect.left() + 9.0 + (d - 1) as f32 * indent;
        p.vline(gx, rect.y_range(), Stroke::new(1.0, pal.border.gamma_multiply(0.55)));
    }
    let base = rect.left() + 2.0 + depth as f32 * indent;
    let chevron_rect = egui::Rect::from_center_size(pos2(base + 7.0, rect.center().y), vec2(16.0, h));
    if let Some(open) = expanded {
        p.text(
            chevron_rect.center(),
            Align2::CENTER_CENTER,
            if open { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT },
            FontId::proportional(11.0),
            pal.faint,
        );
    }
    let x = base + 18.0;
    p.text(pos2(x + 7.0, rect.center().y), Align2::CENTER_CENTER, ico, FontId::proportional(13.0), ico_color);
    if let Some(s) = status {
        p.circle_filled(pos2(x + 13.0, rect.center().y + 5.0), 3.0, s);
    }
    let lx = x + 21.0;
    let detail_font = if mono_detail { theme::mono(10.5) } else { FontId::proportional(11.0) };
    let detail_g = (!detail.is_empty()).then(|| {
        ui.fonts_mut(|f| {
            let mut j = egui::text::LayoutJob::single_section(detail.to_string(), egui::TextFormat::simple(detail_font.clone(), pal.faint));
            j.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - lx) * 0.45);
            f.layout_job(j)
        })
    });
    let detail_w = detail_g.as_ref().map_or(0.0, |g| g.size().x + 12.0);
    let job = {
        let mut j = egui::text::LayoutJob::single_section(
            label.to_string(),
            egui::TextFormat::simple(FontId::proportional(13.0), if selected { pal.text } else { pal.text.gamma_multiply(0.92) }),
        );
        j.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - lx - detail_w - 4.0).max(10.0));
        j
    };
    let g = ui.fonts_mut(|f| f.layout_job(job));
    p.galley(pos2(lx, rect.center().y - g.size().y / 2.0), g, pal.text);
    if let Some(dg) = detail_g {
        let pill = expanded.is_some() && !mono_detail;
        let pos = pos2(rect.right() - 8.0 - dg.size().x, rect.center().y - dg.size().y / 2.0);
        if pill {
            let r = egui::Rect::from_min_size(pos, dg.size()).expand2(vec2(5.0, 1.5));
            p.rect_filled(r, 8.0, pal.bg2);
        }
        p.galley(pos, dg, pal.faint);
    }
    let chevron_clicked =
        expanded.is_some() && resp.clicked() && resp.interact_pointer_pos().is_some_and(|pt| chevron_rect.expand2(vec2(4.0, 0.0)).contains(pt));
    (resp, chevron_clicked)
}

impl RelKind {
    pub fn icon_glyph(self) -> &'static str {
        match self {
            RelKind::Table | RelKind::Partitioned => icon::TABLE,
            RelKind::View => icon::EYE,
            RelKind::MatView => icon::LAYERS,
            RelKind::Foreign => icon::GLOBE,
            RelKind::Sequence => icon::HASH,
        }
    }
}
