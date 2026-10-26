//! Persistent configuration: connection profiles, settings, query history.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

use crate::db::azure::{self, AzureConfig};
use crate::db::wire::{ConnectParams, SslMode};

const KEYRING_SERVICE: &str = "trunq";

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AuthMethod {
    #[default]
    Password,
    /// Microsoft Entra ID access token (Azure Database for PostgreSQL).
    AzureEntra(AzureConfig),
    /// Cloud-neutral helper returning a short-lived password/token on stdout.
    TokenCommand(crate::db::credentials::TokenCommand),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub id: u64,
    pub name: String,
    pub group: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    pub ssl_mode: SslMode,
    pub tls: crate::db::wire::TlsConfig,
    pub auth: AuthMethod,
    pub save_password: bool,
    pub color: Option<[u8; 3]>,
    pub read_only: bool,
    pub show_system_objects: bool,
    /// Only show this database in the browser (instead of all databases).
    pub only_default_db: bool,
    /// Environment label shown everywhere (Production is red).
    pub environment: Environment,
    pub statement_timeout_s: u32,
    pub connect_timeout_s: u32,
    pub ssh: crate::db::ssh::SshConfig,
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            id: rand::random::<u64>() >> 12,
            name: String::new(),
            group: String::new(),
            host: "localhost".into(),
            port: 5432,
            user: std::env::var("USER").unwrap_or_else(|_| "postgres".into()),
            database: "postgres".into(),
            ssl_mode: SslMode::Prefer,
            tls: Default::default(),
            auth: AuthMethod::Password,
            save_password: true,
            color: None,
            read_only: false,
            show_system_objects: false,
            only_default_db: false,
            environment: Environment::None,
            statement_timeout_s: 0,
            connect_timeout_s: 10,
            ssh: Default::default(),
        }
    }
}

impl Profile {
    pub fn display_name(&self) -> String {
        if !self.name.trim().is_empty() { self.name.clone() } else { format!("{}@{}", self.user, self.host) }
    }

    pub fn is_azure(&self) -> bool {
        matches!(self.auth, AuthMethod::AzureEntra(_))
    }

    pub fn credential_target(&self) -> String {
        credential_fingerprint(&(self.host.trim(), self.port, &self.user, &self.database, &self.auth, self.ssh.enabled.then_some(&self.ssh)))
    }

    pub fn ssh_credential_target(&self) -> String {
        credential_fingerprint(&self.ssh)
    }

    fn keyring_user(&self) -> String {
        // Legacy profile-ID-only entries cannot establish which endpoint owns
        // a secret. Do not fall back to them after a target/profile import.
        format!("profile-{}-db-{}", self.id, self.credential_target())
    }

    pub fn stored_password(&self) -> Option<String> {
        keyring::Entry::new(KEYRING_SERVICE, &self.keyring_user()).ok()?.get_password().ok()
    }

