# Changelog

All notable changes to Trunq are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- ER diagram entry points in database, schema and table menus and the command palette.
- Browser groups for roles, tablespaces, indexes, constraints, triggers, RLS and policies.
- Editable `pg_stat_statements` slow-query template and PostgreSQL research notes.
- Multi-stage Docker images (native and noVNC desktop), Compose workspace volume and Docker Hub workflow with a desktop smoke test.
- Screenshot PNG encoding runs off the GPU/UI thread and scripts wait for completed files.
- Isolated `TRUNQ_CONFIG_DIR`, reproducible screenshot script, new screenshots and performance checks.
- PostgreSQL 18 in the protocol test matrix.

### Fixed
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

## [0.1.0] - 2026-10-22

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
