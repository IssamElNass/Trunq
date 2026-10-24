//! Search all server-provided extensions and enable/update the selected version
//! on a dedicated session. PostgreSQL remains the authority on privileges and
//! provider constraints; viewing this dialog never changes the database.

use std::collections::HashMap;

use egui::{RichText, Ui};

use crate::action::{Action, ToastKind};
use crate::config::Profile;
use crate::db::meta::{parse_text_array, quote_ident, quote_literal};
use crate::db::result::ResultSet;
use crate::db::session::{Event, Promise, QueryResult, Session};
use crate::ui::theme::{self, icon};
use crate::ui::widgets as w;

pub(crate) const INVENTORY_SQL: &str = r#"
SELECT COALESCE(a.name,e.extname), a.default_version, e.extversion, n.nspname,
       pg_get_userbyid(e.extowner), COALESCE(a.comment, obj_description(e.oid,'pg_extension'), '')
FROM pg_available_extensions a FULL JOIN pg_extension e ON e.extname=a.name
LEFT JOIN pg_namespace n ON n.oid=e.extnamespace ORDER BY 1;
SELECT name, version, superuser, trusted, relocatable, schema, requires
FROM pg_available_extension_versions ORDER BY name, version;
SELECT current_database(), current_user, pg_is_in_recovery(),
       current_setting('transaction_read_only')::boolean,
       has_database_privilege(current_database(),'CREATE'),
       (SELECT rolsuper FROM pg_roles WHERE rolname=current_user),
       (SELECT setting FROM pg_settings WHERE name='shared_preload_libraries'),
       current_setting('cron.database_name',true);
SELECT nspname FROM pg_namespace
WHERE nspname !~ '^pg_' AND nspname <> 'information_schema'
  AND has_schema_privilege(oid,'USAGE') AND has_schema_privilege(oid,'CREATE') ORDER BY 1;
"#;

#[derive(Clone, Debug)]
struct Version {
    name: String,
    superuser: bool,
    trusted: bool,
    relocatable: bool,
    schema: Option<String>,
    dependencies: Vec<String>,
}

#[derive(Debug)]
struct Extension {
    name: String,
    default_version: Option<String>,
    installed_version: Option<String>,
    installed_schema: Option<String>,
    owner: Option<String>,
    comment: String,
    versions: Vec<Version>,
}

impl Extension {
    fn version(&self, name: &str) -> Option<&Version> {
        self.versions.iter().find(|v| v.name == name)
    }

    fn sql(&self, version: &str, schema: &str, dependencies: bool) -> Result<String, String> {
        let target = self.version(version).ok_or("Select a version offered by this server")?;
        if self.installed_version.as_deref() == Some(version) {
            return Err("This version is already enabled".into());
        }
        if self.installed_version.is_some() {
            Ok(format!("ALTER EXTENSION {} UPDATE TO {};", quote_ident(&self.name), quote_literal(version)))
        } else {
            let mut sql = format!("CREATE EXTENSION {} VERSION {}", quote_ident(&self.name), quote_literal(version));
            if !schema.is_empty() && target.schema.is_none() {
                sql.push_str(&format!(" SCHEMA {}", quote_ident(schema)));
            }
            if dependencies && !target.dependencies.is_empty() {
                sql.push_str(" CASCADE");
            }
            sql.push(';');
            Ok(sql)
        }
    }
}

#[derive(Default)]
struct Inventory {
    extensions: Vec<Extension>,
    database: String,
    role: String,
    recovery: bool,
    read_only: bool,
    can_create: bool,
    superuser: bool,
    preload: Option<String>,
    cron_database: Option<String>,
    schemas: Vec<String>,
}

