# Extensions, scheduled jobs and auditing

[Documentation](README.md) · [Cloud authentication](cloud-authentication.md) · [PostgreSQL research](postgresql-research.md)

Right-click a database in the browser and open **Database tools**. The tools open editable SQL in a new tab; select the correct environment and run it when ready. Expanding the object tree does not install software, schedule jobs, change logging or restart a server. After an extension change, use **Refresh schema** to update browsing and completion.

The database tree now includes installed **Extensions**, **Event triggers** and **Publications**. An installed extension's context menu opens available versions/requirements, an update template or the full inventory. Schema **Types** participate in sidebar search, including when `public` contains types but no tables/functions.

## Extension workflow

1. Open **Extensions · available and installed**. It shows default/installed versions, schema, owner, trusted/superuser flags, dependencies and a `review_sql` column.
2. Open **Extension and scheduler prerequisites** to inspect CREATE privilege, recovery state, preload libraries, restart requirements and visible scheduler settings.
3. Confirm the extension package exists on the server, that the provider allows it, and whether preload/restart is required. An extension shown as available is not guaranteed to be installable by your role.
4. Copy the generated `CREATE EXTENSION` SQL into a query tab, review the target database/schema and run it with an appropriate role.
5. Refresh schema. For upgrades, inspect available versions and the extension's upgrade notes first; the generated `ALTER EXTENSION … UPDATE` selects the server's default target, not a promise that every upgrade path is supported.

