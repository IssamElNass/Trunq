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
