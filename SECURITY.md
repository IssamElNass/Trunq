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

- Passwords, SSH passwords / key passphrases and Azure refresh tokens are stored only in the
  operating system keychain (macOS Keychain, Windows Credential Manager, Secret Service on Linux),
  never in config files.
- Entra ID access tokens are kept in memory only.
- TLS is negotiated according to the connection's `sslmode`; `verify-ca` / `verify-full` validate
  certificates with the platform trust store. Azure connections always require TLS.
- SSH host keys are verified against `~/.ssh/known_hosts` and Trunq's own known hosts file. New hosts
  are trusted on first use; a changed host key is refused.
- Query parameters (`:name`) are always sent as bound parameters, never interpolated into SQL.
- Trunq has no telemetry and makes no network connections other than to your databases, your SSH
  servers and, when you use Entra ID, Microsoft's sign-in endpoints.