    pub fn store_password(&self, pw: &str) {
        if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, &self.keyring_user()) {
            if pw.is_empty() {
                let _ = e.delete_credential();
            } else {
                let _ = e.set_password(pw);
            }
        }
    }

    fn ssh_keyring_user(&self) -> String {
        format!("profile-{}-ssh-{}", self.id, self.ssh_credential_target())
    }

    /// SSH password or key passphrase (keychain).
    pub fn ssh_secret(&self) -> Option<String> {
        keyring::Entry::new(KEYRING_SERVICE, &self.ssh_keyring_user()).ok()?.get_password().ok()
    }

    pub fn store_ssh_secret(&self, secret: &str) {
        if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, &self.ssh_keyring_user()) {
            if secret.is_empty() {
                let _ = e.delete_credential();
            } else {
                let _ = e.set_password(secret);
            }
        }
    }

    pub fn forget_password(&self) {
        if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, &self.keyring_user()) {
            let _ = e.delete_credential();
        }
    }

    pub fn forget_credentials(&self) {
        self.forget_password();
        if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, &self.ssh_keyring_user()) {
            let _ = e.delete_credential();
        }
        for legacy in [format!("profile-{}", self.id), format!("profile-{}-ssh", self.id)] {
            if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, &legacy) {
                let _ = e.delete_credential();
            }
        }
    }

    /// Resolves everything needed to open a connection to `database`.
    /// May block (keychain access, Entra sign-in), call from a worker.
    pub fn connect_params(&self, database: &str, session_password: Option<&str>) -> Result<ConnectParams> {
        let database = if database.is_empty() { self.database.as_str() } else { database };
        if !matches!(self.auth, AuthMethod::Password) && self.host.trim().starts_with('/') {
            return Err(anyhow!("token authentication requires a TLS connection over TCP"));
        }
        let mut user = self.user.clone();
        let password = match &self.auth {
            AuthMethod::Password => session_password
                .map(str::to_string)
                .or_else(|| if self.save_password { self.stored_password() } else { None })
                .or_else(|| pgpass_lookup(&self.host, self.port, database, &self.user)),
            AuthMethod::AzureEntra(cfg) => {
                let token = azure::get_token(cfg, &self.user)?;
                if user.trim().is_empty() {
                    user = token.upn.clone().ok_or_else(|| anyhow!("could not determine the Entra user name; set it in the connection"))?;
                }
                Some(token.token)
            }
            AuthMethod::TokenCommand(command) => Some(command.password(&self.host, self.port, &self.user, database)?),
        };
        let mut options = Vec::new();
        if self.read_only {
            options.push("-c default_transaction_read_only=on".to_string());
        }
        if self.statement_timeout_s > 0 {
            options.push(format!("-c statement_timeout={}s", self.statement_timeout_s));
        }
        Ok(ConnectParams {
            host: self.host.trim().to_string(),
            port: self.port,
            user,
            password,
            database: database.to_string(),
            ssl_mode: if !matches!(self.auth, AuthMethod::Password) && self.ssl_mode < SslMode::Require { SslMode::Require } else { self.ssl_mode },
            tls: self.tls.clone(),
            application_name: "Trunq".into(),
            connect_timeout: Duration::from_secs(self.connect_timeout_s.max(1) as u64),
            options: (!options.is_empty()).then(|| options.join(" ")),
            ssh: self.ssh.enabled.then(|| {
                let secret = match self.ssh.auth {
                    crate::db::ssh::SshAuth::Agent => None,
                    _ => self.ssh_secret(),
                };
                (self.ssh.clone(), secret)
            }),
        })
    }

    /// Parses `postgres://user:pass@host:port/db?sslmode=require`.
    /// Returns the profile and the password, if any.
    pub fn from_url(url: &str) -> Option<(Profile, Option<String>)> {
        let rest = url.trim().strip_prefix("postgresql://").or_else(|| url.trim().strip_prefix("postgres://"))?;
        let (main, query) = rest.split_once('?').unwrap_or((rest, ""));
        let (auth, hostpart) = match main.rsplit_once('@') {
            Some((a, h)) => (Some(a), h),
            None => (None, main),
        };
        let (hostport, db) = hostpart.split_once('/').unwrap_or((hostpart, ""));
        let mut p = Profile::default();
        let mut password = None;
        if let Some(a) = auth {
            let (u, pw) = a.split_once(':').map(|(u, p)| (u, Some(p))).unwrap_or((a, None));
            p.user = pct_decode(u);
            password = pw.map(pct_decode);
        }
        if let Some(bracketed) = hostport.strip_prefix('[') {
            let (host, suffix) = bracketed.split_once(']')?;
            p.host = host.to_string();
            if !suffix.is_empty() {
                p.port = suffix.strip_prefix(':')?.parse().ok()?;
            }
        } else if let Some((host, port)) = hostport.rsplit_once(':') {
            // IPv6 literals in connection URLs must use brackets.
            if host.contains(':') {
                return None;
            }
            p.host = pct_decode(host);
            p.port = port.parse().ok()?;
        } else if !hostport.is_empty() {
            p.host = pct_decode(hostport);
        }
        if !db.is_empty() {
            p.database = pct_decode(db);
        }
        for kv in query.split('&') {
            if let Some(("sslmode", v)) = kv.split_once('=')
                && let Some(m) = SslMode::parse(v)
            {
                p.ssl_mode = m;
            }
        }
        if azure::is_azure_host(&p.host) && p.ssl_mode < SslMode::Require {
            p.ssl_mode = SslMode::Require;
        }
        p.name = format!("{} @ {}", p.database, p.host);
        Some((p, password))
    }
}

