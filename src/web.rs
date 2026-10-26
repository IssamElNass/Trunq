//! Browser client and HTTP API. Each browser owns its database connections;
//! passwords never enter the persisted web workspace.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{Environment, Profile};
use crate::db::meta;
use crate::db::result::{Column, ResultSet};
use crate::db::wire::{CancelToken, PgConn, SslMode, StepOutcome, TlsConfig};

const SESSION_IDLE: Duration = Duration::from_secs(30 * 60);
const PAGE_SIZE: usize = 500;
const MAX_SESSIONS: usize = 32;

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Connection {
    id: String,
    name: String,
    host: String,
    port: u16,
    user: String,
    database: String,
    ssl_mode: SslMode,
    tls: TlsConfig,
    read_only: bool,
    environment: Environment,
}

impl Default for Connection {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            host: "host.docker.internal".into(),
            port: 5432,
            user: "postgres".into(),
            database: "postgres".into(),
            ssl_mode: SslMode::Prefer,
            tls: TlsConfig::default(),
            read_only: false,
            environment: Environment::None,
        }
    }
}

struct Database {
    connection: Connection,
    query: PgConn,
    browser: PgConn,
    columns: Vec<Column>,
    query_id: String,
}

struct BrowserSession {
    csrf: String,
    last_seen: Mutex<Instant>,
    database: tokio::sync::Mutex<Option<Database>>,
    cancel: Mutex<Option<CancelToken>>,
}

impl BrowserSession {
    fn new() -> Self {
        Self { csrf: random_id(), last_seen: Mutex::new(Instant::now()), database: tokio::sync::Mutex::new(None), cancel: Mutex::new(None) }
    }
}

struct Server {
    sessions: Mutex<HashMap<String, Arc<BrowserSession>>>,
    profiles: Mutex<Vec<Connection>>,
    origin: Option<String>,
}

type ApiResult = Result<Json<Value>, ApiError>;
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self(StatusCode::BAD_REQUEST, format!("{error:#}"))
    }
}

fn bad_request(message: &str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message.into())
}

fn busy() -> ApiError {
    ApiError(StatusCode::CONFLICT, "A request is running. Cancel it or wait for it to finish.".into())
}

fn random_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

fn profile_path() -> std::path::PathBuf {
    crate::config::config_dir().join("web-connections.json")
}

fn router(server: Arc<Server>) -> Router {
    Router::new()
        .route("/", get(|| async { Html(include_str!("../web/index.html")) }))
        .route("/app.js", get(|| async { ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], include_str!("../web/app.js")) }))
        .route("/app.css", get(|| async { ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], include_str!("../web/app.css")) }))
        .route("/favicon.png", get(|| async { ([(header::CONTENT_TYPE, "image/png")], include_bytes!("../assets/logo_256.png").as_slice()) }))
        .route("/healthz", get(|| async { Json(json!({"status": "ok"})) }))
        .route("/api/session", get(bootstrap))
        .route("/api/connect", post(connect))
        .route("/api/disconnect", post(disconnect))
        .route("/api/connections", post(save_connection))
        .route("/api/connections/delete", post(delete_connection))
        .route("/api/catalog", post(catalog))
        .route("/api/query", post(query))
        .route("/api/format", post(format_sql))
        .route("/api/fetch", post(fetch))
        .route("/api/cancel", post(cancel))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(middleware::from_fn_with_state(server.clone(), protect))
        .with_state(server)
}

pub fn run() -> anyhow::Result<()> {
    let profiles = match std::fs::read(profile_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    let origin = std::env::var("TRUNQ_WEB_ORIGIN").ok().map(|s| s.trim_end_matches('/').to_string());
    if let Some(origin) = &origin {
        anyhow::ensure!(origin.starts_with("http://") || origin.starts_with("https://"), "TRUNQ_WEB_ORIGIN must be an http(s) origin");
        anyhow::ensure!(!origin.split_once("://").unwrap().1.contains(['/', '?', '#', '@']), "TRUNQ_WEB_ORIGIN must not contain a path, query or credentials");
    }
    let server = Arc::new(Server { sessions: Mutex::new(HashMap::new()), profiles: Mutex::new(profiles), origin });
    let address = std::env::var("TRUNQ_WEB_LISTEN").unwrap_or_else(|_| "127.0.0.1:8080".into());
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(async {
        let listener = tokio::net::TcpListener::bind(&address).await?;
        println!("Trunq browser client listening on http://{}", listener.local_addr()?);
        let cleaner = server.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                cleaner.sessions.lock().retain(|_, session| session.last_seen.lock().elapsed() < SESSION_IDLE);
            }
        });
        axum::serve(listener, router(server)).await?;
        Ok(())
    })
}

