//! Migration safety linter: warns about statements that take heavy locks or
//! rewrite whole tables, the classic causes of production outages, and
//! suggests the safe alternative.

use super::lexer::{Tok, tokenize};

#[derive(Clone, Debug, PartialEq)]
pub struct Lint {
    pub title: String,
    pub why: String,
    pub fix: String,
}

fn words(sql: &str) -> Vec<String> {
    tokenize(sql).into_iter().filter(|t| !matches!(t.kind, Tok::Whitespace | Tok::Comment)).map(|t| sql[t.range].to_ascii_lowercase()).collect()
}

fn has_seq(w: &[String], seq: &[&str]) -> bool {
    w.windows(seq.len()).any(|win| win.iter().zip(seq).all(|(a, b)| a == b))
}

fn has(w: &[String], word: &str) -> bool {
    w.iter().any(|x| x == word)
}

const VOLATILE_DEFAULTS: &[&str] = &["now", "random", "gen_random_uuid", "uuid_generate_v4", "clock_timestamp", "timeofday", "nextval"];

pub fn lint(sql: &str) -> Vec<Lint> {
    let w = words(sql);
    let mut out = Vec::new();
    let first = w.first().map(String::as_str).unwrap_or("");
    let mut push = |title: &str, why: &str, fix: &str| {
        out.push(Lint { title: title.into(), why: why.into(), fix: fix.into() });
    };

    if first == "create" && (has_seq(&w, &["create", "index"]) || has_seq(&w, &["create", "unique", "index"])) && !has(&w, "concurrently") {
        push(
            "CREATE INDEX blocks all writes",
            "A plain CREATE INDEX holds a SHARE lock: INSERT/UPDATE/DELETE on the table wait until the whole index is built.",
            "Use CREATE INDEX CONCURRENTLY (cannot run inside a transaction block).",
        );
    }
    if has_seq(&w, &["drop", "index"]) && !has(&w, "concurrently") {
        push("DROP INDEX takes an ACCESS EXCLUSIVE lock", "Reads and writes on the table queue behind it.", "Use DROP INDEX CONCURRENTLY.");
    }
    if first == "reindex" && !has(&w, "concurrently") {
        push("REINDEX locks the table", "Writes (and reads using the index) are blocked during the rebuild.", "Use REINDEX … CONCURRENTLY (PostgreSQL 12+).");
    }
    if first == "refresh" && has_seq(&w, &["materialized", "view"]) && !has(&w, "concurrently") {
        push(
            "REFRESH MATERIALIZED VIEW blocks readers",
            "Queries on the view wait until the refresh finishes.",
            "Use REFRESH MATERIALIZED VIEW CONCURRENTLY (needs a unique index on the view).",
        );
    }
    if first == "vacuum" && has(&w, "full") {
        push(
            "VACUUM FULL rewrites the table under an ACCESS EXCLUSIVE lock",
            "The table is unavailable for reads and writes until it finishes.",
            "Prefer plain VACUUM, or pg_repack for online compaction.",
        );
    }
    if first == "cluster" {
        push(
            "CLUSTER rewrites the table under an ACCESS EXCLUSIVE lock",
            "The table is unavailable until it finishes.",
            "Consider pg_repack, or schedule a maintenance window.",
        );
    }
    if first == "alter" && has_seq(&w, &["alter", "table"]) {
        // ALTER TABLE t ALTER [COLUMN] c [SET DATA] TYPE ...
        let alters_column = w.iter().skip(1).any(|x| x == "alter");
        if alters_column && has(&w, "type") {
            push(
                "Changing a column type usually rewrites the table",
                "The table is locked (ACCESS EXCLUSIVE) and rewritten, and all indexes rebuilt. Exceptions: varchar length increases, varchar→text.",
                "Add a new column, backfill in batches, then swap, or run during a maintenance window.",
            );
        }
        if has_seq(&w, &["set", "not", "null"]) {
            push(
                "SET NOT NULL scans the whole table under an ACCESS EXCLUSIVE lock",
                "Every row is checked while all reads and writes wait.",
                "First ADD CONSTRAINT … CHECK (col IS NOT NULL) NOT VALID, then VALIDATE CONSTRAINT, then SET NOT NULL (PostgreSQL 12+ skips the scan).",
            );
        }
        // ADD [COLUMN] c ... (but not ADD CONSTRAINT / PRIMARY KEY / ...)
        let adds_column = w
            .iter()
            .position(|x| x == "add")
            .is_some_and(|i| !matches!(w.get(i + 1).map(String::as_str), Some("constraint" | "primary" | "foreign" | "unique" | "check" | "exclude") | None));
        if adds_column {
            let not_null = has_seq(&w, &["not", "null"]);
            let default_pos = w.iter().position(|x| x == "default");
            if let Some(p) = default_pos {
                if w.get(p + 1).is_some_and(|f| VOLATILE_DEFAULTS.contains(&f.as_str())) {
                    push(
                        "Volatile DEFAULT rewrites the whole table",
                        "A default like now() / random() / gen_random_uuid() must be computed per row, so PostgreSQL rewrites the table under an ACCESS EXCLUSIVE lock.",
                        "Add the column without a default (or with a constant), then backfill in batches.",
                    );
                }
            } else if not_null {
                push(
                    "ADD COLUMN … NOT NULL without DEFAULT fails on non-empty tables",
                    "Existing rows would violate the constraint.",
                    "Add a constant DEFAULT (instant since PostgreSQL 11), or add nullable, backfill, then SET NOT NULL safely.",
                );
            }
        }
        let adds_fk = has(&w, "references") || has_seq(&w, &["foreign", "key"]);
        if adds_fk && has(&w, "add") && !has_seq(&w, &["not", "valid"]) {
            push(
                "Adding a FOREIGN KEY validates every row while locking both tables",
                "Writes on the table and the referenced table are blocked during validation.",
                "ADD CONSTRAINT … NOT VALID, then in a separate statement VALIDATE CONSTRAINT (only a light lock).",
            );
        }
        if has_seq(&w, &["add", "constraint"]) && has(&w, "check") && !has_seq(&w, &["not", "valid"]) {
            push(
                "Adding a CHECK constraint scans the table under lock",
                "All rows are validated while writes wait.",
                "ADD CONSTRAINT … NOT VALID, then VALIDATE CONSTRAINT separately.",
            );
        }
        if (has_seq(&w, &["add", "primary", "key"]) || (has_seq(&w, &["add", "constraint"]) && has(&w, "unique"))) && !has_seq(&w, &["using", "index"]) {
            push(
                "Adding a PRIMARY KEY / UNIQUE constraint builds an index under lock",
                "Writes are blocked while the index is built.",
                "CREATE UNIQUE INDEX CONCURRENTLY first, then ADD CONSTRAINT … USING INDEX.",
            );
        }
        if has(&w, "rename") {
            push(
                "Renaming breaks running application code",
                "Queries that use the old name start failing immediately after commit.",
                "Deploy code that handles both names first, or add a view / compatibility column.",
            );
        }
    }
    if !out.is_empty() && (first == "alter" || first == "create" || first == "drop" || first == "reindex" || first == "refresh") {
        out.push(Lint {
            title: "Tip: set a lock_timeout".into(),
            why: "Even a fast DDL statement waits in the lock queue behind long-running queries, and everything else queues behind it.".into(),
            fix: "Run SET lock_timeout = '5s'; first, and retry if it times out.".into(),
        });
    }
    out
}

