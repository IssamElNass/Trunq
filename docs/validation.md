# Validation record

23 October 2026. Commands were run locally; the CI matrix is configuration, not a claim that every CI runner was executed here.

| Check | Result |
|---|---|
| `cargo test --locked --all-targets` with a disposable PostgreSQL 17.10 database | 53 passed, 0 failed, 2 opt-in benchmarks ignored |
| PostgreSQL 18 Docker server: `cargo test --locked db::wire::live_tests -- --test-threads=1` | 6 passed; the opt-in fetch benchmark ignored |
| Trackpad scroll regression, 5,000 rows growing to 8,000, followed by decaying momentum | Failed before the hard-boundary fix; passes after it |
| `cargo fmt --all --check` and Clippy with `-D warnings` | Passed, no warnings |
| Million-row release benchmarks | Both passed; [workload and measured results](performance.md) |
| Website and docs in Chromium at 1440 and 390 pixels | No JavaScript errors, missing HTTP resources or page-wide horizontal overflow; palette search and theme persistence passed |
| Local HTML resource paths and duplicate IDs | Passed |
| Shell entrypoint syntax and Compose configuration | Passed |
| Built Linux Docker desktop, non-root client, HTTP health check and noVNC WebSocket | Passed; Chromium received the 1440×900 desktop |
| Docker desktop querying PostgreSQL 18.6 on an isolated network | Connected, 100 sample rows displayed; browser and query sessions visible in `pg_stat_activity` |
| Native macOS release screenshot script and application close | Twelve fresh screenshots, fixture configuration unchanged, process exited successfully |

The twelve native client views are captured by [the screenshot script](../scripts/screenshots.py), using only sample data and a temporary configuration. The gallery includes query results, JSON, table editing, card/compare views, dark/light diagrams, structure, sessions, health, palette and a compact window. [The Docker screenshot](screenshots/docker.png) also comes from the running client, connected to a separate PostgreSQL container.

## Scope

Real Azure, SSH bastion, keychain, Windows and arm64 runtime tests require their respective environments and credentials; they were not exercised locally. The SSH live test returns without connecting when its environment is absent. Unit coverage of helper code is not a substitute for those integrations.

[The protocol CI matrix](../.github/workflows/ci.yml) covers PostgreSQL 13–18. [Docker CI](../.github/workflows/docker.yml) verifies an amd64 desktop before enabling a multi-platform registry publication. Docker Hub credentials have not been configured or used by this local work.

The native macOS close check initially reproduced the AppKit Touch Bar observer crash described in [egui #2768](https://github.com/emilk/egui/issues/2768). The client now selects eframe’s standalone exit path on macOS, which saves and destroys application state before exiting.