impl Inventory {
    fn parse(results: Vec<ResultSet>) -> Result<Self, String> {
        if results.len() != 4 || results[2].is_empty() {
            return Err("The extension inventory response is incomplete; refresh to retry".into());
        }
        let mut inventory = Self::default();
        let mut names = HashMap::new();
        let installed = &results[0];
        for r in installed.rows() {
            names.insert(installed.s(r, 0).to_owned(), inventory.extensions.len());
            inventory.extensions.push(Extension {
                name: installed.s(r, 0).into(),
                default_version: installed.get(r, 1).map(str::to_string),
                installed_version: installed.get(r, 2).map(str::to_string),
                installed_schema: installed.get(r, 3).map(str::to_string),
                owner: installed.get(r, 4).map(str::to_string),
                comment: installed.s(r, 5).into(),
                versions: Vec::new(),
            });
        }
        let versions = &results[1];
        for r in versions.rows() {
            if let Some(&index) = names.get(versions.s(r, 0)) {
                inventory.extensions[index].versions.push(Version {
                    name: versions.s(r, 1).into(),
                    superuser: versions.s(r, 2) == "t",
                    trusted: versions.s(r, 3) == "t",
                    relocatable: versions.s(r, 4) == "t",
                    schema: versions.get(r, 5).map(str::to_string),
                    dependencies: versions.get(r, 6).map(parse_text_array).unwrap_or_default(),
                });
            }
        }
        let context = &results[2];
        inventory.database = context.s(0, 0).into();
        inventory.role = context.s(0, 1).into();
        inventory.recovery = context.s(0, 2) == "t";
        inventory.read_only = context.s(0, 3) == "t";
        inventory.can_create = context.s(0, 4) == "t";
        inventory.superuser = context.s(0, 5) == "t";
        inventory.preload = context.get(0, 6).map(str::to_string);
        inventory.cron_database = context.get(0, 7).map(str::to_string);
        inventory.schemas = results[3].rows().map(|r| results[3].s(r, 0).into()).collect();
        Ok(inventory)
    }
}

pub struct ExtensionDialog {
    profile: Profile,
    database: String,
    password: Option<String>,
    session: Session,
    loading: Option<Promise<Result<Vec<ResultSet>, String>>>,
    operation: Option<Operation>,
    ready: bool,
    inventory: Inventory,
    search: String,
    category: u8,
    selected: Option<String>,
    version: String,
    schema: String,
    dependencies: bool,
    error: Option<String>,
    success: Option<String>,
}

struct Operation {
    reply: Promise<QueryResult>,
    message: String,
}

impl ExtensionDialog {
    pub fn new(profile: Profile, database: String, selected: Option<String>, password: Option<String>, ctx: &egui::Context) -> Self {
        let session = Session::connect(&profile, &database, "Extension manager", password.clone(), ctx);
        let loading = Some(session.batch(INVENTORY_SQL));
        Self {
            profile,
            database,
            password,
            session,
            loading,
            operation: None,
            ready: false,
            inventory: Inventory::default(),
            search: String::new(),
            category: 0,
            selected,
            version: String::new(),
            schema: String::new(),
            dependencies: false,
            error: None,
            success: None,
        }
    }

    fn select(&mut self, index: usize) {
        let extension = &self.inventory.extensions[index];
        self.selected = Some(extension.name.clone());
        self.version = extension.default_version.clone().or_else(|| extension.versions.first().map(|v| v.name.clone())).unwrap_or_default();
        self.schema.clear();
        self.dependencies = false;
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        if !self.session.is_alive() {
            self.session = Session::connect(&self.profile, &self.database, "Extension manager", self.password.clone(), ctx);
        }
        self.error = None;
        self.ready = false;
        self.loading = Some(self.session.batch(INVENTORY_SQL));
    }

    fn apply(&mut self, sql: String, message: String) {
        if !self.can_apply() {
            self.error = Some("Wait for a writable connection and a complete inventory before applying an extension change".into());
            return;
        }
        self.error = None;
        self.success = None;
        self.operation = Some(Operation { reply: self.session.query(sql, vec![]), message });
    }

    fn can_apply(&self) -> bool {
        self.ready
            && self.session.connected
            && self.loading.is_none()
            && self.operation.is_none()
            && !self.profile.read_only
            && !self.inventory.read_only
            && !self.inventory.recovery
    }

