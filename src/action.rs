//! Requests from UI components to the application (processed after a frame).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

#[derive(Clone)]
pub enum Action {
    NewQuery {
        profile: u64,
        db: String,
        sql: String,
        run: bool,
    },
    OpenTable {
        profile: u64,
        db: String,
        schema: String,
        name: String,
    },
    /// Open table data with a WHERE filter preset (foreign-key navigation).
    OpenTableFiltered {
        profile: u64,
        db: String,
        schema: String,
        name: String,
        filter: String,
    },
    OpenStructure {
        profile: u64,
        db: String,
        oid: u32,
        title: String,
        /// Open on the DDL section instead of table health.
        ddl: bool,
    },
    OpenDashboard {
        profile: u64,
    },
    OpenHealth {
        profile: u64,
        db: String,
    },
    /// Entity relationship diagram of a database, a schema, or the
    /// neighbourhood of one table (oid, name).
    OpenErd {
        profile: u64,
        db: String,
        schema: Option<String>,
        focus: Option<(u32, String)>,
    },
    RefreshCatalog {
        profile: u64,
        db: String,
    },
    EditProfile(u64),
    NewProfile,
    Toast(ToastKind, String),
    /// Remember a password typed by the user for this app session.
    SetPassword(u64, String),
    ShowHistory,
    SaveTablePrefs(String, crate::config::TablePrefs),
    SaveSnippet {
        name: String,
        sql: String,
    },
    /// Fetch the DDL of an object and copy it to the clipboard.
    CopyDdl {
        profile: u64,
        db: String,
        sql: String,
        label: String,
    },
    /// Open the CSV import dialog (file picker if path is None).
    ImportCsv {
        profile: Option<u64>,
        db: Option<String>,
        schema: Option<String>,
        path: Option<std::path::PathBuf>,
    },
}
