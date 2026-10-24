# Security Policy

Trunq handles database credentials and access tokens, so we take security reports seriously.

## Supported versions

Security fixes are released for the latest minor version.

| Version | Supported |
|---|---|
| 0.1.x | ✅ |

## Reporting a vulnerability

Please **do not** open a public issue. Use GitHub's
[private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)
("Security" tab → "Report a vulnerability") and include:

- affected version and platform,
- steps to reproduce or a proof of concept,
- the impact you expect.

You should receive an acknowledgement within a few days. We will keep you informed while we work on
a fix and credit you in the release notes unless you prefer otherwise.

## How Trunq protects your data

- Connection passwords, SSH passwords / key passphrases and Azure refresh tokens supplied through authentication settings are stored in the
  operating system keychain (macOS Keychain, Windows Credential Manager, Secret Service on Linux),
  never in config files.
- Entra ID access tokens are kept in memory only.
- TLS is negotiated according to the connection's `sslmode`; `verify-ca` / `verify-full` validate
  certificates with the platform trust store, and `verify-full` also checks the database hostname.
  `require` encrypts without validating certificate trust or hostname; the default `prefer` permits
  plaintext if the server declines TLS. Azure profiles enforce at least `require`.
- SSH host keys are verified against `~/.ssh/known_hosts` and Trunq's own known hosts file. New hosts
  are trusted on first use; a changed host key is refused.
- Query parameters (`:name`) are always sent as bound parameters, never interpolated into SQL.
- Trunq has no telemetry and makes no network connections other than to your databases, your SSH
  servers and, when you use Entra ID, Microsoft's sign-in endpoints.

## Local workspace and sharing

Settings, SQL tab text, snippets and query history are ordinary local JSON/JSONL files, not encrypted by Trunq. Credentials or sensitive values written into SQL literals, comments or connection URLs in editor text can therefore be saved as text. Review configuration, history, SQL files and screenshots before attaching them to an issue.

`TRUNQ_CONFIG_DIR` changes the workspace directory but does not create a separate OS keychain. Copied profiles keep their credential identifiers. Workspace backups do not include the keychain, SSH private keys or an Azure CLI login; see [settings and backups](docs/settings.md).

Read-only profiles set PostgreSQL's default transaction mode. Database role privileges are the enforcement mechanism when write access must be restricted. See [the connection reference](docs/connections.md) for password precedence, TLS modes, host-key handling and timeout behaviour.
