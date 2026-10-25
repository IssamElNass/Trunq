//! ACL inventory from pg_catalog, including grants hidden by information_schema.
//! This is a current database snapshot, not a history of permission changes.

use super::meta::{quote_ident, quote_literal};
use super::result::ResultSet;
use std::collections::{BTreeSet, HashMap, VecDeque};

pub const SQL: &str = r#"
WITH objects(kind, schema, name, identity, owner, acl, builtin, rls, column_name) AS (
 SELECT CASE c.relkind WHEN 'S' THEN 'Sequence' WHEN 'v' THEN 'View'
          WHEN 'm' THEN 'Materialized view' WHEN 'f' THEN 'Foreign table' ELSE 'Table' END,
        n.nspname, c.relname, format('%I.%I', n.nspname, c.relname), c.relowner,
        COALESCE(c.relacl, acldefault(CASE WHEN c.relkind='S' THEN 's' ELSE 'r' END::"char", c.relowner)),
        c.relacl IS NULL, c.relrowsecurity, ''
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE c.relkind IN ('r','p','v','m','f','S')
 UNION ALL
 SELECT 'Column', n.nspname, c.relname,
        format('%I.%I', n.nspname, c.relname), c.relowner,
        a.attacl, false, c.relrowsecurity, a.attname
 FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE a.attnum>0 AND NOT a.attisdropped AND a.attacl IS NOT NULL
 UNION ALL
 SELECT 'Schema', nspname, nspname, quote_ident(nspname), nspowner,
        COALESCE(nspacl, acldefault('n', nspowner)), nspacl IS NULL, false, '' FROM pg_namespace
 UNION ALL
 SELECT 'Database', '', datname, quote_ident(datname), datdba,
        COALESCE(datacl, acldefault('d', datdba)), datacl IS NULL, false, ''
 FROM pg_database WHERE datname=current_database()
 UNION ALL
 SELECT CASE WHEN p.prokind='p' THEN 'Procedure' ELSE 'Function' END, n.nspname, p.proname,
        format('%I.%I(%s)', n.nspname, p.proname, pg_get_function_identity_arguments(p.oid)), p.proowner,
        COALESCE(p.proacl, acldefault('f', p.proowner)), p.proacl IS NULL, false, ''
 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
 UNION ALL
 SELECT 'Type', n.nspname, t.typname, format('%I.%I', n.nspname, t.typname), t.typowner,
        COALESCE(t.typacl, acldefault('T', t.typowner)), t.typacl IS NULL, false, ''
 FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace
 WHERE (t.typrelid=0 OR EXISTS (SELECT 1 FROM pg_class c WHERE c.oid=t.typrelid AND c.relkind='c'))
   AND t.typelem=0
 UNION ALL
 SELECT 'Language', '', lanname, quote_ident(lanname), lanowner,
        COALESCE(lanacl, acldefault('l', lanowner)), lanacl IS NULL, false, '' FROM pg_language WHERE lanpltrusted
 UNION ALL
 SELECT 'Foreign data wrapper', '', fdwname, quote_ident(fdwname), fdwowner,
        COALESCE(fdwacl, acldefault('F', fdwowner)), fdwacl IS NULL, false, '' FROM pg_foreign_data_wrapper
 UNION ALL
 SELECT 'Foreign server', '', srvname, quote_ident(srvname), srvowner,
        COALESCE(srvacl, acldefault('S', srvowner)), srvacl IS NULL, false, '' FROM pg_foreign_server
 UNION ALL
 SELECT 'Tablespace', '', spcname, quote_ident(spcname), spcowner,
        COALESCE(spcacl, acldefault('t', spcowner)), spcacl IS NULL, false, '' FROM pg_tablespace
 UNION ALL
 SELECT 'Large object', '', oid::text, oid::text, lomowner,
        COALESCE(lomacl, acldefault('L', lomowner)), lomacl IS NULL, false, '' FROM pg_largeobject_metadata
), inventory AS (
 SELECT o.kind, o.schema, o.name, o.identity, pg_get_userbyid(o.owner) AS owner,
        pg_get_userbyid(a.grantor) AS grantor,
        CASE WHEN a.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee) END AS grantee,
        a.privilege_type AS privilege, a.is_grantable, o.builtin, o.rls, false AS future,
        a.grantee=0 AS public, o.column_name
 FROM objects o CROSS JOIN LATERAL aclexplode(o.acl) a
 UNION ALL
 SELECT CASE d.defaclobjtype WHEN 'r' THEN 'Table' WHEN 'S' THEN 'Sequence'
          WHEN 'f' THEN 'Function' WHEN 'T' THEN 'Type' WHEN 'n' THEN 'Schema' END,
        COALESCE(n.nspname,''), 'Future objects', '', pg_get_userbyid(d.defaclrole),
        pg_get_userbyid(a.grantor), CASE WHEN a.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee) END,
        a.privilege_type, a.is_grantable, false, false, true, a.grantee=0, ''
 FROM pg_default_acl d LEFT JOIN pg_namespace n ON n.oid=d.defaclnamespace
 CROSS JOIN LATERAL aclexplode(d.defaclacl) a
)
SELECT * FROM inventory ORDER BY schema, kind, identity, future, grantee, privilege;
SELECT rolname, rolsuper, rolbypassrls FROM pg_roles ORDER BY rolname;
SELECT member.rolname, parent.rolname, pg_get_userbyid(m.grantor), m.admin_option,
       COALESCE((to_jsonb(m)->>'inherit_option')::boolean, member.rolinherit),
       COALESCE((to_jsonb(m)->>'set_option')::boolean, true)