fn credential_fingerprint(value: &impl Serialize) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let digest = sha2::Sha256::digest(serde_json::to_vec(value).expect("serializing credential identity"));
    let mut key = String::with_capacity(64);
    for &byte in digest.iter() {
        key.push(HEX[(byte >> 4) as usize] as char);
        key.push(HEX[(byte & 15) as usize] as char);
    }
    key
}

impl PartialOrd for SslMode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let rank = |m: &SslMode| SslMode::ALL.iter().position(|x| x == m);
        rank(self).partial_cmp(&rank(other))
    }
}

fn pct_decode(s: &str) -> String {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|s| u8::from_str_radix(s, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Looks up a password in `~/.pgpass` (or `$PGPASSFILE`).
pub fn pgpass_lookup(host: &str, port: u16, db: &str, user: &str) -> Option<String> {
    let path = std::env::var("PGPASSFILE").map(PathBuf::from).ok().or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join(".pgpass")))?;
    pgpass_from_file(&path, host, port, db, user)
}

fn pgpass_from_file(path: &std::path::Path, host: &str, port: u16, db: &str, user: &str) -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return None;
        }
    }
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    let port = port.to_string();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let mut fields = Vec::new();
        let mut cur = String::new();
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(n) = chars.next() {
                        cur.push(n)
                    }
                }
                ':' => fields.push(std::mem::take(&mut cur)),
                c => cur.push(c),
            }
        }
        fields.push(cur);
        if fields.len() != 5 {
            continue;
        }
        let m = |pat: &str, v: &str| pat == "*" || pat == v;
        if m(&fields[0], host) && m(&fields[1], &port) && m(&fields[2], db) && m(&fields[3], user) {
            return Some(fields[4].clone());
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Environment {
    #[default]
    None,
    Local,
    Development,
    Test,
    Staging,
    Production,
}

impl Environment {
    pub const ALL: [Environment; 6] =
        [Environment::None, Environment::Local, Environment::Development, Environment::Test, Environment::Staging, Environment::Production];
    pub fn label(self) -> &'static str {
        match self {
            Environment::None => "None",
            Environment::Local => "LOCAL",
            Environment::Development => "DEV",
            Environment::Test => "TEST",
            Environment::Staging => "STAGING",
            Environment::Production => "PRODUCTION",
        }
    }
    /// Badge colour (RGB); production is always red.
    pub fn color(self) -> Option<[u8; 3]> {
        Some(match self {
            Environment::None => return None,
            Environment::Local => [0x94, 0xA3, 0xB8],
            Environment::Development => [0x3D, 0xD6, 0x8C],
            Environment::Test => [0x2D, 0xD4, 0xBF],
            Environment::Staging => [0xF5, 0xB8, 0x4B],
            Environment::Production => [0xF2, 0x55, 0x5A],
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeChoice {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub ui_scale: f32,
    pub editor_font_size: f32,
    pub grid_font_size: f32,
    /// Rows fetched per page from an open cursor.
    pub page_size: usize,
    pub auto_complete: bool,
    pub confirm_destructive: bool,
    pub uppercase_keywords: bool,
    pub max_cell_chars: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeChoice::Dark,
            ui_scale: 1.0,
            editor_font_size: 13.5,
            grid_font_size: 12.5,
            page_size: 1000,
            auto_complete: true,
            confirm_destructive: true,
            uppercase_keywords: true,
            max_cell_chars: 300,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SavedTab {
    pub title: String,
    pub profile_id: Option<u64>,
    pub database: String,
    pub sql: String,
    pub file: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub profiles: Vec<Profile>,
    pub settings: Settings,
    pub tabs: Vec<SavedTab>,
    pub active_tab: usize,
    pub snippets: Vec<Snippet>,
    /// Expanded nodes of the object tree.
    pub tree_open: Vec<String>,
    /// "Remember my table": per `profile/db/schema.table`.
    pub table_prefs: HashMap<String, TablePrefs>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct TablePrefs {
    #[cfg(feature = "desktop")]
    pub grid: crate::ui::grid::GridPrefs,
    pub filter: String,
    pub order: String,
    pub card: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Snippet {
    pub name: String,
    pub sql: String,
}

pub fn config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("TRUNQ_CONFIG_DIR").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    directories::ProjectDirs::from("dev", "trunq", "Trunq").map(|d| d.config_dir().to_path_buf()).unwrap_or_else(|| PathBuf::from(".trunq"))
}

impl Config {
    fn path() -> PathBuf {
        config_dir().join("config.json")
    }

    pub fn load() -> Config {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        crate::storage::write_private_atomic(&Self::path(), &serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub sql: String,
    pub profile: String,
    pub database: String,
    /// Unix seconds.
    pub at: i64,
    pub ms: u64,
    pub ok: bool,
    pub rows: Option<u64>,
}

#[derive(Default)]
pub struct History {
    pub entries: Vec<HistoryEntry>,
    appends: usize,
}

impl History {
    const MAX: usize = 5000;

    fn path() -> PathBuf {
        config_dir().join("history.jsonl")
    }

    pub fn load() -> History {
        Self::load_from(&Self::path())
    }

    fn load_from(path: &std::path::Path) -> History {
        use std::io::BufRead;
        let mut entries = std::collections::VecDeque::new();
        let mut count = 0usize;
        if let Ok(file) = std::fs::File::open(path) {
            for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
                if let Ok(entry) = serde_json::from_str(&line) {
                    count = count.saturating_add(1);
                    entries.push_back(entry);
                    if entries.len() > Self::MAX {
                        entries.pop_front();
                    }
                }
            }
        }
        History { entries: entries.into(), appends: count.min(501) }
    }

    pub fn push(&mut self, e: HistoryEntry) {
        if let Err(error) = self.push_to(e, &Self::path()) {
            eprintln!("trunq: could not persist query history: {error}");
        }
    }

    fn push_to(&mut self, e: HistoryEntry, path: &std::path::Path) -> Result<()> {
        if let Some(last) = self.entries.last()
            && last.sql == e.sql
            && last.database == e.database
            && last.profile == e.profile
        {
            self.entries.pop();
        }
        let line = serde_json::to_string(&e).unwrap_or_default();
        self.entries.push(e);
        self.appends = self.appends.saturating_add(1);
        let over_limit = self.entries.len() > Self::MAX;
        if over_limit {
            self.entries.drain(..self.entries.len() - Self::MAX);
        }
        // Count disk appends, not retained in-memory entries: consecutive
        // duplicate queries otherwise grow the file forever without compacting.
        if self.appends > 500 {
            let text: String = self.entries.iter().filter_map(|e| serde_json::to_string(e).ok()).map(|l| l + "\n").collect();
            crate::storage::write_private_atomic(path, text.as_bytes())?;
            self.appends = 0;
        } else {
            use std::io::Write;
            let mut file = crate::storage::open_private_append(path)?;
            writeln!(file, "{line}")?;
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.appends = 0;
        let _ = std::fs::remove_file(Self::path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_bound_to_the_endpoint_and_bastion() {
        let original = Profile::default();
        let mut changed = original.clone();
        changed.name = "Production".into();
        changed.read_only = true;
        assert_eq!(original.keyring_user(), changed.keyring_user());
        changed.host = "untrusted.example.com".into();
        assert_ne!(original.keyring_user(), changed.keyring_user());
        changed = original.clone();
        changed.database = "different_database".into();
        assert_ne!(original.keyring_user(), changed.keyring_user());
        changed = original.clone();
        changed.ssh.enabled = true;
        changed.ssh.host = "bastion".into();
        assert_ne!(original.keyring_user(), changed.keyring_user());
        assert_ne!(original.ssh_keyring_user(), changed.ssh_keyring_user());
        assert!(!original.keyring_user().ends_with(&original.id.to_string()));
    }

    fn entry(sql: &str, profile: &str) -> HistoryEntry {
        HistoryEntry { sql: sql.into(), profile: profile.into(), database: "app".into(), at: 0, ms: 1, ok: true, rows: None }
    }

    #[test]
    fn repeated_queries_compact_disk_and_profiles_remain_distinct() {
        let dir = std::env::temp_dir().join(format!("trunq-history-{}", rand::random::<u64>()));
        let path = dir.join("history.jsonl");
        let mut history = History::default();
        for _ in 0..1200 {
            history.push_to(entry("SELECT 1", "local"), &path).unwrap();
        }
        assert_eq!(history.entries.len(), 1);
        assert!(std::fs::read_to_string(&path).unwrap().lines().count() <= 501);
        history.push_to(entry("SELECT 1", "production"), &path).unwrap();
        assert_eq!(history.entries.len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loading_old_history_retains_only_the_latest_entries() {
        let dir = std::env::temp_dir().join(format!("trunq-history-load-{}", rand::random::<u64>()));
        let path = dir.join("history.jsonl");
        let text: String = (0..5100).map(|i| serde_json::to_string(&entry(&format!("SELECT {i}"), "local")).unwrap() + "\n").collect();
        crate::storage::write_private_atomic(&path, text.as_bytes()).unwrap();
        let mut history = History::load_from(&path);
        assert_eq!(history.entries.len(), 5000);
        assert_eq!(history.entries[0].sql, "SELECT 100");
        history.push_to(entry("SELECT 5100", "local"), &path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 5000);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pgpass_rejects_group_or_world_readable_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("trunq-pgpass-{}", rand::random::<u64>()));
        let path = dir.join("pgpass");
        crate::storage::write_private_atomic(&path, b"localhost:5432:app:reporting:secret\n").unwrap();
        assert_eq!(pgpass_from_file(&path, "localhost", 5432, "app", "reporting").as_deref(), Some("secret"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(pgpass_from_file(&path, "localhost", 5432, "app", "reporting"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn new_authentication_fields_preserve_old_profiles_and_require_explicit_application_identity() {
        let profile: Profile = serde_json::from_str(r#"{"host":"localhost","auth":"Password"}"#).unwrap();
        assert!(!profile.tls.configured());
        let profile = Profile { auth: AuthMethod::AzureEntra(AzureConfig { flow: azure::AzureFlow::ClientSecret, ..Default::default() }), ..profile };
        assert!(profile.connect_params("", None).unwrap_err().to_string().contains("explicit tenant"));
    }

    #[cfg(unix)]
    #[test]
    fn token_commands_resolve_on_connection_and_enforce_tls() {
        let command =
            crate::db::credentials::TokenCommand { executable: "/usr/bin/printf".into(), arguments: "%s\\n\nlocal-test-token".into(), ..Default::default() };
        let profile = Profile { auth: AuthMethod::TokenCommand(command), ssl_mode: SslMode::Disable, ..Default::default() };
        let params = profile.connect_params("reporting", Some("ignored-password")).unwrap();
        assert_eq!(params.password.as_deref(), Some("local-test-token"));
        assert_eq!(params.database, "reporting");
        assert_eq!(params.ssl_mode, SslMode::Require);
        let socket = Profile { host: "/tmp".into(), ..profile };
        assert!(socket.connect_params("", None).is_err());
    }

    #[test]
    fn ipv6_and_socket_urls() {
        let (p, _) = Profile::from_url("postgres://u@[::1]:55432/db").unwrap();
        assert_eq!(p.host, "::1");
        assert_eq!(p.port, 55432);
        let (p, _) = Profile::from_url("postgres://u@[2001:db8::1]/db").unwrap();
        assert_eq!(p.host, "2001:db8::1");
        assert_eq!(p.port, 5432);
        assert!(Profile::from_url("postgres://u@[::1]:invalid/db").is_none());
        assert!(Profile::from_url("postgres://u@localhost:70000/db").is_none());
        let (p, _) = Profile::from_url("postgres://u@%2Ftmp/db").unwrap();
        assert_eq!(p.host, "/tmp");
        assert_eq!(pct_decode("%aé"), "%aé");
        assert_eq!(pct_decode("caf%C3%A9"), "café");
    }

    #[test]
    fn parse_url() {
        let (p, pw) = Profile::from_url("postgres://bob:s%40cret@db.example.com:6543/app?sslmode=require").unwrap();
        assert_eq!(p.user, "bob");
        assert_eq!(pw.as_deref(), Some("s@cret"));
        assert_eq!(p.host, "db.example.com");
        assert_eq!(p.port, 6543);
        assert_eq!(p.database, "app");
        assert_eq!(p.ssl_mode, SslMode::Require);
        let (p, _) = Profile::from_url("postgresql://x.postgres.database.azure.com/db").unwrap();
        assert_eq!(p.ssl_mode, SslMode::Require);
        assert_eq!(p.port, 5432);
    }
}
