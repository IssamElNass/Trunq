# Changelog

All notable changes to Trunq are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed
- Correct the SCRAM channel-binding flag (`n` instead of `y`) when authenticating over TLS, including Azure PostgreSQL connections.
- Complete every paged fetch so repeated scrolling, Load more and Load all do not leave query tabs stuck busy. Closing a cursor keeps loaded rows and releases its implicit transaction while preserving manual transactions.
- Keep environment badges visible in the sidebar, query tabs and connection selector, including when multiple profiles share a database name. Production stays red independently of the custom profile colour; unlabelled profiles show UNSET.
- Truncate long connection, object and menu names with full text on hover, reserve status-bar controls, and wrap environment choices in the connection dialog.
- Keep the results footer steady by showing live render timing in the row-count tooltip instead of the wrapping control row.

### Changed
- Redesign Grants & access around grouped object/recipient permissions, a plain-language detail pane and explicit direct, inherited, PUBLIC and non-inherited membership labels. Use a neutral outlined selection with an accent marker, compact filters and readable membership options; keep SQL actions scoped to one selected grant.

## [0.1.0] - 2026-10-27

First public release.

### Added
- Native PostgreSQL wire protocol client: TLS, SCRAM-SHA-256, MD5, Unix sockets, `~/.pgpass`,
  cancel requests, notices, LISTEN/NOTIFY, COPY in both directions.
- SSH tunnels through bastion hosts (agent, key file, password) with one shared SSH session per server
  and known-hosts verification.
- Microsoft Entra ID sign-in for Azure Database for PostgreSQL (browser, device code, Azure CLI)
  across Azure global, Azure China (21Vianet) and Azure US Government, with automatic token refresh.
- Object browser: databases, schemas, tables (with columns, keys, foreign keys), views,
  materialized views, sequences, functions and types.
- SQL editor: highlighting, context-aware autocompletion, formatter, comment toggle, error underlining,
  Smart Run, query templates, result naming (`-- name:`), auto-saved scratchpad tabs, history, `.sql` files.
- Query parameters (`:name`) sent as bound parameters, and a saved snippet library.
- Virtualised results grid with server-side cursor paging, natural sort, local search with highlighting,
  click-to-filter chips, column show / hide, hide empty columns, column finder, instant aggregates,
  timestamp switch (raw / local / UTC) and a query / fetch / render timing breakdown.
- Pinned results with a side-by-side view, compare rows, row card view, JSON tree viewer with copy path,
  foreign key peek and navigation, copy as TSV, CSV, JSON, Markdown, SQL INSERT / UPDATE / WHERE,
  TypeScript or an IN list.
- Export of visible rows, loaded rows or the entire result (streamed from the server) to CSV, JSON,
  Markdown, SQL or TypeScript.
- Table data editor with inline edits, inserts, duplicate row, deletes and SQL preview, saved in one
  transaction, and per-table remembered layout.
- One-click charts (bars and lines) from any result.
- CSV / TSV import by drag and drop with type inference, streamed through `COPY FROM STDIN`.
- EXPLAIN / EXPLAIN ANALYZE plan view.
- Lock detective, live progress of long maintenance commands, dry run for DML, migration safety
  linter with quick fixes, forgotten transaction guard.
- Database health check, per-table health and structure view with DDL, active sessions view.
- Connection identity with environment labels (local, dev, test, staging, production).
- Command palette with tables, functions, connections, snippets, recent queries and commands.
- Light and dark themes, workspace restore.
- Leak-proof connection management with a live connections panel.
- Grants inspector with catalog ACLs, inherited role traces, default privileges, PUBLIC / grant-option filters, CSV copying and reviewable permission SQL.
- pg_cron inspector with schedules, SQL editing, recent runs and reviewable pause / enable / update statements; read-only connections disable edits.
- Standalone Windows EXE release assets and direct platform download links in stable and nightly notes.
- Production deployment/upgrade guide, dated security findings, nine new native screenshots and automated Markdown link/image checks.
- Weekly and dependency-change RustSec audit with retained JSON reports.
- Strict pinned-host SSH option; native SSH now requires Ed25519/ECDSA after removing the unresolved RSA timing advisory dependency.
- Searchable extension manager for all server-provided extensions, with version/schema selection, opt-in dependencies, direct enable/update, read-only guards and automatic catalog refresh.
- Cloud-neutral token commands with Azure CLI, AWS RDS and Google Cloud presets; native Azure service-principal secrets from environment variables.
- Per-profile PEM CA bundles and TLS client certificates with PKCS #8 keys.
- Database tools for extension inventory/update SQL, pg_cron job templates, audit/security inspection, foreign data and replication; tree groups for extensions, event triggers and publications.
- Cloud-authentication and administration guides with provider prerequisites, audit-log setup and explicit feature gaps.
- Daily default-branch nightly prereleases with macOS, Windows and Linux packages, checksums, source provenance and manual triggering; packaging changes are built on pull requests without publication.
- Connection/TLS/password reference, settings and workspace backup guide, practical query and diagnostic workflows, and expanded troubleshooting.
- Task-based documentation index, separate installation, usage, feature and developer guides, and a shorter project README.
- Expanded handbook with a first-query walkthrough, SSH setup, bound parameters, export scopes and reading progress.
- ER diagram entry points in database, schema and table menus and the command palette.
- Browser groups for roles, tablespaces, indexes, constraints, triggers, RLS and policies.
- Editable `pg_stat_statements` slow-query template and PostgreSQL research notes.
- Multi-stage Docker images (native and noVNC desktop), Compose workspace volume and Docker Hub workflow with a desktop smoke test.
- Screenshot PNG encoding runs off the GPU/UI thread and scripts wait for completed files.
- Isolated `TRUNQ_CONFIG_DIR`, reproducible screenshot script, new screenshots and performance checks.
- PostgreSQL 18 in the protocol test matrix.

