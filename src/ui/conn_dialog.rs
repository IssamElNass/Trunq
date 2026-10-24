//! New / edit connection dialog.

use std::time::Instant;

use egui::{RichText, Ui, vec2};

use super::theme::{self, icon};
use super::widgets as w;
use crate::config::{AuthMethod, Profile};
use crate::db::azure::{self, AzureCloud, AzureConfig, AzureFlow};
use crate::db::session::{Promise, runtime};
use crate::db::wire::{PgConn, SslMode};

type TestResult = Result<(String, u128), String>;

pub struct ConnDialog {
    pub profile: Profile,
    pub is_new: bool,
    password: String,
    password_dirty: bool,
    url: String,
    test: Option<(Promise<TestResult>, Instant)>,
    test_result: Option<TestResult>,
    show_advanced: bool,
    port_text: String,
    error: Option<String>,
    ssh_secret: String,
    ssh_secret_dirty: bool,
}

#[allow(clippy::large_enum_variant)] // short-lived, one per dialog close
pub enum DialogOutcome {
    /// profile, new database password, new SSH secret
    Save(Profile, Option<String>, Option<String>),
    Cancel,
}

impl ConnDialog {
    pub fn new(profile: Option<Profile>) -> ConnDialog {
        let is_new = profile.is_none();
        let profile = profile.unwrap_or_default();
        ConnDialog {
            port_text: profile.port.to_string(),
            password: String::new(),
            password_dirty: false,
            url: String::new(),
            test: None,
            test_result: None,
            show_advanced: false,
            error: None,
            ssh_secret: String::new(),
            ssh_secret_dirty: false,
            profile,
            is_new,
        }
    }

    fn apply_url(&mut self) {
        if let Some((p, pw)) = Profile::from_url(&self.url) {
            let different_identity = p.host != self.profile.host || p.port != self.profile.port || p.user != self.profile.user;
            let id = self.profile.id;
            let color = self.profile.color;
            let auth = self.profile.auth.clone();
            self.profile = Profile { id, color, auth, ..p };
            self.port_text = self.profile.port.to_string();
            if let Some(pw) = pw {
                self.password = pw;
                self.password_dirty = true;
            } else if different_identity {
                // Do not send a password typed for the previous endpoint (or
                // its profile keychain entry) to an imported, different server.
                self.password.clear();
                self.password_dirty = true;
            }
            self.detect_azure();
            self.url.clear();
            self.error = None;
        } else {
            self.error = Some("Not a valid postgres:// or postgresql:// URL".into());
        }
    }

    fn detect_azure(&mut self) {
        if let Some(cloud) = AzureCloud::from_host(&self.profile.host) {
            if self.profile.ssl_mode < SslMode::Require {
                self.profile.ssl_mode = SslMode::Require;
            }
            if let AuthMethod::AzureEntra(cfg) = &mut self.profile.auth {
                cfg.cloud = cloud;
            }
        }
    }

    fn effective_password(&self) -> Option<String> {
        if self.password_dirty { Some(self.password.clone()) } else { None }
    }

