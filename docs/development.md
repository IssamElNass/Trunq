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

### Releases

Releases are automated by [`.github/workflows/release.yml`](../.github/workflows/release.yml):

1. Bump `version` in `Cargo.toml` and add the notes to `CHANGELOG.md`.
2. `git tag v0.2.0 && git push origin v0.2.0`
3. GitHub Actions builds a universal macOS `.dmg` (and `.app.zip`), a Linux `.deb` and `.tar.gz`, and a Windows `.zip`,
   writes `SHA256SUMS.txt` and publishes them as a GitHub Release with the changelog section as release notes.
   Add the `APPLE_*` secrets to also sign and notarize the macOS build.

CI ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)) checks formatting and clippy (warnings denied), builds and tests on
macOS, Windows and Linux, and runs the protocol tests against PostgreSQL 13 to 18.