    fn poll(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        for event in self.session.poll() {
            if let Event::ConnectFailed { error, .. } | Event::Disconnected(error) = event {
                self.error = Some(error);
                self.loading = None;
                self.operation = None;
                self.ready = false;
            }
        }
        if let Some(result) = self.loading.as_ref().and_then(Promise::take) {
            self.loading = None;
            match result.and_then(Inventory::parse) {
                Ok(inventory) => {
                    self.ready = true;
                    self.inventory = inventory;
                    let selected = self.inventory.extensions.iter().position(|e| Some(&e.name) == self.selected.as_ref());
                    if let Some(index) = selected.or_else(|| (!self.inventory.extensions.is_empty()).then_some(0)) {
                        self.select(index);
                    }
                }
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(result) = self.operation.as_ref().and_then(|operation| operation.reply.take()) {
            let operation = self.operation.take().unwrap();
            match result {
                Ok(_) => {
                    self.success = Some(operation.message);
                    actions.push(Action::Toast(ToastKind::Success, self.success.clone().unwrap()));
                    actions.push(Action::RefreshCatalog { profile: self.profile.id, db: self.database.clone() });
                    self.refresh(ctx);
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    /// True means close the dialog and drop its dedicated connection.
    pub fn show(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) -> bool {
        self.poll(ctx, actions);
        let mut close = false;
        let busy = self.loading.is_some() || self.operation.is_some();
        let body_height = (ctx.content_rect().height() - 260.0).clamp(230.0, 580.0);
        let modal = egui::Modal::new(egui::Id::unique("extension_manager")).frame(w::modal_frame()).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.0).clamp(480.0, 1000.0));
            w::dialog_header(
                ui,
                icon::PLUG,
                theme::p().accent,
                "Extensions",
                &format!("{} · {} · {}", self.profile.display_name(), self.database, self.inventory.role),
            );
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    let changed =
                        w::input(ui, &mut self.search, "Search all extensions by name or description…", Some(icon::SEARCH), ui.available_width() - 210.0)
                            .changed();
                    let changed = w::dropdown(ui, &mut self.category, &[(0, "All"), (1, "Not installed"), (2, "Installed")], 150.0) || changed;
                    if ui.add_enabled_ui(!busy, |ui| w::icon_button(ui, icon::REFRESH, "Refresh available extensions")).inner.clicked() {
                        self.refresh(ctx);
                    }
                    if changed {
                        self.selected = None;
                    }
                });
            });
            ui.add_space(10.0);
            let needle = self.search.trim().to_lowercase();
            let visible: Vec<_> = self
                .inventory
                .extensions
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    (self.category == 0 || (self.category == 2) == e.installed_version.is_some())
                        && (needle.is_empty() || e.name.to_lowercase().contains(&needle) || e.comment.to_lowercase().contains(&needle))
                })
                .map(|(i, _)| i)
                .collect();
            if !visible.iter().any(|&i| Some(&self.inventory.extensions[i].name) == self.selected.as_ref()) {
                self.selected = None;
            }
            if self.selected.is_none()
                && let Some(&index) = visible.first()
            {
                self.select(index);
            }
            ui.columns(2, |columns| {
                let ui = &mut columns[0];
                ui.label(
                    RichText::new(format!(
                        "{} shown · {} installed · {} total",
                        visible.len(),
                        self.inventory.extensions.iter().filter(|e| e.installed_version.is_some()).count(),
                        self.inventory.extensions.len()
                    ))
                    .small()
                    .color(theme::p().dim),
                );
                let mut chosen = None;
                egui::ScrollArea::vertical().id_salt("extension_list").max_height(body_height).show_rows(ui, 54.0, visible.len(), |ui, range| {
                    for row in range {
                        let index = visible[row];
                        let extension = &self.inventory.extensions[index];
                        let label = format!(
                            "{}\n{}",
                            extension.name,
                            extension.installed_version.as_ref().map(|v| format!("Installed · {v}")).unwrap_or_else(|| "Available to enable".into())
                        );
                        if ui
                            .add_sized([ui.available_width(), 50.0], egui::Button::selectable(self.selected.as_deref() == Some(&extension.name), label))
                            .clicked()
                            && !busy
                        {
                            chosen = Some(index);
                        }
                    }
                });
                if let Some(index) = chosen {
                    self.select(index);
                }
                egui::ScrollArea::vertical()
                    .id_salt("extension_details")
                    .max_height(body_height)
                    .show(&mut columns[1], |ui| self.details(ui, busy, actions, &mut close));
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if busy {
                    ui.spinner();
                    ui.label(if self.operation.is_some() { "Applying extension change…" } else { "Loading server inventory…" });
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled_ui(self.operation.is_none(), |ui| w::button(ui, w::Kind::Secondary, "", "Close")).inner.clicked() {
                        close = true;
                    }
                });
            });
        });
        close || modal.should_close() && self.operation.is_none()
    }

    fn details(&mut self, ui: &mut Ui, busy: bool, actions: &mut Vec<Action>, close: &mut bool) {
        if let Some(message) = &self.success {
            w::callout(ui, theme::p().success, icon::CIRCLE_CHECK, message);
        }
        if let Some(error) = &self.error {
            w::callout(ui, theme::p().danger, icon::CIRCLE_X, error);
            w::hint(ui, "Check database privileges, provider allowlists and preload/restart requirements. Refresh to retry a failed connection.");
        }
        let Some(extension) = self.inventory.extensions.iter().find(|e| Some(&e.name) == self.selected.as_ref()) else {
            w::hint(ui, "No matching extensions. Only extensions provided by this server can be enabled here.");
            return;
        };
        ui.heading(&extension.name);
        ui.label(&extension.comment);
        if let Some(installed) = &extension.installed_version {
            ui.label(format!(
                "Installed: {installed} · schema: {} · owner: {}",
                extension.installed_schema.as_deref().unwrap_or("—"),
                extension.owner.as_deref().unwrap_or("—")
            ));
        }
        ui.label(format!("Server default version: {}", extension.default_version.as_deref().unwrap_or("Unavailable")));
        ui.add_space(8.0);
        ui.add_enabled_ui(!busy, |ui| {
            w::form_row(ui, "Version", |ui| {
                let versions: Vec<_> = extension.versions.iter().map(|v| v.name.as_str()).collect();
                w::dropdown_str(ui, &mut self.version, &versions, 0.0);
            });
        });
        if let Some(version) = extension.version(&self.version) {
            let privilege = if version.trusted {
                "Trusted extension; PostgreSQL can permit installation with database CREATE privilege"
            } else if version.superuser {
                "Requires administrator/superuser privileges or provider-specific permission"
            } else {
                "Requires privileges to create the extension's objects"
            };
            w::hint(ui, privilege);
            w::hint(ui, &format!("Current role: CREATE on database {} · superuser {}", self.inventory.can_create, self.inventory.superuser));
            if !version.dependencies.is_empty() {
                ui.label(format!("Dependencies: {}", version.dependencies.join(", ")));
            }
            if extension.installed_version.is_none() {
                if let Some(schema) = &version.schema {
                    ui.label(format!("Required schema: {schema}"));
                } else {
                    ui.add_enabled_ui(!busy, |ui| {
                        w::form_row(ui, "Schema", |ui| {
                            let mut schemas = vec![""];
                            schemas.extend(self.inventory.schemas.iter().map(String::as_str));
                            w::dropdown_str(ui, &mut self.schema, &schemas, 0.0);
                        });
                    });
                    w::hint(
                        ui,
                        if version.relocatable {
                            "Empty uses the server default creation schema. Objects can be relocated if permitted."
                        } else {
                            "Empty uses the server default creation schema. This extension is not relocatable after installation."
                        },
                    );
                }
                if !version.dependencies.is_empty() {
                    ui.add_enabled_ui(!busy, |ui| w::checkbox(ui, &mut self.dependencies, "Also enable required dependencies (CASCADE)"));
                }
            }
        }
        if ["pg_cron", "pgaudit", "pg_stat_statements"].contains(&extension.name.as_str()) {
            w::callout(
                ui,
                theme::p().warn,
                icon::INFO,
                "This extension needs a server preload library. Provider allowlisting and a restart may also be required.",
            );
            ui.label(format!("Visible preload setting: {}", self.inventory.preload.as_deref().unwrap_or("Not visible to this role")));
        }
        if extension.name == "pg_cron" {
            ui.label(format!("Scheduler database: {}", self.inventory.cron_database.as_deref().unwrap_or("Check server configuration; normally postgres")));
            if let Some(database) = &self.inventory.cron_database
                && database != &self.database
                && ui
                    .add_enabled_ui(!busy, |ui| w::small_button(ui, w::Kind::Secondary, icon::DATABASE, &format!("Open scheduler database · {database}")))
                    .inner
                    .clicked()
            {
                actions.push(Action::OpenExtensions { profile: self.profile.id, db: database.clone(), selected: Some("pg_cron".into()) });
                *close = true;
            }
            w::hint(ui, "Enabling pg_cron does not create a recurring job. Use Database tools to schedule and inspect jobs.");
        }
        let blocked = self.profile.read_only || self.inventory.read_only || self.inventory.recovery;
        if blocked {
            w::callout(ui, theme::p().warn, icon::LOCK, "This connection/database is read-only or in recovery. Extension changes are disabled.");
        }
        ui.add_space(8.0);
        let sql = extension.sql(&self.version, &self.schema, self.dependencies);
        match sql {
            Ok(sql) => {
                w::section(ui, "SQL to apply");
                ui.add(egui::Label::new(RichText::new(&sql).monospace()).wrap());
                let label = if extension.installed_version.is_some() { "Update to selected version" } else { "Enable extension" };
                ui.add_space(8.0);
                let message = format!("{} is now enabled at version {}", extension.name, self.version);
                if ui.add_enabled_ui(self.can_apply(), |ui| w::button(ui, w::Kind::Primary, icon::CHECK, label)).inner.clicked() {
                    self.apply(sql.clone(), message);
                }
                if ui.add_enabled_ui(!busy, |ui| w::small_button(ui, w::Kind::Secondary, icon::CODE, "Open SQL in editor")).inner.clicked() {
                    actions.push(Action::NewQuery { profile: self.profile.id, db: self.database.clone(), sql, run: false });
                    *close = true;
                }
            }
            Err(message) => {
                w::callout(ui, theme::p().dim, icon::INFO, &message);
            }
        }
        w::hint(
            ui,
            "Only the selected database is changed. The server validates permissions, dependencies and upgrade paths; unsupported changes show its error here.",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extension() -> Extension {
        Extension {
            name: "odd\"extension".into(),
            default_version: Some("v'1".into()),
            installed_version: None,
            installed_schema: None,
            owner: None,
            comment: String::new(),
            versions: vec![Version {
                name: "v'1".into(),
                superuser: false,
                trusted: true,
                relocatable: true,
                schema: None,
                dependencies: vec!["pgcrypto".into()],
            }],
        }
    }

    #[test]
    fn generated_changes_quote_identifiers_and_versions_and_keep_dependencies_opt_in() {
        let mut extension = extension();
        assert_eq!(extension.sql("v'1", "odd\"schema", false).unwrap(), "CREATE EXTENSION \"odd\"\"extension\" VERSION 'v''1' SCHEMA \"odd\"\"schema\";");
        assert!(extension.sql("v'1", "", true).unwrap().ends_with(" CASCADE;"));
        assert!(extension.sql("not offered", "", false).is_err());
        extension.versions[0].schema = Some("required_schema".into());
        assert!(!extension.sql("v'1", "wrong_schema", false).unwrap().contains("SCHEMA"));
        extension.installed_version = Some("older".into());
        assert_eq!(extension.sql("v'1", "", true).unwrap(), "ALTER EXTENSION \"odd\"\"extension\" UPDATE TO 'v''1';");
        extension.installed_version = Some("v'1".into());
        assert!(extension.sql("v'1", "", false).is_err());
        assert!(Inventory::parse(vec![]).is_err());
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::db::session::runtime;
    use crate::db::wire::PgConn;
    use std::time::{Duration, Instant};

    fn wait(dialog: &mut ExtensionDialog, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            dialog.poll(ctx, actions);
            if dialog.loading.is_none() && dialog.operation.is_none() {
                return;
            }
            assert!(Instant::now() < deadline, "extension manager request timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn query(dialog: &mut ExtensionDialog, sql: &str) {
        let reply = dialog.session.query(sql, vec![]);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !reply.is_ready() {
            dialog.session.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        reply.take().unwrap().unwrap();
    }

    #[test]
    fn manager_enables_missing_dependencies_only_when_explicitly_requested() {
        let Ok(url) = std::env::var("TRUNQ_TEST_DATABASE_URL") else { return };
        let (mut profile, password) = Profile::from_url(&url).unwrap();
        profile.save_password = false;
        let params = profile.connect_params("", password.as_deref()).unwrap();
        let mut admin = runtime().block_on(PgConn::connect(&params)).unwrap();
        let ctx = egui::Context::default();
        let mut dialog = ExtensionDialog::new(profile.clone(), profile.database.clone(), Some("earthdistance".into()), password, &ctx);
        let mut actions = Vec::new();
        wait(&mut dialog, &ctx, &mut actions);
        assert!(dialog.ready, "{:?}", dialog.error);
        if ["earthdistance", "cube"].iter().any(|name| !dialog.inventory.extensions.iter().any(|e| &e.name == name && e.installed_version.is_none())) {
            return;
        }
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let extension = dialog.inventory.extensions.iter().find(|e| e.name == "earthdistance").unwrap();
            assert!(extension.version(&dialog.version).unwrap().dependencies.iter().any(|name| name == "cube"));
            let sql = extension.sql(&dialog.version, "", false).unwrap();
            dialog.apply(sql, "must fail without dependency".into());
            wait(&mut dialog, &ctx, &mut actions);
            assert!(dialog.error.as_ref().is_some_and(|e| e.contains("required extension")), "{:?}", dialog.error);
            let extension = dialog.inventory.extensions.iter().find(|e| e.name == "earthdistance").unwrap();
            dialog.apply(extension.sql(&dialog.version, "", true).unwrap(), "dependencies enabled".into());
            wait(&mut dialog, &ctx, &mut actions);
            assert!(dialog.error.is_none(), "{:?}", dialog.error);
            for name in ["earthdistance", "cube"] {
                assert!(dialog.inventory.extensions.iter().find(|e| e.name == name).unwrap().installed_version.is_some());
            }
        }));
        drop(dialog);
        runtime().block_on(admin.simple_query("DROP EXTENSION IF EXISTS earthdistance; DROP EXTENSION IF EXISTS cube")).unwrap();
        runtime().block_on(admin.terminate());
        if let Err(error) = run {
            std::panic::resume_unwind(error);
        }
    }

    #[test]
    fn manager_installs_and_updates_with_a_non_superuser_and_recovers_from_denied_changes() {
        let Ok(url) = std::env::var("TRUNQ_TEST_DATABASE_URL") else { return };
        let (mut profile, password) = Profile::from_url(&url).unwrap();
        profile.save_password = false;
        let params = profile.connect_params("", password.as_deref()).unwrap();
        let mut admin = runtime().block_on(PgConn::connect(&params)).unwrap();
        let ctx = egui::Context::default();
        let mut dialog = ExtensionDialog::new(profile.clone(), profile.database.clone(), None, password, &ctx);
        let mut actions = Vec::new();
        wait(&mut dialog, &ctx, &mut actions);
        assert!(dialog.ready, "{:?}", dialog.error);
        let candidate = dialog.inventory.extensions.iter().position(|extension| {
            ["hstore", "pg_trgm"].contains(&extension.name.as_str())
                && extension.installed_version.is_none()
                && extension.versions.len() > 1
                && extension.versions.iter().all(|v| v.trusted)
        });
        let Some(index) = candidate else { return }; // needs a disposable contrib-capable server
        dialog.select(index);
        let name = dialog.inventory.extensions[index].name.clone();
        let default_version = dialog.version.clone();
        let old_version = dialog.inventory.extensions[index].versions.iter().find(|v| v.name != default_version).unwrap().name.clone();
        let suffix = rand::random::<u64>();
        let role = format!("trunq_ext_role_{suffix}");
        let schema = format!("trunq_ext_schema_{suffix}");
        let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime()
                .block_on(admin.simple_query(&format!(
                    "CREATE ROLE {}; CREATE SCHEMA {}; GRANT USAGE, CREATE ON SCHEMA {} TO {};",
                    quote_ident(&role),
                    quote_ident(&schema),
                    quote_ident(&schema),
                    quote_ident(&role)
                )))
                .unwrap();
            query(&mut dialog, &format!("SET ROLE {}", quote_ident(&role)));
            dialog.refresh(&ctx);
            wait(&mut dialog, &ctx, &mut actions);
            assert!(!dialog.inventory.superuser);
            assert!(!dialog.inventory.can_create);
            dialog.version = old_version.clone();
            dialog.schema = schema.clone();
            let extension = dialog.inventory.extensions.iter().find(|e| e.name == name).unwrap();
            let sql = extension.sql(&dialog.version, &dialog.schema, false).unwrap();
            dialog.apply(sql.clone(), "installed".into());
            wait(&mut dialog, &ctx, &mut actions);
            assert!(dialog.error.as_ref().is_some_and(|e| e.contains("permission denied")), "{:?}", dialog.error);
            runtime().block_on(admin.simple_query(&format!("GRANT CREATE ON DATABASE {} TO {}", quote_ident(&profile.database), quote_ident(&role)))).unwrap();
            dialog.apply(sql, "installed".into());
            wait(&mut dialog, &ctx, &mut actions);
            assert!(dialog.error.is_none(), "{:?}", dialog.error);
            let extension = dialog.inventory.extensions.iter().find(|e| e.name == name).unwrap();
            assert_eq!(extension.installed_version.as_deref(), Some(old_version.as_str()));
            assert_eq!(extension.installed_schema.as_deref(), Some(schema.as_str()));
            assert_eq!(extension.owner.as_deref(), Some(role.as_str()));
            let sql = extension.sql(&default_version, "", false).unwrap();
            dialog.apply(sql, "updated".into());
            wait(&mut dialog, &ctx, &mut actions);
            assert!(dialog.error.is_none(), "{:?}", dialog.error);
            assert_eq!(dialog.inventory.extensions.iter().find(|e| e.name == name).unwrap().installed_version.as_deref(), Some(default_version.as_str()));
            assert!(actions.iter().any(|a| matches!(a, Action::RefreshCatalog { .. })));
            query(&mut dialog, "SET default_transaction_read_only = on");
            dialog.refresh(&ctx);
            wait(&mut dialog, &ctx, &mut actions);
            assert!(dialog.inventory.read_only);
            assert!(!dialog.can_apply());
            dialog.apply("SELECT 1".into(), "must not execute".into());
            assert!(dialog.operation.is_none());
        }));
        drop(dialog);
        // Only remove the extension that was absent at test start and this
        // test's uniquely named schema/role. Never remove a pre-existing one.
        runtime()
            .block_on(admin.simple_query(&format!(
                "DROP EXTENSION IF EXISTS {}; DROP SCHEMA IF EXISTS {}; DROP OWNED BY {}; DROP ROLE {};",
                quote_ident(&name),
                quote_ident(&schema),
                quote_ident(&role),
                quote_ident(&role)
            )))
            .unwrap();
        runtime().block_on(admin.terminate());
        if let Err(error) = run {
            std::panic::resume_unwind(error);
        }
    }
}
