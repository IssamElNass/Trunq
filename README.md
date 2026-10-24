<p align="center">
  <img src="assets/app_icon.png" width="128" alt="Trunq logo">
</p>

<h1 align="center">Trunq</h1>

<p align="center"><b>The fast, native PostgreSQL client that keeps you out of incidents.</b><br>
Written in Rust, with a native GPU-rendered interface, paged results, blocking-session diagnostics
and warnings before risky schema changes.</p>

<p align="center">
  <a href="../../actions/workflows/ci.yml"><img src="../../actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="../../releases/latest"><img src="https://img.shields.io/badge/download-latest-1CB8F2" alt="Download"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-0A6CFF" alt="MIT license"></a>
  <img src="https://img.shields.io/badge/PostgreSQL-13%2B-336791" alt="PostgreSQL 13+">
  <img src="https://img.shields.io/badge/macOS%20%C2%B7%20Windows%20%C2%B7%20Linux-supported-3DD68C" alt="Platforms">
</p>

<p align="center"><img src="docs/screenshots/query.png" alt="Trunq query editor and results" width="900"></p>

---

## Contents

- [Why Trunq](#why-trunq)
- [Download](#download)
- [Features](#features)
- [Fast by design](#fast-by-design)
- [Docker and Docker Hub](#docker-and-docker-hub)
- [Benchmarks and PostgreSQL research](#benchmarks-and-postgresql-research)
- [Install from source](#install-from-source)
- [First steps](#first-steps)
- [SSH tunnels](#ssh-tunnels)
- [Azure Database for PostgreSQL (Microsoft Entra ID)](#azure-database-for-postgresql-microsoft-entra-id)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Where Trunq stores data](#where-trunq-stores-data)
- [Troubleshooting](#troubleshooting)
- [Development and releases](#development-and-releases)

---

## Why Trunq

Trunq combines paged results, a native GPU-rendered interface and PostgreSQL diagnostics in one client. It fetches rows on demand and keeps database work on background tasks.

Performance depends on the query, server, network and hardware. See the [reproducible performance checks](docs/performance.md) for measured workloads and their limits.

## Download

Grab the latest build from the [**Releases page**](../../releases/latest):

| Platform | File |
|---|---|
| macOS 11+ (Apple Silicon and Intel) | `Trunq-<version>-macos-universal.dmg`: open it and drag **Trunq** to Applications |
| Windows 10/11 (x64) | `Trunq-<version>-windows-x86_64.zip`: unzip and run `Trunq.exe` |
| Debian / Ubuntu (x64) | `trunq_<version>_amd64.deb`: `sudo apt install ./trunq_<version>_amd64.deb` |
| Other Linux (x64) | `trunq-<version>-linux-x86_64.tar.gz`: unpack and run `./trunq` |

Every release lists SHA-256 checksums in `SHA256SUMS.txt`. Prefer to build it yourself? See [Install from source](#install-from-source).

## Features

### A SQL editor that understands PostgreSQL

- **Smart Run** (⌘⏎): runs the selection, or the statement under the cursor when nothing is selected. ⇧⌘⏎ runs the whole script.
- Syntax highlighting, line numbers, a highlight of the statement that will run, and errors underlined at the exact position the server reports.
- **Context-aware autocompletion**: tables, columns (alias aware, `o.` lists the columns of `orders o`), schemas, functions and keywords.
- **Query parameters and snippets**: write `WHERE customer_id = :customer_id AND status = :status` and Trunq asks for the values when you run it. Values are sent as **bound parameters** (`$1`, `$2`), never pasted into the SQL. Save the query as a snippet and run it again next week from the snippet menu or the command palette.
- **Query templates**: one click for SELECT, INSERT, UPDATE, DELETE, COUNT or EXPLAIN ANALYZE from any table.
- **Name your results**: put `-- name: Failed payments` above a statement, or double-click a result tab to rename it.
- **Auto-save scratchpad**: every SQL tab is saved locally a moment after you stop typing and restored on the next start.
- Formatter, comment toggle, indent and outdent, duplicate line, `.sql` files (or drop them on the window), searchable query history.

### Paged, virtualised results

<p><img src="docs/screenshots/pinned.png" alt="Pinned result next to a new result" width="800"></p>

- Results stream through a **server-side cursor**. The first page contains up to 1,000 rows and more load as you scroll, keeping the initial result bounded even for large tables.
- **Virtualised grid**: only visible rows and columns are rendered. Sticky header and row numbers, resizable and auto-fitting columns.
- **Pin query results**: pin a result, run another query and compare the two **side by side**. A new run never overwrites a pinned result.
- **Local search** over the loaded rows with matching cells highlighted, without asking PostgreSQL again.
- **Click to filter**: right-click a value for *Filter by this value*, *Exclude this value*, *Show NULLs*, *Hide NULLs* or *Copy as WHERE*. Filters show as removable chips. In a table view they become a server-side `WHERE`.
- **Column tools**: show and hide columns, **Hide empty columns** (only NULL in the loaded rows), and a **Column finder** (⌘F) that jumps to any column, even in a table with 80 of them.
- **Instant aggregates**: select cells to see COUNT, SUM, AVG, MIN and MAX in the footer.
- **Timestamp switch**: show `timestamptz` values raw, in local time or in UTC.
- **Query timing breakdown**: query time, fetch time and render time are shown separately for every result.
- Local sorting with natural ordering ("file2" before "file10") and precision-preserving numeric comparisons.

### Look closer at your data

<p><img src="docs/screenshots/json-tree.png" alt="JSON tree" width="800"></p>

- **JSON tree**: open any JSON or JSONB value as an interactive tree with search. Right-click a node to copy its path (`$.items[2].sku`), the PostgreSQL accessor (`payload->'items'->2->>'sku'`) or the value. Large documents expand lazily, so multi-megabyte values stay fast.
- Large JSON in the grid shows a compact size badge (`{…} 14 KB`) and a pretty preview on hover, so a big document never breaks the layout.
- **Row card view**: look at one record vertically instead of scrolling sideways through 40 columns. Use ← and → to move between rows.

<p><img src="docs/screenshots/compare.png" alt="Compare rows" width="800"></p>

- **Compare rows**: select two to six records and compare them side by side. Trunq highlights only the values that differ, and *Differences only* hides the rest. Ideal when two customers, configurations or orders are almost identical.
- **Foreign key peek**: hover a foreign key such as `customer_id = 42` and the related record appears in a small popup. **⌘-click** opens it. This works in any query result, not only in table views, because PostgreSQL tells Trunq where every result column comes from.
- **Copy as anything**: copy selected records as TSV, CSV, JSON, Markdown, SQL INSERT, SQL UPDATE (keyed by the primary key), SQL WHERE, a TypeScript array or an `IN (...)` list.

### One-click charts

<p><img src="docs/screenshots/chart.png" alt="Chart of a query result" width="800"></p>

- Turn any result into a bar or line chart. A time or numeric X axis becomes a line, categories become bars.
- A colour-blind-safe palette, hover tooltips, a legend and direct labels. Trunq never mixes very different scales on one axis.

### Edit data safely

<p><img src="docs/screenshots/table.png" alt="Table data editor" width="800"></p>

- Double-click a table to browse it with a `WHERE` and `ORDER BY` bar. Key icons mark the primary key, link icons mark foreign keys.
- Edit cells inline, add rows, set NULLs and delete rows. Changed cells are highlighted.
- **Duplicate row** copies a record and leaves out the primary key and identity or generated columns, so the database assigns fresh values.
- **Preview the generated SQL**, then save everything in one transaction.
- **Remember my table**: per table Trunq remembers hidden columns, column widths, filters, sorting and the grid or card view. Open the same table tomorrow and everything is as you left it.

<p><img src="docs/screenshots/card.png" alt="Row card view" width="800"></p>

### Export exactly this

Export is explicit about what you get, so you never think you exported a full dataset when only the first rows were in memory:

- **Visible rows**: exactly what you see, with search, filters and sorting applied.
- **Loaded rows**: the rows fetched so far, with a note when more rows exist on the server.
- **Entire result from the server**: runs the query again on a separate connection and streams every row straight to the file. Nothing is loaded into memory, and you can cancel at any time.

Formats: CSV, JSON, Markdown, SQL INSERT and TypeScript.

### Safety features other clients do not have

<p><img src="docs/screenshots/lock.png" alt="Lock detective" width="800"></p>

- **Lock detective**: when a query runs longer than 1.5 s Trunq shows what it waits for and *which session blocks it* (user, application, query, transaction age), with one-click *Cancel its query* or *Terminate session*.
- **Live progress** of `CREATE INDEX`, `VACUUM`, `ANALYZE`, `COPY` and `CLUSTER`.
- **Dry run** (⌥⌘⏎): runs an `INSERT`, `UPDATE` or `DELETE` in a transaction, shows exactly which rows would change and always rolls back.

<p><img src="docs/screenshots/lint.png" alt="Migration linter" width="800"></p>

- **Migration linter**: before DDL that would block or rewrite a table (plain `CREATE INDEX`, column type changes, `SET NOT NULL`, foreign keys without `NOT VALID`, volatile defaults, `VACUUM FULL` and more) Trunq explains the risk and the safe alternative, with a one-click *Add CONCURRENTLY*.
- Confirmation for `UPDATE` or `DELETE` without `WHERE`, `DROP` and `TRUNCATE`.
- **Forgotten transaction guard**: a banner appears when a tab holds a transaction, and its locks, for more than a minute.
- **Connection identity**: the status bar always shows the environment, server, user and database. Mark a connection as LOCAL, DEV, TEST, STAGING or PRODUCTION; production is red everywhere, including a strip above the editor.
- Read-only connections and per-connection statement timeouts.

### Health and monitoring

<p><img src="docs/screenshots/health.png" alt="Health check" width="800"></p>

- **Database health check**: finds integer IDs close to overflow, transaction ID wraparound, inactive replication slots filling the disk, foreign keys without an index, invalid, duplicate and unused indexes, bloated or never analyzed tables, long transactions, tables without a primary key and a low cache hit ratio. Every finding comes with fix SQL to review.

<p><img src="docs/screenshots/structure.png" alt="Table health and structure" width="800"></p>

- **Table health and structure**: size of table and indexes, estimated rows, dead tuples, last vacuum and analyze, sequential versus index scans, with tips when something looks wrong. Plus columns, indexes (each with its own DDL), constraints, references in both directions, triggers, partitions, privileges and the full DDL.

<p><img src="docs/screenshots/sessions.png" alt="Active sessions" width="800"></p>

- **Active sessions**: a compact view of active sessions and long-running queries within your privileges, refreshed every two seconds while visible. It shows who is idle in a transaction, waiting on locks or blocked, with cancel and terminate (after a confirmation).
- **EXPLAIN and EXPLAIN ANALYZE** visualiser with per-node time share, loops and row estimate warnings.

### Import CSV by drag and drop

<p><img src="docs/screenshots/import.png" alt="CSV import" width="800"></p>

- Drop a CSV or TSV onto the window. Delimiter, header, column names and **types are inferred** (integer, bigint, numeric, boolean, date, timestamptz, uuid, jsonb).
- Create a new table or append to an existing one. The file is streamed through `COPY FROM STDIN` in a single transaction with live progress, the fastest way to load data into PostgreSQL.

### Navigate everything from the keyboard

<p><img src="docs/screenshots/palette.png" alt="Command palette" width="800"></p>

- **Command palette** (⌘K): one search field for tables, views, functions, connections, saved snippets, recent queries and commands such as *Format SQL* or *Switch this tab to another connection*.
- **Object tree** with databases, schemas, tables (expand to columns with types, keys and foreign keys), views, materialized views, sequences, functions and types, with row estimates and sizes.
- **Quick DDL**: view or copy the DDL of a table, view, function, sequence or index from the context menu.
- Generate SELECT, INSERT, UPDATE, DELETE, EXPLAIN and maintenance commands from the context menu.

### Connect to anything

- TLS (`disable` up to `verify-full`), SCRAM-SHA-256, MD5 and trust, Unix sockets, `~/.pgpass`, or paste a `postgres://` URL.
- **SSH tunnels** through a bastion or jump host, with agent, key file or password authentication.
- **Microsoft Entra ID** for Azure Database for PostgreSQL (Azure global, **Azure China (21Vianet)** and Azure US Government) through the browser, a device code or the Azure CLI, with automatic hourly token refresh.
- Passwords, SSH secrets and refresh tokens live in the operating system keychain, never in files. No telemetry.

### No connection leaks, ever

- Every connection belongs to exactly one tab or browser node and is closed when that goes away. A running statement is cancelled first. Idle query tabs disconnect after 15 minutes and reconnect transparently.
- Monitoring, foreign key peeks and the sessions view reuse the existing browser connection instead of opening new ones. Health checks, exports and imports use a short-lived connection that closes as soon as they finish.
- The status bar shows the live connection count. Click it to see every connection (purpose, PID, age) and close any of them.
- Each connection sets `application_name` (`Trunq: Query 3`, `Trunq: Object browser` and so on) so you can find it in `pg_stat_activity`. Background metadata connections use `lock_timeout` and `statement_timeout` to limit waits during migrations.
- TCP keep-alive detects dead networks. All SSH tunnels of a server share one SSH session. Everything closes cleanly on exit.

## Fast by design

- **Native and GPU rendered** with egui and wgpu. A single binary of about 20 MB, no runtime to start.
- **Text protocol, zero decoding**: Trunq talks to PostgreSQL directly and copies each row from the socket into one compact buffer. A million rows are a few allocations, not millions of strings.
- **Server-side cursors**: only the rows you look at are fetched, and the next page loads while you scroll.
- **Virtualised grid**: only visible rows and columns are laid out. Text layout is cached between frames.
- **Never blocks the UI**: every network call runs on a background runtime. The interface keeps drawing even while a query waits on a lock.
- **Stable scrolling**: hard grid boundaries prevent elastic trackpad scrolling from reversing at page edges, while sticky headers and keyboard navigation keep the same scroll position.
- **Cached search and aggregates**: empty search results stay cached; sorting, filtering, hide-empty columns and selection aggregates track edits and replacement results. Selected-cell aggregates are calculated once per change.
- **Measured**: every result shows its query, fetch and render time, so you can see where the milliseconds go.

## Docker and Docker Hub

```sh
docker compose up --build -d
```

Open [localhost:8080/vnc.html?autoconnect=true&resize=scale](http://localhost:8080/vnc.html?autoconnect=true&resize=scale). The image runs the native client in a browser desktop; the native release avoids desktop-streaming overhead. Your workspace persists in a named volume. The supplied Compose file binds to loopback because the desktop has no built-in authentication.

![Trunq running in Docker against PostgreSQL 18](docs/screenshots/docker.png)

The [Docker guide](docs/docker.md) covers host database connections, the optional native image target, multi-platform builds, and publishing to Docker Hub. Configure `DOCKERHUB_USERNAME` as a repository variable and `DOCKERHUB_TOKEN` as a secret. The [Docker workflow](.github/workflows/docker.yml) smoke-tests the desktop before publishing version tags or an explicit manual publication.

## Benchmarks and PostgreSQL research

[Validation record](docs/validation.md) lists the checks actually run and their limits.

[Performance measurements and commands](docs/performance.md) cover million-row fetching, empty-search caching and startup. They measure specific workloads, not a universal ranking against other clients.

The [PostgreSQL research notes](docs/postgresql-research.md) link to official sources, explain the implemented protocol fixes and slow-query template, and distinguish future candidates such as `pg_stat_io` and serialization timing.

### Explore relationships and database metadata

![ER diagram of the sample shop schema](docs/screenshots/erd.png)

- Right-click a **database**, **schema** or **table** → **ER diagram**, or search for it with **⌘K**. Pan, zoom, move tables, inspect keys and export SVG or Mermaid using the existing diagram tools.
- Expand a table to inspect **indexes**, **constraints**, **triggers**, **row-level security** and **policies**, with details on hover.
- Expand the server's **Roles** and **Tablespaces** groups; roles show attributes, memberships and copyable DDL.
- Right-click a database → **Slow queries (pg_stat_statements)** for an editable query showing execution time and buffer counters. Requires the extension; Trunq does not install it automatically.

<details><summary>Light theme and a compact data viewer</summary>

![Light ER diagram](docs/screenshots/erd-light.png)
![Data viewer at 900 × 620 logical pixels](docs/screenshots/compact.png)

</details>

## Install from source

The easiest way is a [pre-built download](#download). Building requires **Rust 1.95+**, matching the GUI dependencies. To build it yourself:

### macOS

```bash
# 1. Install Rust (once)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# 2. Get the code and build Trunq.app
git clone <this repository> trunq
cd trunq
./scripts/bundle-macos.sh --install      # builds and copies Trunq.app to /Applications
```

Then open **Trunq** from Launchpad or Spotlight like any other app.

```bash
./scripts/bundle-macos.sh          # only build target/release/bundle/Trunq.app
./scripts/bundle-macos.sh --dmg    # also create target/release/bundle/Trunq.dmg
```

> The app is ad-hoc signed. If macOS says it "can't be opened because it is from an unidentified developer",
> right-click **Trunq.app** and choose **Open** once, or run `xattr -dr com.apple.quarantine /Applications/Trunq.app`.

Requirements: macOS 11 or newer (Intel or Apple Silicon) and the Xcode command line tools (`xcode-select --install`).

### Linux

```bash
# Debian / Ubuntu build dependencies
sudo apt install build-essential pkg-config libssl-dev libxkbcommon-dev libwayland-dev
# Fedora: sudo dnf install gcc pkgconf openssl-devel libxkbcommon-devel wayland-devel

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
git clone <this repository> trunq && cd trunq
cargo build --release
sudo install -m 755 target/release/trunq /usr/local/bin/trunq
trunq
```

Passwords are stored in the Secret Service (GNOME Keyring or KWallet).

### Windows

1. Install [Rust](https://rustup.rs) and choose the MSVC toolchain (it installs the Visual Studio build tools).
2. In a terminal:

```powershell
git clone <this repository> trunq
cd trunq
cargo build --release
.\target\release\trunq.exe
```

Passwords are stored in the Windows Credential Manager.

### Run from source

```bash
cargo run              # debug build, fastest to compile
cargo run --release    # optimised build, what you want for real use
cargo test             # run the test suite
```

The first build downloads and compiles the dependencies and takes a few minutes. Later builds are incremental.

## First steps

1. Click **New connection** (the plug icon in the top bar).
2. Paste a URL such as `postgres://user:pass@db.example.com:5432/app?sslmode=require` and press **Apply**,
   or fill in host, port, database and user.
3. Pick an **environment** (production turns red everywhere), press **Test connection**, then **Save**.
4. The connection appears in the sidebar. Expand it to browse databases, schemas, tables, views, functions and types.
   Double-click a table to open its data and right-click anything for more actions.
5. Press **⌘T** (Ctrl+T on Windows and Linux) for a SQL editor and **⌘⏎** to run the statement under the cursor.

Passwords are saved in your operating system keychain. Trunq also understands `~/.pgpass` and Unix sockets
(enter the socket directory, for example `/tmp`, as host).

## SSH tunnels

Open the connection dialog, expand **SSH tunnel** and enable *Connect through an SSH server*.

- **SSH host, port and user** of the bastion or jump host.
- **Authentication**: the running SSH agent (with your default keys as fallback), a private key file (with an optional passphrase), or a password.
- The database **Host** and **Port** at the top of the dialog are now resolved *from the SSH server*, usually `localhost` or a private IP address.

Host keys are checked against `~/.ssh/known_hosts` and Trunq's own known hosts file. A new host is trusted on first use and recorded. A *changed* host key is refused, because it can mean a man-in-the-middle attack.

## Azure Database for PostgreSQL (Microsoft Entra ID)

Azure accepts a short-lived (about one hour) Entra access token as the password. Trunq obtains it and **refreshes it
automatically** before every new connection, so you never paste tokens by hand.

1. New connection, with a host such as `myserver.postgres.database.azure.com`. Trunq detects Azure hosts and selects SSL `require`.
2. **Authentication**: Microsoft Entra ID.
3. **Azure cloud** (detected from the host name):
   - *Azure (global)*: `*.postgres.database.azure.com`
   - *Azure China (21Vianet)*: `*.postgres.database.chinacloudapi.cn`
   - *Azure US Government*: `*.postgres.database.usgovcloudapi.net`
4. **Sign-in method**:
   - **Interactive (browser)** opens the Microsoft sign-in page. After the first sign-in the refresh token is kept in
     your keychain and new tokens are created silently for about 90 days.
   - **Device code** is for machines without a browser (remote desktops, SSH). Trunq shows a code to enter at
     `microsoft.com/devicelogin` or the China equivalent.
   - **Azure CLI** reuses your `az login` session. For China run `az cloud set --name AzureChinaCloud` first.
5. **User**: your Entra user principal (for example `jane@contoso.com`) or the Entra group or role that was
   granted access. Leave it empty to use the signed-in account.
6. Optional: **Tenant** (`contoso.onmicrosoft.com` or a tenant ID). Under *Advanced* you can set your own
   **client ID** if your organisation blocks the Azure CLI public client.

## Keyboard shortcuts

On Windows and Linux use **Ctrl** instead of **⌘**.

| Shortcut | Action |
|---|---|
| ⌘⏎ | Smart Run: the selection, or the statement under the cursor |
| ⇧⌘⏎ | Run the whole script |
| ⌥⌘⏎ | Dry run (INSERT, UPDATE, DELETE, always rolled back) |
| ⌘E / ⇧⌘E | Explain / Explain Analyze |
| Esc | Cancel the running query |
| ⇧⌘F | Format SQL |
| ⌘/ | Toggle line comment |
| ⌘D | Duplicate line |
| Tab / ⇧Tab | Indent / outdent |
| ⌃Space | Show completions |
| ⌘S | Save the SQL file |
| ⌘K or ⌘P | Command palette |
| ⌘T / ⌘W | New tab / close tab |
| ⌃Tab / ⌃⇧Tab | Next / previous tab |
| ⌘B | Toggle the sidebar |
| ⌘F (in a grid) | Column finder |
| ⌘A, ⌘C, ⇧⌘C | Grid: select all, copy, copy with headers |
| Enter, F2 or double-click | Grid: edit a cell (table view) or open the value / JSON tree |
| ⌘-click | Follow a foreign key |
| ← / → | Card view: previous / next row |

## Where Trunq stores data

| | macOS | Linux | Windows |
|---|---|---|---|
| Settings, connections, open tabs, table layouts, snippets | `~/Library/Application Support/dev.trunq.Trunq/config.json` | `~/.config/trunq/config.json` | `%APPDATA%\trunq\Trunq\config\config.json` |
| Query history | `history.jsonl` in the same folder | same | same |
| Trusted SSH host keys | `known_hosts` in the same folder | same | same |
| Passwords, SSH secrets, Azure refresh tokens | Keychain | Secret Service | Credential Manager |

`TRUNQ_CONFIG_DIR=/path/to/workspace` overrides the settings and history directory for a portable or isolated workspace. Delete that workspace folder to reset it.

## Try it with the sample database

`dev/sample.sql` creates a realistic shop database (customers, orders, one million analytics events, views, functions, enums, JSON):

```bash
createdb shop
psql -d shop -f dev/sample.sql
```

## Troubleshooting

- **"password authentication failed" or "no password supplied"**: type the password in the field that appears in the
  sidebar or above the editor. It is remembered for the session and saved in the keychain if "Save in keychain" is on.
- **Azure `AADSTS…` errors**: check the tenant, that your account (or its group) was added as an Entra admin or role on the
  server, and that the right cloud is selected. Try *Device code* if the browser flow is blocked by policy.
- **SSH "host key has CHANGED"**: the server presents a different key than before. If the server was reinstalled,
  remove the old line from `known_hosts` and connect again.
- **The window is blank on an old GPU or a virtual machine**: run with `WGPU_BACKEND=gl` to use OpenGL.
- **Keychain prompts on macOS**: click *Always Allow* once so Trunq can read its saved passwords.

## Development and releases

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

```
src/
  db/        wire protocol client, sessions (async actors), catalog, SSH tunnels, Azure Entra auth
  sql/       lexer, autocompletion, formatter, migration linter
  ui/        theme, editor, data grid, JSON tree, charts, connection and import dialogs, explain view
  tabs/      query and table tab (results in tabs/query/), live monitor, health, sessions, structure
  app.rs     window layout, sidebar, tabs, command palette, dialogs
```

- `cargo test` runs the unit tests: protocol parsing, SQL splitting, completion, formatter, linter, export, CSV inference, Azure helpers and more.
- `TRUNQ_TEST_DATABASE_URL=postgres://… cargo test live_tests` runs protocol tests against a real server. The CI matrix covers PostgreSQL 13 to 18.
- Scripted UI runs for visual checks, without any OS permissions:

  ```bash
  TRUNQ_SCRIPT="wait:3;run;wait:2;shot:/tmp/query.png;open:shop.orders;wait:2;shot:/tmp/table.png;quit" cargo run
  ```

  Regenerate the sample gallery with `TRUNQ_TEST_DATABASE_URL=postgres://… python3 scripts/screenshots.py` after loading `dev/sample.sql` into a disposable database. The script uses an isolated temporary configuration and converts images to WebP when `cwebp` is installed.

  Steps: `wait:<s>`, `run`, `runall`, `dryrun`, `explain[:analyze]`, `sql:<text>` (separate with `;;`), `open:<schema.table>`,
  `tab:<n>`, `palette:<query>`, `health`, `sessions`, `structure:<schema.table>`, `erd[:schema]`, `resize:<width>,<height>`, `chart`, `card`, `compare`, `value:<row>,<col>`,
  `pin`, `side`, `search:<text>`, `ts:utc|local|raw`, `finder`, `hideempty`, `import:<file>`, `importrun`, `newconn`,
  `connections`, `close`, `theme:light|dark`, `shot:<file.png>`, `quit`. Scripted runs never overwrite your saved workspace.

### Releases

Releases are automated by [`.github/workflows/release.yml`](.github/workflows/release.yml):

1. Bump `version` in `Cargo.toml` and add the notes to `CHANGELOG.md`.
2. `git tag v0.2.0 && git push origin v0.2.0`
3. GitHub Actions builds a universal macOS `.dmg` (and `.app.zip`), a Linux `.deb` and `.tar.gz`, and a Windows `.zip`,
   writes `SHA256SUMS.txt` and publishes them as a GitHub Release with the changelog section as release notes.
   Add the `APPLE_*` secrets to also sign and notarize the macOS build.

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) checks formatting and clippy (warnings denied), builds and tests on
macOS, Windows and Linux, and runs the protocol tests against PostgreSQL 13 to 18.

## License

[MIT](LICENSE). Fonts: Inter and JetBrains Mono (SIL Open Font License). Icons: Lucide (ISC). Their licenses are in `assets/fonts/`.