/// Quick fix: inserts CONCURRENTLY where it is missing (CREATE/DROP INDEX,
/// REINDEX, REFRESH MATERIALIZED VIEW). Returns None if not applicable.
pub fn add_concurrently(sql: &str) -> Option<String> {
    let w = words(sql);
    if has(&w, "concurrently") {
        return None;
    }
    let toks: Vec<_> = tokenize(sql).into_iter().filter(|t| !matches!(t.kind, Tok::Whitespace | Tok::Comment)).collect();
    let lw = |i: usize| toks.get(i).map(|t| sql[t.range.clone()].to_ascii_lowercase()).unwrap_or_default();
    // find the token after which CONCURRENTLY goes
    let after = match (lw(0).as_str(), lw(1).as_str(), lw(2).as_str()) {
        ("create", "index", _) => Some(1),
        ("create", "unique", "index") => Some(2),
        ("drop", "index", _) => Some(1),
        ("reindex", _, _) => Some(1),
        ("refresh", "materialized", "view") => Some(2),
        _ => None,
    }?;
    let pos = toks.get(after)?.range.end;
    Some(format!("{} CONCURRENTLY{}", &sql[..pos], &sql[pos..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(sql: &str) -> Vec<String> {
        lint(sql).into_iter().map(|l| l.title).collect()
    }

    #[test]
    fn index_rules() {
        assert!(titles("create index on t (a)").iter().any(|t| t.contains("CREATE INDEX")));
        assert!(titles("CREATE INDEX CONCURRENTLY i ON t (a)").is_empty());
        assert!(titles("drop index i").iter().any(|t| t.contains("DROP INDEX")));
    }

    #[test]
    fn alter_rules() {
        assert!(titles("alter table t add column c timestamptz default now()").iter().any(|t| t.contains("Volatile")));
        assert!(titles("alter table t add column c int not null").iter().any(|t| t.contains("NOT NULL")));
        assert!(!titles("alter table t add column c int not null default 0").iter().any(|t| t.contains("fails")));
        assert!(titles("alter table t alter column c type bigint").iter().any(|t| t.contains("type")));
        assert!(titles("alter table t add constraint f foreign key (a) references u(id)").iter().any(|t| t.contains("FOREIGN KEY")));
        assert!(!titles("alter table t add constraint f foreign key (a) references u(id) not valid").iter().any(|t| t.contains("FOREIGN KEY")));
        assert!(titles("alter table t alter column c set not null").iter().any(|t| t.contains("SET NOT NULL")));
    }

    #[test]
    fn quick_fix() {
        assert_eq!(add_concurrently("CREATE INDEX i ON t (a)").as_deref(), Some("CREATE INDEX CONCURRENTLY i ON t (a)"));
        assert_eq!(add_concurrently("create unique index on t(a)").as_deref(), Some("create unique index CONCURRENTLY on t(a)"));
        assert_eq!(add_concurrently("REINDEX TABLE t").as_deref(), Some("REINDEX TABLE CONCURRENTLY t"));
        assert!(add_concurrently("select 1").is_none());
    }

    #[test]
    fn harmless() {
        assert!(titles("select * from t").is_empty());
        assert!(titles("update t set a = 1 where id = 2").is_empty());
    }
}
