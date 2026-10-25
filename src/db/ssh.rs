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
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Weak};
use std::task::{Context as TaskContext, Poll};

use anyhow::{Context, Result, anyhow, bail};
use russh::client;
use russh::keys::{self, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
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
    /// False requires an already-pinned raw host key (recommended in production).
    pub trust_new_hosts: bool,
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
            trust_new_hosts: true,
        }
    }
}

impl SshConfig {
    fn key(&self) -> String {
        serde_json::to_string(self).expect("serializing SSH configuration")
    }
}

struct Handler {
    host: String,
    port: u16,
    trust_new_hosts: bool,
}

fn recorded_keys(path: &std::path::Path, host: &str, port: u16, presented: &russh::keys::PublicKey) -> Result<bool> {
    // russh treats every open error as an absent file. Check explicitly so an
    // unreadable pinned-hosts file cannot silently downgrade to first use.
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("could not read SSH known_hosts; refusing to trust a new key"),
    };
    for line in text.lines().filter(|line| line.split_whitespace().next() == Some("@revoked")) {
        let encoded = line.split_whitespace().nth(3).ok_or_else(|| anyhow!("invalid revoked SSH host-key record"))?;
        let revoked = keys::parse_public_key_base64(encoded).context("invalid revoked SSH host key")?;
        if &revoked == presented {
            bail!("SSH host key is revoked in a checked known_hosts file");
        }
    }
    let recorded = keys::known_hosts::known_host_keys_path(host, port, path).context("could not parse SSH known_hosts")?;
    if recorded.iter().any(|(_, key)| key == presented) {
        return Ok(true);
    }
    if !recorded.is_empty() {
        bail!("SSH host key for {host}:{port} has CHANGED; verify the new fingerprint with your administrator");
    }
    Ok(false)
}

fn trunq_known_hosts() -> PathBuf {
    crate::config::config_dir().join("known_hosts")
}

impl client::Handler for Handler {
    type Error = anyhow::Error;

    async fn check_server_key(&mut self, server_key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let key = match server_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(_) => bail!("SSH host certificates are not supported; configure a pinned raw host key"),
        };
        if key.algorithm().is_rsa() {
            bail!("RSA SSH host keys are disabled; configure an Ed25519/ECDSA host key on the bastion");
        }
        let fingerprint = key.fingerprint(keys::HashAlg::Sha256);
        // A changed key in either file is fatal (possible man-in-the-middle).
        let system_path =
            directories::BaseDirs::new().ok_or_else(|| anyhow!("could not determine SSH known_hosts home directory"))?.home_dir().join(".ssh/known_hosts");
        let system = recorded_keys(&system_path, &self.host, self.port, &key)?;
        let ours = recorded_keys(&trunq_known_hosts(), &self.host, self.port, &key)?;
        if system || ours {
            return Ok(true);
        }
        if !self.trust_new_hosts {
            bail!("SSH host is not pinned; verify and add its raw host key to known_hosts before connecting. Fingerprint: {fingerprint}");
        }
        // Trust on first use, recorded in Trunq's own known_hosts file.
        use std::io::Write;
        let mut file = crate::storage::open_private_append(&trunq_known_hosts()).context("could not record SSH host key")?;
        let host = if self.port == 22 { self.host.clone() } else { format!("[{}]:{}", self.host, self.port) };
        // Assemble one append record first so formatting cannot interleave
        // separate field writes from simultaneous bastion handshakes.
        file.write_all(format!("\n{host} {}\n", key.to_openssh()?).as_bytes()).context("could not record SSH host key")?;
        eprintln!("trunq: trusted new SSH host key for {}:{} ({fingerprint})", self.host, self.port);
        Ok(true)
    }
}

type Shared = Arc<client::Handle<Handler>>;
type Slot = Arc<Mutex<Weak<client::Handle<Handler>>>>;
static SESSIONS: LazyLock<Mutex<HashMap<String, Slot>>> = LazyLock::new(Default::default);

/// Channels own the handle; the cache holds only a weak reference. When the
/// final stream closes, russh's command receiver closes and its task exits.
pub struct TunnelStream {
    stream: russh::ChannelStream<client::Msg>,
    _session: Shared,
}