// Custom headers plus a per-browser token prevent cross-site requests. Host
// validation also prevents DNS rebinding against a locally exposed client.
async fn protect(State(server): State<Arc<Server>>, mut request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    if path != "/healthz" {
        let host = request.headers().get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
        let allowed = match &server.origin {
            Some(origin) => origin.split_once("://").is_some_and(|(_, authority)| authority == host),
            None => local_host(host),
        };
        if !allowed || request.headers().get("sec-fetch-site").is_some_and(|v| v == "cross-site") {
            return ApiError(StatusCode::FORBIDDEN, "This origin is not allowed. Configure TRUNQ_WEB_ORIGIN for a reverse proxy.".into()).into_response();
        }
    }
    if path.starts_with("/api/") && path != "/api/session" {
        let id = session_cookie(request.headers());
        let session = id.and_then(|id| server.sessions.lock().get(id).cloned());
        let Some(session) = session else {
            return ApiError(StatusCode::UNAUTHORIZED, "Browser session expired. Reload the page to reconnect.".into()).into_response();
        };
        if session.last_seen.lock().elapsed() >= SESSION_IDLE {
            return ApiError(StatusCode::UNAUTHORIZED, "Browser session expired. Reload the page to reconnect.".into()).into_response();
        }
        if request.method() != Method::GET && request.headers().get("x-trunq-csrf").and_then(|v| v.to_str().ok()) != Some(session.csrf.as_str()) {
            return ApiError(StatusCode::FORBIDDEN, "Invalid browser request token.".into()).into_response();
        }
        *session.last_seen.lock() = Instant::now();
        request.extensions_mut().insert(session);
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("content-security-policy", HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'"));
    response
}

fn local_host(host: &str) -> bool {
    if host.contains('@') {
        return false;
    }
    let authority = host.parse::<axum::http::uri::Authority>();
    authority.is_ok_and(|a| matches!(a.host(), "localhost" | "127.0.0.1" | "[::1]"))
}

fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::COOKIE)?.to_str().ok()?.split(';').find_map(|part| part.trim().strip_prefix("trunq_session="))
}

async fn bootstrap(State(server): State<Arc<Server>>, headers: HeaderMap) -> Result<Response, ApiError> {
    let mut sessions = server.sessions.lock();
    sessions.retain(|_, s| s.last_seen.lock().elapsed() < SESSION_IDLE);
    let existing = session_cookie(&headers).and_then(|id| sessions.get(id).map(|s| (id.to_string(), s.clone())));
    let (id, session) = match existing {
        Some(pair) => pair,
        None => {
            if sessions.len() >= MAX_SESSIONS {
                return Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, "Too many browser sessions. Try again later.".into()));
            }
            let id = random_id();
            let session = Arc::new(BrowserSession::new());
            sessions.insert(id.clone(), session.clone());
            (id, session)
        }
    };
    *session.last_seen.lock() = Instant::now();
    drop(sessions);
    let current = session
        .database
        .try_lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|db| json!({"connection": db.connection, "server_version": db.query.server_version()})));
    let mut response = Json(json!({"csrf": session.csrf, "connections": *server.profiles.lock(), "page_size": PAGE_SIZE, "current": current})).into_response();
    let secure = if server.origin.as_ref().is_some_and(|s| s.starts_with("https://")) { "; Secure" } else { "" };
    response
        .headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_str(&format!("trunq_session={id}; Path=/; HttpOnly; SameSite=Strict{secure}")).unwrap());
    Ok(response)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectRequest {
    connection: Connection,
    password: Option<String>,
}

async fn connect(Extension(session): Extension<Arc<BrowserSession>>, Json(request): Json<ConnectRequest>) -> ApiResult {
    let mut database = session.database.try_lock().map_err(|_| busy())?;
    let mut c = request.connection;
    if c.id.is_empty() {
        c.id = random_id();
    }
    let saved_connection = c.clone();
    validate_connection(&c)?;
    let profile = Profile {
        host: c.host,
        port: c.port,
        user: c.user,
        database: c.database,
        ssl_mode: c.ssl_mode,
        tls: c.tls,
        read_only: c.read_only,
        save_password: false,
        statement_timeout_s: 300,
        ..Profile::default()
    };
    let mut params =
        tokio::task::spawn_blocking(move || profile.connect_params("", request.password.as_deref())).await.map_err(|e| bad_request(&e.to_string()))??;
    params.application_name = "Trunq: Browser query".into();
    params.options = Some(format!("{} -c idle_in_transaction_session_timeout=60s", params.options.unwrap_or_default()));
    let query = PgConn::connect(&params).await?;
    params.application_name = "Trunq: Browser catalog".into();
    params.options = Some("-c default_transaction_read_only=on -c statement_timeout=60s -c lock_timeout=5s -c idle_in_transaction_session_timeout=60s".into());
    let browser = PgConn::connect(&params).await?;
    let result = json!({"server_version": query.server_version(), "pid": query.backend_pid(), "connection": saved_connection});
    *session.cancel.lock() = Some(query.cancel_token());
    *database = Some(Database { connection: saved_connection, query, browser, columns: Vec::new(), query_id: String::new() });
    Ok(Json(result))
}