FROM pg_auth_members m JOIN pg_roles member ON member.oid=m.member JOIN pg_roles parent ON parent.oid=m.roleid
ORDER BY 1,2;
"#;

#[derive(Clone, Debug)]
pub struct Grant {
    pub kind: String,
    pub schema: String,
    pub name: String,
    pub identity: String,
    pub owner: String,
    pub grantor: String,
    pub grantee: String,
    pub privilege: String,
    pub grantable: bool,
    pub builtin: bool,
    pub rls: bool,
    pub future: bool,
    pub public: bool,
    pub column: String,
}

impl Grant {
    pub fn object_label(&self) -> String {
        if self.future {
            format!("Future {} in {}", self.kind, if self.schema.is_empty() { "all schemas" } else { &self.schema })
        } else if self.kind == "Column" {
            format!("{} ({})", self.identity, quote_ident(&self.column))
        } else {
            self.identity.clone()
        }
    }

    pub fn source(&self) -> &'static str {
        if self.future {
            "Default privileges"
        } else if self.builtin {
            "Built-in ACL"
        } else {
            "Object ACL"
        }
    }

    pub fn is_system(&self) -> bool {
        self.schema == "information_schema" || self.schema.starts_with("pg_") || (self.kind == "Tablespace" && self.name.starts_with("pg_"))
    }

    /// SQL preview only: never submitted by the inventory UI.
    pub fn sql(&self, revoke: bool) -> String {
        let grantee = if self.public { "PUBLIC".into() } else { quote_ident(&self.grantee) };
        let verb = if revoke { "REVOKE" } else { "GRANT" };
        let to = if revoke { "FROM" } else { "TO" };
        let option = if !revoke && self.grantable { " WITH GRANT OPTION" } else { "" };
        if self.future {
            let schema = if self.schema.is_empty() { String::new() } else { format!(" IN SCHEMA {}", quote_ident(&self.schema)) };
            let objects = match self.kind.as_str() {
                "Sequence" => "SEQUENCES",
                "Function" => "FUNCTIONS",
                "Type" => "TYPES",
                "Schema" => "SCHEMAS",
                _ => "TABLES",
            };
            return format!(
                "ALTER DEFAULT PRIVILEGES FOR ROLE {}{schema}\n{verb} {} ON {objects} {to} {grantee}{option};",
                quote_ident(&self.owner),
                self.privilege
            );
        }
        let (object, privilege) = if self.kind == "Column" {
            (format!("TABLE {}", self.identity), format!("{} ({})", self.privilege, quote_ident(&self.column)))
        } else {
            let kind = match self.kind.as_str() {
                "View" | "Materialized view" | "Foreign table" => "TABLE",
                "Foreign data wrapper" => "FOREIGN DATA WRAPPER",
                "Foreign server" => "FOREIGN SERVER",
                "Large object" => "LARGE OBJECT",
                other => other,
            };
            (format!("{} {}", kind.to_uppercase(), self.identity), self.privilege.clone())
        };
        format!("{verb} {privilege} ON {object} {to} {grantee}{option};")
    }
}

#[derive(Clone, Debug)]
pub struct Membership {
    pub member: String,
    pub parent: String,
    pub grantor: String,
    pub admin: bool,
    pub inherit: bool,
    pub set_role: bool,
}

#[derive(Default)]
pub struct Inventory {
    pub grants: Vec<Grant>,
    /// (name, superuser, bypass RLS)
    pub roles: Vec<(String, bool, bool)>,
    pub memberships: Vec<Membership>,
    pub schemas: Vec<String>,
    pub user_schemas: Vec<String>,
    pub kinds: Vec<String>,
    pub user_object_count: usize,
    pub public_count: usize,
    /// Prepared once, so search does not allocate lowercase copies per field.
    pub search_text: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub roles: Vec<String>,
    pub inherited: bool,
}

