//! Catalog introspection. One multi-statement batch loads everything the
//! object browser, autocompletion and the command palette need for a
//! database, in a single network round trip.

use std::collections::HashMap;

use super::result::ResultSet;
use super::types::{self, Cat};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RelKind {
    Table,
    Partitioned,
    View,
    MatView,
    Foreign,
    Sequence,
}

impl RelKind {
    fn from_code(c: &str) -> Option<RelKind> {
        Some(match c {
            "r" => RelKind::Table,
            "p" => RelKind::Partitioned,
            "v" => RelKind::View,
            "m" => RelKind::MatView,
            "f" => RelKind::Foreign,
            "S" => RelKind::Sequence,
            _ => return None,
        })
    }
    pub fn label(self) -> &'static str {
        match self {
            RelKind::Table => "table",
            RelKind::Partitioned => "partitioned table",
            RelKind::View => "view",
            RelKind::MatView => "materialized view",
            RelKind::Foreign => "foreign table",
            RelKind::Sequence => "sequence",
        }
    }
    pub fn is_table_like(self) -> bool {
        matches!(self, RelKind::Table | RelKind::Partitioned | RelKind::Foreign)
    }
    pub fn has_data(self) -> bool {
        self != RelKind::Sequence
    }
}

#[derive(Clone, Debug)]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_oid: u32,
    pub ref_columns: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ColumnInfo {
    pub attnum: i16,
    pub name: String,
    pub data_type: String,
    pub not_null: bool,
    pub default: Option<String>,
    pub is_pk: bool,
    pub comment: Option<String>,
    /// GENERATED … AS IDENTITY
    pub identity: bool,
    /// GENERATED ALWAYS AS (…) STORED
    pub generated: bool,
}

#[derive(Clone, Debug)]
pub struct Relation {
    pub oid: u32,
    pub schema: String,
    pub name: String,
    pub kind: RelKind,
    pub est_rows: i64,
    pub comment: Option<String>,
    pub columns: Vec<ColumnInfo>,
    /// Partition child of another table (hidden under its parent in the tree).
    pub is_partition: bool,
    pub fks: Vec<ForeignKey>,
    pub indexes: Vec<IndexInfo>,
    /// Primary key, unique, check and exclusion constraints.
    pub constraints: Vec<ConstraintInfo>,
    pub triggers: Vec<TriggerInfo>,
    pub policies: Vec<PolicyInfo>,
    /// Row level security is enabled.
    pub rls: bool,
}

#[derive(Clone, Debug)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    pub valid: bool,
    pub method: String,
    pub partial: bool,
}

#[derive(Clone, Debug)]
pub struct ConstraintInfo {
    pub name: String,
    /// p = primary key, u = unique, c = check, x = exclusion
    pub kind: char,
    pub columns: Vec<String>,
    /// CHECK expression (only for check constraints)
    pub definition: Option<String>,
    pub validated: bool,
}

impl ConstraintInfo {
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            'p' => "primary key",
            'u' => "unique",
            'c' => "check",
            'x' => "exclusion",
            _ => "constraint",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TriggerInfo {
    pub name: String,
    pub enabled: bool,
    /// e.g. "BEFORE INSERT OR UPDATE FOR EACH ROW"
    pub when: String,
    pub function: String,
}

#[derive(Clone, Debug)]
pub struct PolicyInfo {
    pub name: String,
    pub command: String,
    pub permissive: bool,
    pub roles: Vec<String>,
}

/// Decodes `pg_trigger.tgtype`.
pub fn trigger_timing(tgtype: i32) -> String {
    let timing = if tgtype & 64 != 0 {
        "INSTEAD OF"
    } else if tgtype & 2 != 0 {
        "BEFORE"
    } else {
        "AFTER"
    };
    let events: Vec<&str> = [(4, "INSERT"), (16, "UPDATE"), (8, "DELETE"), (32, "TRUNCATE")].iter().filter(|(b, _)| tgtype & b != 0).map(|(_, n)| *n).collect();
    let each = if tgtype & 1 != 0 { "FOR EACH ROW" } else { "FOR EACH STATEMENT" };
    format!("{timing} {} {each}", events.join(" OR "))
}

impl Relation {
    pub fn qualified(&self) -> String {
        format!("{}.{}", quote_ident(&self.schema), quote_ident(&self.name))
    }
    /// Single-column foreign key on `column`, if any.
    pub fn fk_for(&self, column: &str) -> Option<&ForeignKey> {
        self.fks.iter().find(|f| f.columns.len() == 1 && f.columns[0] == column)
    }

