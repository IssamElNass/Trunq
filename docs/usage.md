# Using Trunq

[Documentation](README.md) · [Install](install.md) · [Feature reference](features.md)

## Contents

- [First connection](#first-steps)
- [Queries and parameters](#queries-and-parameters)
- [Editing and transactions](#editing-and-transactions)
- [Choosing an export](#choosing-an-export)
- [Importing CSV](#importing-csv)
- [SSH](#ssh-tunnels) and [Azure](#azure-database-for-postgresql-microsoft-entra-id)
- [Shortcuts](#keyboard-shortcuts)
- [Storage](#where-trunq-stores-data)
- [Sample database](#try-it-with-the-sample-database)
- [Troubleshooting](#troubleshooting)

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

## Queries and parameters

Press **⌘T** (Ctrl+T on Windows/Linux) to open an editor. **⌘Enter** runs the selection, or the statement under the cursor. **Shift+⌘Enter** runs the whole script, including when text is selected. Statements run in order and stop at the first error.

```sql
-- name: Connection check
SELECT current_database(), current_user, version();
```

Use named placeholders for values:

```sql
SELECT :customer_id::bigint AS customer_id, :status::text AS status;
```

Trunq prompts for each name and sends the values as PostgreSQL parameters. Repeated names share a value. Parameters represent values, so table and column names must be written in the SQL. In the parameter dialog, entering `NULL` sends SQL NULL; an empty field sends empty text.

Results initially contain up to the configured page size (1,000 rows by default). Scroll to fetch more, or choose **Load all** to retain the full result in memory. Search, local filters, sorting and aggregates operate on loaded rows. A table view's filter bar instead adds a server-side WHERE clause.

## Editing and transactions

Double-click a table to browse it. A primary key is required for editing existing records. Edit a cell, add or duplicate a row, or mark rows for deletion, then use **Preview SQL** before **Save**.

Save applies the pending statements together. A statement error rolls back that save and keeps your pending edits available to correct and retry. If the tab already has an explicit transaction, Save uses a savepoint and leaves that transaction open for you to commit or roll back. Closing a paged result ends its implicit cursor transaction.

For SQL changes, the confirmation dialog warns about UPDATE or DELETE without a WHERE at the same query level, including modifying CTEs. A WHERE inside a subquery does not restrict the outer update. These checks are heuristics: inspect the SQL and the selected connection before executing.

**Dry run** executes supported data changes in a transaction or savepoint and rolls back before reporting completion. It also prompts for named parameters. An earlier paged result is closed first; an existing explicit transaction is preserved. PostgreSQL sequences can advance despite rollback. **EXPLAIN ANALYZE** actually executes the statement; use plain EXPLAIN when you only want the plan.

## Choosing an export

| Scope | Rows included | Database work |
|---|---|---|
| Visible rows | Loaded rows matching local search/filters, in the displayed sort order | None |
| Loaded rows | All rows fetched so far | None |
| Entire result from the server | Every row returned by running the SQL again | Separate connection; one chunk held at a time |

The visible option appears when its row count differs from the loaded count. An entire-result export runs the statement again, so it can observe newer data and repeat any side effects in that SQL. Formats are CSV, JSON, Markdown, SQL INSERT and TypeScript.

The destination is replaced only after the export finishes successfully. Cancellation, query errors and write failures leave an existing destination unchanged; temporary files are cleaned up during normal error handling. Leave room on the destination filesystem for the new export.

CSV writes SQL NULL as an unquoted empty field and empty text as `""`, matching PostgreSQL COPY CSV. SQL exports quote NaN and positive/negative Infinity so PostgreSQL can read them back. JSON represents non-finite values as strings; consumers using JavaScript numbers can lose precision for large numeric values.

## Importing CSV

Drop a UTF-8 CSV or TSV file onto the window, then check the delimiter, header, target table and inferred column types before importing. Quoted delimiters and escaped quotes are supported. Inference samples the beginning of the file; later values may require a wider type such as text.

When header names normalize to the same SQL identifier, Trunq adds suffixes to keep them unique. Timestamp inference accepts positive and negative timezone offsets. The import streams through COPY in one transaction; errors and cancellation roll back the import. For an existing table, review **Truncate** before starting because it removes existing rows within that transaction.

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

`TRUNQ_CONFIG_DIR=/path/to/workspace` overrides the settings and history directory for a portable or isolated workspace. Close Trunq and back up that workspace folder before removing it to reset settings.

## Try it with the sample database

[`dev/sample.sql`](../dev/sample.sql) drops and recreates its sample schemas in a disposable database, producing a realistic shop database (customers, orders, one million analytics events, views, functions, enums, JSON):

```bash
createdb shop
psql -d shop -v ON_ERROR_STOP=1 -f dev/sample.sql
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
