//! A small, purpose-built PostgreSQL wire-protocol client.
//!
//! Unlike general purpose drivers we always request *text* result format, which
//! means every type (including extension types such as PostGIS geometries,
//! hstore, ltree, enums, composites...) is rendered exactly as `psql` would
//! render it, without any per-type decoding work on our side. Rows are copied
//! straight from the socket buffer into a compact [`ResultSet`].
//!
//! Large results are fetched incrementally through an unnamed portal
//! (`Execute` with a row limit), so opening a 50M-row table is instant.

use std::collections::HashMap;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use bytes::BytesMut;
use fallible_iterator::FallibleIterator;
use postgres_protocol::authentication;
use postgres_protocol::authentication::sasl;
use postgres_protocol::message::{backend::Message, frontend};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;

use super::result::{Column, ResultSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    pub const ALL: [SslMode; 5] = [SslMode::Disable, SslMode::Prefer, SslMode::Require, SslMode::VerifyCa, SslMode::VerifyFull];
    pub fn label(self) -> &'static str {
        match self {
            SslMode::Disable => "disable",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyCa => "verify-ca",
            SslMode::VerifyFull => "verify-full",
        }
    }
    pub fn parse(s: &str) -> Option<SslMode> {
        SslMode::ALL.into_iter().find(|m| m.label() == s)
    }
}

#[derive(Clone, Debug)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub database: String,
    pub ssl_mode: SslMode,
    pub application_name: String,
    pub connect_timeout: Duration,
    /// Extra startup parameters (e.g. `options`, `search_path`).
    pub options: Option<String>,
    /// Optional SSH tunnel and its secret (password or key passphrase).
    pub ssh: Option<(super::ssh::SshConfig, Option<String>)>,
}

/// A database error as reported by the server, with all the fields that are
/// useful to show in a UI (position for highlighting, hint, detail...).
#[derive(Clone, Debug, Default)]
pub struct DbError {
    pub severity: String,
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
    /// 1-based character position into the statement text.
    pub position: Option<usize>,
    pub where_: Option<String>,
    pub schema: Option<String>,
    pub table: Option<String>,
    pub column: Option<String>,
    pub constraint: Option<String>,
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.severity, self.message)?;
        if let Some(d) = &self.detail {
            write!(f, "\nDETAIL: {d}")?;
        }
        if let Some(h) = &self.hint {
            write!(f, "\nHINT: {h}")?;
        }
        if let Some(w) = &self.where_ {
            write!(f, "\nCONTEXT: {w}")?;
        }
        Ok(())
    }
}

impl std::error::Error for DbError {}

impl DbError {
    fn from_fields<'a>(mut fields: impl FallibleIterator<Item = postgres_protocol::message::backend::ErrorField<'a>, Error = io::Error>) -> DbError {
        let mut e = DbError::default();
        while let Ok(Some(f)) = fields.next() {
            let v = String::from_utf8_lossy(f.value_bytes()).into_owned();
            match f.type_() {
                b'V' => e.severity = v,
                b'S' if e.severity.is_empty() => e.severity = v,
                b'C' => e.code = v,
                b'M' => e.message = v,
                b'D' => e.detail = Some(v),
                b'H' => e.hint = Some(v),
                b'P' => e.position = v.parse().ok(),
                b'W' => e.where_ = Some(v),
                b's' => e.schema = Some(v),
                b't' => e.table = Some(v),
                b'c' => e.column = Some(v),
                b'n' => e.constraint = Some(v),
                _ => {}
            }
        }
        e
    }
}

#[derive(Clone, Debug)]
pub struct Notice {
    pub severity: String,
    pub message: String,
    pub detail: Option<String>,
    pub hint: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Notification {
    pub pid: i32,
    pub channel: String,
    pub payload: String,
}

/// Everything needed to cancel the query running on a connection, from any thread.
#[derive(Clone, Debug)]
pub struct CancelToken {
    params: ConnectParams,
    used_tls: bool,
    pid: i32,
    secret: i32,
}

impl CancelToken {
    pub async fn cancel(&self) -> Result<()> {
        let mut stream = open_stream(&self.params, self.used_tls).await?.0;
        let mut buf = BytesMut::new();
        frontend::cancel_request(self.pid, self.secret, &mut buf);
        stream.write_all(&buf).await?;
        stream.flush().await?;
        // The server closes the connection once it processed the request.
        let mut sink = [0u8; 16];
        let _ = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut sink)).await;
        Ok(())
    }
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

