//! Microsoft Entra ID (Azure AD) authentication for Azure Database for PostgreSQL.
//!
//! Azure accepts a short-lived (≈1h) Entra access token as the connection
//! password. We obtain it with one of three flows and transparently refresh
//! it before every new connection:
//!
//! * **Browser** – OAuth 2.0 authorization code + PKCE through a loopback
//!   redirect. The refresh token is kept in the OS keychain, so after the
//!   first sign-in new tokens are minted silently (for ~90 days).
//! * **Device code** – for machines without a browser (SSH, remote desktops).
//! * **Azure CLI** – reuses an existing `az login` session.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

/// Azure clouds have separate identity endpoints and resource scopes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AzureCloud {
    #[default]
    Global,
    /// Azure China, operated by 21Vianet.
    China,
    UsGovernment,
}

impl AzureCloud {
    pub const ALL: [AzureCloud; 3] = [AzureCloud::Global, AzureCloud::China, AzureCloud::UsGovernment];
    pub fn label(self) -> &'static str {
        match self {
            AzureCloud::Global => "Azure (global)",
            AzureCloud::China => "Azure China (21Vianet)",
            AzureCloud::UsGovernment => "Azure US Government",
        }
    }
    pub fn login_host(self) -> &'static str {
        match self {
            AzureCloud::Global => "https://login.microsoftonline.com",
            AzureCloud::China => "https://login.chinacloudapi.cn",
            AzureCloud::UsGovernment => "https://login.microsoftonline.us",
        }
    }
    /// Scope of the Azure Database for PostgreSQL resource in this cloud.
    pub fn scope(self) -> &'static str {
        match self {
            AzureCloud::Global => "https://ossrdbms-aad.database.windows.net/.default",
            AzureCloud::China => "https://ossrdbms-aad.database.chinacloudapi.cn/.default",
            AzureCloud::UsGovernment => "https://ossrdbms-aad.database.usgovcloudapi.net/.default",
        }
    }
    fn cli_name(self) -> &'static str {
        match self {
            AzureCloud::Global => "AzureCloud",
            AzureCloud::China => "AzureChinaCloud",
            AzureCloud::UsGovernment => "AzureUSGovernment",
        }
    }
    /// Detects the cloud from a server host name.
    pub fn from_host(host: &str) -> Option<AzureCloud> {
        let h = host.to_ascii_lowercase();
        if h.ends_with(".chinacloudapi.cn") {
            Some(AzureCloud::China)
        } else if h.ends_with(".usgovcloudapi.net") {
            Some(AzureCloud::UsGovernment)
        } else if h.ends_with(".azure.com") {
            Some(AzureCloud::Global)
        } else {
            None
        }
    }
}
/// Public client id of the Azure CLI; pre-authorised for the ossrdbms resource.
/// Organisations that block it can register their own public client app.
pub const DEFAULT_CLIENT_ID: &str = "04b07795-8ddb-461a-bbee-02f9e1bf7b46";
const KEYRING_SERVICE: &str = "trunq-azure";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AzureFlow {
    #[default]
    Browser,
    DeviceCode,
    AzureCli,
    /// OAuth client credentials; the secret is read from an environment variable.
    ClientSecret,
}