    pub fn pk_columns(&self) -> Vec<&str> {
        self.columns.iter().filter(|c| c.is_pk).map(|c| c.name.as_str()).collect()
    }
}

#[derive(Clone, Debug)]
pub struct Function {
    pub oid: u32,
    pub schema: String,
    pub name: String,
    pub args: String,
    pub result: String,
    /// f = function, p = procedure, a = aggregate, w = window
    pub kind: char,
}

impl Function {
    pub fn signature(&self) -> String {
        format!("{}.{}({})", quote_ident(&self.schema), quote_ident(&self.name), self.args)
    }
}

#[derive(Clone, Debug)]
pub struct TypeInfo {
    pub schema: String,
    pub name: String,
    /// e = enum, c = composite, d = domain, r = range
    pub kind: char,
    pub detail: String,
}

#[derive(Clone, Debug, Default)]
pub struct Schema {
    pub name: String,
    pub owner: String,
    pub is_system: bool,
    pub relations: Vec<Relation>,
    pub functions: Vec<Function>,
    pub types: Vec<TypeInfo>,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub schemas: Vec<Schema>,
    /// type oid -> (display name, category)
    pub types: HashMap<u32, (String, Cat)>,
    pub extensions: Vec<(String, String)>,
    pub search_path: Vec<String>,
    /// (name, event, enabled, function)
    pub event_triggers: Vec<(String, String, bool, String)>,
    /// (name, all tables, operations)
    pub publications: Vec<(String, bool, String)>,
}

impl Catalog {
    pub fn type_name(&self, oid: u32) -> String {
        if let Some((n, _)) = types::builtin(oid) {
            return n.to_string();
        }
        self.types.get(&oid).map(|t| t.0.clone()).unwrap_or_else(|| format!("oid {oid}"))
    }

    pub fn type_cat(&self, oid: u32) -> Cat {
        if let Some((_, c)) = types::builtin(oid) {
            return c;
        }
        self.types.get(&oid).map(|t| t.1).unwrap_or(Cat::Other)
    }

    pub fn find_relation(&self, schema: Option<&str>, name: &str) -> Option<&Relation> {
        let name_matches = |r: &&Relation| r.name == name || r.name.eq_ignore_ascii_case(name);
        match schema {
            Some(s) => self.schemas.iter().find(|x| x.name == s || x.name.eq_ignore_ascii_case(s))?.relations.iter().find(name_matches),
            None => {
                for sp in &self.search_path {
                    if let Some(sc) = self.schemas.iter().find(|x| &x.name == sp)
                        && let Some(r) = sc.relations.iter().find(name_matches)
                    {
                        return Some(r);
                    }
                }
                self.relations().find(name_matches)
            }
        }
    }

    pub fn relation_by_oid(&self, oid: u32) -> Option<&Relation> {
        self.relations().find(|r| r.oid == oid)
    }

    pub fn relations(&self) -> impl Iterator<Item = &Relation> {
        self.schemas.iter().flat_map(|s| s.relations.iter())
    }

    pub fn functions(&self) -> impl Iterator<Item = &Function> {
        self.schemas.iter().flat_map(|s| s.functions.iter())
    }

    /// Is `schema` on the search path (so names don't need qualification)?
    pub fn on_search_path(&self, schema: &str) -> bool {
        self.search_path.iter().any(|s| s == schema)
    }
}

pub fn quote_ident(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !crate::sql::lexer::is_reserved(s);
    if plain { s.to_string() } else { format!("\"{}\"", s.replace('"', "\"\"")) }
}

pub fn quote_literal(s: &str) -> String {
    if s.contains('\\') { format!("E'{}'", s.replace('\\', "\\\\").replace('\'', "''")) } else { format!("'{}'", s.replace('\'', "''")) }
}

/// Loads the full catalog of the current database (one round trip).
///
/// Deliberately avoids functions that lock relations (e.g. `pg_get_expr`
/// takes an AccessShareLock), so browsing never queues behind a migration
/// holding an ACCESS EXCLUSIVE lock.
pub fn catalog_sql(show_system: bool) -> String {
    let sys_filter = if show_system {
        "n.nspname !~ '^pg_toast' AND n.nspname !~ '^pg_temp'"
    } else {
        "n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_toast' AND n.nspname !~ '^pg_temp'"
    };
    format!(
        r#"
SELECT n.nspname, pg_get_userbyid(n.nspowner), n.nspname IN ('pg_catalog','information_schema')
FROM pg_namespace n WHERE {sys_filter} ORDER BY 1;

SELECT c.oid, n.nspname, c.relname, c.relkind, GREATEST(c.reltuples, -1)::bigint,
       obj_description(c.oid, 'pg_class'), c.relispartition, c.relrowsecurity
FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind IN ('r','p','v','m','f','S') AND {sys_filter}
ORDER BY 2, 3;

SELECT a.attrelid, a.attname, format_type(a.atttypid, a.atttypmod), a.attnotnull,
       NULL::text,
       EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = a.attrelid AND i.indisprimary AND a.attnum = ANY(i.indkey)),
       col_description(a.attrelid, a.attnum), a.attnum, a.attidentity <> '', a.attgenerated <> ''
FROM pg_attribute a
JOIN pg_class c ON c.oid = a.attrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE a.attnum > 0 AND NOT a.attisdropped AND c.relkind IN ('r','p','v','m','f') AND {sys_filter}
ORDER BY a.attrelid, a.attnum;

SELECT p.oid, n.nspname, p.proname, pg_get_function_identity_arguments(p.oid),
       COALESCE(pg_get_function_result(p.oid), ''), p.prokind
FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
WHERE {sys_filter} AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid = p.oid AND d.deptype = 'e')
ORDER BY 2, 3;

SELECT n.nspname, t.typname, t.typtype,
       CASE t.typtype
         WHEN 'e' THEN (SELECT string_agg(quote_literal(enumlabel), ', ' ORDER BY enumsortorder) FROM pg_enum WHERE enumtypid = t.oid)
         WHEN 'd' THEN format_type(t.typbasetype, t.typtypmod)
         ELSE '' END
FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
WHERE t.typtype IN ('e','d','r') AND {sys_filter}
   OR (t.typtype = 'c' AND {sys_filter} AND EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = t.typrelid AND c.relkind = 'c'))
