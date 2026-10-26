# Troubleshooting

[Documentation](README.md) · [Connections](connections.md) · [Settings and backups](settings.md)

## Identify where the failure happens

Start with the exact error in **Messages** or the connection dialog. Check the profile, database and user shown in the status bar. A successful connection test does not prove that a particular table, schema or operation is permitted. The object browser, query tabs and imports/exports can use separate sessions.

| Symptom | First check |
|---|---|
| Connection refused | Host, port and whether PostgreSQL is listening there |
| Connection timeout | Network route, VPN/firewall and the profile's connect timeout |
| TLS handshake or certificate error | [TLS mode, trust store and hostname](#tls-handshake-or-certificate-errors) |
| Password authentication failed | [Selected user and password source](#password-or-authentication-failures) |
| Query tab works but metadata times out | Locks and the browser's shorter metadata timeouts |
| Query reports a failed transaction | [Rollback](#transaction-failed-or-query-cancelled) before retrying |
| Data is missing from search, a chart or an export | [Loaded versus server data](#missing-rows-columns-or-export-values) |

For Docker, the database host is relative to the container. `localhost` means that container, not the host computer. See [Docker connection setup](docker.md#build-and-run).

## Password or authentication failures

Confirm the database User and database name. A currently supplied session password takes precedence over the saved keychain entry and password file. If a password has changed, supply the new value and check **Save in keychain** as appropriate. See [password precedence and pgpass syntax](connections.md#password-sources-and-pgpass).

For `no pg_hba.conf entry`, PostgreSQL rejected the connection's source address, database, user or encryption combination. Ask the server administrator to check the intended rule. Increasing the connect timeout does not change an authentication rule.

If Linux cannot remember a password, check that a Secret Service provider such as GNOME Keyring or KWallet is available and unlocked in that desktop session. The Docker desktop has no configured keychain service; use session entry or a protected password file there. macOS may request keychain access for the application; allow access if this is the app you intend to use.

A fatal `SCRAM channel binding negotiation error` mentioning the letter `n` is a client protocol error fixed in the next Trunq build. Update the client and reconnect with your existing TLS setting. This applies after browser sign-in too, because PostgreSQL authentication is a separate step. If Entra authentication is still rejected, verify the case-sensitive database role or group, tenant and signed-in account; a successful browser login alone does not grant database access.

## TLS handshake or certificate errors

`require` requires encryption but does not validate the server certificate or hostname. `verify-ca` validates certificate trust; `verify-full` also validates the database hostname. For verification failures, check the computer's clock, the certificate chain, the platform trust store and the Host field's match with the certificate.

For SSH, the checked database hostname is still the database Host, not the bastion. Do not replace a certificate hostname with a raw IP unless the certificate includes that IP. See [the mode table and local socket behaviour](connections.md#what-each-tls-mode-checks).

## SSH or Azure sign-in problems

For SSH, check the bastion address and credentials first, then whether PostgreSQL's host and port are reachable from the bastion. A changed host key is a separate problem from a database password. Verify its fingerprint through a trusted source before changing the old known-hosts entry; either the system file or Trunq's workspace file can contain it.

For Azure `AADSTS…` errors, check the tenant, selected cloud, sign-in method and the principal configured on the server. A browser/device login proves Microsoft sign-in, not database authorization. Azure CLI mode needs the intended `az login` account and cloud. Try **Device code** when browser callback handling is unavailable, or use a permitted app registration when your organization blocks the default client. See [Azure setup](usage.md#azure-database-for-postgresql-microsoft-entra-id).

## Transaction failed or query cancelled

After an error in an explicit transaction, choose **Rollback**, correct the statement and try again. The error `current transaction is aborted` is a consequence of the earlier failure; look earlier in Messages for its cause. Switching the Auto-commit/Manual selector does not finish an already-open transaction.

Esc sends a PostgreSQL cancellation request for a running query. Cancellation can leave an explicit transaction failed, and it does not undo a statement that already committed. A statement-timeout error and a lock-timeout error also need their underlying query or blocker investigated rather than repeatedly retried.

A paged SELECT can itself keep an implicit transaction open. Finish fetching its rows or close its session when it is no longer needed. See [transaction and paging behaviour](workflows.md#auto-commit-manual-and-failed-transactions).

## Table is read-only or Save fails

Editing existing records requires a primary key in the table view. An arbitrary SELECT result is a viewer, even if its columns originate from a table. Database privileges and a profile's read-only setting can independently prevent writes.

For a failed save, inspect the PostgreSQL error for a unique, not-null, check, foreign-key, generated-column or permission constraint. That save is rolled back and pending edits remain available to correct or discard. A duplicated row omits primary-key, identity and generated values, but another unique column may still need changing. **Preview SQL** shows what Save will attempt.

The row editor is not an automatic conflict-resolution tool. Refresh and review before changing rows that other users may also be editing. Save or discard pending edits before replacing the result with a new query.

## Missing rows, columns or export values

- Grid search and local sorting cannot inspect unfetched rows. Run the condition in SQL or fetch the necessary pages.
- Table WHERE/ORDER BY text is SQL on the server. Check it and any remembered table filters before assuming data is missing.
- **Columns → Show all columns** restores manually hidden columns. Turn off **Hide empty columns** when needed; emptiness is based on the loaded rows.
- If a paged result stops after the second scroll and stays busy, update Trunq: earlier builds failed to finish the fetch operation. **Stop loading** closes a healthy paused cursor and retains loaded rows. A manual transaction still requires Commit or Rollback.
- A pinned result retains loaded rows, but rerunning closes its old cursor. Run a large SELECT separately if you need to continue paging it after a script.
- Charts use loaded result rows and their own axes, not grid search/filter chips. Bars are limited to 300 rows; aggregate or limit deliberately in SQL.
- File exports include all result columns, including hidden columns. Select the required columns in SQL for a narrower export.
- Save pending table edits and refresh before exporting their saved values. Entire-result export runs on a separate connection and cannot use a query tab's temporary tables or uncommitted changes.

See [the scope reference](workflows.md#understand-which-data-a-tool-uses) before choosing an export option. If a numeric JSON value loses precision in another tool, export that column as text. For CSV, preserve quoted empty fields when another program distinguishes empty text from NULL.

## CSV import errors or surprising types

Review **First row is a header**, the delimiter, normalized column names, inferred types and, for an existing table, every target-column mapping. Initial target mapping tries names and then positions. Selecting an existing table also exposes **Empty it first**, which truncates that table within the import transaction.

The preview reads up to the first 1 MiB and type inference samples up to 2,001 records, including the header when enabled. A later value can therefore fail a type inferred from the beginning. Choose a wider type or clean the source file, then retry. Import into an existing table must also satisfy its constraints and permissions.

Use a UTF-8 CSV/TSV file. Check quoting when a field contains a delimiter, newline or quote. PostgreSQL COPY CSV treats an unquoted empty field as NULL and a quoted empty field as empty text; the preview does not display that distinction as separate types. A literal `NULL` field is text unless the target type interprets or rejects it. Errors and cancellation roll back the import transaction.

## Blank window or graphics startup failure

Try OpenGL when the default backend fails on an older GPU or VM. In a shell where `trunq` is on PATH:

```sh
WGPU_BACKEND=gl trunq
```

Windows PowerShell, from the release directory:

```powershell
$env:WGPU_BACKEND = 'gl'
.\Trunq.exe
```

Linux needs an available graphical display; use the [Docker desktop](docker.md) when you need the packaged virtual-display setup. Record the exact startup error, OS and graphics environment when reporting the issue.

## Settings or SQL tabs appear to have disappeared

Check whether the app was started with a different `TRUNQ_CONFIG_DIR`, from a different user account or inside Docker. These can select different settings directories. Query editors are saved, but loaded results, pinned rows and pending table edits are not restored as data.

A missing or malformed configuration loads defaults. Close the app and preserve any existing `config.json` before attempting recovery; subsequent settings saves can write a new configuration. Restore a known-good copy while the app is closed. See [backup and restore](settings.md#back-up-or-move-a-workspace).

## Report a useful bug

Include the Trunq version/commit, OS, PostgreSQL version, exact error text and the smallest query or sequence of UI steps that reproduces it. State whether it uses SSH, Azure, a special TLS mode or Docker. For result/export issues, include the row scope and whether edits were pending or rows were still unfetched.

Use sample data and redact passwords, tokens, connection URLs with credentials and sensitive SQL/results. Do not attach an entire configuration or history file without reviewing it. Use [the issue forms](https://github.com/IssamElNass/Trunq/issues/new/choose) for ordinary bugs and [the security policy](../SECURITY.md) for vulnerabilities.

## After the security hardening update

| Symptom | Explanation and next action |
|---|---|
| Saved password/passphrase no longer works after upgrading | Older ID-only secrets are not reused. Re-enter the database/SSH secret for the verified target; see [migration details](production.md#upgrade-and-credential-changes). |
| Password disappears after editing host/user/database/bastion | Target changes deliberately clear previously typed credentials. Review the target, then enter its intended credential. |
| pgpass is ignored on Unix | Verify it is a regular readable file with no group/other permission bits, for example `chmod 600 ~/.pgpass`. |
| SSH host is not pinned | Add an independently verified raw host key or use the intended trust policy. Do not enable first-use trust merely to bypass a changed/revoked-key error. |
| RSA SSH key fails | The vulnerable optional RSA backend was removed. Arrange Ed25519/ECDSA user and host keys; host certificates are also unsupported. |
| Protocol message exceeds client limit | Reconnect the affected session and reduce very large fields or use server-side chunking. The individual-frame limit does not limit total result size. |
| Notice/notification output truncated | Inspect server logs or the authoritative event system; bounded client queues are not durable delivery. |
| Workspace write rejected | Check parent-directory ownership/ACLs and regular-file destinations; Unix append paths reject symlinks. Preserve your original files before repairing a workspace. |

[Security findings and remaining limits](security-review.md) distinguish tested fixes from deployment-specific checks.