Extension names are quoted as identifiers in generated SQL. No `CASCADE` is added automatically. The client does not download extension binaries to database servers. Most extension SQL objects live per database, even when their library/preload configuration applies to an entire instance. For trusted extensions PostgreSQL can allow installation with database CREATE privilege; other extensions or managed providers impose stronger requirements. See [CREATE EXTENSION](https://www.postgresql.org/docs/18/sql-createextension.html).

## Provider setup differs

Research checked on **25 October 2026**. Availability and versions depend on engine version, region, service tier and provider policy; inspect your actual instance.

| Deployment | pg_cron prerequisites | Audit prerequisites / log destination |
|---|---|---|
| Self-managed PostgreSQL | Install the matching server package; add `pg_cron` to `shared_preload_libraries`; select `cron.database_name`; restart | Matching pgAudit package and preload; install the extension, configure audit classes and a durable PostgreSQL log collector |
| Azure Database for PostgreSQL Flexible Server | Allow `pg_cron` in `azure.extensions`, preserve existing preload entries, add its library and restart | Allow/load/create `pgaudit`; set audit parameters; route PostgreSQL resource logs to Azure Monitor/storage/Event Hubs as required |
| Amazon RDS PostgreSQL | Custom parameter group with pg_cron preload; allowlist where configured; reboot; install with `rds_superuser` in the scheduler database | Parameter group with pgAudit preload, reboot, create extension and configure classes; arrange PostgreSQL log export/retention |
| Google Cloud SQL PostgreSQL | Enable `cloudsql.enable_pg_cron`; create the supported extension; size worker capacity | Enable `cloudsql.enable_pgaudit` (restarts instance), install/configure pgAudit and enable the required Cloud Logging Data Access policy |

Read existing parameter/flag values before updating them. Some provider CLI updates replace the entire list. Preserve unrelated libraries/flags and schedule disruptive restarts with your DBA. Trunq's SQL inspection cannot change control-plane flags or bypass provider allowlists.

Provider references: [Azure extension considerations](https://learn.microsoft.com/en-us/azure/postgresql/extensions/concepts-extensions-considerations), [RDS pg_cron](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/PostgreSQL_pg_cron.html), [Cloud SQL extensions](https://docs.cloud.google.com/sql/docs/postgres/extensions), [Cloud SQL pgAudit](https://docs.cloud.google.com/sql/docs/postgres/pg-audit).

## pg_cron in practice

Use the scheduler's metadata database, normally `postgres`; inspect `cron.database_name` where exposed. Upstream pg_cron is installed in one database per cluster; use `cron.schedule_in_database` for a different target database when supported. Managed services can customize packaging and behaviour, so follow their instructions. The server runs jobs even when Trunq is closed. Scheduler timezone and worker/connection capacity belong to the server, not your desktop timezone. [Upstream pg_cron](https://github.com/citusdata/pg_cron) documents its functions and settings.

After the DBA has completed preload/allowlist setup, **pg_cron · install template** opens:

```sql
CREATE EXTENSION pg_cron;
```

Grant only the intended scheduler users access to the `cron` schema and the target objects. Jobs execute with their configured database identity; scheduler access is not a grant to every table. On self-managed deployments choose an appropriate worker or libpq authentication configuration; do not weaken authentication to make jobs run. Cloud SQL uses background-worker mode.

### Schedule, inspect, pause and remove

The menu's **schedule a job** template prompts for `job_name`, `schedule` and `command` as bound parameters. Reusing a job name updates that user's existing named job. Job SQL is deliberately stored by the scheduler: do not embed passwords or sensitive literals in it.

A disposable smoke job might be:

```sql
SELECT cron.schedule('trunq-smoke', '0 * * * *', 'SELECT current_timestamp');
```

This runs hourly, not immediately. To inspect scheduling and recent outcomes, open **pg_cron · jobs and recent runs**. It reads `cron.job` and the most recent 200 `cron.job_run_details` records. Visibility is limited by permissions/RLS. An empty result is not proof that no other user has jobs. Run-history recording can be disabled and needs a retention policy.

To pause or remove one known job, use the matching menu template and provide its job ID:

```sql
-- Substitute the ID returned by cron.schedule; 123 is an example.
SELECT cron.alter_job(123, active := false);
SELECT cron.unschedule(123);
```

Pausing/removing a schedule does not roll back completed work or establish that an already-running job stopped. Inspect the current backend/run before intervening. `cron.alter_job` and these text-parameter templates target pg_cron 1.4+. Upstream revokes public EXECUTE on `cron.alter_job` and `cron.schedule_in_database`: schema USAGE alone is insufficient. Ask your DBA to review and grant the specific function only where appropriate. The prerequisites tool lists visible function signatures and your EXECUTE permissions. Use the version inventory before applying templates to an older installation; do not assume a provider grants the same functions by default.

For other target databases, review the provider's `cron.schedule_in_database` support and grants. A job running `VACUUM` or another command that disallows transaction blocks needs the scheduler's appropriate execution mode; a client's dry run is not a scheduler execution test. Review overlapping workloads, locks, timeouts, failure alerts and retention before using recurring production jobs. [RDS's scheduler guide](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/PostgreSQL_pg_cron.html) covers privileges, workers and run-history management.

## Auditing workflow

**Auditing · configuration inspection** reports the current login/effective role, database, application name, visible logging/pgAudit settings and installed audit/statistics extensions. **Security · roles, RLS and grants** reports role attributes, direct membership, table RLS flags, explicit schema ACLs and default ACLs. These are point-in-time inspection results. Hidden settings, a query error or incomplete privilege visibility must not be interpreted as a clean audit.

Trunq deliberately omits role password hashes, FDW options/user mappings and subscription connection strings from its supplied inspection queries. Users' own SQL and pg_cron command text can still contain sensitive data; review exports before sharing them.

### Server audit events

pgAudit writes through PostgreSQL's logging system. Install/preload the matching extension version before configuring its logging classes. The **pgAudit · install template** opens CREATE EXTENSION and a commented session-test setting:

```sql
-- After preload/restart and extension creation, test in a disposable session:
SET pgaudit.log = 'write,ddl,role';
-- Run representative permitted DML/DDL/role operations and verify server logs.
```

A session setting affects only that connection. For lasting coverage configure the required role/database/server scope through your DBA/provider, reconnect test clients, and verify the destination receives events. Starting with a narrow scope helps measure overhead and log volume. Consider parameter logging, statement contents and sensitive data before enabling wider classes; `pg_stat_statements` is aggregated performance telemetry, not a durable audit trail. [pgAudit's documentation](https://github.com/pgaudit/pgaudit) describes session/object logging and configuration; use the branch matching your PostgreSQL major version.

Keep database events separate from cloud control-plane events (for example changing a firewall, rotating a server credential or resizing an instance). Set up both where your audit requirements need them. Export logs to an independently controlled destination, define retention/access controls, and test missing-event alerts. Local query history is editable and clearable and cannot provide a tamper-resistant, complete server audit trail. There is no cloud log-ingestion/SIEM viewer in this update.

- **Azure:** configure PostgreSQL resource-log destinations; audit messages can be queried in Azure Monitor. Review settings again after major upgrades. [Azure auditing](https://learn.microsoft.com/en-us/azure/postgresql/security/security-audit).
- **RDS:** configure the custom parameter group, pgAudit extension and desired classes, then verify PostgreSQL log delivery. [RDS pgAudit setup](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/Appendix.PostgreSQL.CommonDBATasks.pgaudit.basic-setup.html).
- **Cloud SQL:** instance flag/preload is only one part; configure audit logging and the applicable Data Access logging policy, then verify Cloud Logging entries. [Cloud SQL pgAudit setup](https://docs.cloud.google.com/sql/docs/postgres/pg-audit).

## Other database objects and current limits

| Area | Available now | Remaining gap |
|---|---|---|
| Types, domains, enums, ranges, composites | Schema tree, details and sidebar search; server text representation in results | Dedicated type creation/alteration forms and complete type DDL extraction |
| Roles and tablespaces | Server tree, attributes/membership and copyable role DDL | Full privilege editor, ownership/dependency migration and tablespace creation wizard |
| Extensions | Installed tree, available-version/dependency inventory, reviewed create/update SQL | Automatic provider flag changes, binary installation and restart orchestration |
| Event triggers and publications | Database tree from the existing metadata snapshot | Full editing forms and publication table/row-filter details |
| Foreign data wrappers/servers | On-demand names/owners/table inventory | Secret-aware mapping editor; no options shown by default |
| Logical replication | Publication operations including TRUNCATE, slot and subscription-worker inspection | Subscription editor, connection-secret management, lag alerts and version-specific configuration UI |
| Auditing | Configuration and security inspections, install/test guidance | Complete effective-privilege analysis, log ingestion, policy enforcement and compliance reporting |
| Jobs | pg_cron SQL templates and recent-run inspection | Interactive calendar/job editor, execution alerts, provider-specific schedulers |
| Other catalogs | Normal SQL access | Dedicated views for collations, languages, operators/opclasses, casts, text search and extended statistics |

These gaps are explicit: connecting successfully is not proof of full administrative support for a managed service. The [research notes](postgresql-research.md) prioritize further work by capability and measured cost.

The function permission defaults are also visible in [pg_cron’s 1.4 migration](https://github.com/citusdata/pg_cron/blob/main/pg_cron--1.3--1.4.sql).
