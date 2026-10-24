//! A token-based SQL formatter. It only rearranges whitespace and keyword
//! case; it never changes the meaning of a statement (strings, quoted
//! identifiers, dollar-quoted bodies and comments are preserved verbatim).

use super::lexer::{Tok, tokenize};

const INDENT: &str = "    ";

/// Keywords that start a new top-level clause line.
const CLAUSES: &[&str] =
    &["select", "from", "where", "having", "limit", "offset", "returning", "values", "set", "window", "union", "intersect", "except", "fetch", "using"];

const JOIN_STARTERS: &[&str] = &["join", "left", "right", "full", "inner", "cross", "natural"];

/// Keywords that behave like functions: no space before "(".
const FUNC_KW: &[&str] = &[
    "cast",
    "coalesce",
    "nullif",
    "greatest",
    "least",
    "extract",
    "substring",
    "position",
    "overlay",
    "trim",
    "row",
    "array",
    "exists",
    "any",
    "some",
    "all",
    "json_object",
    "json_array",
    "json_table",
    "normalize",
    "treat",
    "filter",
    "over",
    "grouping",
    "xmlelement",
    "left",
    "right",
    "format",
    "char",
    "varchar",
    "numeric",
    "decimal",
    "timestamp",
    "time",
    "interval",
    "bit",
    "character",
    "float",
    "replace",
    "in",
    "values",
];

#[derive(Clone, Copy, PartialEq)]
enum Paren {
    /// `( SELECT ...)` – indented block
    Subquery,
    /// column list of CREATE TABLE etc. – one item per line
    List,
    Inline,
}

