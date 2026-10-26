# Settings, workspaces and backups

[Documentation](README.md) · [User guide](usage.md) · [Connections](connections.md)

## Application settings

Open the gear icon, or search for **Settings** in the command palette. Changes save automatically.

| Setting | Default | Available values and effect |
|---|---|---|
| Theme | Dark | Dark or Light |
| Interface zoom | 100% | 75–160%; changes the whole interface |
| Editor font | 13.5 px | 10–22 px |
| Grid font | 12.5 px | 10–18 px |
| Rows per fetch | 1,000 | 100–100,000; how many rows a page request fetches |
| Suggest while typing | On | Automatic completions; explicit completion is available with Ctrl+Space |
| UPPERCASE completions and formatting | On | Controls generated keyword casing |
| Confirm destructive statements | On | Enables the destructive-query confirmation and migration-linter check before running SQL |

A larger fetch size can reduce round trips but increases the amount transferred and retained at once. It does not add a SQL LIMIT or cap the total result. **Load all** retains the full result in memory; an entire-result file export streams chunks instead. See [result and export limits](workflows.md#understand-which-data-a-tool-uses).

Connection-level settings, including read-only mode and timeouts, live in the connection dialog's **Advanced** section, not the global Settings dialog. See [the profile reference](connections.md#advanced-profile-settings).

## What the workspace saves

| Data | Location | Restored on launch? |
|---|---|---|
| Connection profiles and their IDs, application settings | `config.json` | Yes |
| SQL query-tab text, file paths, selected tab | `config.json` | Query editors are restored |
| Snippets and expanded browser nodes | `config.json` | Yes |
| Per-table filters, ordering, grid/card preference, hidden columns and column widths | `config.json` | Applied when that table is opened again |
| Query text and execution summaries | `history.jsonl` | Available in Query history |
| Newly trusted SSH host keys | `known_hosts` | Used for later SSH checks |
| Database passwords, SSH passwords/passphrases, Entra refresh tokens | OS keychain | Looked up when needed; separate from workspace files |

Loaded result rows, pinned result contents, pending table edits, open transactions and temporary database tables are not workspace backups. A saved file path also does not copy the `.sql` file into the workspace. Save or export anything you need before closing the app.

SQL text, snippets, profile names and query history are ordinary local files. SQL literals or comments can contain sensitive information even though authentication secrets are stored separately. Choose what to share when sending a configuration or history file with a bug report.

## Default locations

| Platform | Settings directory |
|---|---|
| macOS | `~/Library/Application Support/dev.trunq.Trunq` |
| Linux | `~/.config/trunq` |
| Windows | `%APPDATA%\trunq\Trunq\config` |

`config.json`, `history.jsonl` and Trunq's `known_hosts` live in that directory. System SSH known hosts and private keys remain under your SSH configuration. Exported files are saved wherever you choose in the file dialog.

## Start an isolated workspace

Set `TRUNQ_CONFIG_DIR` before launching the app. Use an absolute path so the location does not depend on the launch directory.

macOS/Linux, when `trunq` is on PATH:

```sh
TRUNQ_CONFIG_DIR="$PWD/trunq-workspace" trunq
```

Windows PowerShell, from the directory containing `Trunq.exe`:

```powershell
$env:TRUNQ_CONFIG_DIR = Join-Path $PWD 'trunq-workspace'
.\Trunq.exe
# Remove the override before a later normal launch from this shell:
Remove-Item Env:TRUNQ_CONFIG_DIR
```

The override affects settings, history and Trunq's SSH known-hosts file. It does not create a separate OS keychain. Copying profiles preserves their IDs and target fingerprints, so unchanged profiles can refer to the same keychain entries on the same computer. Changing their target selects different entries; older ID-only entries require credential re-entry after this update. Use separate directories for concurrent app instances to avoid competing writes to one configuration.

In Docker, the supplied image uses `/data` for its workspace and Compose persists that directory in `trunq-data`. See [the Docker guide](docker.md) for browser downloads, session-only passwords and the separate web workspace.

## Back up or move a workspace

1. Finish or roll back open transactions. Save pending table edits, SQL files and any result exports you need.
2. Close Trunq so its final configuration has been written.
3. Copy the entire settings directory to your backup location. Include `history.jsonl` and `known_hosts` if you want history and SSH trust records.
4. Copy external `.sql` files and exports separately; only their paths may be recorded in settings.
5. Restore into the destination settings directory while Trunq is closed, or launch it with `TRUNQ_CONFIG_DIR` pointing to the restored copy.
6. Recheck hosts, environment labels, file paths and SSH key paths on the destination computer. Supply credentials there through its keychain, password file or sign-in flow.

Copying the workspace does not export the OS keychain, a running SSH agent, provider CLI logins, secret environment values or TLS certificate/key files. Helper executable/argument configuration and TLS paths do move; review those before connecting in the new workspace. A workspace backup also does not contain the database's schemas or rows; database backups need PostgreSQL backup tooling.

## Reset settings or remove history

To try a clean configuration, launch with a new empty workspace directory. To reset the existing workspace, close Trunq and rename its settings directory so you can restore it later. This resets the local workspace without removing database objects or credentials already stored in the OS keychain.

Use **Query history → Clear** to clear history. This does not clear snippets or open editor text. If a damaged `config.json` causes Trunq to open with defaults, preserve a copy of the original file before launching again or changing settings; the loader falls back to defaults when it cannot read valid configuration.

## Launch environment reference

| Variable | Purpose |
|---|---|
| `TRUNQ_CONFIG_DIR` | Override the local workspace directory |
| `PGPASSFILE` | Select a PostgreSQL password file |
| `AZURE_CLIENT_SECRET` (or configured variable name) | Supply the native service-principal secret to the app process |
| Provider CLI environment/profile selectors | Inherited by token helpers; use explicit CLI arguments or launch environment to select the intended identity |
| `WGPU_BACKEND=gl` | Request OpenGL when the default graphics backend does not work |
| `TRUNQ_SCRIPT` | Drive development UI runs; suppresses normal workspace saving |
| `TRUNQ_BENCH` | Enable the development benchmark path; suppresses normal workspace saving |

For graphics troubleshooting, use [the troubleshooting guide](troubleshooting.md#blank-window-or-graphics-startup-failure). Developer test and benchmark variables are described in [development](development.md) and [performance](performance.md).

## File protection and history retention

On Unix, Trunq creates new workspace directories with `0700` and writes configuration, history and learned host keys with `0600`. Existing directory permissions are preserved; Windows follows its directory ACLs. Config saves/history compaction stage into a private, exclusive random file before atomic replacement. Keep the parent directory trusted, especially when using `TRUNQ_CONFIG_DIR`.

History loads at most the latest 5,000 valid entries and periodically compacts its disk log after 501 appends. Consecutive duplicate queries still advance that compaction counter. The file can temporarily retain additional older entries between compactions; clearing history does not remove copies in backups or SQL in open editors/snippets. See [production data handling](production.md#protect-local-data).