impl AzureFlow {
    pub const ALL: [AzureFlow; 4] = [AzureFlow::Browser, AzureFlow::DeviceCode, AzureFlow::AzureCli, AzureFlow::ClientSecret];
    pub fn label(self) -> &'static str {
        match self {
            AzureFlow::Browser => "Interactive (browser)",
            AzureFlow::DeviceCode => "Device code",
            AzureFlow::AzureCli => "Azure CLI (az login)",
            AzureFlow::ClientSecret => "Service principal (secret from environment)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AzureConfig {
    pub flow: AzureFlow,
    #[serde(default)]
    pub cloud: AzureCloud,
    /// Tenant id or domain; empty means `organizations`.
    #[serde(default)]
    pub tenant: String,
    /// Custom app registration; empty means the Azure CLI public client.
    #[serde(default)]
    pub client_id: String,
    /// Variable name only. The value is never serialized or stored in keyring.
    #[serde(default)]
    pub client_secret_env: String,
}

impl AzureConfig {
    fn tenant(&self) -> &str {
        if self.tenant.trim().is_empty() { "organizations" } else { self.tenant.trim() }
    }
    fn client_id(&self) -> &str {
        if self.client_id.trim().is_empty() { DEFAULT_CLIENT_ID } else { self.client_id.trim() }
    }
    fn cache_key(&self, login_hint: &str) -> String {
        // Profiles for different accounts/groups must not silently share the
        // first interactive sign-in or refresh token in the same tenant.
        serde_json::to_string(&(self.flow, self.cloud, self.tenant(), self.client_id(), login_hint.trim(), self.client_secret_env.trim()))
            .expect("serializing string fields")
    }
}

#[derive(Clone, Debug)]
pub struct AccessToken {
    pub token: String,
    pub expires_at: SystemTime,
    /// User principal name from the token (used as the Postgres user name).
    pub upn: Option<String>,
}

impl AccessToken {
    fn fresh(&self) -> bool {
        self.expires_at > SystemTime::now() + Duration::from_secs(300)
    }
    pub fn minutes_left(&self) -> i64 {
        self.expires_at.duration_since(SystemTime::now()).map(|d| d.as_secs() as i64 / 60).unwrap_or(0)
    }
}

/// What the UI should show while an interactive sign-in is in progress.
#[derive(Clone, Debug)]
pub enum SignInPrompt {
    Browser { url: String },
    DeviceCode { url: String, code: String },
}

static TOKENS: LazyLock<Mutex<HashMap<String, AccessToken>>> = LazyLock::new(Default::default);
/// Serialises interactive sign-ins so that opening five tabs at once only
/// produces one browser window.
static SIGN_IN_LOCK: LazyLock<Mutex<()>> = LazyLock::new(Default::default);
pub static PROMPT: LazyLock<Mutex<Option<SignInPrompt>>> = LazyLock::new(Default::default);
pub static CANCEL_SIGN_IN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Returns a valid access token, refreshing or signing in as needed.
/// Blocking; call from a worker thread.
pub fn get_token(cfg: &AzureConfig, login_hint: &str) -> Result<AccessToken> {
    let key = cfg.cache_key(login_hint);
    // The CLI's selected identity can change outside Trunq (az login/account
    // set). Let its cache own renewal and account selection on every connection.
    if cfg.flow == AzureFlow::AzureCli {
        let token = azure_cli_token(cfg)?;
        TOKENS.lock().insert(key, token.clone());
        return Ok(token);
    }
    if let Some(t) = TOKENS.lock().get(&key)
        && t.fresh()
    {
        return Ok(t.clone());
    }
    if cfg.flow == AzureFlow::ClientSecret {
        let token = client_secret_token(cfg, &key)?;
        TOKENS.lock().insert(key, token.clone());
        return Ok(token);
    }
    let _guard = SIGN_IN_LOCK.lock();
    // Another thread may have signed in while we waited.
    if let Some(t) = TOKENS.lock().get(&key)
        && t.fresh()
    {
        return Ok(t.clone());
    }
    let token = match cfg.flow {
        AzureFlow::AzureCli => azure_cli_token(cfg)?,
        AzureFlow::ClientSecret => client_secret_token(cfg, &key)?,
        AzureFlow::Browser | AzureFlow::DeviceCode => {
            let refreshed = load_refresh_token(&key).and_then(|rt| match refresh(cfg, &key, &rt) {
                Ok(t) => Some(t),
                Err(e) => {
                    eprintln!("trunq: token refresh failed, signing in again: {e:#}");
                    None
                }
            });
            match refreshed {
                Some(t) => t,
                None if cfg.flow == AzureFlow::Browser => browser_sign_in(cfg, &key, login_hint)?,
                None => device_code_sign_in(cfg, &key)?,
            }
        }
    };
    TOKENS.lock().insert(key, token.clone());
    Ok(token)
}

/// Forgets cached tokens and the stored refresh token (sign out).
pub fn sign_out(cfg: &AzureConfig, login_hint: &str) {
    let key = cfg.cache_key(login_hint);
    TOKENS.lock().remove(&key);
    if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, &key) {
        let _ = e.delete_credential();
    }
}

pub fn cached_token_info(cfg: &AzureConfig, login_hint: &str) -> Option<(Option<String>, i64)> {
    // The dialog needs identity/expiry, not a copy of a multi-kilobyte bearer
    // credential on every rendered frame.
    TOKENS.lock().get(&cfg.cache_key(login_hint)).map(|token| (token.upn.clone(), token.minutes_left()))
}