ORDER BY 1, 2;

SELECT t.oid, format_type(t.oid, NULL), t.typcategory FROM pg_type t WHERE t.oid >= 10000 OR t.typtype IN ('e','d','r','m');

SELECT extname, extversion FROM pg_extension ORDER BY 1;

SELECT unnest(current_schemas(false));

SELECT c.conrelid, c.confrelid, c.conname,
       (SELECT array_agg(a.attname ORDER BY k.n) FROM unnest(c.conkey) WITH ORDINALITY k(attnum, n)
          JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum),
       (SELECT array_agg(a.attname ORDER BY k.n) FROM unnest(c.confkey) WITH ORDINALITY k(attnum, n)
          JOIN pg_attribute a ON a.attrelid = c.confrelid AND a.attnum = k.attnum)
FROM pg_constraint c JOIN pg_namespace n ON n.oid = c.connamespace
WHERE c.contype = 'f' AND {sys_filter};

SELECT i.indrelid, ic.relname, i.indisunique, i.indisprimary, i.indisvalid, am.amname,
       ARRAY(SELECT COALESCE(a.attname::text, 'expr') FROM unnest(i.indkey::int2[]) WITH ORDINALITY k(attnum, n)
             LEFT JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum
             WHERE k.n <= i.indnkeyatts ORDER BY k.n),
       i.indpred IS NOT NULL
FROM pg_index i
JOIN pg_class ic ON ic.oid = i.indexrelid
JOIN pg_class c ON c.oid = i.indrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_am am ON am.oid = ic.relam
WHERE {sys_filter}
ORDER BY 1, NOT i.indisprimary, 2;

SELECT c.conrelid, c.conname, c.contype,
       ARRAY(SELECT a.attname::text FROM unnest(c.conkey) WITH ORDINALITY k(attnum, n)
             JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum ORDER BY k.n),
       CASE WHEN c.contype = 'c' THEN pg_get_constraintdef(c.oid) END, c.convalidated
FROM pg_constraint c JOIN pg_namespace n ON n.oid = c.connamespace
WHERE c.contype IN ('p','u','c','x') AND c.conrelid <> 0 AND {sys_filter}
ORDER BY 1, 2;

SELECT t.tgrelid, t.tgname, t.tgenabled <> 'D', t.tgtype, pn.nspname || '.' || p.proname
FROM pg_trigger t
JOIN pg_class c ON c.oid = t.tgrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_proc p ON p.oid = t.tgfoid
JOIN pg_namespace pn ON pn.oid = p.pronamespace
WHERE NOT t.tgisinternal AND {sys_filter}
ORDER BY 1, 2;

SELECT p.polrelid, p.polname,
       CASE p.polcmd WHEN 'r' THEN 'SELECT' WHEN 'a' THEN 'INSERT' WHEN 'w' THEN 'UPDATE' WHEN 'd' THEN 'DELETE' ELSE 'ALL' END,
       p.polpermissive,
       CASE WHEN 0 = ANY(p.polroles) THEN ARRAY['public'] ELSE ARRAY(SELECT r.rolname::text FROM pg_roles r WHERE r.oid = ANY(p.polroles) ORDER BY 1) END
FROM pg_policy p
JOIN pg_class c ON c.oid = p.polrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE {sys_filter}
ORDER BY 1, 2;

SELECT evtname, evtevent, evtenabled <> 'D', evtfoid::regproc::text FROM pg_event_trigger ORDER BY 1;

