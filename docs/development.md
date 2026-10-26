# Development and validation

[Documentation](README.md) · [Contribution guidelines](../CONTRIBUTING.md) · [Validation record](validation.md)

## Local checks

Use Rust 1.95 or newer and the [platform build dependencies](install.md#build-from-source). From the repository root:

```sh
cargo fmt --all --check
cargo build --locked
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
python3 scripts/check-docs.py
```

Three benchmarks are ignored by default. Database and SSH tests return early when their environment variables are absent; a green unit-test run alone does not verify those integrations.

## PostgreSQL integration tests

Point the tests at a disposable database owned by a role permitted to create schemas, temporary tables and test roles:

```sh
TRUNQ_TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres \
  cargo test --locked live_tests -- --test-threads=1
```

This includes protocol paging and error recovery, COPY, catalog queries, atomic table saves, failed export recovery, CSV/SQL export round trips and the extension manager's trusted-extension install/upgrade/recovery with a restricted role. The latter needs an absent `hstore` or `pg_trgm` extension and its server files; it cleans up only its own extension/schema/role fixtures. Tests create fixtures; do not point them at a production database. CI runs PostgreSQL 13–18 with TLS and SCRAM authentication. Set `?sslmode=require` on the disposable test URL to exercise the TLS SCRAM regression locally; the fixture must enable TLS and use SCRAM for host logins. SSH needs its separately configured test environment; Azure and operating-system keychains need their respective services.

Optional fixture checks:

- `TRUNQ_TEST_ADMIN_URL`: a disposable database with pg_cron and pgAudit preloaded/installed. `cargo test --locked scheduler_templates_bind -- --nocapture` checks parameter binding, pause/remove and audit settings without leaving a job behind.
- `TRUNQ_TEST_TLS_CA`, `TRUNQ_TEST_TLS_CERT`, `TRUNQ_TEST_TLS_KEY`: PEM CA/client chain and unencrypted PKCS #8 key. Point `TRUNQ_TEST_DATABASE_URL` at a TLS-enabled server reached as `localhost`, whose server certificate covers only `localhost`, and configure certificate-only authentication for role `cert_user` (certificate CN). `cargo test --locked custom_ca_and_client_certificate_authentication` verifies chain/hostname failures and client authentication. Include IPv4/IPv6 local access in the fixture.
- Without those variables, these optional tests return early. The ordinary suite still tests helper bounds, SSL timeouts, argument substitution and restricted-role catalog queries. Real provider logins/CLI selection require separate cloud infrastructure.

For pg_cron integration, supply `TRUNQ_TEST_CRON_DATABASE_URL` for a separate disposable superuser database with pg_cron in `shared_preload_libraries` and matching `cron.database_name`. The test creates role/job fixtures in a rolled-back transaction; no active fixture is committed. It checks scoped edits, literal escaping, restricted roles, missing schema privileges and row security:

```sh
TRUNQ_TEST_CRON_DATABASE_URL=postgres://postgres@localhost:5432/postgres \
  cargo test --locked pg_cron -- --nocapture
```

The [performance guide](performance.md) explains the opt-in benchmarks and their measurement limits. The [website guide](../website/README.md#check-website-changes) covers the browser checks, including the handbook's mobile navigation, search and copy controls.

## Code layout and scripted UI

```
src/
  db/        wire protocol client, sessions (async actors), catalog, SSH tunnels, Azure Entra auth
  sql/       lexer, autocompletion, formatter, migration linter
  ui/        theme, editor, data grid, JSON tree, charts, connection and import dialogs, explain view
  tabs/      query and table tab (results in tabs/query/), live monitor, health, sessions, structure
  app.rs     window layout, sidebar, tabs, command palette, dialogs
```

- `cargo test` runs the unit tests: protocol parsing, SQL splitting, completion, formatter, linter, export, CSV inference, Azure helpers and more.
- `TRUNQ_TEST_DATABASE_URL=postgres://… cargo test --locked live_tests -- --test-threads=1` runs protocol tests against a real server. The CI matrix covers PostgreSQL 13 to 18.
- Scripted UI runs for visual checks, without any OS permissions:

  ```bash
  TRUNQ_SCRIPT="wait:3;run;wait:2;shot:/tmp/query.png;open:shop.orders;wait:2;shot:/tmp/table.png;quit" cargo run
  ```

  Regenerate the sample gallery with `TRUNQ_TEST_DATABASE_URL=postgres://… python3 scripts/screenshots.py` after loading `dev/sample.sql` into a disposable database. The script uses an isolated temporary configuration and converts images to WebP when `cwebp` is installed.

  Steps: `wait:<s>`, `run`, `runall`, `dryrun`, `explain[:analyze]`, `sql:<text>` (separate with `;;`), `open:<schema.table>`,
  `tab:<n>`, `palette:<query>`, `health`, `grants`, `cron`, `inspect:view=objects|defaults|memberships`, `inspect:role=<name>`, `inspect:search=<text>`, `sessions`, `structure:<schema.table>`, `erd[:schema]`, `resize:<width>,<height>`, `chart`, `card`, `compare`, `value:<row>,<col>`,
  `pin`, `side`, `bottom`, `stoploading`, `search:<text>`, `ts:utc|local|raw`, `finder`, `hideempty`, `import:<file>`, `importrun`, `newconn[:aws|azure|tls|ssh]`, `extensions[:name]`,
  `connections`, `close`, `theme:light|dark`, `shot:<file.png>`, `quit`. Scripted runs never overwrite your saved workspace.

## Release and nightly builds

The [Release workflow](../.github/workflows/release.yml) uses one set of packaging jobs for stable tags, nightly snapshots and packaging previews. Source is resolved to one commit before platform jobs start; all platforms check out that exact SHA.

### Stable releases

1. Update `version` in `Cargo.toml`, keep `Cargo.lock` in sync, and move the relevant **Unreleased** notes into a matching section in `CHANGELOG.md`.
2. Complete the normal checks and commit the release changes.
3. Create and push the matching tag, for example `git tag v0.2.0 && git push origin v0.2.0` for package version `0.2.0`.
4. The workflow verifies the tag/version match, builds a universal macOS `.dmg` and `.app.zip`, a Linux `.deb` and `.tar.gz`, and a Windows portable `.exe` plus `.zip`.
5. After every platform succeeds, it assembles `SHA256SUMS.txt` and release notes, then publishes the GitHub Release. A Cargo prerelease version produces a GitHub prerelease.

To rebuild an existing tag, run **Actions → Release → Run workflow** from the updated default branch and enter that tag as the `tag` input. The workflow's metadata tooling is loaded separately from the source tag, so old tags do not need to contain the new nightly helper. Add the `APPLE_*` secrets listed in the workflow to enable signing and notarization; otherwise macOS uses the bundle script's ad-hoc signing.

### Daily nightly snapshots

| Item | Behaviour |
|---|---|
| Trigger | Daily at 02:00 UTC; manual dispatch with `tag=nightly` |
| Source | Latest default-branch source for scheduled/manual nightly builds, fixed to one SHA for every package |
| Identity | `nightly-YYYYMMDD-<run-number>-g<short-sha>` |
| Checks | Unit tests in each platform job, followed by all platform release builds and packaging |
| Downloads | Same platform formats as stable, with direct download links, plus `BUILD_INFO.txt` and `SHA256SUMS.txt` |
| Publication | A separate dated prerelease; it does not become the latest stable release |
| Debian version | `<base-version>~nightly.<date>.<run>+g<sha>`, which sorts before the matching stable version |
| Actions artifact retention | 7 days for snapshots; release assets remain on the GitHub Release |

The schedule starts only once this workflow is on the default branch. GitHub scheduled events may be delayed. New workflow runs have distinct snapshot tags, so the workflow does not move older nightly tags or delete previous releases. Assets upload to a draft before publication. Once published, an immutable release cannot accept replacement or additional assets; start a new manual dispatch for a fresh snapshot instead of re-running a published identity. Failed platform builds leave previous published snapshots available.

The app's displayed version remains the Cargo source version. Identify a snapshot from its filename, release commit and `BUILD_INFO.txt`. Nightlies are application snapshots built with the stable Rust toolchain; they do not require Rust nightly. Unit tests without database/SSH fixtures do not exercise those integrations; the separate CI protocol matrix covers PostgreSQL 13–18.

### Packaging previews and workflow maintenance

Pull requests changing the release workflow, metadata/download helpers and tests or macOS bundle script build snapshot packages and assemble a `release-bundle` artifact. Publication is skipped on pull requests, and macOS signing/notarization is skipped there too. Preview artifacts require access to that Actions run and are retained for seven days.

The metadata step has Python 3.12 configured; local checks require Python 3.11 or newer. Run its standard-library checks locally:

```sh
python3 -m unittest discover -s scripts -p 'test_release_*.py' -v
```

Run `actionlint .github/workflows/release.yml` when actionlint is available. Keep `scripts/release-metadata.py` responsible for channel routing, version validation, exact source identity and Debian ordering; do not duplicate those decisions across platform jobs. The assembly job creates provenance, checksums and notes before the publishing job receives write permission.

If a nightly is missing, inspect **Actions → Release** for a failed or queued job, check that scheduled workflows are enabled, and confirm that the workflow exists on the default branch. A green package preview does not mean a prerelease was published. Maintainers can request a fresh default-branch snapshot with a manual `nightly` dispatch.

CI ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)) separately checks formatting and Clippy, builds and tests on macOS, Windows and Linux, checks the website, and runs protocol tests against PostgreSQL 13–18.

## Dependency and production checks

Install the pinned scanner outside the project dependencies, then audit the locked graph:

```sh
cargo install cargo-audit --version 0.22.2 --locked
cargo audit
python3 scripts/check-docs.py
```

[Audit CI](../.github/workflows/security.yml) also runs weekly and retains its JSON result. Review maintenance warnings as well as vulnerability failures; [the current findings](security-review.md#dependency-audit) record the remaining upstream work. Do not add blanket advisory ignores to make a build green.

The SSH fixture uses `TRUNQ_TEST_SSH=user@host:port|/absolute/path/to/ed25519_key` plus `TRUNQ_TEST_DATABASE_URL` as seen from the bastion. The live test checks two channels sharing one login, cancel delivery and releasing the final handle. Use a disposable bastion with verified raw host keys; RSA and SSH host certificates are unsupported. The protocol live suite also exercises diagnostic flooding and queue reset.

To capture only the nine new extension/authentication/audit/chart views, run `python3 scripts/screenshots.py --binary target/debug/trunq --suite new` with a display and the sample database. The default suite captures 21 views. Example cloud forms do not log in or save profiles; extension captures inspect inventory without enabling/updating anything. Review every image for sensitive data before committing.