impl Inventory {
    pub fn parse(sets: &[ResultSet]) -> Result<Self, String> {
        if sets.len() != 3 {
            return Err("Incomplete grants snapshot; refresh to retry.".into());
        }
        let g = &sets[0];
        let grants: Vec<Grant> = g
            .rows()
            .map(|r| Grant {
                kind: g.s(r, 0).into(),
                schema: g.s(r, 1).into(),
                name: g.s(r, 2).into(),
                identity: g.s(r, 3).into(),
                owner: g.s(r, 4).into(),
                grantor: g.s(r, 5).into(),
                grantee: g.s(r, 6).into(),
                privilege: g.s(r, 7).into(),
                grantable: g.s(r, 8) == "t",
                builtin: g.s(r, 9) == "t",
                rls: g.s(r, 10) == "t",
                future: g.s(r, 11) == "t",
                public: g.s(r, 12) == "t",
                column: g.s(r, 13).into(),
            })
            .collect();
        let r = &sets[1];
        let roles = r.rows().map(|i| (r.s(i, 0).into(), r.s(i, 1) == "t", r.s(i, 2) == "t")).collect();
        let m = &sets[2];
        let memberships = m
            .rows()
            .map(|r| Membership {
                member: m.s(r, 0).into(),
                parent: m.s(r, 1).into(),
                grantor: m.s(r, 2).into(),
                admin: m.s(r, 3) == "t",
                inherit: m.s(r, 4) == "t",
                set_role: m.s(r, 5) == "t",
            })
            .collect();
        let schemas = grants.iter().map(|g| g.schema.clone()).collect::<BTreeSet<_>>().into_iter().collect();
        let user_schemas = grants.iter().filter(|g| !g.is_system()).map(|g| g.schema.clone()).collect::<BTreeSet<_>>().into_iter().collect();
        let kinds = grants.iter().map(|g| g.kind.clone()).collect::<BTreeSet<_>>().into_iter().collect();
        let user_object_count = grants.iter().filter(|g| !g.future && !g.is_system()).map(|g| (&g.kind, &g.identity, &g.column)).collect::<BTreeSet<_>>().len();
        let public_count = grants.iter().filter(|g| g.public && !g.future && !g.is_system()).count();
        let search_text = grants
            .iter()
            .map(|g| {
                [&g.schema, &g.name, &g.identity, &g.column, &g.owner, &g.grantee, &g.grantor, &g.privilege]
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
                    .to_lowercase()
            })
            .collect();
        Ok(Self { grants, roles, memberships, schemas, user_schemas, kinds, user_object_count, public_count, search_text })
    }

    /// Prefer an automatically inherited route. Keep membership-only paths
    /// visible too, without presenting them as effective authorization.
    pub fn routes(&self, role: &str) -> HashMap<String, Route> {
        let mut found = HashMap::new();
        let mut edges: HashMap<&str, Vec<&Membership>> = HashMap::new();
        for edge in &self.memberships {
            edges.entry(&edge.member).or_default().push(edge);
        }
        let mut queue = VecDeque::from([Route { roles: vec![role.into()], inherited: true }]);
        while let Some(route) = queue.pop_front() {
            let current = route.roles.last().unwrap();
            if found.get(current).is_some_and(|old: &Route| old.inherited || !route.inherited) {
                continue;
            }
            found.insert(current.clone(), route.clone());
            for edge in edges.get(current.as_str()).into_iter().flatten() {
                if route.roles.contains(&edge.parent) {
                    continue;
                }
                let mut next = route.clone();
                next.roles.push(edge.parent.clone());
                next.inherited &= edge.inherit;
                queue.push_back(next);
            }
        }
        found
    }
}

/// Safely inspect the selected object's row policies in the SQL editor.
pub fn policies_sql(schema: &str, table: &str) -> String {
    format!("SELECT * FROM pg_policies WHERE schemaname = {} AND tablename = {};", quote_literal(schema), quote_literal(table))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_route_wins_and_cycles_terminate() {
        let edge =
            |a: &str, b: &str, inherit| Membership { member: a.into(), parent: b.into(), grantor: "owner".into(), admin: false, inherit, set_role: true };
        let i = Inventory {
            memberships: vec![edge("login", "readers", false), edge("login", "team", true), edge("team", "readers", true), edge("readers", "login", true)],
            ..Default::default()
        };
        let r = i.routes("login");
        assert!(r["readers"].inherited);
        assert_eq!(r["readers"].roles, ["login", "team", "readers"]);
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn sql_quotes_roles_columns_and_default_scope() {
        let mut g = Grant {
            kind: "Column".into(),
            schema: "Mixed Schema".into(),
            name: "t.x".into(),
            identity: "\"s\".\"t\"".into(),
            owner: "o\"wner".into(),
            grantor: "owner".into(),
            grantee: "PUBLIC".into(),
            privilege: "SELECT".into(),
            grantable: true,
            builtin: false,
            rls: false,
            future: false,
            public: false,
            column: "quoted\"column".into(),
        };
        assert_eq!(g.sql(false), "GRANT SELECT (\"quoted\"\"column\") ON TABLE \"s\".\"t\" TO \"PUBLIC\" WITH GRANT OPTION;");
        g.future = true;
        g.kind = "Table".into();
        g.public = true;
        assert_eq!(g.sql(true), "ALTER DEFAULT PRIVILEGES FOR ROLE \"o\"\"wner\" IN SCHEMA \"Mixed Schema\"\nREVOKE SELECT ON TABLES FROM PUBLIC;");
    }
}
