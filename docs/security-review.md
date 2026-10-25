# Production and security review — 25 October 2026

[Documentation](README.md) · [Production operations](production.md) · [Validation record](validation.md)

Scope: credential routing and persistence, SSH authentication/host-key validation/lifetime, local configuration/history writes, PostgreSQL protocol resource handling and the locked dependency graph. Findings below describe the branch's changes; previously published binaries do not acquire fixes until rebuilt/released. Impact labels are review judgments, not CVSS scores.

## Findings and fixes

| Finding | Potential impact | Change and evidence |
|---|---|---|
| Manual endpoint edits could retain typed/in-memory passwords; keychain entries identified only the profile | High: send a credential to a different target after an edit/import | Bind keychain lookup and typed secrets to the connection target; clear the running password cache when it changes. Query/sidebar prompts also discard unsent secrets, and queued password actions must still match the current target. Regression tests cover host/database/bastion changes and preserving harmless label changes. Legacy entries are not reused; re-entry is required. |
| SSH host-key comparison depended on the presented algorithm, and unreadable known-host files could look absent | High: an unexpected algorithm/key might be treated as first use | Compare all recorded matching raw keys, refuse mismatches/read errors/revoked keys, reject unsupported host certificates and offer strict pinned-host mode. Public-key fixture tests cover algorithm rotation, revocation and read failures. |
| SSH cache retained strong session references | Moderate: unused authenticated tunnels and connection resources remained alive | Channels own the handle; the cache uses weak references and per-configuration handshake locks. A real OpenSSH/PG test verifies sharing, cancellation and release after the final channel. |
| Repeated duplicate SQL never reached the history compaction trigger | Moderate: unbounded disk growth and expensive startup | Count actual appends and stream only the latest 5,000 entries during load. Tests cover 1,200 identical queries and loading/compacting an older oversized file. |
| Workspace saves used predictable staging names and inherited creation permissions | Moderate: local SQL disclosure or redirected writes in an unsafe workspace | Private exclusive random temporary files, atomic replacement and Unix no-follow regular-file append writes. Tests verify permissions, replacement and final-component symlink resistance. Protect the parent directory separately. |
| Password-file permissions were not checked | Moderate: using a credential file readable by other local users | Reject group/other-readable Unix pgpass files; regression checks `0600` accepted and `0644` ignored. |
| Server frames and retained diagnostics lacked explicit limits | Moderate: large frames or notice/notification floods could exhaust client memory | Reject oversized headers and cap retained queues with warnings. Boundary tests plus a real PostgreSQL notice flood verify bounds and recovery after draining. Total loaded results remain user-controlled. |

The extension-manager inventory also avoids directly reading a restricted `cron.database_name` setting. With pg_cron loaded, limited roles now receive an unknown scheduler setting instead of losing the entire inventory. The real pg_cron fixture exposed this failure, and the restricted-role install/upgrade regression now checks inventory success explicitly.

Disabling database-password saving no longer deletes the separate SSH secret. Profile deletion clears its running database-password cache and current/legacy keychain entries. Token-command sessions no longer enter the password-auth persistence path. Real OS keychain UI/backend behavior was not tested in this environment.

## Dependency audit

`cargo-audit 0.22.2` checked `Cargo.lock` against RustSec on 25 October 2026. The initial scan reported three vulnerable packages and one yanked package. After the changes, it reports **zero vulnerable packages and no yanked packages**, with four maintenance advisories below. No advisory ignore list was added.

| Advisory | Remediation |
|---|---|
| [RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387.html), crypto-bigint timing behavior | Updated 0.7.5 → 0.7.6 |
| [RUSTSEC-2026-0385](https://rustsec.org/advisories/RUSTSEC-2026-0385.html), ctutils timing behavior | Updated 0.4.3 → 0.4.5 |
| [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html), RSA Marvin attack | No upstream patched version; removed the optional RSA SSH backend and its dependency. Native SSH requires Ed25519/ECDSA. |
| Yanked clipboard-win 5.4.1 | Updated to 5.4.3 |

Remaining upstream **unmaintained** advisories: [ttf-parser](https://rustsec.org/advisories/RUSTSEC-2026-0192.html), [zbus_names](https://rustsec.org/advisories/RUSTSEC-2026-0371.html), [zvariant](https://rustsec.org/advisories/RUSTSEC-2026-0370.html) and [zvariant_derive](https://rustsec.org/advisories/RUSTSEC-2026-0372.html). These are open dependency-maintenance work, not evidence of an exploitable leak in Trunq. Updating/replacing their UI/keyring parent dependencies needs compatibility validation. A clean vulnerability count is time-specific and does not mean the dependency graph has no risk.

[Dependency audit CI](../.github/workflows/security.yml) runs for manifest/lockfile changes, weekly on the default branch and on manual dispatch. It fails on RustSec vulnerability findings and retains the JSON report for 14 days, including on failure. Maintenance warnings remain visible in that report. Re-run before deployment; advisory data changes independently of the source.

## Evidence and limits

The [validation record](validation.md) records commands, real PostgreSQL/OpenSSH fixtures, screenshot captures and remote CI links. New screenshots are produced by the running native client with a disposable sample database and isolated workspace; cloud connection forms contain example values and perform no provider login.

This was a focused source/dependency review and regression pass. It did not include an external penetration test, memory-forensics assessment, real cloud-provider accounts, OS-keychain end-to-end checks or a formal effective-privilege analysis. See [production operations](production.md) for TLS/SSH defaults, credential migration, audit retention and resource limitations. Report further vulnerabilities using [the private security-reporting process](../SECURITY.md).