### Fixed
- Rejected token-command/Entra credentials no longer produce an unrelated static-password prompt.
- Extension inventory remains usable for restricted roles when pg_cron hides its scheduler setting.
- Typed, in-memory and saved credentials are bound to the connection target; endpoint edits cannot forward an older password. Older database/SSH keychain entries require one-time re-entry.
- SSH host-key checks reject algorithm changes, revoked keys and unreadable known-host files; idle tunnels are released after the final channel closes.
- Config/history writes use private exclusive staging and Unix no-follow append protection; insecure Unix pgpass permissions are rejected.
- Duplicate SQL now triggers history compaction; startup retains at most 5,000 history entries.
- Oversized PostgreSQL frames and retained notice/notification queues have explicit resource bounds and truncation feedback.
- Updated crypto-bigint/ctutils timing fixes and replaced the yanked clipboard-win version.
- Chart frames reuse sorted/parsed data and pixel-decimated geometry; timestamp offsets, finite-number handling and extrema order are corrected.
- Azure interactive token caches distinguish configured users; CLI authentication respects account changes and has bounded execution.
- TLS negotiation and local sign-in redirect reads cannot wait indefinitely; malformed Unicode OAuth percent escapes do not panic.
- URL import clears a previous endpoint's password when the identity changes.
- Sidebar search includes schema types, type-only public schemas remain visible, and publication operations include TRUNCATE.
- Failed or cancelled exports preserve an existing destination; successful exports replace it after staging in the same directory.
- CSV exports distinguish empty strings from NULL, and SQL exports quote non-finite numeric values.
- Failed table saves roll back their changes and recover the connection; saves inside a user transaction use savepoints.
- Dry runs close a previous paged cursor before choosing a transaction or savepoint, preserve bound parameters, and roll back in the session actor before reporting completion.
- Destructive-query warnings inspect the modified query level and modifying CTEs, including WHERE clauses nested in subqueries.
- CSV previews handle quoted empty fields, quoted delimiters, UTF-8 BOMs and unique normalized column names; timestamp inference validates both timezone offset directions.
- Run Whole Script includes the full editor even with a selection; the confirmation dialog preserves bound parameters for dry runs.
- Error highlighting tolerates offsets inside Unicode characters.
- SQL highlighting refreshes glyphs after font-atlas changes instead of reusing stale meshes that display unrelated characters; syntax caches discard earlier edits.
- Comment toggling handles mixed Unicode indentation without slicing inside a character.
- Mouse and keyboard completion share parenthesis handling, report text changes and respect read-only editors.
- Editor commands and text replacements dismiss outdated completions; stale completion ranges cannot crash the editor.
- Built-in SQL functions are suggested before a connection catalog has loaded.
- Documentation navigation preserves the scroll destination, search can be closed with keyboard or touch, and theme switching works when browser storage is unavailable.
- Website code copying preserves the displayed text and selects it when clipboard access is unavailable; build tabs support arrow keys, Home and End.
- Trackpad scrolling no longer springs backwards at a loaded page boundary; keyboard navigation restores the correct scroll state, and sticky headers use the same painted offset as the body.
- Empty search results no longer trigger a full row scan on every frame.
- Result replacement and edited values correctly invalidate search, sorting, hidden empty columns and aggregate caches.
- Numeric sort preserves large bigint/numeric values and consistently orders NaN.
- Selection aggregates are cached and include edited values; non-finite numbers do not poison their totals.
- Out-of-range column access cannot read a value from the next row.
- Suspended portals report their open transaction; COPY output supplies result-column metadata.
- Bracketed IPv6 URLs with ports, encoded socket paths and malformed Unicode percent escapes.
- Data viewer sizing on compact windows; scripted runs no longer autosave over a workspace.
- Removed unused UI tokens/helpers and wired unfinished features to eliminate compiler warnings.
- macOS close uses eframe’s standalone event-loop exit path to avoid the Touch Bar observer teardown crash.
- Correct minimum Rust version to 1.95 to match current GUI dependencies.
