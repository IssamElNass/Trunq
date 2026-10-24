//! SSH tunnels (bastion / jump hosts).
//!
//! One SSH session is shared per tunnel configuration and every PostgreSQL
//! connection of that server is a `direct-tcpip` channel over it, so twenty
//! open tabs still mean a single SSH login. The session is dropped once the
//! last channel closes and the cache entry is pruned on the next use.
//!
//! Host keys are verified against `~/.ssh/known_hosts` and Trunq's own
//! known-hosts file; unknown hosts are trusted on first use and recorded,
//! a *changed* key is a hard error.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};

use anyhow::{Context, Result, anyhow, bail};
use russh::client;
use russh::keys::{self, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SshAuth {
    /// Use the running ssh-agent (`SSH_AUTH_SOCK`), then default key files.
    #[default]
    Agent,
    /// A private key file (optionally passphrase protected).
    KeyFile,
    Password,
}

impl SshAuth {
    pub const ALL: [SshAuth; 3] = [SshAuth::Agent, SshAuth::KeyFile, SshAuth::Password];
    pub fn label(self) -> &'static str {
        match self {
            SshAuth::Agent => "SSH agent / default keys",
            SshAuth::KeyFile => "Private key file",
            SshAuth::Password => "Password",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SshConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    pub key_path: String,
}

impl Default for SshConfig {
    fn default() -> Self {
        SshConfig {
            enabled: false,
            host: String::new(),
            port: 22,
            user: std::env::var("USER").unwrap_or_default(),
            auth: SshAuth::Agent,
            key_path: String::new(),
        }
    }
}

impl SshConfig {
    fn key(&self) -> String {
        format!("{}@{}:{}|{:?}|{}", self.user, self.host, self.port, self.auth, self.key_path)
    }
}

struct Handler {
    host: String,
    port: u16,
}

fn trunq_known_hosts() -> PathBuf {
    crate::config::config_dir().join("known_hosts")
}

impl client::Handler for Handler {
    type Error = anyhow::Error;

    async fn check_server_key(&mut self, server_key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let key = match server_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(c) => c.public_key().clone().into(),
        };
        let fingerprint = key.fingerprint(keys::HashAlg::Sha256);
        // A changed key in either file is fatal (possible man-in-the-middle).
        let system = keys::check_known_hosts(&self.host, self.port, &key);
        let ours = keys::check_known_hosts_path(&self.host, self.port, &key, trunq_known_hosts());
        for r in [&system, &ours] {
            if let Err(keys::Error::KeyChanged { line }) = r {
                bail!(
                    "SSH host key for {} has CHANGED (known_hosts line {line}). This could be a man-in-the-middle attack. \
                     If the server was reinstalled, remove the old key and try again. New fingerprint: {fingerprint}",
                    self.host
                );
            }
        }
        if matches!(system, Ok(true)) || matches!(ours, Ok(true)) {
            return Ok(true);
        }
        // Trust on first use, recorded in Trunq's own known_hosts file.
        keys::known_hosts::learn_known_hosts_path(&self.host, self.port, &key, trunq_known_hosts())
            .map_err(|e| anyhow!("could not record SSH host key: {e}"))?;
        eprintln!("trunq: trusted new SSH host key for {}:{} ({fingerprint})", self.host, self.port);
        Ok(true)
    }
}

type Shared = Arc<client::Handle<Handler>>;

static SESSIONS: LazyLock<Mutex<HashMap<String, Shared>>> = LazyLock::new(Default::default);

async fn connect(cfg: &SshConfig, secret: Option<&str>) -> Result<Shared> {
    if cfg.host.trim().is_empty() {
        bail!("SSH tunnel is enabled but no SSH host is set");
    }
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(std::time::Duration::from_secs(30)),
        keepalive_max: 3,
        ..Default::default()
    });
    let handler = Handler { host: cfg.host.trim().to_string(), port: cfg.port };
    let mut session = tokio::time::timeout(std::time::Duration::from_secs(15), client::connect(config, (cfg.host.trim(), cfg.port), handler))
        .await
        .map_err(|_| anyhow!("timeout connecting to SSH host {}:{}", cfg.host, cfg.port))?
        .with_context(|| format!("SSH connection to {}:{} failed", cfg.host, cfg.port))?;

    let user = cfg.user.trim().to_string();
    let mut ok = false;
    match cfg.auth {
        SshAuth::Password => {
            let pw = secret.ok_or_else(|| anyhow!("SSH password required"))?;
            ok = session.authenticate_password(&user, pw).await?.success();
        }
        SshAuth::KeyFile => {
            let path = expand_home(&cfg.key_path);
            let key = keys::load_secret_key(&path, secret).with_context(|| format!("could not load SSH key {}", path.display()))?;
            let hash = session.best_supported_rsa_hash().await?.flatten();
            ok = session.authenticate_publickey(&user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await?.success();
        }
        SshAuth::Agent => {
            #[cfg(unix)]
            if let Ok(mut agent) = keys::agent::client::AgentClient::connect_env().await
                && let Ok(ids) = agent.request_identities().await
            {
                for id in ids {
                    let hash = session.best_supported_rsa_hash().await?.flatten();
                    let public = id.public_key().into_owned();
                    if session.authenticate_publickey_with(&user, public, hash, &mut agent).await.map(|r| r.success()).unwrap_or(false) {
                        ok = true;
                        break;
                    }
                }
            }
            if !ok {
                // Fall back to the usual unencrypted default keys.
                for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
                    let path = expand_home(&format!("~/.ssh/{name}"));
                    if !path.exists() {
                        continue;
                    }
                    if let Ok(key) = keys::load_secret_key(&path, None) {
                        let hash = session.best_supported_rsa_hash().await?.flatten();
                        if session.authenticate_publickey(&user, PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await?.success() {
                            ok = true;
                            break;
                        }
                    }
                }
            }
        }
    }
    if !ok {
        bail!("SSH authentication failed for {user}@{}", cfg.host);
    }
    Ok(Arc::new(session))
}

fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/")
        && let Some(home) = directories::BaseDirs::new()
    {
        return home.home_dir().join(rest);
    }
    PathBuf::from(p)
}

/// Opens a tunnelled stream to `host:port` (as seen from the SSH server).
pub async fn open_channel(cfg: &SshConfig, secret: Option<&str>, host: &str, port: u16) -> Result<russh::ChannelStream<client::Msg>> {
    let key = cfg.key();
    let mut sessions = SESSIONS.lock().await;
    // Drop sessions that died (network change, server restart...).
    sessions.retain(|_, s| !s.is_closed());
    let session = match sessions.get(&key) {
        Some(s) => s.clone(),
        None => {
            let s = connect(cfg, secret).await?;
            sessions.insert(key, s.clone());
            s
        }
    };
    drop(sessions);
    let channel =
        session.channel_open_direct_tcpip(host, port as u32, "127.0.0.1", 0).await.with_context(|| format!("SSH server could not reach {host}:{port}"))?;
    Ok(channel.into_stream())
}

/// Closes all idle SSH sessions (called on exit).
pub async fn close_all() {
    let sessions: Vec<Shared> = SESSIONS.lock().await.drain().map(|(_, s)| s).collect();
    for s in sessions {
        let _ = s.disconnect(russh::Disconnect::ByApplication, "bye", "en").await;
    }
}

/// Runs only when `TRUNQ_TEST_SSH` is set to `user@host:port|/path/to/key`
/// together with `TRUNQ_TEST_DATABASE_URL` (database host as seen from the SSH server).
#[cfg(test)]
mod live_tests {
    use super::*;

    #[test]
    fn tunnel_roundtrip() {
        let (Ok(ssh), Ok(url)) = (std::env::var("TRUNQ_TEST_SSH"), std::env::var("TRUNQ_TEST_DATABASE_URL")) else { return };
        let (target, key) = ssh.split_once('|').unwrap();
        let (user, hostport) = target.split_once('@').unwrap();
        let (host, port) = hostport.split_once(':').unwrap();
        let cfg = SshConfig { enabled: true, host: host.into(), port: port.parse().unwrap(), user: user.into(), auth: SshAuth::KeyFile, key_path: key.into() };
        let (profile, pw) = crate::config::Profile::from_url(&url).unwrap();
        let mut params = profile.connect_params("", pw.as_deref()).unwrap();
        params.password = pw;
        params.ssh = Some((cfg, None));
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
            // two connections share one SSH session
            let mut a = crate::db::wire::PgConn::connect(&params).await.expect("connect through tunnel");
            let mut b = crate::db::wire::PgConn::connect(&params).await.expect("second connection");
            let r = a.simple_query("SELECT inet_client_addr() IS NULL OR true, 42").await.unwrap();
            assert_eq!(r[0].get(0, 1), Some("42"));
            assert_eq!(b.simple_query("SELECT 1").await.unwrap()[0].get(0, 0), Some("1"));
            assert_eq!(SESSIONS.lock().await.len(), 1);
            // cancel requests go through the tunnel too
            let tok = a.cancel_token();
            let h = tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                tok.cancel().await.unwrap();
            });
            let err = a.simple_query("SELECT pg_sleep(5)").await.unwrap_err();
            assert!(format!("{err}").contains("cancel"));
            h.await.unwrap();
            close_all().await;
        });
    }
}