SELECT pubname, puballtables,
       concat_ws(', ', CASE WHEN pubinsert THEN 'insert' END, CASE WHEN pubupdate THEN 'update' END, CASE WHEN pubdelete THEN 'delete' END)
FROM pg_publication ORDER BY 1;
"#
    )
}

pub fn parse_catalog(results: Vec<ResultSet>) -> Catalog {
    let mut cat = Catalog::default();
    let mut it = results.into_iter();
    let (Some(ns), Some(rels), Some(cols), Some(funcs), Some(tys), Some(alltypes), Some(exts), Some(sp)) =
        (it.next(), it.next(), it.next(), it.next(), it.next(), it.next(), it.next(), it.next())
    else {
        return cat;
    };
    let mut fk_map: HashMap<u32, Vec<ForeignKey>> = HashMap::new();
    let fks = it.next();
    let (idx_rs, con_rs, trg_rs, pol_rs, evt_rs, pub_rs) = (it.next(), it.next(), it.next(), it.next(), it.next(), it.next());
    let mut idx_map: HashMap<u32, Vec<IndexInfo>> = HashMap::new();
    if let Some(x) = &idx_rs {
        for r in x.rows() {
            idx_map.entry(x.s(r, 0).parse().unwrap_or(0)).or_default().push(IndexInfo {
                name: x.s(r, 1).to_string(),
                unique: x.s(r, 2) == "t",
                primary: x.s(r, 3) == "t",
                valid: x.s(r, 4) == "t",
                method: x.s(r, 5).to_string(),
                columns: parse_text_array(x.s(r, 6)),
                partial: x.s(r, 7) == "t",
            });
        }
    }
    let mut con_map: HashMap<u32, Vec<ConstraintInfo>> = HashMap::new();
    if let Some(x) = &con_rs {
        for r in x.rows() {
            con_map.entry(x.s(r, 0).parse().unwrap_or(0)).or_default().push(ConstraintInfo {
                name: x.s(r, 1).to_string(),
                kind: x.s(r, 2).chars().next().unwrap_or('c'),
                columns: parse_text_array(x.s(r, 3)),
                definition: x.get(r, 4).map(str::to_string),
                validated: x.s(r, 5) == "t",
            });
        }
    }
    let mut trg_map: HashMap<u32, Vec<TriggerInfo>> = HashMap::new();
    if let Some(x) = &trg_rs {
        for r in x.rows() {
            trg_map.entry(x.s(r, 0).parse().unwrap_or(0)).or_default().push(TriggerInfo {
                name: x.s(r, 1).to_string(),
                enabled: x.s(r, 2) == "t",
                when: trigger_timing(x.s(r, 3).parse().unwrap_or(0)),
                function: x.s(r, 4).to_string(),
            });
        }
    }
    let mut pol_map: HashMap<u32, Vec<PolicyInfo>> = HashMap::new();
    if let Some(x) = &pol_rs {
        for r in x.rows() {
            pol_map.entry(x.s(r, 0).parse().unwrap_or(0)).or_default().push(PolicyInfo {
                name: x.s(r, 1).to_string(),
                command: x.s(r, 2).to_string(),
                permissive: x.s(r, 3) == "t",
                roles: parse_text_array(x.s(r, 4)),
            });
        }
    }
    if let Some(x) = &evt_rs {
        cat.event_triggers = x.rows().map(|r| (x.s(r, 0).to_string(), x.s(r, 1).to_string(), x.s(r, 2) == "t", x.s(r, 3).to_string())).collect();
    }
    if let Some(x) = &pub_rs {
        cat.publications = x.rows().map(|r| (x.s(r, 0).to_string(), x.s(r, 1) == "t", x.s(r, 2).to_string())).collect();
    }
    if let Some(f) = fks {
        for r in f.rows() {
            fk_map.entry(f.s(r, 0).parse().unwrap_or(0)).or_default().push(ForeignKey {
                name: f.s(r, 2).to_string(),
                columns: parse_text_array(f.s(r, 3)),
                ref_oid: f.s(r, 1).parse().unwrap_or(0),
                ref_columns: parse_text_array(f.s(r, 4)),
            });
        }
    }
    let mut schema_idx = HashMap::new();
    for r in ns.rows() {
        schema_idx.insert(ns.s(r, 0).to_string(), cat.schemas.len());
        cat.schemas.push(Schema { name: ns.s(r, 0).to_string(), owner: ns.s(r, 1).to_string(), is_system: ns.s(r, 2) == "t", ..Default::default() });
    }
    let mut col_map: HashMap<u32, Vec<ColumnInfo>> = HashMap::new();
    for r in cols.rows() {
        col_map.entry(cols.s(r, 0).parse().unwrap_or(0)).or_default().push(ColumnInfo {
            attnum: cols.s(r, 7).parse().unwrap_or(0),
            name: cols.s(r, 1).to_string(),
            data_type: cols.s(r, 2).to_string(),
            not_null: cols.s(r, 3) == "t",
            default: cols.get(r, 4).map(str::to_string),
            is_pk: cols.s(r, 5) == "t",
            comment: cols.get(r, 6).map(str::to_string),
            identity: cols.s(r, 8) == "t",
            generated: cols.s(r, 9) == "t",
        });
    }
    for r in rels.rows() {
        let Some(&si) = schema_idx.get(rels.s(r, 1)) else { continue };
        let Some(kind) = RelKind::from_code(rels.s(r, 3)) else { continue };
        let oid: u32 = rels.s(r, 0).parse().unwrap_or(0);
        cat.schemas[si].relations.push(Relation {
            oid,
            schema: rels.s(r, 1).to_string(),
            name: rels.s(r, 2).to_string(),
            kind,
            est_rows: rels.s(r, 4).parse().unwrap_or(-1),
            comment: rels.get(r, 5).map(str::to_string),
            columns: col_map.remove(&oid).unwrap_or_default(),
            is_partition: rels.s(r, 6) == "t",
            fks: fk_map.remove(&oid).unwrap_or_default(),
            indexes: idx_map.remove(&oid).unwrap_or_default(),
            constraints: con_map.remove(&oid).unwrap_or_default(),
            triggers: trg_map.remove(&oid).unwrap_or_default(),
            policies: pol_map.remove(&oid).unwrap_or_default(),
            rls: rels.s(r, 7) == "t",
        });
    }
    for r in funcs.rows() {
        let Some(&si) = schema_idx.get(funcs.s(r, 1)) else { continue };
        cat.schemas[si].functions.push(Function {
            oid: funcs.s(r, 0).parse().unwrap_or(0),
            schema: funcs.s(r, 1).to_string(),
            name: funcs.s(r, 2).to_string(),
            args: funcs.s(r, 3).to_string(),
            result: funcs.s(r, 4).to_string(),
            kind: funcs.s(r, 5).chars().next().unwrap_or('f'),
        });
    }
    for r in tys.rows() {
        let Some(&si) = schema_idx.get(tys.s(r, 0)) else { continue };
        cat.schemas[si].types.push(TypeInfo {
            schema: tys.s(r, 0).to_string(),
            name: tys.s(r, 1).to_string(),
            kind: tys.s(r, 2).chars().next().unwrap_or('b'),
            detail: tys.s(r, 3).to_string(),
        });
    }
    for r in alltypes.rows() {
        let oid: u32 = alltypes.s(r, 0).parse().unwrap_or(0);
        let mut c = Cat::from_typcategory(alltypes.s(r, 2));
        let name = alltypes.s(r, 1).to_string();
        if name == "json" || name == "jsonb" {
            c = Cat::Json;
        }
        cat.types.insert(oid, (name, c));
    }
    for r in exts.rows() {
        cat.extensions.push((exts.s(r, 0).to_string(), exts.s(r, 1).to_string()));
    }
    cat.search_path = sp.rows().map(|r| sp.s(r, 0).to_string()).collect();
    cat
}

