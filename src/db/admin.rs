//! On-demand database administration queries. Nothing here changes a server
//! merely by expanding the browser; all recipes open in an editable SQL tab.

pub struct Recipe {
    pub label: &'static str,
    pub sql: &'static str,
}

pub const EXTENSIONS: &str = r#"-- Available on this server does not imply allowed by your cloud provider.
-- Copy a command to a query tab and review privileges, preload and dependencies.
SELECT a.name, a.default_version, a.installed_version,
       n.nspname AS installed_schema, pg_get_userbyid(e.extowner) AS owner,
       v.superuser, v.trusted, v.relocatable, v.schema AS required_schema,
       v.requires AS dependencies, a.comment,
       CASE WHEN a.installed_version IS NULL
            THEN format('CREATE EXTENSION %I;', a.name)
            ELSE format('ALTER EXTENSION %I UPDATE;', a.name) END AS review_sql
FROM pg_available_extensions a
LEFT JOIN pg_extension e ON e.extname = a.name
LEFT JOIN pg_namespace n ON n.oid = e.extnamespace
LEFT JOIN pg_available_extension_versions v ON v.name = a.name AND v.version = a.default_version
ORDER BY (a.installed_version IS NOT NULL) DESC, a.name;
"#;

pub const EXTENSION_PREFLIGHT: &str = r#"-- Missing/hidden settings are not proof a feature is disabled.
SELECT current_database() AS database, current_user AS role,
       has_database_privilege(current_database(), 'CREATE') AS can_create_in_database,
       pg_is_in_recovery() AS in_recovery,
       current_setting('server_version_num')::integer AS server_version_num;
SELECT name, setting, unit, context, pending_restart
FROM pg_settings
WHERE name IN ('shared_preload_libraries', 'max_worker_processes', 'max_replication_slots', 'wal_level',
               'azure.extensions', 'cloudsql.enable_pg_cron', 'cloudsql.enable_pgaudit')
   OR name LIKE 'cron.%'
ORDER BY name;
SELECT p.oid::regprocedure AS scheduler_function,
       has_function_privilege(p.oid, 'EXECUTE') AS can_execute
FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
WHERE n.nspname='cron' AND p.proname IN ('schedule','schedule_in_database','alter_job','unschedule')
ORDER BY 1;
"#;

pub const AUDIT: &str = r#"-- A configuration inspection, not an audit log or compliance certification.
SELECT session_user AS login_role, current_user AS effective_role, current_database() AS database,
       current_setting('application_name') AS application_name,
       current_setting('server_version') AS server_version;
SELECT name, setting, context, pending_restart
FROM pg_settings
WHERE name LIKE 'pgaudit.%'
   OR name IN ('shared_preload_libraries', 'logging_collector', 'log_destination', 'log_line_prefix',
               'log_connections', 'log_disconnections', 'log_statement', 'log_min_duration_statement',
               'log_parameter_max_length', 'log_parameter_max_length_on_error')
ORDER BY name;
SELECT e.extname, e.extversion, n.nspname AS schema
FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace
WHERE e.extname IN ('pgaudit', 'pg_stat_statements');
"#;

pub const SECURITY: &str = r#"-- Visibility depends on your role. Review membership and inherited privileges.
-- No password hashes, subscription connection strings or FDW secrets are read.
SELECT rolname, rolcanlogin, rolsuper, rolcreaterole, rolcreatedb, rolreplication,
       rolbypassrls, rolinherit, rolconnlimit, rolvaliduntil
FROM pg_roles WHERE rolname !~ '^pg_' ORDER BY rolname;
SELECT pg_get_userbyid(roleid) AS granted_role, pg_get_userbyid(member) AS member,
       admin_option, pg_get_userbyid(grantor) AS grantor
FROM pg_auth_members ORDER BY 1, 2;
SELECT n.nspname AS schema, c.relname AS relation, c.relrowsecurity AS rls_enabled,
       c.relforcerowsecurity AS rls_forced, pg_get_userbyid(c.relowner) AS owner
FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind IN ('r', 'p') AND n.nspname !~ '^pg_' AND n.nspname <> 'information_schema'
ORDER BY 1, 2;
SELECT nspname AS schema, pg_get_userbyid(nspowner) AS owner, nspacl AS explicit_acl
FROM pg_namespace WHERE nspname !~ '^pg_' AND nspname <> 'information_schema' ORDER BY 1;
SELECT defaclrole::regrole AS owner, defaclnamespace::regnamespace AS schema,
       defaclobjtype AS object_type, defaclacl AS default_acl FROM pg_default_acl ORDER BY 1, 2;