fn load_refresh_token(key: &str) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, key).ok()?.get_password().ok()
}

fn store_refresh_token(key: &str, rt: &str) {
    if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, key) {
        let _ = e.set_password(rt);
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(Duration::from_secs(30))).build().new_agent()
}

fn endpoint(cfg: &AzureConfig, path: &str) -> String {
    format!("{}/{}/oauth2/v2.0/{path}", cfg.cloud.login_host(), cfg.tenant())
}

fn scopes(cfg: &AzureConfig) -> String {
    format!("{} offline_access openid profile", cfg.cloud.scope())
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

fn post_token(cfg: &AzureConfig, form: &[(&str, &str)]) -> Result<TokenResponse> {
    let mut resp = agent().post(endpoint(cfg, "token")).send_form(form.iter().copied())?;
    let body = resp.body_mut().read_to_string()?;
    serde_json::from_str(&body).context("unexpected token response format")
}

fn finish(key: &str, r: TokenResponse) -> Result<AccessToken> {
    if let Some(err) = r.error {
        bail!("{err}: {}", r.error_description.unwrap_or_default().lines().next().unwrap_or(""));
    }
    let token = r.access_token.ok_or_else(|| anyhow!("no access token in response"))?;
    if let Some(rt) = &r.refresh_token {
        store_refresh_token(key, rt);
    }
    let claims = jwt_claims(&token);
    let expires_at = claims
        .as_ref()
        .and_then(|c| c.get("exp")?.as_u64())
        .map(|exp| UNIX_EPOCH + Duration::from_secs(exp))
        .unwrap_or_else(|| SystemTime::now() + Duration::from_secs(r.expires_in.unwrap_or(3600)));
    Ok(AccessToken { upn: claims.as_ref().and_then(upn_from_claims), token, expires_at })
}

fn refresh(cfg: &AzureConfig, key: &str, rt: &str) -> Result<AccessToken> {
    let scope = scopes(cfg);
    let r = post_token(cfg, &[("client_id", cfg.client_id()), ("grant_type", "refresh_token"), ("refresh_token", rt), ("scope", &scope)])?;
    finish(key, r)
}

fn client_secret_token(cfg: &AzureConfig, key: &str) -> Result<AccessToken> {
    if cfg.tenant.trim().is_empty() || cfg.client_id.trim().is_empty() {
        bail!("service principal authentication requires an explicit tenant and application client id");
    }
    let name = if cfg.client_secret_env.trim().is_empty() { "AZURE_CLIENT_SECRET" } else { cfg.client_secret_env.trim() };
    let secret = std::env::var(name).context("service principal secret environment variable is missing or is not UTF-8")?;
    if secret.is_empty() {
        bail!("service principal secret environment variable is empty");
    }
    let r = post_token(cfg, &[("client_id", cfg.client_id()), ("client_secret", &secret), ("grant_type", "client_credentials"), ("scope", cfg.cloud.scope())])?;
    // Client credentials never need a persistent refresh token.
    finish(key, TokenResponse { refresh_token: None, ..r })
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn browser_sign_in(cfg: &AzureConfig, key: &str, login_hint: &str) -> Result<AccessToken> {
    let listener = TcpListener::bind("127.0.0.1:0").context("could not open a local port for the sign-in redirect")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let redirect = format!("http://localhost:{port}");
    let verifier = b64url(&rand::random::<[u8; 32]>());
    let challenge = b64url(&sha2::Sha256::digest(verifier.as_bytes()));
    let state = b64url(&rand::random::<[u8; 16]>());
    let mut url = format!(
        "{}?client_id={}&response_type=code&redirect_uri={}&response_mode=query&scope={}&state={}&code_challenge={}&code_challenge_method=S256&prompt=select_account",
        endpoint(cfg, "authorize"),
        cfg.client_id(),
        urlencode(&redirect),
        urlencode(&scopes(cfg)),
        state,
        challenge
    );
    if !login_hint.is_empty() && login_hint.contains('@') {
        url.push_str("&login_hint=");
        url.push_str(&urlencode(login_hint));
    }
    *PROMPT.lock() = Some(SignInPrompt::Browser { url: url.clone() });
    CANCEL_SIGN_IN.store(false, std::sync::atomic::Ordering::SeqCst);
    let _ = open::that_detached(&url);

    let result = (|| -> Result<String> {
        let deadline = SystemTime::now() + Duration::from_secs(300);
        loop {
            if CANCEL_SIGN_IN.load(std::sync::atomic::Ordering::SeqCst) {
                bail!("sign-in cancelled");
            }
            if SystemTime::now() > deadline {
                bail!("timed out waiting for browser sign-in");
            }
            let (mut stream, _) = match listener.accept() {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            stream.set_nonblocking(false)?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut line = String::new();
            if BufReader::new((&stream).take(8193)).read_line(&mut line).is_err() || line.len() > 8192 || !line.ends_with('\n') {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("");
            let query = path.split_once('?').map(|x| x.1).unwrap_or("");
            let params: HashMap<String, String> = query.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), urldecode(v))).collect();
            if params.is_empty() {
                // favicon or similar
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                continue;
            }
            let ok = params.contains_key("code");
            let body = if ok {
                "<html><body style='font-family:system-ui;background:#0f1720;color:#e6edf3;display:flex;align-items:center;justify-content:center;height:100vh'><div><h2>Signed in to Trunq</h2><p>You can close this window and return to the app.</p></div></body></html>".to_string()
            } else {
                format!(
                    "<html><body style='font-family:system-ui'><h2>Sign-in failed</h2><p>{}</p></body></html>",
                    html_escape(params.get("error_description").map(String::as_str).unwrap_or("unknown error"))
                )
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            if params.get("state").map(String::as_str) != Some(state.as_str()) {
                bail!("sign-in response had an invalid state parameter");
            }
            if let Some(code) = params.get("code") {
                return Ok(code.clone());
            }
            bail!("{}: {}", params.get("error").cloned().unwrap_or_default(), params.get("error_description").cloned().unwrap_or_default());
        }
    })();
    *PROMPT.lock() = None;
    let code = result?;
    let scope = scopes(cfg);
    let r = post_token(
        cfg,
        &[
            ("client_id", cfg.client_id()),
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("code_verifier", &verifier),
            ("scope", &scope),
        ],
    )?;
    finish(key, r)
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: Option<String>,
    user_code: Option<String>,
    verification_uri: Option<String>,
    interval: Option<u64>,
    expires_in: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

fn device_code_sign_in(cfg: &AzureConfig, key: &str) -> Result<AccessToken> {
    let scope = scopes(cfg);
    let mut resp = agent().post(endpoint(cfg, "devicecode")).send_form([("client_id", cfg.client_id()), ("scope", scope.as_str())])?;
    let body = resp.body_mut().read_to_string()?;
    let dc: DeviceCodeResponse = serde_json::from_str(&body).context("unexpected device code response format")?;
    if let Some(e) = dc.error {
        bail!("{e}: {}", dc.error_description.unwrap_or_default());
    }
    let device_code = dc.device_code.ok_or_else(|| anyhow!("no device code returned"))?;
    let url = dc.verification_uri.unwrap_or_else(|| "https://microsoft.com/devicelogin".into());
    *PROMPT.lock() = Some(SignInPrompt::DeviceCode { url: url.clone(), code: dc.user_code.unwrap_or_default() });
    CANCEL_SIGN_IN.store(false, std::sync::atomic::Ordering::SeqCst);
    let mut interval = dc.interval.unwrap_or(5).clamp(1, 30);
    let deadline = SystemTime::now() + Duration::from_secs(dc.expires_in.unwrap_or(900).clamp(1, 900));
    let result = loop {
        let poll_at = std::time::Instant::now() + Duration::from_secs(interval);
        while std::time::Instant::now() < poll_at && !CANCEL_SIGN_IN.load(std::sync::atomic::Ordering::SeqCst) && SystemTime::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        if CANCEL_SIGN_IN.load(std::sync::atomic::Ordering::SeqCst) {
            break Err(anyhow!("sign-in cancelled"));
        }
        if SystemTime::now() > deadline {
            break Err(anyhow!("device code expired"));
        }
        let r =
            post_token(cfg, &[("client_id", cfg.client_id()), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"), ("device_code", &device_code)]);
        match r {
            Ok(r) if r.error.as_deref() == Some("authorization_pending") => continue,
            Ok(r) if r.error.as_deref() == Some("slow_down") => interval = (interval + 5).min(30),
            Ok(r) => break finish(key, r),
            Err(e) => break Err(e),
        }
    };
    *PROMPT.lock() = None;
    result
}

fn azure_cli_token(cfg: &AzureConfig) -> Result<AccessToken> {
    let mut args: Vec<String> = ["account", "get-access-token", "--scope", cfg.cloud.scope(), "--output", "json"].into_iter().map(str::to_string).collect();
    // `--scope` (rather than `--resource-type`) works regardless of which
    // cloud the CLI is currently pointed at, as long as the user signed in there.
    if !cfg.tenant.trim().is_empty() {
        args.extend(["--tenant".into(), cfg.tenant.trim().into()]);
    }
    let out = super::runtime()
        .block_on(super::credentials::run(if cfg!(windows) { "az.cmd" } else { "az" }, &args, Duration::from_secs(30)))
        .with_context(|| format!("Azure CLI token acquisition failed; check `az cloud set --name {}` and your CLI login", cfg.cloud.cli_name()))?;
    let v: serde_json::Value = serde_json::from_slice(&out).context("Azure CLI returned invalid JSON")?;
    let token = v["accessToken"].as_str().ok_or_else(|| anyhow!("az returned no accessToken"))?.to_string();
    let claims = jwt_claims(&token);
    let expires_at = v["expires_on"]
        .as_u64()
        .or_else(|| claims.as_ref()?.get("exp")?.as_u64())
        .map(|e| UNIX_EPOCH + Duration::from_secs(e))
        .unwrap_or_else(|| SystemTime::now() + Duration::from_secs(45 * 60));
    Ok(AccessToken { upn: claims.as_ref().and_then(upn_from_claims), token, expires_at })
}

fn jwt_claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn upn_from_claims(c: &serde_json::Value) -> Option<String> {
    ["upn", "preferred_username", "unique_name", "email"].iter().find_map(|k| c.get(*k)?.as_str().map(str::to_string))
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                if let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|s| u8::from_str_radix(s, 16).ok()) {
                    out.push(v);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Heuristic used to pre-select Entra auth & SSL for Azure hosts.
pub fn is_azure_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    [".postgres.database.azure.com", ".postgres.cosmos.azure.com", ".postgres.database.chinacloudapi.cn", ".postgres.database.usgovcloudapi.net"]
        .iter()
        .any(|s| h.ends_with(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_roundtrip() {
        let s = "a b/c?d=e&f=ü";
        assert_eq!(urldecode(&urlencode(s)), s);
        assert_eq!(urldecode("x%2"), "x%2");
        assert_eq!(urldecode("%é🙂"), "%é🙂");
    }

    #[test]
    fn token_cache_separates_accounts_and_flows() {
        let cfg = AzureConfig::default();
        assert_ne!(cfg.cache_key("alice@example.com"), cfg.cache_key("bob@example.com"));
        assert_eq!(cfg.cache_key(" alice@example.com "), cfg.cache_key("alice@example.com"));
        assert_ne!(cfg.cache_key("alice"), AzureConfig { flow: AzureFlow::ClientSecret, ..cfg.clone() }.cache_key("alice"));
        let old: AzureConfig = serde_json::from_str(r#"{"flow":"AzureCli","tenant":"","client_id":""}"#).unwrap();
        assert_eq!(old.client_secret_env, "");
    }

    #[test]
    fn claims() {
        let payload = b64url(br#"{"upn":"jane@contoso.com","exp":2000000000}"#);
        let c = jwt_claims(&format!("h.{payload}.s")).unwrap();
        assert_eq!(upn_from_claims(&c).as_deref(), Some("jane@contoso.com"));
    }

    #[test]
    fn clouds() {
        assert_eq!(AzureCloud::from_host("x.postgres.database.chinacloudapi.cn"), Some(AzureCloud::China));
        assert_eq!(AzureCloud::from_host("x.postgres.database.azure.com"), Some(AzureCloud::Global));
        assert!(is_azure_host("x.postgres.database.usgovcloudapi.net"));
        let cfg = AzureConfig { cloud: AzureCloud::China, ..Default::default() };
        assert!(endpoint(&cfg, "token").starts_with("https://login.chinacloudapi.cn/organizations/"));
    }
}