/// Parses a PostgreSQL text array literal such as `{a,"b c",d}`.
pub fn parse_text_array(s: &str) -> Vec<String> {
    let inner = s.trim().trim_start_matches('{').trim_end_matches('}');
    if inner.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' if quoted => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}

pub const DATABASES_SQL: &str = "SELECT datname, CASE WHEN has_database_privilege(oid, 'CONNECT') THEN pg_size_pretty(pg_database_size(oid)) END, \
     has_database_privilege(oid, 'CONNECT'), pg_get_userbyid(datdba), pg_encoding_to_char(encoding), datcollate \
     FROM pg_database WHERE NOT datistemplate ORDER BY datname = current_database() DESC, datname";

/// Server-wide objects shown in the browser: roles and tablespaces.
pub const SERVER_OBJECTS_SQL: &str = r#"
SELECT r.rolname, r.rolsuper, r.rolinherit, r.rolcreaterole, r.rolcreatedb, r.rolcanlogin, r.rolreplication, r.rolbypassrls,
       r.rolconnlimit, r.rolvaliduntil,
       ARRAY(SELECT b.rolname::text FROM pg_auth_members m JOIN pg_roles b ON b.oid = m.roleid WHERE m.member = r.oid ORDER BY 1),
       shobj_description(r.oid, 'pg_authid'), r.rolname = current_user
FROM pg_roles r ORDER BY r.rolname;

SELECT spcname, pg_get_userbyid(spcowner), pg_tablespace_location(oid) FROM pg_tablespace ORDER BY 1;
"#;

#[derive(Clone, Debug, Default)]
pub struct RoleInfo {
    pub name: String,
    pub superuser: bool,
    pub inherit: bool,
    pub create_role: bool,
    pub create_db: bool,
    pub login: bool,
    pub replication: bool,
    pub bypass_rls: bool,
    pub conn_limit: i32,
    pub valid_until: Option<String>,
    pub member_of: Vec<String>,
    pub comment: Option<String>,
    pub is_current: bool,
}

impl RoleInfo {
    pub fn is_system(&self) -> bool {
        self.name.starts_with("pg_")
    }

    /// Short attribute summary, e.g. "superuser · create db".
    pub fn attributes(&self) -> Vec<&'static str> {
        let mut a = Vec::new();
        if self.superuser {
            a.push("superuser");
        }
        if self.create_role {
            a.push("create role");
        }
        if self.create_db {
            a.push("create db");
        }
        if self.replication {
            a.push("replication");
        }
        if self.bypass_rls {
            a.push("bypass RLS");
        }
        if !self.inherit {
            a.push("no inherit");
        }
        a
    }

    /// CREATE ROLE statement reproducing this role's attributes and memberships.
    pub fn ddl(&self) -> String {
        let flag = |on: bool, yes: &str, no: &str| if on { yes.to_string() } else { no.to_string() };
        let mut opts = vec![
            flag(self.login, "LOGIN", "NOLOGIN"),
            flag(self.superuser, "SUPERUSER", "NOSUPERUSER"),
            flag(self.create_db, "CREATEDB", "NOCREATEDB"),
            flag(self.create_role, "CREATEROLE", "NOCREATEROLE"),
            flag(self.inherit, "INHERIT", "NOINHERIT"),
            flag(self.replication, "REPLICATION", "NOREPLICATION"),
            flag(self.bypass_rls, "BYPASSRLS", "NOBYPASSRLS"),
        ];
        if self.conn_limit >= 0 {
            opts.push(format!("CONNECTION LIMIT {}", self.conn_limit));
        }
        if let Some(v) = &self.valid_until {
            opts.push(format!("VALID UNTIL {}", quote_literal(v)));
        }
        let mut out = format!("CREATE ROLE {} WITH\n    {};\n", quote_ident(&self.name), opts.join("\n    "));
        for m in &self.member_of {
            out.push_str(&format!("GRANT {} TO {};\n", quote_ident(m), quote_ident(&self.name)));
        }
        if let Some(c) = &self.comment {
            out.push_str(&format!("COMMENT ON ROLE {} IS {};\n", quote_ident(&self.name), quote_literal(c)));
        }
        out
    }
}