    fn start_test(&mut self, ctx: &egui::Context) {
        let profile = self.profile.clone();
        let pw = self.effective_password();
        let ssh_secret = self.ssh_secret_dirty.then(|| self.ssh_secret.clone());
        let promise = Promise::new();
        let p2 = promise.clone();
        let ctx = ctx.clone();
        runtime().spawn(async move {
            let t0 = Instant::now();
            let prof = profile.clone();
            let params = tokio::task::spawn_blocking(move || prof.connect_params("", pw.as_deref())).await;
            let r = match params {
                Ok(Ok(mut params)) => {
                    if let (Some(secret), Some(ssh)) = (ssh_secret, params.ssh.as_mut()) {
                        ssh.1 = Some(secret);
                    }
                    match PgConn::connect(&params).await {
                        Ok(c) => {
                            let v = c.server_version();
                            let ms = t0.elapsed().as_millis();
                            c.terminate().await;
                            Ok((v, ms))
                        }
                        Err(e) => Err(format!("{e:#}")),
                    }
                }
                Ok(Err(e)) => Err(format!("{e:#}")),
                Err(e) => Err(e.to_string()),
            };
            p2.set(r);
            ctx.request_repaint();
        });
        self.test = Some((promise, Instant::now()));
        self.test_result = None;
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<DialogOutcome> {
        let pal = theme::p();
        let mut outcome = None;
        if let Some((p, _)) = &self.test
            && let Some(r) = p.take()
        {
            self.test_result = Some(r);
            self.test = None;
        }
        let max_body = (ctx.content_rect().height() - 240.0).max(220.0);
        let modal = egui::Modal::new(egui::Id::unique("conn_dialog")).frame(w::modal_frame()).show(ctx, |ui| {
            ui.set_width(600.0);
            let (title, subtitle) = if self.is_new {
                ("New connection", "PostgreSQL, Azure Database for PostgreSQL or any wire-compatible server")
            } else {
                ("Edit connection", "Changes apply to new sessions; open tabs keep their connection")
            };
            w::dialog_header(ui, icon::PLUG, pal.accent, title, subtitle);

            // URL paste
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let r = w::input(
                    ui,
                    &mut self.url,
                    "Paste a URL: postgres://user:pass@host:5432/db?sslmode=require",
                    Some(icon::LINK),
                    ui.available_width() - 88.0,
                );
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let has_url = !self.url.trim().is_empty();
                if (ui.add_enabled_ui(has_url, |ui| w::button(ui, w::Kind::Secondary, "", "Apply")).inner.clicked() || enter) && has_url {
                    self.apply_url();
                }
            });
            ui.add_space(14.0);

            egui::ScrollArea::vertical().max_height(max_body).auto_shrink([false, true]).show(ui, |ui| {
                ui.set_width(ui.available_width() - 4.0);
                self.general_section(ui);
                self.server_section(ui);
                self.auth_section(ui);
                ui.add_space(6.0);
                self.ssh_section(ui);
                self.advanced_section(ui);
            });

            if let Some(e) = &self.error {
                ui.add_space(4.0);
                w::callout(ui, pal.danger, icon::CIRCLE_X, e);
            }
            match (&self.test, &self.test_result) {
                (Some((_, started)), _) => {
                    ui.add_space(4.0);
                    w::callout(ui, pal.accent, icon::LOADER, &format!("Connecting… {:.1}s", started.elapsed().as_secs_f32()));
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                }
                (None, Some(Ok((ver, ms)))) => {
                    ui.add_space(4.0);
                    w::callout(ui, pal.success, icon::CIRCLE_CHECK, &format!("Connected in {ms} ms · PostgreSQL {ver}"));
                }
                (None, Some(Err(e))) => {
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().id_salt("test_err").max_height(110.0).show(ui, |ui| {
                        w::callout(ui, pal.danger, icon::CIRCLE_X, e);
                    });
                }
                _ => {}
            }

            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let testing = self.test.is_some();
                if ui.add_enabled_ui(!testing, |ui| w::button(ui, w::Kind::Secondary, icon::ZAP, "Test connection")).inner.clicked() {
                    self.start_test(ui.ctx());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let save_label = if self.is_new { "Add connection" } else { "Save changes" };
                    if w::button(ui, w::Kind::Primary, icon::CHECK, save_label).clicked() {
                        if self.profile.host.trim().is_empty() {
                            self.error = Some("Host is required".into());
                        } else {
                            outcome = Some(DialogOutcome::Save(
                                self.profile.clone(),
                                self.effective_password(),
                                self.ssh_secret_dirty.then(|| self.ssh_secret.clone()),
                            ));
                        }
                    }
                    if w::button(ui, w::Kind::Ghost, "", "Cancel").clicked() {
                        outcome = Some(DialogOutcome::Cancel);
                    }
                });
            });
        });
        if modal.should_close() && outcome.is_none() {
            outcome = Some(DialogOutcome::Cancel);
        }
        outcome
    }

    fn general_section(&mut self, ui: &mut Ui) {
        w::section(ui, "General");
        ui.add_space(6.0);
        w::form_row(ui, "Name", |ui| {
            let hint = self.profile.display_name();
            w::input(ui, &mut self.profile.name, &hint, Some(icon::TYPE), ui.available_width() - 40.0);
            ui.add_space(4.0);
            color_picker(ui, &mut self.profile.color);
        });
        w::form_row(ui, "Environment", |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for env in crate::config::Environment::ALL {
                let selected = self.profile.environment == env;
                let label = if env == crate::config::Environment::None { "None" } else { env.label() };
                if w::choice_chip(ui, selected, label, env.color().map(theme::rgb)).clicked() {
                    self.profile.environment = env;
                    // production connections get a red colour unless one was chosen
                    if env == crate::config::Environment::Production && self.profile.color.is_none() {
                        self.profile.color = env.color();
                    }
                }
            }
        });
        w::form_row(ui, "Group", |ui| {
            w::input(ui, &mut self.profile.group, "Optional, e.g. Customers or Analytics", Some(icon::FOLDER), 0.0);
        });
        ui.add_space(10.0);
    }

    fn server_section(&mut self, ui: &mut Ui) {
        w::section(ui, "Server");
        ui.add_space(6.0);
        w::form_row(ui, "Host", |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let r = w::input(ui, &mut self.profile.host, "localhost, IP address or socket directory", Some(icon::SERVER), ui.available_width() - 128.0);
            if r.changed() {
                self.detect_azure();
            }
            ui.label(RichText::new("Port").color(theme::p().dim));
            if w::input(ui, &mut self.port_text, "5432", None, 0.0).changed()
                && let Ok(p) = self.port_text.trim().parse()
            {
                self.profile.port = p;
            }
        });
        w::form_row(ui, "Database", |ui| {
            w::input(ui, &mut self.profile.database, "postgres", Some(icon::DATABASE), 0.0);
        });
        w::form_row(ui, "SSL", |ui| {
            let opts: Vec<(SslMode, &str)> = SslMode::ALL.iter().map(|m| (*m, m.label())).collect();
            w::dropdown(ui, &mut self.profile.ssl_mode, &opts, 0.0);
        });
        ui.add_space(10.0);
    }

    fn auth_section(&mut self, ui: &mut Ui) {
        let pal = theme::p();
        w::section(ui, "Authentication");
        ui.add_space(6.0);
        w::form_row(ui, "Method", |ui| {
            let mut method = match self.profile.auth {
                AuthMethod::Password => 0,
                AuthMethod::AzureEntra(_) => 1,
                AuthMethod::TokenCommand(_) => 2,
            };
            if w::dropdown(ui, &mut method, &[(0, "Password / client certificate"), (1, "Microsoft Entra ID"), (2, "Token command (any provider)")], 0.0) {
                if method == 1 {
                    let cloud = AzureCloud::from_host(&self.profile.host).unwrap_or_default();
                    self.profile.auth = AuthMethod::AzureEntra(AzureConfig { cloud, ..Default::default() });
                    if self.profile.ssl_mode < SslMode::Require {
                        self.profile.ssl_mode = SslMode::Require;
                    }
                } else if method == 2 {
                    self.profile.auth = AuthMethod::TokenCommand(Default::default());
                    self.profile.ssl_mode = SslMode::VerifyFull;
                } else {
                    self.profile.auth = AuthMethod::Password;
                }
            }
        });
        w::form_row(ui, "User", |ui| {
            let hint = if self.profile.is_azure() { "name@contoso.com (empty = signed-in account)" } else { "postgres" };
            w::input(ui, &mut self.profile.user, hint, Some(icon::USER), 0.0);
        });
        match &mut self.profile.auth {
            AuthMethod::Password => {
                let hint = if !self.is_new && !self.password_dirty { "Unchanged" } else { "" };
                w::form_row(ui, "Password", |ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    if w::password_input(ui, &mut self.password, hint, ui.available_width() - 150.0).changed() {
                        self.password_dirty = true;
                    }
                    w::checkbox(ui, &mut self.profile.save_password, "Save in keychain");
                });
            }
            AuthMethod::AzureEntra(cfg) => {
                w::form_row(ui, "Azure cloud", |ui| {
                    let opts: Vec<(AzureCloud, &str)> = AzureCloud::ALL.iter().map(|c| (*c, c.label())).collect();
                    w::dropdown(ui, &mut cfg.cloud, &opts, 0.0);
                });
                w::form_row(ui, "Sign-in", |ui| {
                    let opts: Vec<(AzureFlow, &str)> = AzureFlow::ALL.iter().map(|f| (*f, f.label())).collect();
                    w::dropdown(ui, &mut cfg.flow, &opts, 0.0);
                });
                w::form_row(ui, "Tenant", |ui| {
                    w::input(ui, &mut cfg.tenant, "contoso.onmicrosoft.com or tenant id (optional)", Some(icon::GLOBE), 0.0);
                });
                if cfg.flow == AzureFlow::ClientSecret {
                    w::form_row(ui, "Application id", |ui| {
                        w::code_input(ui, &mut cfg.client_id, "Service principal client id", None, 0.0);
                    });
                    w::form_row(ui, "Secret variable", |ui| {
                        w::code_input(ui, &mut cfg.client_secret_env, "AZURE_CLIENT_SECRET", None, 0.0);
                    });
                    w::hint(ui, "Enter the environment variable name, not the secret. Set an explicit PostgreSQL role in User above.");
                }
                ui.horizontal(|ui| {
                    ui.add_space(118.0);
                    ui.vertical(|ui| match azure::cached_token_info(cfg, &self.profile.user) {
                        Some((upn, minutes)) if minutes > 0 => {
                            ui.horizontal(|ui| {
                                let who = upn.as_ref().map(|u| format!(" as {u}")).unwrap_or_default();
                                ui.label(RichText::new(format!("{}  Signed in{who} · token valid {minutes} min", icon::CIRCLE_CHECK)).color(pal.success));
                                if w::small_button(ui, w::Kind::Ghost, icon::LOG_OUT, "Sign out").clicked() {
                                    azure::sign_out(cfg, &self.profile.user);
                                }
                            });
                        }
                        _ => w::hint(ui, "You sign in when connecting. Tokens expire hourly and are refreshed automatically."),
                    });
                });
                ui.add_space(6.0);
            }
            AuthMethod::TokenCommand(command) => {
                w::form_row(ui, "Preset", |ui| {
                    if w::small_button(ui, w::Kind::Secondary, "", "Azure CLI").clicked() {
                        *command = crate::db::credentials::TokenCommand::azure_cli();
                    }
                    if w::small_button(ui, w::Kind::Secondary, "", "AWS RDS").clicked() {
                        *command = crate::db::credentials::TokenCommand::aws_rds();
                    }
                    if w::small_button(ui, w::Kind::Secondary, "", "Google Cloud").clicked() {
                        *command = crate::db::credentials::TokenCommand::google_cloud();
                    }
                });
                w::form_row(ui, "Executable", |ui| {
                    w::code_input(ui, &mut command.executable, "Path to CLI or credential helper", None, 0.0);
                });
                w::form_row(ui, "Arguments", |ui| {
                    ui.add(egui::TextEdit::multiline(&mut command.arguments).font(theme::mono(12.0)).desired_rows(5).desired_width(ui.available_width()));
                });
                w::form_row(ui, "Timeout", |ui| {
                    w::number(ui, "token_timeout", &mut command.timeout_s, 1, 300, 90.0);
                    ui.label("seconds");
                });
                w::hint(ui, "One argument per line. {host}, {port}, {user}, {database} are substituted. No shell. Prints one token; requires TLS.");
                w::hint(
                    ui,
                    "Use your CLI login for managed identity, federation or SSO. Set the AWS region and use the original RDS endpoint. Keep secrets out of arguments.",
                );
            }
        }
    }

    fn ssh_section(&mut self, ui: &mut Ui) {
        let detail = if self.profile.ssh.enabled { format!("via {}", self.profile.ssh.host) } else { "Off".to_string() };
        w::disclosure(ui, "ssh", icon::SHIELD, "SSH tunnel", &detail, self.profile.ssh.enabled, |ui| {
            w::checkbox(ui, &mut self.profile.ssh.enabled, "Connect through an SSH server (bastion or jump host)");
            ui.add_space(8.0);
            ui.add_enabled_ui(self.profile.ssh.enabled, |ui| {
                w::form_row(ui, "SSH host", |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    w::input(ui, &mut self.profile.ssh.host, "bastion.example.com", Some(icon::SERVER), ui.available_width() - 128.0);
                    ui.label(RichText::new("Port").color(theme::p().dim));
                    w::number(ui, "ssh_port", &mut self.profile.ssh.port, 1, 65535, 0.0);
                });
                w::form_row(ui, "SSH user", |ui| {
                    w::input(ui, &mut self.profile.ssh.user, "ubuntu", Some(icon::USER), 0.0);
                });
                w::form_row(ui, "Auth", |ui| {
                    let opts: Vec<(crate::db::ssh::SshAuth, &str)> = crate::db::ssh::SshAuth::ALL.iter().map(|a| (*a, a.label())).collect();
                    w::dropdown(ui, &mut self.profile.ssh.auth, &opts, 0.0);
                });
                match self.profile.ssh.auth {
                    crate::db::ssh::SshAuth::KeyFile => {
                        w::form_row(ui, "Private key", |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            w::input(ui, &mut self.profile.ssh.key_path, "~/.ssh/id_ed25519", Some(icon::KEY), ui.available_width() - 96.0);
                            if w::button(ui, w::Kind::Secondary, icon::FOLDER_OPEN, "Browse").clicked()
                                && let Some(p) = rfd::FileDialog::new().pick_file()
                            {
                                self.profile.ssh.key_path = p.display().to_string();
                            }
                        });
                        w::form_row(ui, "Passphrase", |ui| {
                            if w::password_input(ui, &mut self.ssh_secret, "Only if the key is encrypted", 0.0).changed() {
                                self.ssh_secret_dirty = true;
                            }
                        });
                    }
                    crate::db::ssh::SshAuth::Password => {
                        w::form_row(ui, "SSH password", |ui| {
                            if w::password_input(ui, &mut self.ssh_secret, "", 0.0).changed() {
                                self.ssh_secret_dirty = true;
                            }
                        });
                    }
                    crate::db::ssh::SshAuth::Agent => {}
                }
                w::hint(
                    ui,
                    "Host and port above are resolved from the SSH server (for example localhost or a private IP). Secrets are stored in the keychain.",
                );
            });
        });
    }

    fn advanced_section(&mut self, ui: &mut Ui) {
        let detail = if self.profile.read_only { "Read-only" } else { "" };
        w::disclosure(ui, "advanced", icon::SLIDERS, "Advanced", detail, self.show_advanced, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            w::checkbox(ui, &mut self.profile.read_only, "Read-only connection (default_transaction_read_only)");
            w::checkbox(ui, &mut self.profile.show_system_objects, "Show system schemas (pg_catalog, information_schema)");
            w::checkbox(ui, &mut self.profile.only_default_db, "Only show the default database");
            ui.add_space(8.0);
            w::form_row(ui, "Statement timeout", |ui| {
                w::number(ui, "stmt_timeout", &mut self.profile.statement_timeout_s, 0, 86400, 90.0);
                ui.label(RichText::new("seconds, 0 = none").color(theme::p().faint));
            });
            w::form_row(ui, "Connect timeout", |ui| {
                w::number(ui, "conn_timeout", &mut self.profile.connect_timeout_s, 1, 300, 90.0);
                ui.label(RichText::new("seconds").color(theme::p().faint));
            });
            w::form_row(ui, "Root CA bundle", |ui| {
                w::input(ui, &mut self.profile.tls.root_ca, "Absolute path to PEM CA bundle (optional)", None, 0.0);
            });
            w::form_row(ui, "Client certificate", |ui| {
                w::input(ui, &mut self.profile.tls.client_cert, "Absolute path to PEM certificate chain (optional)", None, 0.0);
            });
            w::form_row(ui, "Client key", |ui| {
                w::input(ui, &mut self.profile.tls.client_key, "Absolute path to unencrypted PKCS #8 PEM key", None, 0.0);
            });
            w::hint(ui, "Use verify-full to verify the server identity. Certificate paths are saved; private key contents are read only when connecting.");
            if let AuthMethod::AzureEntra(cfg) = &mut self.profile.auth {
                w::form_row(ui, "Client id", |ui| {
                    w::code_input(ui, &mut cfg.client_id, azure::DEFAULT_CLIENT_ID, None, 0.0);
                });
                w::hint(ui, "Use your own Entra app registration if your organisation blocks the Azure CLI client.");
            }
        });
    }
}