impl AsyncRead for TunnelStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}
impl AsyncWrite for TunnelStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

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
    let handler = Handler { host: cfg.host.trim().to_string(), port: cfg.port, trust_new_hosts: cfg.trust_new_hosts };
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
            if key.algorithm().is_rsa() {
                bail!("RSA SSH keys are disabled because the upstream RSA backend has an unpatched advisory; use Ed25519 or ECDSA");
            }
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
                    if public.algorithm().is_rsa() {
                        continue;
                    }
                    if session.authenticate_publickey_with(&user, public, hash, &mut agent).await.map(|r| r.success()).unwrap_or(false) {
                        ok = true;
                        break;
                    }
                }
            }
            if !ok {
                // Fall back to the usual unencrypted default keys.
                for name in ["id_ed25519", "id_ecdsa"] {
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
pub async fn open_channel(cfg: &SshConfig, secret: Option<&str>, host: &str, port: u16) -> Result<TunnelStream> {
    let key = cfg.key();
    let mut sessions = SESSIONS.lock().await;
    sessions.retain(|_, slot| Arc::strong_count(slot) > 1 || slot.try_lock().map_or(true, |s| s.upgrade().is_some_and(|s| !s.is_closed())));
    let slot = sessions.entry(key).or_default().clone();
    drop(sessions);
    // Serialize only this configuration's handshake, so a stalled bastion
    // cannot block unrelated servers behind a global authentication lock.
    let mut cached = slot.lock().await;
    let session = match cached.upgrade().filter(|s| !s.is_closed()) {
        Some(session) => session,
        None => {
            let session = connect(cfg, secret).await?;
            *cached = Arc::downgrade(&session);
            session
        }
    };
    drop(cached);
    let channel =
        session.channel_open_direct_tcpip(host, port as u32, "127.0.0.1", 0).await.with_context(|| format!("SSH server could not reach {host}:{port}"))?;
    Ok(TunnelStream { stream: channel.into_stream(), _session: session })
}

/// Closes all idle SSH sessions (called on exit).
pub async fn close_all() {
    let slots: Vec<Slot> = SESSIONS.lock().await.drain().map(|(_, s)| s).collect();
    for slot in slots {
        if let Some(s) = slot.lock().await.upgrade() {
            let _ = s.disconnect(russh::Disconnect::ByApplication, "bye", "en").await;
        }
    }
}

/// Runs only when `TRUNQ_TEST_SSH` is set to `user@host:port|/path/to/key`
/// together with `TRUNQ_TEST_DATABASE_URL` (database host as seen from the SSH server).
#[cfg(test)]
mod live_tests {
    use super::*;

    #[test]
    fn pinned_host_keys_reject_rotation_revocation_and_unreadable_files() {
        let dir = std::env::temp_dir().join(format!("trunq-host-key-test-{}", rand::random::<u64>()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("known_hosts");
        // Public keys only, generated for a disposable regression fixture.
        let ed = "AAAAC3NzaC1lZDI1NTE5AAAAIJ9KgHxNtmf6lzT/Uq3PVZxfvR99ajadiFvm4Jac4YGi";
        let ec = "AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBP0O6pziWzzzGlS63RzKAwKnrtZyp8W0cIjBzabBmzER0nMhUx0IOykDsDrTxqHt0K9uwf4vANi5ap+mPx04wMQ=";
        let key = keys::parse_public_key_base64(ed).unwrap();
        assert!(!recorded_keys(&path, "db.example.com", 22, &key).unwrap());
        std::fs::write(&path, format!("db.example.com ssh-ed25519 {ed}\n")).unwrap();
        assert!(recorded_keys(&path, "db.example.com", 22, &key).unwrap());
        let other_algorithm = keys::parse_public_key_base64(ec).unwrap();
        assert!(recorded_keys(&path, "db.example.com", 22, &other_algorithm).unwrap_err().to_string().contains("CHANGED"));
        std::fs::write(&path, format!("@revoked db.example.com ssh-ed25519 {ed}\n")).unwrap();
        assert!(recorded_keys(&path, "db.example.com", 22, &key).unwrap_err().to_string().contains("revoked"));
        assert!(recorded_keys(&dir, "db.example.com", 22, &key).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tunnel_cache_keys_cannot_collide_via_field_separators() {
        let a = SshConfig { host: "host:user".into(), user: "operator".into(), ..Default::default() };
        let b = SshConfig { host: "host".into(), user: "user:operator".into(), ..Default::default() };
        assert_ne!(a.key(), b.key());
        assert_ne!(a.key(), SshConfig { trust_new_hosts: false, ..a.clone() }.key());
    }

    #[test]
    fn tunnel_roundtrip() {
        let (Ok(ssh), Ok(url)) = (std::env::var("TRUNQ_TEST_SSH"), std::env::var("TRUNQ_TEST_DATABASE_URL")) else { return };
        let (target, key) = ssh.split_once('|').unwrap();
        let (user, hostport) = target.split_once('@').unwrap();
        let (host, port) = hostport.split_once(':').unwrap();
        let cfg = SshConfig {
            enabled: true,
            host: host.into(),
            port: port.parse().unwrap(),
            user: user.into(),
            auth: SshAuth::KeyFile,
            key_path: key.into(),
            ..Default::default()
        };
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
            let slot = SESSIONS.lock().await.values().next().unwrap().clone();
            assert!(slot.lock().await.strong_count() > 0);
            a.terminate().await;
            assert!(slot.lock().await.strong_count() > 0, "second connection still owns tunnel");
            b.terminate().await;
            assert_eq!(slot.lock().await.strong_count(), 0, "cache must not retain an idle SSH handle");
            close_all().await;
        });
    }
}