struct Stream(Box<dyn Io>);

impl AsyncRead for Stream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.0).poll_read(cx, buf)
    }
}
impl AsyncWrite for Stream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.0).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.0).poll_shutdown(cx)
    }
}

/// Opens a TCP / unix socket stream and performs TLS negotiation.
/// Returns the stream and whether TLS is in use.
async fn open_stream(p: &ConnectParams, want_tls: bool) -> Result<(Stream, bool)> {
    let host = if p.host.is_empty() { "localhost" } else { p.host.as_str() };
    let mut base: Stream = if let Some((ssh, secret)) = &p.ssh {
        // Tunnel: the database host/port are resolved by the SSH server.
        let ch = tokio::time::timeout(p.connect_timeout + Duration::from_secs(10), super::ssh::open_channel(ssh, secret.as_deref(), host, p.port))
            .await
            .map_err(|_| anyhow!("timeout opening SSH tunnel"))??;
        Stream(Box::new(ch))
    } else {
        #[cfg(unix)]
        if p.host.starts_with('/') {
            let path = format!("{}/.s.PGSQL.{}", p.host.trim_end_matches('/'), p.port);
            let s = tokio::net::UnixStream::connect(&path).await.with_context(|| format!("could not connect to socket {path}"))?;
            return Ok((Stream(Box::new(s)), false));
        }
        let tcp = tokio::time::timeout(p.connect_timeout, TcpStream::connect((host, p.port)))
            .await
            .map_err(|_| anyhow!("timeout connecting to {host}:{}", p.port))?
            .with_context(|| format!("could not connect to {host}:{}", p.port))?;
        tcp.set_nodelay(true).ok();
        // Detect dead peers (VPN drops, sleeping laptops, NAT timeouts) so a
        // broken connection is noticed instead of hanging around forever.
        let ka = socket2::TcpKeepalive::new().with_time(Duration::from_secs(30));
        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        let ka = ka.with_interval(Duration::from_secs(10));
        let _ = socket2::SockRef::from(&tcp).set_tcp_keepalive(&ka);
        Stream(Box::new(tcp))
    };
    if !want_tls || p.ssl_mode == SslMode::Disable {
        return Ok((base, false));
    }
    let mut buf = BytesMut::new();
    frontend::ssl_request(&mut buf);
    base.write_all(&buf).await?;
    base.flush().await?;
    let mut answer = [0u8; 1];
    base.read_exact(&mut answer).await?;
    if answer[0] != b'S' {
        if p.ssl_mode == SslMode::Prefer {
            return Ok((base, false));
        }
        bail!("server does not support SSL, but sslmode={} requires it", p.ssl_mode.label());
    }
    let mut builder = native_tls::TlsConnector::builder();
    match p.ssl_mode {
        SslMode::Prefer | SslMode::Require => {
            builder.danger_accept_invalid_certs(true).danger_accept_invalid_hostnames(true);
        }
        SslMode::VerifyCa => {
            builder.danger_accept_invalid_hostnames(true);
        }
        _ => {}
    }
    let connector = tokio_native_tls::TlsConnector::from(builder.build()?);
    let tls = connector.connect(host, base).await.context("TLS handshake failed")?;
    Ok((Stream(Box::new(tls)), true))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatus {
    Idle,
    InTransaction,
    Failed,
}

/// Outcome of executing (part of) a statement through the extended protocol.
#[derive(Debug)]
pub enum StepOutcome {
    /// Statement finished; command tag such as `SELECT 10` / `UPDATE 3`.
    Done { tag: String },
    /// More rows are available in the open portal.
    Suspended,
    /// Statement was empty.
    Empty,
}

pub struct PgConn {
    stream: Stream,
    rbuf: BytesMut,
    wbuf: BytesMut,
    pub params: HashMap<String, String>,
    pub tx_status: TxStatus,
    pub notices: Vec<Notice>,
    pub notifications: Vec<Notification>,
    cancel: CancelToken,
    /// An unnamed portal is open and waiting for more `Execute`s.
    portal_open: bool,
}

impl PgConn {
    pub async fn connect(p: &ConnectParams) -> Result<PgConn> {
        let (stream, used_tls) = open_stream(p, true).await?;
        let mut conn = PgConn {
            stream,
            rbuf: BytesMut::with_capacity(64 * 1024),
            wbuf: BytesMut::with_capacity(8 * 1024),
            params: HashMap::new(),
            tx_status: TxStatus::Idle,
            notices: Vec::new(),
            notifications: Vec::new(),
            cancel: CancelToken { params: p.clone(), used_tls, pid: 0, secret: 0 },
            portal_open: false,
        };
        tokio::time::timeout(p.connect_timeout + Duration::from_secs(5), conn.startup(p)).await.map_err(|_| anyhow!("timeout during authentication"))??;
        Ok(conn)
    }

    async fn startup(&mut self, p: &ConnectParams) -> Result<()> {
        let mut params: Vec<(&str, &str)> = vec![
            ("user", p.user.as_str()),
            ("database", p.database.as_str()),
            ("application_name", p.application_name.as_str()),
            ("client_encoding", "UTF8"),
            ("DateStyle", "ISO, YMD"),
            ("IntervalStyle", "postgres"),
            ("extra_float_digits", "3"),
        ];
        if let Some(o) = &p.options {
            params.push(("options", o.as_str()));
        }
        frontend::startup_message(params, &mut self.wbuf)?;
        self.flush().await?;

        loop {
            match self.read_raw().await? {
                Message::AuthenticationOk => {}
                Message::AuthenticationCleartextPassword => {
                    let pw = self.require_password(p)?;
                    frontend::password_message(pw.as_bytes(), &mut self.wbuf)?;
                    self.flush().await?;
                }
                Message::AuthenticationMd5Password(body) => {
                    let pw = self.require_password(p)?;
                    let hash = authentication::md5_hash(p.user.as_bytes(), pw.as_bytes(), body.salt());
                    frontend::password_message(hash.as_bytes(), &mut self.wbuf)?;
                    self.flush().await?;
                }
                Message::AuthenticationSasl(body) => {
                    let pw = self.require_password(p)?;
                    let mut has_scram = false;
                    let mut mechs = body.mechanisms();
                    while let Some(m) = mechs.next()? {
                        if m == sasl::SCRAM_SHA_256 {
                            has_scram = true;
                        }
                    }
                    if !has_scram {
                        bail!("server requested an unsupported SASL mechanism");
                    }
                    let mut scram = sasl::ScramSha256::new(pw.as_bytes(), sasl::ChannelBinding::unrequested());
                    frontend::sasl_initial_response(sasl::SCRAM_SHA_256, scram.message(), &mut self.wbuf)?;
                    self.flush().await?;
                    match self.read_raw().await? {
                        Message::AuthenticationSaslContinue(b) => scram.update(b.data())?,
                        Message::ErrorResponse(e) => return Err(DbError::from_fields(e.fields()).into()),
                        _ => bail!("unexpected message during SASL authentication"),
                    }
                    frontend::sasl_response(scram.message(), &mut self.wbuf)?;
                    self.flush().await?;
                    match self.read_raw().await? {
                        Message::AuthenticationSaslFinal(b) => scram.finish(b.data())?,
                        Message::ErrorResponse(e) => return Err(DbError::from_fields(e.fields()).into()),
                        _ => bail!("unexpected message during SASL authentication"),
                    }
                }
                Message::AuthenticationKerberosV5 | Message::AuthenticationScmCredential | Message::AuthenticationGss | Message::AuthenticationSspi => {
                    bail!("unsupported authentication method requested by server")
                }
                Message::BackendKeyData(k) => {
                    self.cancel.pid = k.process_id();
                    self.cancel.secret = k.secret_key();
                }
                Message::ParameterStatus(ps) => {
                    self.params.insert(ps.name()?.to_string(), ps.value()?.to_string());
                }
                Message::NoticeResponse(n) => self.push_notice(DbError::from_fields(n.fields())),
                Message::ReadyForQuery(r) => {
                    self.set_status(r.status());
                    return Ok(());
                }
                Message::ErrorResponse(e) => return Err(DbError::from_fields(e.fields()).into()),
                _ => bail!("unexpected message during startup"),
            }
        }
    }

    fn require_password(&self, p: &ConnectParams) -> Result<String> {
        p.password.clone().ok_or_else(|| anyhow!("password authentication failed: no password supplied"))
    }

    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    pub fn backend_pid(&self) -> i32 {
        self.cancel.pid
    }

    pub fn server_version(&self) -> String {
        self.params.get("server_version").cloned().unwrap_or_default()
    }

    pub fn portal_open(&self) -> bool {
        self.portal_open
    }

    fn set_status(&mut self, s: u8) {
        self.tx_status = match s {
            b'T' => TxStatus::InTransaction,
            b'E' => TxStatus::Failed,
            _ => TxStatus::Idle,
        }
    }

    fn push_notice(&mut self, e: DbError) {
        self.notices.push(Notice { severity: e.severity, message: e.message, detail: e.detail, hint: e.hint });
    }

    async fn flush(&mut self) -> Result<()> {
        self.stream.write_all(&self.wbuf).await?;
        self.stream.flush().await?;
        self.wbuf.clear();
        Ok(())
    }

    async fn read_raw(&mut self) -> Result<Message> {
        loop {
            if let Some(m) = Message::parse(&mut self.rbuf)? {
                return Ok(m);
            }
            if self.rbuf.capacity() - self.rbuf.len() < 8192 {
                self.rbuf.reserve(64 * 1024);
            }
            let n = self.stream.read_buf(&mut self.rbuf).await?;
            if n == 0 {
                bail!("server closed the connection unexpectedly");
            }
        }
    }

    /// Reads the next message, handling asynchronous messages transparently.
    async fn read(&mut self) -> Result<Message> {
        loop {
            match self.read_raw().await? {
                Message::NoticeResponse(n) => self.push_notice(DbError::from_fields(n.fields())),
                Message::ParameterStatus(ps) => {
                    self.params.insert(ps.name()?.to_string(), ps.value()?.to_string());
                }
                Message::NotificationResponse(n) => {
                    self.notifications.push(Notification { pid: n.process_id(), channel: n.channel()?.to_string(), payload: n.message()?.to_string() })
                }
                m => return Ok(m),
            }
        }
    }

    async fn read_until_ready(&mut self) -> Result<()> {
        loop {
            if let Message::ReadyForQuery(r) = self.read().await? {
                self.set_status(r.status());
                return Ok(());
            }
        }
    }

    /// Runs one or more statements through the simple query protocol and
    /// collects every result set. Intended for catalog / housekeeping queries.
    pub async fn simple_query(&mut self, sql: &str) -> Result<Vec<ResultSet>> {
        self.close_portal().await?;
        frontend::query(sql, &mut self.wbuf)?;
        self.flush().await?;
        let mut results = Vec::new();
        let mut current: Option<ResultSet> = None;
        let mut error: Option<DbError> = None;
        loop {
            match self.read().await? {
                Message::RowDescription(rd) => current = Some(ResultSet::new(columns_from(&rd)?)),
                Message::DataRow(row) => {
                    if let Some(rs) = current.as_mut() {
                        rs.push_row(&row)?;
                    }
                }
                Message::CommandComplete(cc) => {
                    let mut rs = current.take().unwrap_or_else(|| ResultSet::new(Vec::new()));
                    rs.tag = cc.tag()?.to_string();
                    results.push(rs);
                }
                Message::EmptyQueryResponse => {}
                Message::ErrorResponse(e) => error = Some(DbError::from_fields(e.fields())),
                Message::ReadyForQuery(r) => {
                    self.set_status(r.status());
                    break;
                }
                Message::CopyInResponse(_) => {
                    frontend::copy_fail("COPY FROM STDIN is not supported here", &mut self.wbuf)?;
                    self.flush().await?;
                }
                _ => {}
            }
        }
        match error {
            Some(e) => Err(e.into()),
            None => Ok(results),
        }
    }

    /// Runs a parameterised statement (text parameters) to completion.
    pub async fn query_params(&mut self, sql: &str, params: &[Option<String>]) -> Result<ResultSet> {
        let (cols, mut rs) = self.start(sql, params, 0).await?;
        let mut out = ResultSet::new(cols.unwrap_or_default());
        match rs {
            Ok((ref mut chunk, outcome)) => {
                out.append(std::mem::take(chunk));
                if let StepOutcome::Done { tag } = outcome {
                    out.tag = tag;
                }
                Ok(out)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Starts executing a single statement via the extended protocol and
    /// fetches up to `max_rows` rows (0 = all). If the result is larger the
    /// portal stays open and [`PgConn::fetch`] continues it.
    ///
    /// Returns the column description (None for statements that return no
    /// rows) and either the first chunk or a server error.
    #[allow(clippy::type_complexity)]
    pub async fn start(
        &mut self,
        sql: &str,
        params: &[Option<String>],
        max_rows: usize,
    ) -> Result<(Option<Vec<Column>>, std::result::Result<(ResultSet, StepOutcome), DbError>)> {
        self.close_portal().await?;
        frontend::parse("", sql, std::iter::empty(), &mut self.wbuf)?;
        frontend::bind(
            "",
            "",
            std::iter::empty(),
            params.iter(),
            |v, buf| match v {
                Some(s) => {
                    buf.extend_from_slice(s.as_bytes());
                    Ok(postgres_protocol::IsNull::No)
                }
                None => Ok(postgres_protocol::IsNull::Yes),
            },
            std::iter::empty(),
            &mut self.wbuf,
        )
        .map_err(|_| anyhow!("could not encode statement parameters"))?;
        frontend::describe(b'P', "", &mut self.wbuf)?;
        frontend::execute("", max_rows.min(i32::MAX as usize) as i32, &mut self.wbuf)?;
        frontend::flush(&mut self.wbuf);
        self.flush().await?;

        let mut columns = None;
        loop {
            match self.read().await? {
                Message::ParseComplete | Message::BindComplete => {}
                Message::RowDescription(rd) => {
                    columns = Some(columns_from(&rd)?);
                    break;
                }
                Message::NoData => break,
                Message::ErrorResponse(e) => {
                    let err = DbError::from_fields(e.fields());
                    self.sync().await?;
                    return Ok((columns, Err(err)));
                }
                _ => {}
            }
        }
        let ncols = columns.as_ref().map(|c| c.len()).unwrap_or(0);
        let r = self.read_rows(ncols).await?;
        Ok((columns, r))
    }

    /// Fetches up to `max_rows` more rows (0 = all) from the open portal.
    pub async fn fetch(&mut self, ncols: usize, max_rows: usize) -> Result<std::result::Result<(ResultSet, StepOutcome), DbError>> {
        if !self.portal_open {
            bail!("no open cursor");
        }
        frontend::execute("", max_rows.min(i32::MAX as usize) as i32, &mut self.wbuf)?;
        frontend::flush(&mut self.wbuf);
        self.flush().await?;
        self.read_rows(ncols).await
    }

    async fn read_rows(&mut self, ncols: usize) -> Result<std::result::Result<(ResultSet, StepOutcome), DbError>> {
        let mut rs = ResultSet::with_ncols(ncols);
        loop {
            match self.read().await? {
                Message::DataRow(row) => rs.push_row(&row)?,
                Message::CommandComplete(cc) => {
                    let tag = cc.tag()?.to_string();
                    self.portal_open = false;
                    self.sync().await?;
                    return Ok(Ok((rs, StepOutcome::Done { tag })));
                }
                Message::EmptyQueryResponse => {
                    self.portal_open = false;
                    self.sync().await?;
                    return Ok(Ok((rs, StepOutcome::Empty)));
                }
                Message::PortalSuspended => {
                    self.portal_open = true;
                    // Flush leaves the implicit transaction open until Sync.
                    self.tx_status = TxStatus::InTransaction;
                    return Ok(Ok((rs, StepOutcome::Suspended)));
                }
                Message::ErrorResponse(e) => {
                    let err = DbError::from_fields(e.fields());
                    self.portal_open = false;
                    self.sync().await?;
                    return Ok(Err(err));
                }
                Message::CopyInResponse(_) => {
                    frontend::copy_fail("COPY FROM STDIN is not supported in the query editor; use the import tool", &mut self.wbuf)?;
                    self.flush().await?;
                }
                Message::CopyOutResponse(_) => {
                    // Present COPY TO STDOUT output as a single text column.
                    rs = ResultSet::with_ncols(1);
                    rs.columns = std::sync::Arc::new(vec![Column::text("copy")]);
                    loop {
                        match self.read().await? {
                            Message::CopyData(d) => {
                                let line = String::from_utf8_lossy(d.data());
                                rs.push_text_row(&[Some(line.trim_end_matches('\n'))]);
                            }
                            Message::CopyDone => break,
                            Message::ErrorResponse(e) => {
                                let err = DbError::from_fields(e.fields());
                                self.sync().await?;
                                return Ok(Err(err));
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    async fn sync(&mut self) -> Result<()> {
        frontend::sync(&mut self.wbuf);
        self.flush().await?;
        self.read_until_ready().await
    }

    /// Closes the open portal (if any), ending the implicit transaction.
    pub async fn close_portal(&mut self) -> Result<()> {
        if !self.portal_open {
            return Ok(());
        }
        self.portal_open = false;
        frontend::close(b'P', "", &mut self.wbuf)?;
        self.sync().await
    }

    /// Streams `reader` into `COPY ... FROM STDIN`. `progress` receives the
    /// number of bytes sent so far. Returns the command tag (`COPY 1234`).
    pub async fn copy_in<R: tokio::io::AsyncRead + Unpin>(
        &mut self,
        sql: &str,
        mut reader: R,
        progress: &std::sync::atomic::AtomicU64,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<String> {
        self.close_portal().await?;
        frontend::query(sql, &mut self.wbuf)?;
        self.flush().await?;
        loop {
            match self.read().await? {
                Message::CopyInResponse(_) => break,
                Message::ErrorResponse(e) => {
                    let err = DbError::from_fields(e.fields());
                    self.read_until_ready().await?;
                    return Err(err.into());
                }
                Message::ReadyForQuery(r) => {
                    self.set_status(r.status());
                    bail!("statement is not a COPY FROM STDIN");
                }
                _ => {}
            }
        }
        let mut buf = vec![0u8; 256 * 1024];
        let mut sent = 0u64;
        let mut failed: Option<String> = None;
        loop {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                failed = Some("import cancelled".into());
                break;
            }
            let n = match reader.read(&mut buf).await {
                Ok(n) => n,
                Err(e) => {
                    failed = Some(format!("could not read file: {e}"));
                    break;
                }
            };
            if n == 0 {
                break;
            }
            // CopyData message: 'd' + len + payload
            self.wbuf.extend_from_slice(b"d");
            self.wbuf.extend_from_slice(&((n as i32 + 4).to_be_bytes()));
            self.wbuf.extend_from_slice(&buf[..n]);
            self.flush().await?;
            sent += n as u64;
            progress.store(sent, std::sync::atomic::Ordering::Relaxed);
        }
        match &failed {
            Some(msg) => frontend::copy_fail(msg, &mut self.wbuf)?,
            None => frontend::copy_done(&mut self.wbuf),
        }
        self.flush().await?;
        let mut tag = String::new();
        let mut error = None;
        loop {
            match self.read().await? {
                Message::CommandComplete(cc) => tag = cc.tag()?.to_string(),
                Message::ErrorResponse(e) => error = Some(DbError::from_fields(e.fields())),
                Message::ReadyForQuery(r) => {
                    self.set_status(r.status());
                    break;
                }
                _ => {}
            }
        }
        if let Some(e) = error {
            return Err(e.into());
        }
        if let Some(msg) = failed {
            bail!(msg);
        }
        Ok(tag)
    }

    pub async fn terminate(mut self) {
        frontend::terminate(&mut self.wbuf);
        let _ = self.flush().await;
    }
}

fn columns_from(rd: &postgres_protocol::message::backend::RowDescriptionBody) -> Result<Vec<Column>> {
    let mut cols = Vec::new();
    let mut fields = rd.fields();
    while let Some(f) = fields.next()? {
        cols.push(Column { name: f.name().to_string(), type_oid: f.type_oid(), table_oid: f.table_oid(), column_id: f.column_id() });
    }
    Ok(cols)
}

/// Tests against a real server. They run only when `TRUNQ_TEST_DATABASE_URL`
/// is set (CI runs them against several PostgreSQL versions), e.g.
/// `TRUNQ_TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres cargo test`.
#[cfg(test)]
mod live_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64};

    fn params() -> Option<ConnectParams> {
        let url = std::env::var("TRUNQ_TEST_DATABASE_URL").ok()?;
        let (p, pw) = crate::config::Profile::from_url(&url)?;
        let mut cp = p.connect_params("", pw.as_deref()).ok()?;
        cp.password = pw;
        cp.ssl_mode = SslMode::Prefer;
        Some(cp)
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    #[test]
    fn simple_and_extended_queries() {
        let Some(p) = params() else { return };
        rt().block_on(async {
            let mut c = PgConn::connect(&p).await.expect("connect");
            let rs = c.simple_query("SELECT 1 AS a, NULL::text AS b, 'héllo'::text AS c; SELECT 2").await.unwrap();
            assert_eq!(rs.len(), 2);
            assert_eq!(rs[0].get(0, 0), Some("1"));
            assert_eq!(rs[0].get(0, 1), None);
            assert_eq!(rs[0].get(0, 2), Some("héllo"));
            let r = c.query_params("SELECT $1::int + 1, $2::text IS NULL", &[Some("41".into()), None]).await.unwrap();
            assert_eq!(r.get(0, 0), Some("42"));
            assert_eq!(r.get(0, 1), Some("t"));
            // errors carry position and leave the connection usable
            let err = c.simple_query("SELECT * FROM does_not_exist").await.unwrap_err();
            let db = err.downcast_ref::<DbError>().expect("db error");
            assert_eq!(db.code, "42P01");
            assert!(db.position.is_some());
            assert_eq!(c.query_params("SELECT 1", &[]).await.unwrap().get(0, 0), Some("1"));
        });
    }

    #[test]
    fn portal_paging() {
        let Some(p) = params() else { return };
        rt().block_on(async {
            let mut c = PgConn::connect(&p).await.unwrap();
            let (cols, r) = c.start("SELECT g FROM generate_series(1, 2500) g", &[], 1000).await.unwrap();
            assert_eq!(cols.unwrap().len(), 1);
            let (rs, outcome) = r.unwrap();
            assert_eq!(rs.nrows(), 1000);
            assert!(matches!(outcome, StepOutcome::Suspended));
            let (rs2, _) = c.fetch(1, 1000).await.unwrap().unwrap();
            assert_eq!(rs2.get(0, 0), Some("1001"));
            let (rs3, outcome) = c.fetch(1, 0).await.unwrap().unwrap();
            assert_eq!(rs3.nrows(), 500);
            assert!(matches!(outcome, StepOutcome::Done { .. }));
            assert_eq!(c.tx_status, TxStatus::Idle);
        });
    }

    #[test]
    fn portal_close_and_errors_preserve_transactions() {
        let Some(p) = params() else { return };
        rt().block_on(async {
            let mut c = PgConn::connect(&p).await.unwrap();
            c.start("SELECT g FROM generate_series(1, 4) g", &[], 1).await.unwrap().1.unwrap();
            assert_eq!(c.tx_status, TxStatus::InTransaction);
            c.close_portal().await.unwrap();
            assert_eq!(c.tx_status, TxStatus::Idle);
            c.simple_query("BEGIN").await.unwrap();
            c.start("SELECT g FROM generate_series(1, 4) g", &[], 1).await.unwrap().1.unwrap();
            c.close_portal().await.unwrap();
            assert_eq!(c.tx_status, TxStatus::InTransaction);
            assert!(c.query_params("SELECT 1/0", &[]).await.is_err());
            assert_eq!(c.tx_status, TxStatus::Failed);
            c.simple_query("ROLLBACK").await.unwrap();
            assert_eq!(c.tx_status, TxStatus::Idle);
            assert_eq!(c.query_params("SELECT 42", &[]).await.unwrap().get(0, 0), Some("42"));
        });
    }

    #[test]
    fn copy_output_and_import_error_recovery() {
        let Some(p) = params() else { return };
        rt().block_on(async {
            let mut c = PgConn::connect(&p).await.unwrap();
            let rs = c.query_params("COPY (SELECT 'héllo'::text) TO STDOUT", &[]).await.unwrap();
            assert_eq!(rs.ncols(), 1);
            assert_eq!(rs.columns[0].name, "copy");
            assert_eq!(rs.get(0, 0), Some("héllo"));
            c.simple_query("CREATE TEMP TABLE bad_copy (v int)").await.unwrap();
            let bad: &[u8] = b"not_an_integer\n";
            assert!(c.copy_in("COPY bad_copy FROM STDIN", bad, &AtomicU64::new(0), &AtomicBool::new(false)).await.is_err());
            assert_eq!(c.query_params("SELECT count(*) FROM bad_copy", &[]).await.unwrap().get(0, 0), Some("0"));
        });
    }

    #[test]
    fn catalog_and_table_details() {
        let Some(p) = params() else { return };
        rt().block_on(async {
            let mut c = PgConn::connect(&p).await.unwrap();
            c.simple_query("BEGIN; CREATE SCHEMA trunq_test; CREATE TABLE trunq_test.parent(id int PRIMARY KEY); CREATE TABLE trunq_test.child(id int PRIMARY KEY, parent_id int REFERENCES trunq_test.parent(id), v int CHECK(v > 0)); ALTER TABLE trunq_test.child ENABLE ROW LEVEL SECURITY; CREATE POLICY reader ON trunq_test.child FOR SELECT USING (true)").await.unwrap();
            let sets = c.simple_query(&crate::db::meta::catalog_sql(false)).await.unwrap();
            let cat = crate::db::meta::parse_catalog(sets);
            let rel = cat.find_relation(Some("trunq_test"), "child").expect("child metadata");
            assert!(rel.rls);
            assert_eq!(rel.fks.len(), 1);
            assert!(!rel.indexes.is_empty());
            assert!(!rel.constraints.is_empty());
            assert_eq!(rel.policies[0].name, "reader");
            let details = c.simple_query(&crate::db::meta::table_details_sql(rel.oid)).await.unwrap();
            assert!(crate::db::meta::build_table_ddl(&details).contains("CREATE TABLE"));
            let roles = c.simple_query(crate::db::meta::SERVER_OBJECTS_SQL).await.unwrap();
            assert!(crate::db::meta::parse_roles(&roles[0]).iter().any(|r| r.is_current));
            c.simple_query("ROLLBACK").await.unwrap();
        });
    }

    /// `cargo test --release bench_fetch_million -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_fetch_million() {
        let Some(p) = params() else { return };
        let sql = std::env::var("TRUNQ_BENCH_SQL").unwrap_or_else(|_| "SELECT * FROM analytics.events LIMIT 1000000".into());
        rt().block_on(async {
            let mut c = PgConn::connect(&p).await.unwrap();
            let t0 = std::time::Instant::now();
            let (cols, first) = c.start(&sql, &[], 50_000).await.unwrap();
            let ncols = cols.map_or(0, |c| c.len());
            let (mut all, mut outcome) = first.unwrap();
            while matches!(outcome, StepOutcome::Suspended) {
                let (rs, o) = c.fetch(ncols, 50_000).await.unwrap().unwrap();
                all.append(rs);
                outcome = o;
            }
            let secs = t0.elapsed().as_secs_f64();
            println!(
                "TRUNQ_BENCH fetch rows={} cols={} seconds={:.3} rows_per_s={:.0} bytes_in_memory={}",
                all.nrows(),
                ncols,
                secs,
                all.nrows() as f64 / secs,
                all.byte_size()
            );
        });
    }

    #[test]
    fn copy_in_and_cancel() {
        let Some(p) = params() else { return };
        rt().block_on(async {
            let mut c = PgConn::connect(&p).await.unwrap();
            c.simple_query("CREATE TEMP TABLE t (a int, b text)").await.unwrap();
            let data: &[u8] = b"1,one\n2,\"t,wo\"\n";
            let progress = AtomicU64::new(0);
            let tag = c.copy_in("COPY t FROM STDIN WITH (FORMAT csv)", data, &progress, &AtomicBool::new(false)).await.unwrap();
            assert_eq!(tag, "COPY 2");
            let rs = c.query_params("SELECT b FROM t ORDER BY a", &[]).await.unwrap();
            assert_eq!(rs.get(1, 0), Some("t,wo"));

            let token = c.cancel_token();
            let canceller = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                token.cancel().await.unwrap();
            });
            let err = c.simple_query("SELECT pg_sleep(10)").await.unwrap_err();
            assert_eq!(err.downcast_ref::<DbError>().unwrap().code, "57014");
            canceller.await.unwrap();
        });
    }
}