"#;

pub const FOREIGN_DATA: &str = r#"-- Options/user mappings can contain credentials, so are intentionally omitted.
SELECT w.fdwname AS wrapper, s.srvname AS server, pg_get_userbyid(s.srvowner) AS owner
FROM pg_foreign_data_wrapper w LEFT JOIN pg_foreign_server s ON s.srvfdw = w.oid ORDER BY 1, 2;
SELECT foreign_table_schema, foreign_table_name, foreign_server_name
FROM information_schema.foreign_tables ORDER BY 1, 2;
"#;

pub const REPLICATION: &str = r#"-- Deliberately omit subscription connection strings (may contain credentials).
-- Access to subscription metadata may require extra privileges.
SELECT pubname, pg_get_userbyid(pubowner) AS owner, puballtables,
       pubinsert, pubupdate, pubdelete, pubtruncate FROM pg_publication ORDER BY 1;
SELECT slot_name, slot_type, database, active, restart_lsn, confirmed_flush_lsn
FROM pg_replication_slots ORDER BY 1;
SELECT subname, pid, relid::regclass AS relation, received_lsn, latest_end_lsn,
       last_msg_send_time, last_msg_receipt_time FROM pg_stat_subscription ORDER BY 1, 3;
"#;

pub const CRON_JOBS: &str = r#"-- Requires pg_cron in this database. RLS/permissions determine visible jobs.
-- Job command text can contain sensitive SQL; review before exporting results.
SELECT jobid, jobname, schedule, command, database, username, active
FROM cron.job ORDER BY jobid;
-- Recent runs across jobs; retention is managed separately on the server.
SELECT jobid, runid, status, return_message, start_time, end_time
FROM cron.job_run_details ORDER BY runid DESC LIMIT 200;
"#;

pub const RECIPES: &[Recipe] = &[
    Recipe { label: "Extensions · available and installed", sql: EXTENSIONS },
    Recipe { label: "Extension and scheduler prerequisites", sql: EXTENSION_PREFLIGHT },
    Recipe { label: "Auditing · configuration inspection", sql: AUDIT },
    Recipe { label: "Security · roles, RLS and grants", sql: SECURITY },
    Recipe { label: "Foreign data wrappers and servers", sql: FOREIGN_DATA },
    Recipe { label: "Replication · publications, slots and subscriptions", sql: REPLICATION },
    Recipe { label: "pg_cron · jobs and recent runs", sql: CRON_JOBS },
    Recipe {
        label: "pg_cron · install template",
        sql: "-- Configure provider allowlist/preload and restart first; see docs/administration.md.\n-- Connect to cron.database_name (normally postgres). Requires suitable privileges.\nCREATE EXTENSION pg_cron;\n",
    },
    Recipe {
        label: "pg_cron · schedule a job",
        sql: "-- Requires pg_cron 1.4+ and USAGE on cron. Review the command and timezone.\n-- Reusing a name replaces that user's existing named schedule.\nSELECT cron.schedule(:job_name, :schedule, :command);\n",
    },
    Recipe {
        label: "pg_cron · pause a job",
        sql: "-- Requires pg_cron 1.4+ and an explicit EXECUTE grant on cron.alter_job. Only change your own job.\nSELECT cron.alter_job(:job_id::bigint, active := false);\n",
    },
    Recipe {
        label: "pg_cron · remove a job",
        sql: "-- Removes the schedule; it does not undo SQL already executed by the job.\nSELECT cron.unschedule(:job_id::bigint);\n",
    },
    Recipe {
        label: "pgAudit · install template",
        sql: "-- Configure provider preload/flags and restart first; see docs/administration.md.\n-- Install before setting pgaudit.log; CREATE EXTENSION alone does not enable logs.\nCREATE EXTENSION pgaudit;\n-- Test in this session, then configure scope and durable log collection with your DBA.\n-- SET pgaudit.log = 'write,ddl,role';\n",
    },
];

pub fn extension_update(name: &str) -> String {
    format!("-- Review available versions and extension-specific upgrade instructions first.\nALTER EXTENSION {} UPDATE;\n", super::meta::quote_ident(name))
}

pub fn extension_details(name: &str) -> String {
    format!(
        "SELECT name, version, installed, superuser, trusted, relocatable, schema, requires, comment\nFROM pg_available_extension_versions WHERE name = {} ORDER BY version;\n",
        super::meta::quote_literal(name)
    )
}
