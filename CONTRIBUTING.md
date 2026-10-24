# Contributing to Trunq

Thanks for helping make Trunq better! Bug reports, ideas, documentation and code are all welcome.

## Ground rules

- Be kind: see the [Code of Conduct](CODE_OF_CONDUCT.md).
- For anything larger than a small fix, please open an issue first so we can agree on the approach.
- Security problems: **do not** open a public issue: see [SECURITY.md](SECURITY.md).

## Getting started

```bash
# Rust stable (1.95+) via https://rustup.rs
git clone <this repository> trunq && cd trunq
cargo run            # start the app
cargo test           # unit tests
```

Linux needs a few system libraries: see the [README](README.md#linux).

### A database to play with

`dev/sample.sql` creates a realistic sample database:

```bash
createdb shop && psql -d shop -f dev/sample.sql
```

### Protocol tests against a real server

```bash
TRUNQ_TEST_DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres cargo test live_tests
```

CI runs these against PostgreSQL 13 to 18.

### Looking at the UI without clicking around

`TRUNQ_SCRIPT` drives the app and saves screenshots (see the README's *Development* section), e.g.

```bash
TRUNQ_SCRIPT="wait:3;run;wait:2;shot:/tmp/query.png;quit" cargo run
```

Please attach before/after screenshots to pull requests that change the UI.

## Code guidelines

- `cargo fmt` and `cargo clippy --all-targets -- -D warnings` must pass (CI enforces both, and builds
  with warnings denied).
- Keep the UI thread non-blocking: anything that touches the network or disk belongs in a session
  command or a background task, and reports back through a `Promise` or session event.
- Every connection must be owned by exactly one `Session`. Never clone senders that keep an actor
  alive: use `Session::handle()` (weak) for helpers.
- Metadata queries must not take relation locks (no `pg_get_expr`, `pg_get_viewdef`, … in bulk
  catalog queries) so browsing never blocks behind a migration.
- Add tests for parsing / SQL-generation logic (`sql/`, `export.rs`, `db/meta.rs` are good examples).
- Match the surrounding style: small focused functions, comments that explain *why*.

## Pull requests

1. Fork and create a branch from `main`.
2. Make your change with tests.
3. Update `CHANGELOG.md` under **Unreleased**.
4. Open the PR and fill in the template.

## Releasing (maintainers)

1. Bump `version` in `Cargo.toml` and move the **Unreleased** notes to a new version section in `CHANGELOG.md`.
2. Commit, then tag and push: `git tag v0.2.0 && git push origin v0.2.0`.
3. The **Release** workflow builds macOS (universal `.dmg`), Linux (`.deb`, `.tar.gz`) and Windows (`.zip`)
   packages and publishes them, with checksums, as a GitHub Release. Release notes come from `CHANGELOG.md`.
4. Optional macOS signing/notarization: add the `APPLE_*` secrets listed in `.github/workflows/release.yml`.