pub fn parse_roles(rs: &ResultSet) -> Vec<RoleInfo> {
    rs.rows()
        .map(|r| RoleInfo {
            name: rs.s(r, 0).to_string(),
            superuser: rs.s(r, 1) == "t",
            inherit: rs.s(r, 2) == "t",
            create_role: rs.s(r, 3) == "t",
            create_db: rs.s(r, 4) == "t",
            login: rs.s(r, 5) == "t",
            replication: rs.s(r, 6) == "t",
            bypass_rls: rs.s(r, 7) == "t",
            conn_limit: rs.s(r, 8).parse().unwrap_or(-1),
            valid_until: rs.get(r, 9).filter(|v| *v != "infinity").map(str::to_string),
            member_of: parse_text_array(rs.s(r, 10)),
            comment: rs.get(r, 11).map(str::to_string),
            is_current: rs.s(r, 12) == "t",
        })
        .collect()
}

/// Detailed info used by the table "Structure" view.
pub fn table_details_sql(oid: u32) -> String {
    format!(
        r#"
SELECT a.attnum, a.attname, format_type(a.atttypid, a.atttypmod), a.attnotnull, pg_get_expr(d.adbin, d.adrelid),
       a.attidentity, a.attgenerated, col_description(a.attrelid, a.attnum),
       CASE WHEN a.attcollation <> t.typcollation THEN (SELECT collname FROM pg_collation WHERE oid = a.attcollation) END
FROM pg_attribute a JOIN pg_type t ON t.oid = a.atttypid
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
WHERE a.attrelid = {oid} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum;

SELECT conname, contype, pg_get_constraintdef(oid, true),
       CASE WHEN contype = 'f' THEN confrelid::regclass::text END
FROM pg_constraint WHERE conrelid = {oid} ORDER BY contype = 'p' DESC, contype, conname;

SELECT i.relname, pg_get_indexdef(x.indexrelid), x.indisunique, x.indisprimary,
       pg_size_pretty(pg_relation_size(x.indexrelid)), COALESCE(s.idx_scan, 0),
       EXISTS (SELECT 1 FROM pg_constraint c WHERE c.conindid = x.indexrelid AND c.conrelid = {oid}),
       x.indisvalid
FROM pg_index x JOIN pg_class i ON i.oid = x.indexrelid
LEFT JOIN pg_stat_all_indexes s ON s.indexrelid = x.indexrelid
WHERE x.indrelid = {oid} ORDER BY x.indisprimary DESC, i.relname;

SELECT tgname, pg_get_triggerdef(oid, true), tgenabled FROM pg_trigger WHERE tgrelid = {oid} AND NOT tgisinternal ORDER BY tgname;

SELECT c.relkind, pg_get_userbyid(c.relowner), obj_description(c.oid, 'pg_class'),
       pg_size_pretty(pg_total_relation_size(c.oid)), pg_size_pretty(pg_relation_size(c.oid)),
       pg_size_pretty(pg_indexes_size(c.oid)), GREATEST(c.reltuples, -1)::bigint,
       s.n_live_tup, s.n_dead_tup, s.last_vacuum, s.last_autovacuum, s.last_analyze, s.last_autoanalyze,
       s.seq_scan, s.idx_scan, c.relpersistence,
       CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END,
       CASE WHEN c.relispartition THEN pg_get_expr(c.relpartbound, c.oid) END,
       (SELECT string_agg(inhparent::regclass::text, ', ') FROM pg_inherits WHERE inhrelid = c.oid),
       c.reloptions::text, n.nspname, c.relname,
       CASE WHEN c.relkind IN ('v','m') THEN pg_get_viewdef(c.oid, true) END,
       (SELECT spcname FROM pg_tablespace WHERE oid = c.reltablespace)
FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_stat_all_tables s ON s.relid = c.oid
WHERE c.oid = {oid};

SELECT c.oid::regclass::text, pg_get_expr(c.relpartbound, c.oid)
FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid WHERE i.inhparent = {oid} ORDER BY 1;

SELECT conname, conrelid::regclass::text, pg_get_constraintdef(oid, true)
FROM pg_constraint WHERE confrelid = {oid} AND contype = 'f' ORDER BY 2;

SELECT grantee, string_agg(privilege_type, ', ' ORDER BY privilege_type)
FROM information_schema.role_table_grants g
JOIN pg_class c ON c.oid = {oid} JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE g.table_schema = n.nspname AND g.table_name = c.relname GROUP BY grantee ORDER BY grantee;
"#
    )
}

