# Validation record

## 25 October 2026: production hardening and screenshot follow-up

Linux cloud environment, Rust 1.95.0, PostgreSQL 17.11, a separate pg_cron 1.6.5/pgAudit 17.1 TLS fixture and a real OpenSSH 10.0p2 bastion. Only disposable sample data and test identities were used.

| Check | Result |
|---|---|
| `cargo test --locked --all-targets -- --test-threads=1` | 104 passed, 0 failed, 3 opt-in benchmarks ignored; ordinary database configured (14 tests actually connect) |
| Format, locked build and Clippy with warnings denied | Passed |
| Credential/storage regressions | Endpoint/default-database/bastion changes isolate secrets; harmless label edits preserve them; query/sidebar prompts discard unsent input; repeated SQL compacts disk history; old history retains its latest window; Unix pgpass permissions, private atomic writes and symlink resistance verified |
| Protocol resource checks | Oversized headers rejected; real 1,500-notice PostgreSQL flood bounded and queue drain resets the budget; notification overflow bounded with feedback |
| Real TLS/admin fixture | 21 live-test entries passed, 1 benchmark ignored (some entries are local checks or return early); custom CA/hostname rejection, required client certificate, restricted-role extension enable/upgrade, explicit dependencies, bound pg_cron schedule/pause/remove and pgAudit writes exercised |
| Restricted role with pg_cron loaded | Initially failed because `current_setting('cron.database_name',true)` still enforces privilege checks; switched to visibility-aware `pg_settings` lookup. The restricted-role regression now explicitly requires successful inventory loading and passes. |
| Real OpenSSH tunnel | Two database channels shared one SSH login; cancellation delivered; the final close left zero strong cached handles. Repeated successfully after RSA removal and private host-key persistence. |
| `cargo-audit 0.22.2` | Zero vulnerable/yanked packages; four upstream maintenance warnings remain. RustSec database revision `21d909e4c8bd4e14c7aea58bf980c881a0053ab1`. [Findings and compatibility changes](security-review.md#dependency-audit). |
| Native screenshots | Nine new views captured with `scripts/screenshots.py --suite new`, isolated config unchanged; PNGs and WebPs linked from the guides/handbook. Cloud forms use illustrative values and perform no login. |
| Markdown | 271 local page/heading/image links resolve across 25 Markdown files; checker now runs in CI |
| Chromium | 13 browser groups passed, including new production search/navigation and requests for all 14 handbook screenshots, with desktop/mobile themes and existing copy/menu/FAQ behavior |
| Release metadata | Five routing/identity tests passed; existing nightly schedule remains daily at 02:00 UTC once on the default branch |

[PR #10 checks](https://github.com/IssamElNass/Trunq/pull/10/checks) report the latest remote revision. The previous `49758ab` checkpoint passed [all eleven CI jobs](https://github.com/IssamElNass/Trunq/actions/runs/40142829479) and [Docker smoke](https://github.com/IssamElNass/Trunq/actions/runs/40142829468); those runs predate this hardening. A new dependency-audit workflow adds a separate check.

No live Azure/AWS/Google identity, OS-keychain integration or production provider policy was exercised. Tests do not establish absence of all leaks. The [production guide](production.md) documents one-time secret re-entry, RSA compatibility, local unencrypted SQL/history and remaining resource/audit limitations.

## 25 October 2026: cloud authentication, extension manager and chart iteration

Checks ran in the Linux cloud environment with Rust 1.95.0, disposable PostgreSQL 17.11 databases and a separate TLS/extension fixture. Actual provider logins were not configured.

| Check | Result |
|---|---|
| `cargo test --locked --all-targets -- --test-threads=1` | 90 passed, 0 failed, 3 opt-in benchmarks ignored |
| Ordinary database fixtures | 13 tests connected, including two extension-manager tests; the fake TLS-stall test runs separately without PostgreSQL |
| Extension manager, restricted role | Permission denial recovered; after explicit grants a non-superuser installed hstore 1.4 into its chosen schema, upgraded to 1.8, and the manager refreshed; read-only guards blocked further apply requests |
| Extension dependencies | earthdistance failed without cube; explicit dependency inclusion enabled both; the test removed only its own fixtures |
| Real TLS/mTLS fixture | Custom CA trusted, wrong hostname/untrusted CA rejected, missing client certificate refused, certificate-authenticated `cert_user` connected |
| pg_cron 1.6.5 and pgAudit 17.1 fixture | Bound schedule/pause/remove ran as a restricted role with explicit function permission; a separate one-second server job completed `SELECT 1`; schedules were removed; pgAudit DDL/WRITE log entries observed |
| Native UI | 50,000-row chart and administration palette rendered; the running extension manager searched 47 available entries and enabled hstore 1.8 through its button; installed state and success feedback refreshed |
| Native cleanup | No Trunq sessions remained after exit; the native hstore smoke extension and test roles/schedules were removed |
| Website | Twelve Chromium browser groups passed; new cloud/admin section anchors align on mobile/desktop and homepage search navigates to the cloud guide |
| Documentation | Local links, headings and images resolve, including the new cloud-authentication, administration and extension quick-start guides |
| Million-row release chart benchmark | 62.630 ms initial preparation; 0.053 ms for 1,000 cache validations; [workload and limits](performance.md#chart-preparation-cache-25-october-2026) |

The earlier cloud/authentication checkpoint `f907238` passed [all eleven CI jobs](https://github.com/IssamElNass/Trunq/actions/runs/40141494718), including macOS/Windows/Linux, Clippy, the website and PostgreSQL 13–18. Its [Docker desktop smoke run](https://github.com/IssamElNass/Trunq/actions/runs/40141494689) also passed. These dated runs precede the final extension-manager follow-up; [PR #10 checks](https://github.com/IssamElNass/Trunq/pull/10/checks) track its current revision.

Live Azure/AWS/Google authentication, real provider allowlists/restarts, SSH and OS keychains were not exercised locally. The ordinary suite returns early for the optional TLS/pg_cron fixtures unless their variables are provided; they were run separately above. Installing SQL objects does not configure provider flags or durable audit-log collection.

## 24 October 2026: regression and documentation pass

These checks ran in the Linux cloud environment with Rust 1.95.0 and a disposable PostgreSQL 17.11 database. They do not establish results for other operating systems or the full CI matrix.

| Check | Result |
|---|---|
| Baseline `cargo test --locked --all-targets -- --test-threads=1` | 56 passed, 0 failed, 2 opt-in benchmarks ignored |
| Updated `cargo test --locked --all-targets -- --test-threads=1` | 73 passed, 0 failed, 2 opt-in benchmarks ignored |
| PostgreSQL integration checks | 10 database tests connected: protocol/COPY/catalog checks, export round trips, table-save recovery, dry-run rollback and failed/cancelled exports |
| `cargo build --locked`, `cargo fmt --all --check`, `cargo clippy --locked --all-targets -- -D warnings` | Passed |
| `TRUNQ_CHROMIUM=/usr/bin/chromium node scripts/check-website.cjs` | 12 browser groups passed: layouts/themes, anchors, keyboard tabs, clipboard fallback, search, FAQ, mobile menus and restricted storage |
| Native scripted UI in the virtual display | Query returned 100 sample rows; table browsing and both themes rendered correctly; a dry-run preview showed 20 and the next query confirmed the original value 10; app exited successfully |
| Workspace and connection cleanup | Scripted run left the saved configuration hash unchanged; no Trunq sessions remained in `pg_stat_activity` after exit |
| Documentation reorganization | Local page/heading links and image paths checked after moving the detailed material into separate guides |

The new regressions cover CSV NULL versus empty text, SQL non-finite values, preserving existing export files, transaction/savepoint recovery, WHERE clauses at the correct query level, whole-script selection behavior, Unicode diagnostic offsets, stale completion ranges and CSV inference. Export, destructive-query and stale-completion tests failed before their fixes. CSV and SQL exports were also imported back into PostgreSQL to check the values. A headless editor test drives the comment shortcut and checks that it closes the outdated completion popup.

The SSH test still returns early without its external fixture. Real SSH, Azure, keychains, macOS, Windows and other PostgreSQL versions were not exercised in this pass. Website checks are now configured in CI; the local run above used Chromium, not a remote Actions runner.

### Documentation and nightly-build follow-up

- The additional connection, settings/backup, workflow and troubleshooting guides were checked against the current source. Their local page, heading and image links resolve.
- The practice-table, bound-value, transaction/rollback and chart SQL examples ran successfully against the disposable PostgreSQL 17 database.
- `python3 -m unittest discover -s scripts -p test_release_metadata.py -v`: five release-routing tests passed, covering stable tag validation, prerelease detection, nightly identity, repeated runs and invalid input.
- `actionlint` 1.7.7 accepted the updated Release workflow. A CLI metadata run produced a dated nightly identity with an exact source SHA, and `dpkg --compare-versions` confirmed the nightly sorts before its matching stable version.
- The twelve Chromium browser groups passed after expanding the handbook, FAQ, settings and nightly documentation.

The [Release preview for source revision 2b87ab5](https://github.com/IssamElNass/Trunq/actions/runs/40087506934) subsequently passed: source/channel resolution, macOS universal, Linux x86_64, Windows x86_64 and package assembly. Assembly checked the presence of all five platform archives/installers and produced build provenance and checksums. Its `release-bundle` Actions artifact contains the reviewable downloads; publication was skipped on the pull request.

A scheduled nightly starts only after the workflow is on the default branch. This preview did not publish a public nightly release. The same revision's ordinary CI completed all eleven jobs successfully, and its Docker desktop smoke test passed.

## 23 October 2026: earlier validation

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

Real Azure, OS-keychain, Windows and arm64 runtime tests require their respective environments and credentials; they were not exercised locally. A real OpenSSH bastion was exercised in the latest follow-up above. The SSH live test returns without connecting when its environment is absent. Unit coverage of helper code is not a substitute for those integrations.

[The protocol CI matrix](../.github/workflows/ci.yml) covers PostgreSQL 13–18. [Docker CI](../.github/workflows/docker.yml) verifies an amd64 browser client before enabling a multi-platform registry publication. Registry credentials were not configured or used by this local check.

The native macOS close check initially reproduced the AppKit Touch Bar observer crash described in [egui #2768](https://github.com/emilk/egui/issues/2768). The client now selects eframe’s standalone exit path on macOS, which saves and destroys application state before exiting.

## 25 October 2026: grants, extensions, pg_cron and release downloads

Linux, Rust 1.95, PostgreSQL 17, plus a separate disposable PostgreSQL 17 server with pg_cron 1.6.5 preloaded. Source, tests, SQL paths and native UI were reviewed for automatic mutations, SQL quoting, permissions and repeated work.

| Check | Result |
|---|---|
| Build, rustfmt and Clippy with warnings denied | Passed |
| Full suite with database and pg_cron fixtures | 109 passed, 0 failed; four opt-in benchmarks also run separately and passed |
| Release metadata / direct download links | Six Python checks passed; actionlint 1.7.12 passed |
| Documentation and public handbook | 279 local links/images and 13 browser checks passed; the privilege reference explains SELECT/read, write rights, ALL expansion and grant option; pg_cron instructions match the new UI |
| Grants integration | Tables, views, materialized views, columns, sequences, routines, enum/composite types, schemas, original grantor, PUBLIC, grant options, RLS and future defaults checked; generated GRANT / REVOKE statements executed in savepoints |
| Role tracing and restricted readers | Nested role paths, NO INHERIT and membership cycles checked; restricted readers can inspect catalog ACLs omitted by information_schema |
| pg_cron integration | Pause / enable / schedule / command changes affect the selected job only; quoting round trips; restricted settings and missing schema privileges do not break detection; row security hides other roles' jobs |
| Mutation boundary | Inspector reads use repeatable-read, read-only transactions. New grant and cron mutation actions generate SQL with `run: false`. The existing extension manager retains its explicit Apply action and production/read-only checks. Cron does not change database or run-as role |
| Native UI | Grants, defaults, memberships, extensions and cron rendered; dark/light themes, a 1024×768 window, and a read-only connection inspected. Screenshots use a separate preview database; all preview jobs are paused |
| Performance | The [same-environment checks](performance.md#25-october-2026-grants-inspector-and-query-path-check) retain the existing query-cache behavior and measure virtualized frames for 100,000 grants |

No critical issue remained in the paths exercised by these checks. The permission review found and fixed pg_cron detection failures for roles lacking schema USAGE or access to restricted server settings. Catalog processing moved off the UI thread, and grant facets, filters and role paths avoid repeated scans. The inventory is an ACL snapshot, not a historical audit or a complete effective-access calculation.

The final work is integrated with main commit `9bf1377`, preserving its extension manager and security changes. macOS / Windows packaging, signing and the full PostgreSQL 13–18 matrix require CI. Local pg_cron integration needs `TRUNQ_TEST_CRON_DATABASE_URL`; CI's ordinary PostgreSQL service does not preload that extension. SSH, Azure and OS keychains were not exercised here. The release workflow still skips publication and signing for pull requests; adding the EXE does not publish a release from this review.


## Docker browser client checkpoint — 2026-10-26

The Docker default now builds the headless browser backend, with embedded HTML/CSS/JavaScript, instead of a virtual desktop. Local validation used that backend directly on macOS and an isolated PostgreSQL 17.10 server; it did not run inside Docker.

- Native tests: 109 passed, four ignored. Headless tests: 63 passed, one ignored. Optional live-provider tests without their environment variables returned early.
- Both headless and combined desktop/web Clippy checks passed with warnings denied; formatting and Compose configuration validation passed.
- The HTTP integration check passed against PostgreSQL: cookie/session isolation, CSRF/host rejection, catalog browsing while a query cursor is open, 1,201-row paging, transaction errors/recovery, cancellation and startup read-only mode. Saved connection JSON rejects password fields.
- Chromium passed query/paging, bounded grid rendering, filtering, loaded-row CSV download, HTML escaping, error recovery, workspace/session restore, light/dark themes and 390/768/1440-pixel layouts. The updated screenshot is from this real browser session. All 13 handbook/website browser checks and the Markdown link checker passed.
- cargo-audit 0.22.2 reported zero vulnerabilities. Existing maintenance warnings remain for ttf-parser, zbus_names, zvariant and zvariant_derive, plus a yanked crypto-bigint version in the existing lockfile. These are not introduced by the new HTTP dependencies.
- The Docker image build/runtime check could not run locally: the installed Colima VM failed during startup. The updated Docker workflow runs the API and browser checks against an isolated PostgreSQL 18 container before publication. This branch has not published an image.

The browser client currently covers connections, table browsing and SQL/query workflows. It does not claim native feature parity; [the Docker guide](docker.md#browser-workflow-and-current-scope) lists the remaining desktop-only features and session/query limits.