pub fn color_picker(ui: &mut Ui, color: &mut Option<[u8; 3]>) {
    let pal = theme::p();
    let current = color.map(theme::rgb).unwrap_or(pal.faint);
    let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
    ui.painter().circle_filled(rect.center(), 8.0, current);
    ui.painter().circle_stroke(rect.center(), 10.0, egui::Stroke::new(1.0, pal.border));
    let resp = resp.on_hover_text("Connection color (shown on tabs and the editor, e.g. red for production)");
    egui::Popup::menu(&resp).show(|ui| {
        ui.horizontal(|ui| {
            for (c, name) in theme::CONN_COLORS {
                let rgb = theme::rgb_hex(c);
                let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), egui::Sense::click());
                ui.painter().circle_filled(r.center(), 8.0, theme::rgb(rgb));
                if *color == Some(rgb) {
                    ui.painter().circle_stroke(r.center(), 10.5, egui::Stroke::new(1.5, pal.text));
                }
                if resp.on_hover_text(name).clicked() {
                    *color = Some(rgb);
                    ui.close();
                }
            }
            if ui.small_button("None").clicked() {
                *color = None;
                ui.close();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_url_does_not_reuse_another_endpoints_password() {
        let mut dialog = ConnDialog::new(None);
        dialog.password = "old-secret".into();
        dialog.password_dirty = true;
        dialog.url = "postgres://new_user@other.example/db".into();
        dialog.apply_url();
        assert_eq!(dialog.effective_password(), Some(String::new()));
        dialog.url = "postgres://new_user:new-secret@other.example/db".into();
        dialog.apply_url();
        assert_eq!(dialog.effective_password().as_deref(), Some("new-secret"));
    }
}