/// Builds `CREATE TABLE` (+ indexes, triggers, comments) DDL from the
/// results of [`table_details_sql`].
pub fn build_table_ddl(r: &[ResultSet]) -> String {
    use std::fmt::Write;
    let (Some(cols), Some(cons), Some(idx), Some(trg), Some(info)) = (r.first(), r.get(1), r.get(2), r.get(3), r.get(4)) else {
        return String::new();
    };
    if info.nrows() == 0 {
        return "-- object not found".into();
    }
    let schema = info.s(0, 20);
    let name = info.s(0, 21);
    let qname = format!("{}.{}", quote_ident(schema), quote_ident(name));
    let relkind = info.s(0, 0);
    let mut out = String::new();
    if relkind == "v" || relkind == "m" {
        let what = if relkind == "v" { "VIEW" } else { "MATERIALIZED VIEW" };
        let create = if relkind == "v" { "CREATE OR REPLACE VIEW" } else { "CREATE MATERIALIZED VIEW" };
        let _ = writeln!(out, "-- DROP {what} {qname};\n");
        let _ = writeln!(out, "{create} {qname} AS\n{}", info.s(0, 22).trim_end());
        for i in idx.rows() {
            let _ = writeln!(out, "\n{};", idx.s(i, 1));
        }
    } else {
        let unlogged = if info.s(0, 15) == "u" { "UNLOGGED " } else { "" };
        let _ = writeln!(out, "-- DROP TABLE {qname};\n");
        let _ = write!(out, "CREATE {unlogged}TABLE {qname}");
        if let Some(bound) = info.get(0, 17) {
            let parent = info.s(0, 18);
            let _ = write!(out, " PARTITION OF {parent}\n{bound}");
        } else {
            out.push_str(" (\n");
            let mut lines = Vec::new();
            for c in cols.rows() {
                let mut l = format!("    {} {}", quote_ident(cols.s(c, 1)), cols.s(c, 2));
                if let Some(coll) = cols.get(c, 8) {
                    let _ = write!(l, " COLLATE {}", quote_ident(coll));
                }
                match cols.s(c, 5) {
                    "a" => l.push_str(" GENERATED ALWAYS AS IDENTITY"),
                    "d" => l.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
                    _ => {}
                }
                if cols.s(c, 6) == "s" {
                    let _ = write!(l, " GENERATED ALWAYS AS ({}) STORED", cols.s(c, 4));
                } else if let Some(d) = cols.get(c, 4)
                    && cols.s(c, 5).is_empty()
                {
                    let _ = write!(l, " DEFAULT {d}");
                }
                if cols.s(c, 3) == "t" {
                    l.push_str(" NOT NULL");
                }
                lines.push(l);
            }
            for c in cons.rows() {
                // NOT NULL constraints (PG18+) are already expressed inline.
                if cons.s(c, 1) == "n" {
                    continue;
                }
                lines.push(format!("    CONSTRAINT {} {}", quote_ident(cons.s(c, 0)), cons.s(c, 2)));
            }
            out.push_str(&lines.join(",\n"));
            out.push_str("\n)");
            if let Some(parents) = info.get(0, 18) {
                let _ = write!(out, "\nINHERITS ({parents})");
            }
        }
        if let Some(pk) = info.get(0, 16) {
            let _ = write!(out, "\nPARTITION BY {pk}");
        }
        if let Some(opts) = info.get(0, 19) {
            let _ = write!(out, "\nWITH ({})", opts.trim_matches(|c| c == '{' || c == '}'));
        }
        if let Some(ts) = info.get(0, 23) {
            let _ = write!(out, "\nTABLESPACE {}", quote_ident(ts));
        }
        out.push_str(";\n");
        for i in idx.rows() {
            if idx.s(i, 6) == "t" {
                continue; // backs a constraint
            }
            let _ = writeln!(out, "\n{};", idx.s(i, 1));
        }
    }
    for t in trg.rows() {
        let _ = writeln!(out, "\n{};", trg.s(t, 1));
        if trg.s(t, 2) == "D" {
            let _ = writeln!(out, "ALTER TABLE {qname} DISABLE TRIGGER {};", quote_ident(trg.s(t, 0)));
        }
    }
    let owner = info.s(0, 1);
    if !owner.is_empty() {
        let kw = match relkind {
            "v" => "VIEW",
            "m" => "MATERIALIZED VIEW",
            _ => "TABLE",
        };
        let _ = writeln!(out, "\nALTER {kw} {qname} OWNER TO {};", quote_ident(owner));
    }
    if let Some(c) = info.get(0, 2) {
        let kw = match relkind {
            "v" => "VIEW",
            "m" => "MATERIALIZED VIEW",
            _ => "TABLE",
        };
        let _ = writeln!(out, "\nCOMMENT ON {kw} {qname} IS {};", quote_literal(c));
    }
    for c in cols.rows() {
        if let Some(cm) = cols.get(c, 7) {
            let _ = writeln!(out, "COMMENT ON COLUMN {qname}.{} IS {};", quote_ident(cols.s(c, 1)), quote_literal(cm));
        }
    }
    out
}