async fn disconnect(Extension(session): Extension<Arc<BrowserSession>>) -> ApiResult {
    let token = session.cancel.lock().clone();
    if let Some(token) = token {
        let _ = token.cancel().await;
    }
    // A cancelled request still owns the stream until its error is drained.
    let mut database = session.database.lock().await;
    *session.cancel.lock() = None;
    *database = None;
    Ok(Json(json!({"ok": true})))
}

fn validate_connection(c: &Connection) -> Result<(), ApiError> {
    if c.host.trim().is_empty() || c.user.trim().is_empty() || c.database.trim().is_empty() || c.port == 0 {
        return Err(bad_request("Host, port, database and user are required."));
    }
    if [&c.name, &c.host, &c.user, &c.database].iter().any(|s| s.len() > 1024 || s.contains('\0')) {
        return Err(bad_request("Connection field is too long or contains a NUL character."));
    }
    Ok(())
}

async fn save_connection(State(server): State<Arc<Server>>, Json(mut connection): Json<Connection>) -> ApiResult {
    validate_connection(&connection)?;
    let mut profiles = server.profiles.lock();
    let mut next = profiles.clone();
    if connection.id.is_empty() {
        connection.id = random_id();
    }
    if let Some(existing) = next.iter_mut().find(|c| c.id == connection.id) {
        *existing = connection;
    } else {
        if next.len() >= 100 {
            return Err(bad_request("At most 100 connections can be saved."));
        }
        next.push(connection);
    }
    crate::storage::write_private_atomic(&profile_path(), &serde_json::to_vec_pretty(&next).map_err(anyhow::Error::from)?).map_err(anyhow::Error::from)?;
    *profiles = next;
    Ok(Json(json!({"connections": *profiles})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdRequest {
    id: String,
}

async fn delete_connection(State(server): State<Arc<Server>>, Json(request): Json<IdRequest>) -> ApiResult {
    let mut profiles = server.profiles.lock();
    let next: Vec<_> = profiles.iter().filter(|c| c.id != request.id).cloned().collect();
    crate::storage::write_private_atomic(&profile_path(), &serde_json::to_vec_pretty(&next).map_err(anyhow::Error::from)?).map_err(anyhow::Error::from)?;
    *profiles = next;
    Ok(Json(json!({"connections": *profiles})))
}

async fn catalog(Extension(session): Extension<Arc<BrowserSession>>) -> ApiResult {
    let mut guard = session.database.try_lock().map_err(|_| busy())?;
    let db = guard.as_mut().ok_or_else(|| bad_request("Connect to a database first."))?;
    let catalog = meta::parse_catalog(db.browser.simple_query(&meta::catalog_sql(false)).await?);
    Ok(Json(json!({"schemas": catalog.schemas.iter().map(|schema| json!({
        "name": schema.name,
        "relations": schema.relations.iter().map(|r| json!({
            "name": r.name, "kind": r.kind.label(), "sql_name": r.qualified(), "rows": r.est_rows,
            "columns": r.columns.iter().map(|c| json!({"name": c.name, "type": c.data_type})).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })).collect::<Vec<_>>()})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryRequest {
    sql: String,
}

fn result_json(rows: &ResultSet, columns: &[Column], outcome: &StepOutcome) -> Value {
    json!({
        "columns": columns.iter().map(|c| json!({"name": c.name, "type": crate::db::types::builtin(c.type_oid).map(|(name, _)| name.to_string()).unwrap_or_else(|| format!("oid {}", c.type_oid))})).collect::<Vec<_>>(),
        "rows": rows.rows().map(|r| (0..rows.ncols()).map(|c| rows.get(r, c)).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "tag": match outcome { StepOutcome::Done { tag } => tag.as_str(), _ => "" },
        "more": matches!(outcome, StepOutcome::Suspended),
    })
}

async fn query(Extension(session): Extension<Arc<BrowserSession>>, Json(request): Json<QueryRequest>) -> ApiResult {
    let statements = crate::sql::lexer::split_statements(&request.sql);
    if statements.is_empty() {
        return Err(bad_request("Enter a SQL statement."));
    }
    if statements.len() > 32 {
        return Err(bad_request("Run at most 32 statements at once."));
    }
    let mut guard = session.database.try_lock().map_err(|_| busy())?;
    let db = guard.as_mut().ok_or_else(|| bad_request("Connect to a database first."))?;
    let result = run_query(db, &request.sql, &statements).await;
    match result {
        Ok(value) => Ok(Json(value)),
        Err(error) => {
            // A transport failure can leave the PostgreSQL stream between
            // messages. Reconnect instead of reusing an unsynchronised stream.
            *guard = None;
            *session.cancel.lock() = None;
            Err(error.into())
        }
    }
}

async fn run_query(db: &mut Database, sql: &str, statements: &[crate::sql::lexer::Statement]) -> anyhow::Result<Value> {
    db.query_id = random_id();
    let mut results = Vec::new();
    let started = Instant::now();
    for (index, statement) in statements.iter().enumerate() {
        let (columns, result) = db.query.start(statement.text(sql), &[], PAGE_SIZE).await?;
        db.columns = columns.unwrap_or_default();
        match result {
            Ok((rows, outcome)) => {
                if db.columns.is_empty() && !rows.columns.is_empty() {
                    db.columns = rows.columns.as_ref().clone();
                }
                let mut value = result_json(&rows, &db.columns, &outcome);
                if index + 1 < statements.len() && matches!(outcome, StepOutcome::Suspended) {
                    db.query.close_portal().await?;
                    value["more"] = json!(false);
                    value["truncated"] = json!(true);
                }
                results.push(value);
            }
            Err(error) => {
                results.push(json!({"error": error.to_string(), "code": error.code, "position": error.position}));
                break;
            }
        }
    }
    let notices: Vec<_> = std::mem::take(&mut db.query.notices).into_iter().map(|n| n.message).collect();
    Ok(
        json!({"results": results, "query_id": db.query_id, "ms": started.elapsed().as_millis(), "transaction": format!("{:?}", db.query.tx_status), "notices": notices}),
    )
}

async fn fetch(Extension(session): Extension<Arc<BrowserSession>>, Json(request): Json<IdRequest>) -> ApiResult {
    let mut guard = session.database.try_lock().map_err(|_| busy())?;
    let db = guard.as_mut().ok_or_else(|| bad_request("Connect to a database first."))?;
    if request.id != db.query_id {
        return Err(bad_request("This result has been replaced. Run the query again."));
    }
    let (rows, outcome) = db.query.fetch(db.columns.len(), PAGE_SIZE).await?.map_err(anyhow::Error::from)?;
    Ok(Json(result_json(&rows, &db.columns, &outcome)))
}

async fn cancel(Extension(session): Extension<Arc<BrowserSession>>) -> ApiResult {
    let token = session.cancel.lock().clone().ok_or_else(|| bad_request("No database connection."))?;
    token.cancel().await?;
    Ok(Json(json!({"ok": true})))
}

async fn format_sql(Json(request): Json<QueryRequest>) -> ApiResult {
    Ok(Json(json!({"sql": crate::sql::format::format_sql(&request.sql, true)})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_validation_rejects_rebinding_and_lookalikes() {
        for host in ["localhost:8080", "127.0.0.1:8080", "[::1]:8080"] {
            assert!(local_host(host), "{host}");
        }
        for host in ["evil.test", "localhost.evil.test:8080", "user@localhost:8080", "127.0.0.1.evil.test"] {
            assert!(!local_host(host), "{host}");
        }
    }
    #[test]
    fn persisted_connections_cannot_contain_passwords_or_commands() {
        assert!(serde_json::from_value::<Connection>(json!({"password": "secret"})).is_err());
        assert!(serde_json::from_value::<Connection>(json!({"auth": {"TokenCommand": {"executable": "sh"}}})).is_err());
        assert!(!serde_json::to_string(&Connection::default()).unwrap().contains("password"));
    }
    #[test]
    fn result_serialization_keeps_nulls_empty_text_and_large_integers() {
        let columns = vec![Column::text("value")];
        let mut rows = ResultSet::new(columns.clone());
        for value in [None, Some(""), Some("9223372036854775807"), Some("<script>bad()</script>")] {
            rows.push_text_row(&[value]);
        }
        let value = result_json(&rows, &columns, &StepOutcome::Suspended);
        assert_eq!(value["rows"], json!([[null], [""], ["9223372036854775807"], ["<script>bad()</script>"]]));
        assert_eq!(value["more"], true);
    }
}