pub fn format_sql(src: &str, upper: bool) -> String {
    let toks = tokenize(src);
    let sig: Vec<(Tok, &str)> = toks.iter().filter(|t| t.kind != Tok::Whitespace).map(|t| (t.kind, &src[t.range.clone()])).collect();
    let mut out = String::with_capacity(src.len() + src.len() / 4);
    let mut level: usize = 0; // indent level of current clause
    let mut parens: Vec<(Paren, usize)> = Vec::new();
    let mut clause = String::new();
    let mut stmt_first_kw = String::new();
    let mut prev: Option<(Tok, String)> = None;
    let mut need_space = false;
    let mut between_depth = 0usize;

    let newline = |out: &mut String, level: usize| {
        while out.ends_with(' ') {
            out.pop();
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        for _ in 0..level {
            out.push_str(INDENT);
        }
    };

    for (idx, &(kind, text)) in sig.iter().enumerate() {
        let lower = text.to_ascii_lowercase();
        let next = sig.get(idx + 1);
        let next_lower = next.map(|n| n.1.to_ascii_lowercase()).unwrap_or_default();
        let prev_lower = prev.as_ref().map(|p| p.1.to_ascii_lowercase()).unwrap_or_default();
        let in_inline = matches!(parens.last(), Some((Paren::Inline, _)));
        let word = if kind == Tok::Keyword && upper && (should_upper(&lower) || keyword_pair(&prev_lower, &lower)) {
            text.to_ascii_uppercase()
        } else {
            text.to_string()
        };

        if kind == Tok::Comment {
            if text.starts_with("--") {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(text);
                newline(&mut out, level + usize::from(!clause.is_empty()));
                need_space = false;
            } else {
                if need_space {
                    out.push(' ');
                }
                out.push_str(text);
                need_space = true;
            }
            continue;
        }
        if kind == Tok::Meta {
            newline(&mut out, 0);
            out.push_str(text);
            out.push('\n');
            need_space = false;
            continue;
        }
        if kind == Tok::Semicolon {
            while out.ends_with(' ') || out.ends_with('\n') {
                out.pop();
            }
            out.push_str(";\n\n");
            level = 0;
            parens.clear();
            clause.clear();
            stmt_first_kw.clear();
            prev = None;
            need_space = false;
            continue;
        }
        if stmt_first_kw.is_empty() && kind == Tok::Keyword {
            stmt_first_kw = lower.clone();
        }

        // --- line breaks before clause keywords -------------------------
        let is_join_start = kind == Tok::Keyword
            && JOIN_STARTERS.contains(&lower.as_str())
            && !JOIN_STARTERS.contains(&prev_lower.as_str())
            && (lower == "join" || JOIN_STARTERS.contains(&next_lower.as_str()) || next_lower == "outer" || next_lower == "join")
            && next_lower != "(";
        let is_clause = kind == Tok::Keyword
            && !in_inline
            && (CLAUSES.contains(&lower.as_str())
                || (lower == "group" || lower == "order" || lower == "partition") && next_lower == "by"
                || lower == "on" && next_lower == "conflict"
                || lower == "insert"
                || lower == "update" && prev_lower != "for" && prev_lower != "do"
                || lower == "delete"
                || lower == "with" && prev.is_none());
        let valid_clause_pos = !(lower == "using" && stmt_first_kw != "delete" && stmt_first_kw != "merge")
            && !(lower == "set" && (prev_lower == "do" || stmt_first_kw == "alter"))
            && !(lower == "values" && prev_lower == "default")
            && !(lower == "from" && (clause == "extract" || prev_lower == "distinct" || in_inline))
            && !(lower == "fetch" && clause != "offset" && clause != "order" && clause != "limit");
        if (is_clause && valid_clause_pos) || (is_join_start && !in_inline && clause != "create") {
            if stmt_first_kw == "create" && lower != "select" && lower != "with" && !matches!(parens.last(), Some((Paren::Subquery, _))) {
                // CREATE ... AS SELECT: keep DDL mostly as-is, only break before SELECT.
            } else {
                let base = parens.last().map_or(0, |p| p.1);
                level = base;
                newline(&mut out, level);
                need_space = false;
                if lower == "union" || lower == "intersect" || lower == "except" {
                    clause = lower.clone();
                } else {
                    clause = if is_join_start { "join".into() } else { lower.clone() };
                }
            }
        } else if kind == Tok::Keyword
            && (lower == "and" || lower == "or")
            && !in_inline
            && between_depth == 0
            && matches!(clause.as_str(), "where" | "having" | "join" | "on")
        {
            newline(&mut out, level + 1);
            need_space = false;
        }
        if kind == Tok::Keyword && lower == "between" {
            between_depth += 1;
        } else if kind == Tok::Keyword && lower == "and" && between_depth > 0 {
            between_depth -= 1;
        }

        // --- the token itself -------------------------------------------
        match text {
            "(" => {
                let is_sub = matches!(next_lower.as_str(), "select" | "with" | "values") && next.is_some_and(|n| n.0 == Tok::Keyword);
                let is_list =
                    !is_sub && parens.is_empty() && stmt_first_kw == "create" && matches!(prev.as_ref().map(|p| p.0), Some(Tok::Ident | Tok::QuotedIdent));
                let tight = match prev.as_ref() {
                    Some((Tok::Ident | Tok::QuotedIdent, _)) => true,
                    Some((Tok::Keyword, w)) => {
                        FUNC_KW.contains(&w.to_ascii_lowercase().as_str())
                            && !matches!(w.to_ascii_lowercase().as_str(), "in" | "values" | "exists" | "any" | "all" | "some")
                    }
                    Some((Tok::Punct, p)) => p == "(" || p == "[",
                    Some((Tok::Operator, _)) => false,
                    _ => false,
                };
                if (need_space && !tight) || is_list {
                    out.push(' ');
                }
                out.push('(');
                if is_sub {
                    parens.push((Paren::Subquery, level + 1));
                    level += 1;
                    clause.clear();
                } else if is_list {
                    parens.push((Paren::List, level + 1));
                    newline(&mut out, level + 1);
                } else {
                    parens.push((Paren::Inline, level));
                }
                need_space = false;
            }
            ")" => {
                match parens.pop() {
                    Some((Paren::Subquery, l)) => {
                        level = l.saturating_sub(1);
                        newline(&mut out, level);
                        clause = "sub".into();
                    }
                    Some((Paren::List, l)) => {
                        newline(&mut out, l.saturating_sub(1));
                    }
                    _ => {}
                }
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push(')');
                need_space = true;
            }
            "," => {
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push(',');
                match parens.last() {
                    Some((Paren::List, l)) => {
                        let l = *l;
                        newline(&mut out, l);
                        need_space = false;
                    }
                    Some((Paren::Inline, _)) => need_space = true,
                    _ if matches!(clause.as_str(), "select" | "set" | "returning" | "group" | "order") => {
                        newline(&mut out, level + 1);
                        need_space = false;
                    }
                    _ => need_space = true,
                }
            }
            "." | "::" | "[" => {
                while out.ends_with(' ') {
                    out.pop();
                }
                out.push_str(text);
                need_space = false;
            }
            "]" => {
                out.push(']');
                need_space = true;
            }
            _ => {
                if need_space {
                    out.push(' ');
                }
                out.push_str(&word);
                need_space = true;
                // SELECT / SET / etc. lists: items on their own lines.
                if kind == Tok::Keyword && matches!(lower.as_str(), "select" | "returning") && !in_inline && clause == lower {
                    // put the first item on the next line only if there are several items
                    if has_top_level_comma(&sig[idx + 1..]) {
                        let distinct = next_lower == "distinct";
                        if !distinct {
                            newline(&mut out, level + 1);
                            need_space = false;
                        }
                    }
                } else if kind == Tok::Keyword && lower == "distinct" && clause == "select" && prev_lower == "select" {
                    if next_lower == "on" {
                        // DISTINCT ON (...) stays inline
                    } else if has_top_level_comma(&sig[idx + 1..]) {
                        newline(&mut out, level + 1);
                        need_space = false;
                    }
                } else if kind == Tok::Keyword && lower == "by" && matches!(clause.as_str(), "group" | "order") && has_top_level_comma(&sig[idx + 1..]) {
                    newline(&mut out, level + 1);
                    need_space = false;
                }
            }
        }
        prev = Some((kind, text.to_string()));
    }
    let mut s = out.trim_end().to_string();
    if src.trim_end().ends_with(';') && !s.ends_with(';') {
        s.push(';');
    }
    s.push('\n');
    s
}

/// Non-reserved keywords (`name`, `type`, `value`...) are very often used as
/// column names, so we only uppercase reserved words and a curated set of
/// words that are practically never identifiers.
fn should_upper(w: &str) -> bool {
    const COMMON: &[&str] = &[
        "add",
        "after",
        "alter",
        "always",
        "begin",
        "before",
        "between",
        "bigint",
        "boolean",
        "by",
        "call",
        "cascade",
        "char",
        "character",
        "coalesce",
        "commit",
        "conflict",
        "constraint",
        "copy",
        "cost",
        "database",
        "declare",
        "delete",
        "drop",
        "each",
        "exists",
        "explain",
        "extension",
        "first",
        "function",
        "generated",
        "identity",
        "if",
        "immutable",
        "index",
        "inout",
        "insert",
        "int",
        "integer",
        "interval",
        "language",
        "last",
        "materialized",
        "merge",
        "no",
        "nothing",
        "nulls",
        "numeric",
        "over",
        "owner",
        "partition",
        "plpgsql",
        "preceding",
        "procedure",
        "real",
        "recursive",
        "refresh",
        "rename",
        "replace",
        "restrict",
        "returns",
        "revoke",
        "rollback",
        "rows",
        "savepoint",
        "schema",
        "sequence",
        "serial",
        "bigserial",
        "set",
        "setof",
        "smallint",
        "stable",
        "strict",
        "text",
        "timestamp",
        "timestamptz",
        "transaction",
        "trigger",
        "truncate",
        "unbounded",
        "update",
        "uuid",
        "vacuum",
        "values",
        "varchar",
        "varying",
        "view",
        "volatile",
        "without",
        "zone",
        "jsonb",
        "json",
        "temporary",
        "temp",
        "unlogged",
        "within",
        "filter",
        "following",
        "ties",
        "nowait",
        "skip",
        "locked",
        "precision",
        "double",
        "decimal",
        "date",
        "time",
        "interval",
        "returning",
    ];
    super::lexer::is_reserved(w) || COMMON.contains(&w)
}

/// Non-reserved keywords that are unambiguous after a specific keyword.
fn keyword_pair(prev: &str, w: &str) -> bool {
    matches!(
        (prev, w),
        ("primary" | "foreign", "key")
            | ("no", "action")
            | ("data", "type")
            | ("set", "data" | "default")
            | ("is", "distinct")
            | ("with", "time")
            | ("character", "varying")
            | ("ordinality", _)
            | ("current", "row")
            | ("for", "share" | "key")
    )
}

/// Does the current clause (until the next clause keyword / closing paren
/// at this level) contain a comma at depth 0?
fn has_top_level_comma(rest: &[(Tok, &str)]) -> bool {
    let mut depth = 0i32;
    for &(k, t) in rest {
        match t {
            "(" | "[" => depth += 1,
            ")" | "]" => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            "," if depth == 0 => return true,
            _ => {}
        }
        if k == Tok::Semicolon {
            return false;
        }
        if depth == 0 && k == Tok::Keyword && CLAUSES.contains(&t.to_ascii_lowercase().as_str()) {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_select() {
        let f = format_sql("select a, b from t join u on u.id = t.uid where x = 1 and y in (1,2) order by a", true);
        assert_eq!(f, "SELECT\n    a,\n    b\nFROM t\nJOIN u ON u.id = t.uid\nWHERE x = 1\n    AND y IN (1, 2)\nORDER BY a\n");
    }

    #[test]
    fn preserves_strings_and_subqueries() {
        let f = format_sql("select count(*) from (select 'a,b' as x) s;", true);
        assert_eq!(f, "SELECT count(*)\nFROM (\n    SELECT 'a,b' AS x\n) s;\n");
    }

    #[test]
    fn create_table() {
        let f = format_sql("create table t (id int primary key, name text not null);", true);
        assert_eq!(f, "CREATE TABLE t (\n    id INT PRIMARY KEY,\n    name TEXT NOT NULL\n);\n");
    }
}