pub fn function_ddl_sql(oid: u32) -> String {
    format!(
        "SELECT CASE WHEN p.prokind = 'a' THEN '-- aggregate ' || p.oid::regprocedure::text ELSE pg_get_functiondef(p.oid) END FROM pg_proc p WHERE p.oid = {oid}"
    )
}

pub fn sequence_ddl_sql(schema: &str, name: &str) -> String {
    format!(
        "SELECT format('CREATE SEQUENCE %I.%I\n    AS %s\n    INCREMENT BY %s\n    MINVALUE %s\n    MAXVALUE %s\n    START WITH %s\n    CACHE %s%s;\n\n-- current value: %s', \
         schemaname, sequencename, data_type, increment_by, min_value, max_value, start_value, cache_size, \
         CASE WHEN cycle THEN '\n    CYCLE' ELSE '' END, COALESCE(last_value::text, 'not yet used')) \
         FROM pg_sequences WHERE schemaname = {} AND sequencename = {}",
        quote_literal(schema),
        quote_literal(name)
    )
}

/// PostgreSQL 13+; extension installation remains an explicit DBA decision.
pub const SLOW_QUERIES_SQL: &str = r#"-- Requires pg_stat_statements in shared_preload_libraries and
-- CREATE EXTENSION pg_stat_statements in this database (ask your DBA).
-- Query text visibility depends on your role. Times are milliseconds.
SELECT queryid, calls, total_exec_time, mean_exec_time, rows,
       shared_blks_hit, shared_blks_read, temp_blks_written,
       left(query, 2000) AS query
FROM pg_stat_statements
WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
ORDER BY total_exec_time DESC
LIMIT 50;"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_arrays() {
        assert_eq!(parse_text_array("{a,b}"), vec!["a", "b"]);
        assert_eq!(parse_text_array(r#"{"a,b",c}"#), vec!["a,b", "c"]);
        assert!(parse_text_array("{}").is_empty());
    }

    #[test]
    fn quoting() {
        assert_eq!(quote_ident("users"), "users");
        assert_eq!(quote_ident("Users"), "\"Users\"");
        assert_eq!(quote_ident("user"), "\"user\"");
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_literal("it's"), "'it''s'");
        assert_eq!(quote_literal("a\\b"), "E'a\\\\b'");
    }
}
