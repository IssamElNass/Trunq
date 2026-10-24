# Development and validation

[Documentation](README.md) · [Contribution guidelines](../CONTRIBUTING.md) · [Validation record](validation.md)

## Local checks

Use Rust 1.95 or newer and the [platform build dependencies](install.md#build-from-source). From the repository root:

```sh
cargo fmt --all --check
cargo build --locked
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
```

Two benchmarks are ignored by default. Database and SSH tests return early when their environment variables are absent; a green unit-test run alone does not verify those integrations.

## PostgreSQL integration tests

Point the tests at a disposable database owned by a role permitted to create schemas and temporary tables:

```sh
TRUNQ_TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres \
  cargo test --locked live_tests -- --test-threads=1
```

This includes protocol paging and error recovery, COPY, catalog queries, atomic table saves, failed export recovery, and CSV/SQL export round trips. Tests create fixtures; do not point them at a production database. CI runs PostgreSQL 13–18. SSH needs its separately configured test environment; Azure and operating-system keychains need their respective services.

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
  `tab:<n>`, `palette:<query>`, `health`, `sessions`, `structure:<schema.table>`, `erd[:schema]`, `resize:<width>,<height>`, `chart`, `card`, `compare`, `value:<row>,<col>`,
  `pin`, `side`, `search:<text>`, `ts:utc|local|raw`, `finder`, `hideempty`, `import:<file>`, `importrun`, `newconn`,
  `connections`, `close`, `theme:light|dark`, `shot:<file.png>`, `quit`. Scripted runs never overwrite your saved workspace.

## Release and nightly builds

The [Release workflow](../.github/workflows/release.yml) uses one set of packaging jobs for stable tags, nightly snapshots and packaging previews. Source is resolved to one commit before platform jobs start; all platforms check out that exact SHA.

### Stable releases

1. Update `version` in `Cargo.toml`, keep `Cargo.lock` in sync, and move the relevant **Unreleased** notes into a matching section in `CHANGELOG.md`.
2. Complete the normal checks and commit the release changes.
3. Create and push the matching tag, for example `git tag v0.2.0 && git push origin v0.2.0` for package version `0.2.0`.
4. The workflow verifies the tag/version match, builds a universal macOS `.dmg` and `.app.zip`, a Linux `.deb` and `.tar.gz`, and a Windows `.zip`.
5. After every platform succeeds, it assembles `SHA256SUMS.txt` and release notes, then publishes the GitHub Release. A Cargo prerelease version produces a GitHub prerelease.

To rebuild an existing tag, run **Actions → Release → Run workflow** from the updated default branch and enter that tag as the `tag` input. The workflow's metadata tooling is loaded separately from the source tag, so old tags do not need to contain the new nightly helper. Add the `APPLE_*` secrets listed in the workflow to enable signing and notarization; otherwise macOS uses the bundle script's ad-hoc signing.

### Daily nightly snapshots

| Item | Behaviour |
|---|---|
| Trigger | Daily at 02:00 UTC; manual dispatch with `tag=nightly` |
| Source | Latest default-branch source for scheduled/manual nightly builds, fixed to one SHA for every package |
| Identity | `nightly-YYYYMMDD-<run-number>-g<short-sha>` |
| Checks | Unit tests in each platform job, followed by all platform release builds and packaging |
| Downloads | Same platform formats as stable, plus `BUILD_INFO.txt` and `SHA256SUMS.txt` |
| Publication | A separate immutable prerelease; it does not become the latest stable release |
| Debian version | `<base-version>~nightly.<date>.<run>+g<sha>`, which sorts before the matching stable version |
| Actions artifact retention | 7 days for snapshots; release assets remain on the GitHub Release |

The schedule starts only once this workflow is on the default branch. GitHub scheduled events may be delayed. Each successful run has a distinct tag, so the workflow does not move an older nightly tag or delete previous releases. Failed builds leave previous published snapshots available.

The app's displayed version remains the Cargo source version. Identify a snapshot from its filename, release commit and `BUILD_INFO.txt`. Nightlies are application snapshots built with the stable Rust toolchain; they do not require Rust nightly. Unit tests without database/SSH fixtures do not exercise those integrations; the separate CI protocol matrix covers PostgreSQL 13–18.

### Packaging previews and workflow maintenance

Pull requests changing the release workflow, metadata helpers/tests or macOS bundle script build snapshot packages and assemble a `release-bundle` artifact. Publication is skipped on pull requests, and macOS signing/notarization is skipped there too. Preview artifacts require access to that Actions run and are retained for seven days.

The metadata step has Python 3.12 configured; local checks require Python 3.11 or newer. Run its standard-library checks locally:

```sh
python3 -m unittest discover -s scripts -p test_release_metadata.py -v
```

Run `actionlint .github/workflows/release.yml` when actionlint is available. Keep `scripts/release-metadata.py` responsible for channel routing, version validation, exact source identity and Debian ordering; do not duplicate those decisions across platform jobs. The assembly job creates provenance, checksums and notes before the publishing job receives write permission.

If a nightly is missing, inspect **Actions → Release** for a failed or queued job, check that scheduled workflows are enabled, and confirm that the workflow exists on the default branch. A green package preview does not mean a prerelease was published. Maintainers can request a fresh default-branch snapshot with a manual `nightly` dispatch.

CI ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)) separately checks formatting and Clippy, builds and tests on macOS, Windows and Linux, checks the website, and runs protocol tests against PostgreSQL 13–18.
