# PostgreSQL research and implementation decisions

Reviewed 23 October 2026 against PostgreSQL's official documentation. PostgreSQL 18 is the stable documentation target; protocol compatibility tests also cover 13–17. Compatibility with 13 does not imply that PostgreSQL still maintains that server release.

| Area | Implemented in this update | Why / next step |
|---|---|---|
| Extended-query portals | A suspended cursor now marks its transaction as open. Closing it resynchronizes the protocol. Tests cover implicit and explicit transactions and error recovery. | `Flush` does not commit; `Sync` ends the implicit transaction. Holding a partially fetched result can retain a snapshot and locks. |
| Query statistics | Database context menu → **Slow queries (pg_stat_statements)** opens editable SQL with calls, total/mean execution time, rows and buffer/temp-block counters. | It is opt-in: Trunq does not install extensions or restart servers. The DBA must configure `shared_preload_libraries` and install the extension in that database. |
| Relationships | Database/schema/table context menus and the command palette open the existing ER diagram. | Reuses catalog data without another round trip; can export SVG and Mermaid. |
| Catalog visibility | Expanded tables show indexes, constraints, triggers, row security and policies. Server groups show roles and tablespaces, with copyable role DDL. | Makes already fetched metadata useful without extra polling or queries. |
| COPY | Regression coverage for COPY output and recovery after invalid COPY input; COPY output column metadata reaches the results UI. | Keep bulk import/export streamed. Avoid converting every field into separately allocated strings. |
| Diagnostics | Existing lock/progress and health tools remain available. | Statistics are observations, not proof an index is safe to drop. Visibility depends on the database role. |

## Candidates for a later measured change

- **`pg_stat_io` on PostgreSQL 16+**: a version-aware I/O dashboard. Preserve a fallback for older servers and label cumulative counters versus interval deltas.
- **`EXPLAIN (ANALYZE, BUFFERS, SERIALIZE TEXT)` on PostgreSQL 17+**: distinguish executor work from output serialization. `ANALYZE` executes the query; require the same review as the original SQL. Serialization measurements exclude network transmission.
- **`pg_stat_statements` dashboard**: discover the extension's schema, handle missing privileges and differences between server versions, and fetch only while visible.
- **Protocol pipelining**: benchmark independent metadata operations on high-latency connections first. Queries sharing transaction state and cursor lifetimes cannot be reordered indiscriminately.

These are research candidates, not implemented features. Client rendering, network transfer, server execution and desktop streaming need separate measurements; “fastest client” requires a reproducible comparison with other clients on the same workload.

## Primary sources

- [PostgreSQL protocol message flow](https://www.postgresql.org/docs/18/protocol-flow.html): extended queries, portals, Sync/Flush and COPY.
- [pg_stat_statements](https://www.postgresql.org/docs/18/pgstatstatements.html): setup, fields and visibility.
- [Monitoring statistics](https://www.postgresql.org/docs/18/monitoring-stats.html): activity and I/O statistics.
- [EXPLAIN](https://www.postgresql.org/docs/18/sql-explain.html): execution, buffers and serialization.
- [Docker multi-platform GitHub Actions](https://docs.docker.com/build/ci/github-actions/multi-platform/): Buildx, emulation and registry login.
- [Docker test before push](https://docs.docker.com/build/ci/github-actions/test-before-push/): smoke-test the desktop before publishing.

## Cloud-neutral administration follow-up (25 October 2026)

The client now separates token acquisition from PostgreSQL transport. Existing Entra browser/device/CLI flows gain per-user cache isolation; a native client-secret flow and a bounded external-helper contract cover application identities and multiple cloud CLIs. CA bundles/client certificates support servers whose trust/authentication policy differs from a public-CA password deployment. No provider SDK or control-plane credential is required for ordinary PostgreSQL browsing. [Implementation and exact limits](cloud-authentication.md).

Database tools expose existing cached extension/event-trigger/publication metadata and request further administrative queries only on demand. This avoids adding background catalog polling or privileged extension-version scans to every connection. Install/update/job changes open reviewable SQL. Provider allowlists, preload settings, server binaries and disruptive restarts remain explicit operational work. [Provider matrix, job and audit workflows](administration.md).

| Next capability | Value | Design requirement |
|---|---|---|
| Effective privilege report | Explain why a user can access an object | Account for ownership, PUBLIC, direct/inherited membership, SET ROLE rules, default ACLs, SECURITY DEFINER and RLS; role attributes alone are insufficient |
| Extension/job editor | Reduce copying SQL while retaining review | Discover installed function signatures/versions and provider constraints; preserve job ownership and bound values |
| Replication dashboard | Catch retained WAL and failed subscriptions | Use version-specific columns/permissions; never display stored connection secrets by default; distinguish lag from inactive slots |
| More object editors | Types/domains, collations, languages, operators, casts, text search and extended statistics | Show dependencies and complete DDL before mutation; avoid treating provider admin roles as real superusers |
| Audit log viewer | Correlate database and control-plane events | Separate read-only log credentials from database roles, expose retention/gaps and support provider adapters; local query history cannot substitute |
| Connection startup profile | Measure CLI, SSH, TLS and catalog costs separately | Preserve account selection and cancellation; do not introduce a shared PostgreSQL pool that leaks transactions/temp state between tabs |

The graph preparation cache is an independent client improvement: sorted X values, finite Y values and pixel-bucket extrema are reused until data, axes, kind or width changes. This avoids repeated parsing/sorting of loaded results during mouse movement. The first preparation still costs O(rows × series), sorting costs O(rows log rows), and cached chart values consume additional RAM. See [measured checks](performance.md); this does not accelerate server execution or network transfer.

Sources for the provider comparison: [Entra PostgreSQL setup](https://learn.microsoft.com/en-us/azure/postgresql/security/security-entra-configure), [Entra application credentials](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow), [RDS IAM](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/UsingWithRDS.IAMDBAuth.html), [Cloud SQL IAM](https://docs.cloud.google.com/sql/docs/postgres/iam-logins), [Azure extensions](https://learn.microsoft.com/en-us/azure/postgresql/extensions/concepts-extensions-considerations), [RDS pg_cron](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/PostgreSQL_pg_cron.html), [Cloud SQL extensions](https://docs.cloud.google.com/sql/docs/postgres/extensions), [pgAudit](https://github.com/pgaudit/pgaudit). These documents establish available mechanisms; local tests do not certify provider integrations.
