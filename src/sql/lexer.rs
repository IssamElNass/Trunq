//! A fast, forgiving PostgreSQL lexer used for syntax highlighting,
//! splitting scripts into statements and understanding completion context.
//! It never fails: unterminated strings/comments simply run to the end.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Whitespace,
    Comment,
    Keyword,
    Ident,
    QuotedIdent,
    String,
    Number,
    Operator,
    Punct,
    Param,
    /// `;` statement terminator
    Semicolon,
    /// psql meta command (`\d`), highlighted but not executed
    Meta,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: Tok,
    pub range: Range<usize>,
}

pub fn tokenize(src: &str) -> Vec<Token> {
    let b = src.as_bytes();
    let n = b.len();
    let mut out = Vec::with_capacity(n / 4);
    let mut i = 0;
    while i < n {
        let start = i;
        let c = b[i];
        let kind = match c {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0c => {
                while i < n && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0c) {
                    i += 1;
                }
                Tok::Whitespace
            }
            b'-' if i + 1 < n && b[i + 1] == b'-' => {
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
                Tok::Comment
            }
            b'/' if i + 1 < n && b[i + 1] == b'*' => {
                i += 2;
                let mut depth = 1;
                while i < n && depth > 0 {
                    if b[i] == b'/' && i + 1 < n && b[i + 1] == b'*' {
                        depth += 1;
                        i += 2;
                    } else if b[i] == b'*' && i + 1 < n && b[i + 1] == b'/' {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                Tok::Comment
            }
            b'\'' => {
                i = skip_string(b, i + 1, false);
                Tok::String
            }
            b'E' | b'e' if i + 1 < n && b[i + 1] == b'\'' => {
                i = skip_string(b, i + 2, true);
                Tok::String
            }
            b'B' | b'b' | b'X' | b'x' | b'N' | b'n' if i + 1 < n && b[i + 1] == b'\'' => {
                i = skip_string(b, i + 2, false);
                Tok::String
            }
            b'U' | b'u' if i + 2 < n && b[i + 1] == b'&' && (b[i + 2] == b'\'' || b[i + 2] == b'"') => {
                let q = b[i + 2];
                i += 3;
                while i < n {
                    if b[i] == q {
                        if i + 1 < n && b[i + 1] == q {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                if q == b'"' { Tok::QuotedIdent } else { Tok::String }
            }
            b'"' => {
                i += 1;
                while i < n {
                    if b[i] == b'"' {
                        if i + 1 < n && b[i + 1] == b'"' {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                Tok::QuotedIdent
            }
            b'$' => {
                if i + 1 < n && b[i + 1].is_ascii_digit() {
                    i += 1;
                    while i < n && b[i].is_ascii_digit() {
                        i += 1;
                    }
                    Tok::Param
                } else if let Some(end) = dollar_tag_end(b, i) {
                    let tag = &src[i..end];
                    i = end;
                    match src[i..].find(tag) {
                        Some(p) => i += p + tag.len(),
                        None => i = n,
                    }
                    Tok::String
                } else {
                    i += 1;
                    Tok::Operator
                }
            }
            b'0'..=b'9' => {
                i = skip_number(b, i);
                Tok::Number
            }
            b'.' if i + 1 < n && b[i + 1].is_ascii_digit() => {
                i = skip_number(b, i);
                Tok::Number
            }
            b';' => {
                i += 1;
                Tok::Semicolon
            }
            b'(' | b')' | b'[' | b']' | b',' | b'.' => {
                i += 1;
                Tok::Punct
            }
            b':' if i + 1 < n && b[i + 1] == b':' => {
                i += 2;
                Tok::Operator
            }
            b'\\' if is_line_start(b, i) => {
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
                Tok::Meta
            }
            b'+' | b'-' | b'*' | b'/' | b'<' | b'>' | b'=' | b'~' | b'!' | b'@' | b'#' | b'%' | b'^' | b'&' | b'|' | b'`' | b'?' | b':' => {
                i += 1;
                while i < n && matches!(b[i], b'+' | b'*' | b'<' | b'>' | b'=' | b'~' | b'!' | b'@' | b'#' | b'%' | b'^' | b'&' | b'|' | b'?') {
                    i += 1;
                }
                Tok::Operator
            }
            _ if is_ident_start(c) => {
                while i < n && is_ident_char(b[i]) {
                    i += 1;
                }
                if is_keyword(&src[start..i]) { Tok::Keyword } else { Tok::Ident }
            }
            _ => {
                // Unknown byte (or a multi-byte char not starting an identifier).
                i += utf8_len(c);
                Tok::Punct
            }
        };
        let end = i.min(n);
        out.push(Token { kind, range: start..end });
        i = end;
    }
    out
}

fn utf8_len(c: u8) -> usize {
    match c {
        0xF0..=0xFF => 4,
        0xE0..=0xEF => 3,
        0xC0..=0xDF => 2,
        _ => 1,
    }
}

fn is_line_start(b: &[u8], i: usize) -> bool {
    let mut j = i;
    while j > 0 {
        j -= 1;
        match b[j] {
            b'\n' => return true,
            b' ' | b'\t' => {}
            _ => return false,
        }
    }
    true
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c >= 0x80
}

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

fn skip_string(b: &[u8], mut i: usize, backslash: bool) -> usize {
    let n = b.len();
    while i < n {
        match b[i] {
            b'\\' if backslash => i += 2,
            b'\'' => {
                if i + 1 < n && b[i + 1] == b'\'' {
                    i += 2;
                } else {
                    return i + 1;
                }
            }
            _ => i += 1,
        }
    }
    n
}

fn skip_number(b: &[u8], mut i: usize) -> usize {
    let n = b.len();
    if b[i] == b'0' && i + 1 < n && matches!(b[i + 1], b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
        i += 2;
        while i < n && (b[i].is_ascii_hexdigit() || b[i] == b'_') {
            i += 1;
        }
        return i;
    }
    while i < n && (b[i].is_ascii_digit() || b[i] == b'_') {
        i += 1;
    }
    if i < n && b[i] == b'.' && !(i + 1 < n && b[i + 1] == b'.') {
        i += 1;
        while i < n && (b[i].is_ascii_digit() || b[i] == b'_') {
            i += 1;
        }
    }
    if i < n && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < n && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < n && b[j].is_ascii_digit() {
            i = j;
            while i < n && b[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    i
}

/// If a dollar-quote opening tag starts at `i`, returns its end (exclusive).
fn dollar_tag_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'$' => return Some(j + 1),
            c if c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80 => j += 1,
            _ => return None,
        }
    }
    None
}

/// A statement in a script, with its byte range in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    pub range: Range<usize>,
}

impl Statement {
    pub fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.range.clone()]
    }
}

/// Splits a script into statements, trimming surrounding whitespace and
/// dropping statements that consist only of comments.
pub fn split_statements(src: &str) -> Vec<Statement> {
    let toks = tokenize(src);
    let mut out = Vec::new();
    let mut first: Option<usize> = None;
    let mut last_end = 0;
    let mut has_code = false;
    // `CREATE FUNCTION ... BEGIN ATOMIC ... END;` bodies contain semicolons.
    let mut atomic_depth = 0i32;
    let mut prev_kw = String::new();
    for t in &toks {
        match t.kind {
            Tok::Whitespace => continue,
            Tok::Semicolon if atomic_depth == 0 => {
                if let Some(s) = first.take()
                    && has_code
                {
                    out.push(Statement { range: s..last_end });
                }
                has_code = false;
                prev_kw.clear();
                continue;
            }
            Tok::Meta => {
                // psql meta commands are not sent to the server.
                if let Some(s) = first.take()
                    && has_code
                {
                    out.push(Statement { range: s..last_end });
                }
                has_code = false;
                continue;
            }
            Tok::Keyword => {
                let w = &src[t.range.clone()];
                // CASE ... END nests inside BEGIN ATOMIC ... END
                if (w.eq_ignore_ascii_case("atomic") && prev_kw.eq_ignore_ascii_case("begin")) || (w.eq_ignore_ascii_case("case") && atomic_depth > 0) {
                    atomic_depth += 1;
                } else if w.eq_ignore_ascii_case("end") && atomic_depth > 0 {
                    atomic_depth -= 1;
                }
                prev_kw = w.to_string();
            }
            _ => {}
        }
        if first.is_none() {
            first = Some(t.range.start);
        }
        if t.kind != Tok::Comment {
            has_code = true;
        }
        last_end = t.range.end;
    }
    if let Some(s) = first
        && has_code
    {
        out.push(Statement { range: s..last_end });
    }
    // Leading comments belong to the statement but we trim them away so
    // that error positions line up with what the user sees highlighted.
    out
}

/// Finds the statement under (or immediately before) the cursor.
pub fn statement_at(src: &str, cursor: usize) -> Option<Statement> {
    let stmts = split_statements(src);
    if stmts.is_empty() {
        return None;
    }
    // Inside a statement (or right after its last char / its semicolon)?
    for s in &stmts {
        if cursor >= s.range.start && cursor <= s.range.end {
            return Some(s.clone());
        }
    }
    // Between statements: prefer the one ending on the cursor's line or
    // just before it; otherwise the next one.
    let before = stmts.iter().rev().find(|s| s.range.end <= cursor);
    let after = stmts.iter().find(|s| s.range.start >= cursor);
    match (before, after) {
        (Some(b), Some(a)) => {
            let gap_before = &src[b.range.end..cursor];
            let blank_line_between = gap_before.matches('\n').count() >= 2;
            if blank_line_between { Some(a.clone()) } else { Some(b.clone()) }
        }
        (Some(b), None) => Some(b.clone()),
        (None, Some(a)) => Some(a.clone()),
        (None, None) => None,
    }
}

/// Finds `:name` parameter placeholders (not `::casts`, not array slices,
/// not inside strings / comments). Returns (name, byte range of `:name`).
pub fn find_params(src: &str) -> Vec<(String, Range<usize>)> {
    let toks = tokenize(src);
    let mut out = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if t.kind != Tok::Operator || &src[t.range.clone()] != ":" {
            continue;
        }
        let Some(next) = toks.get(i + 1) else { continue };
        if next.kind != Tok::Ident && next.kind != Tok::Keyword || next.range.start != t.range.end {
            continue;
        }
        let before = src[..t.range.start].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ']' || c == ')' || c == ':' || c == '"') {
            continue;
        }
        out.push((src[next.range.clone()].to_string(), t.range.start..next.range.end));
    }
    out
}

/// Rewrites `:name` placeholders to `$1, $2, …` (the same name reuses its
/// number) and returns the rewritten SQL and the parameter names in order.
pub fn bind_params(src: &str) -> (String, Vec<String>) {
    let mut names: Vec<String> = Vec::new();
    let mut out = String::with_capacity(src.len());
    let mut last = 0;
    for (name, range) in find_params(src) {
        out.push_str(&src[last..range.start]);
        let n = match names.iter().position(|x| *x == name) {
            Some(i) => i + 1,
            None => {
                names.push(name);
                names.len()
            }
        };
        out.push_str(&format!("${n}"));
        last = range.end;
    }
    out.push_str(&src[last..]);
    (out, names)
}

pub fn is_keyword(w: &str) -> bool {
    let mut buf = [0u8; 32];
    if w.len() > buf.len() {
        return false;
    }
    for (i, c) in w.bytes().enumerate() {
        buf[i] = c.to_ascii_lowercase();
    }
    let lw = std::str::from_utf8(&buf[..w.len()]).unwrap_or("");
    KEYWORDS.binary_search(&lw).is_ok()
}

pub fn is_reserved(w: &str) -> bool {
    let lw = w.to_ascii_lowercase();
    RESERVED.binary_search(&lw.as_str()).is_ok()
}

/// Reserved keywords that require quoting when used as identifiers.
pub const RESERVED: &[&str] = &[
    "all",
    "analyse",
    "analyze",
    "and",
    "any",
    "array",
    "as",
    "asc",
    "asymmetric",
    "authorization",
    "binary",
    "both",
    "case",
    "cast",
    "check",
    "collate",
    "collation",
    "column",
    "concurrently",
    "constraint",
    "create",
    "cross",
    "current_catalog",
    "current_date",
    "current_role",
    "current_schema",
    "current_time",
    "current_timestamp",
    "current_user",
    "default",
    "deferrable",
    "desc",
    "distinct",
    "do",
    "else",
    "end",
    "except",
    "false",
    "fetch",
    "for",
    "foreign",
    "freeze",
    "from",
    "full",
    "grant",
    "group",
    "having",
    "ilike",
    "in",
    "initially",
    "inner",
    "intersect",
    "into",
    "is",
    "isnull",
    "join",
    "lateral",
    "leading",
    "left",
    "like",
    "limit",
    "localtime",
    "localtimestamp",
    "natural",
    "not",
    "notnull",
    "null",
    "offset",
    "on",
    "only",
    "or",
    "order",
    "outer",
    "overlaps",
    "placing",
    "primary",
    "references",
    "returning",
    "right",
    "select",
    "session_user",
    "similar",
    "some",
    "symmetric",
    "system_user",
    "table",
    "tablesample",
    "then",
    "to",
    "trailing",
    "true",
    "union",
    "unique",
    "user",
    "using",
    "variadic",
    "verbose",
    "when",
    "where",
    "window",
    "with",
];

/// Keywords highlighted in the editor and offered by autocompletion (sorted).
pub const KEYWORDS: &[&str] = &[
    "abort",
    "absolute",
    "access",
    "action",
    "add",
    "admin",
    "after",
    "aggregate",
    "all",
    "also",
    "alter",
    "always",
    "analyse",
    "analyze",
    "and",
    "any",
    "array",
    "as",
    "asc",
    "asensitive",
    "assertion",
    "assignment",
    "asymmetric",
    "at",
    "atomic",
    "attach",
    "attribute",
    "authorization",
    "backward",
    "before",
    "begin",
    "between",
    "bigint",
    "bigserial",
    "binary",
    "bit",
    "boolean",
    "both",
    "breadth",
    "buffers",
    "by",
    "cache",
    "call",
    "called",
    "cascade",
    "cascaded",
    "case",
    "cast",
    "catalog",
    "chain",
    "char",
    "character",
    "characteristics",
    "check",
    "checkpoint",
    "class",
    "close",
    "cluster",
    "coalesce",
    "collate",
    "collation",
    "column",
    "columns",
    "comment",
    "comments",
    "commit",
    "committed",
    "compression",
    "concurrently",
    "conditional",
    "configuration",
    "conflict",
    "connection",
    "constraint",
    "constraints",
    "content",
    "continue",
    "conversion",
    "copy",
    "cost",
    "costs",
    "create",
    "cross",
    "csv",
    "cube",
    "current",
    "current_catalog",
    "current_date",
    "current_role",
    "current_schema",
    "current_time",
    "current_timestamp",
    "current_user",
    "cursor",
    "cycle",
    "data",
    "database",
    "date",
    "day",
    "deallocate",
    "dec",
    "decimal",
    "declare",
    "default",
    "defaults",
    "deferrable",
    "deferred",
    "definer",
    "delete",
    "delimiter",
    "delimiters",
    "depends",
    "depth",
    "desc",
    "detach",
    "dictionary",
    "disable",
    "discard",
    "distinct",
    "do",
    "document",
    "domain",
    "double",
    "drop",
    "each",
    "else",
    "empty",
    "enable",
    "encoding",
    "encrypted",
    "end",
    "enum",
    "error",
    "escape",
    "event",
    "except",
    "exclude",
    "excluding",
    "exclusive",
    "execute",
    "exists",
    "explain",
    "expression",
    "extension",
    "external",
    "extract",
    "false",
    "family",
    "fetch",
    "filter",
    "finalize",
    "first",
    "float",
    "following",
    "for",
    "force",
    "foreign",
    "format",
    "forward",
    "freeze",
    "from",
    "full",
    "function",
    "functions",
    "generated",
    "global",
    "grant",
    "granted",
    "greatest",
    "group",
    "grouping",
    "groups",
    "handler",
    "having",
    "header",
    "hold",
    "hour",
    "identity",
    "if",
    "ilike",
    "immediate",
    "immutable",
    "implicit",
    "import",
    "in",
    "include",
    "including",
    "increment",
    "indent",
    "index",
    "indexes",
    "inherit",
    "inherits",
    "initially",
    "inline",
    "inner",
    "inout",
    "input",
    "insensitive",
    "insert",
    "instead",
    "int",
    "integer",
    "intersect",
    "interval",
    "into",
    "invoker",
    "is",
    "isnull",
    "isolation",
    "join",
    "json",
    "json_array",
    "json_object",
    "json_table",
    "jsonb",
    "keep",
    "key",
    "keys",
    "label",
    "language",
    "large",
    "last",
    "lateral",
    "leading",
    "leakproof",
    "least",
    "left",
    "level",
    "like",
    "limit",
    "listen",
    "load",
    "local",
    "localtime",
    "localtimestamp",
    "location",
    "lock",
    "locked",
    "logged",
    "mapping",
    "match",
    "matched",
    "materialized",
    "maxvalue",
    "merge",
    "merge_action",
    "method",
    "minute",
    "minvalue",
    "mode",
    "month",
    "move",
    "name",
    "names",
    "national",
    "natural",
    "nchar",
    "nested",
    "new",
    "next",
    "nfc",
    "nfd",
    "nfkc",
    "nfkd",
    "no",
    "none",
    "normalize",
    "normalized",
    "not",
    "nothing",
    "notify",
    "notnull",
    "nowait",
    "null",
    "nullif",
    "nulls",
    "numeric",
    "object",
    "objects",
    "of",
    "off",
    "offset",
    "oids",
    "old",
    "omit",
    "on",
    "only",
    "operator",
    "option",
    "options",
    "or",
    "order",
    "ordinality",
    "others",
    "out",
    "outer",
    "over",
    "overlaps",
    "overlay",
    "overriding",
    "owned",
    "owner",
    "parallel",
    "parameter",
    "parser",
    "partial",
    "partition",
    "passing",
    "password",
    "path",
    "period",
    "placing",
    "plan",
    "plans",
    "policy",
    "position",
    "preceding",
    "precision",
    "prepare",
    "prepared",
    "preserve",
    "primary",
    "prior",
    "privileges",
    "procedural",
    "procedure",
    "procedures",
    "program",
    "publication",
    "quote",
    "quotes",
    "range",
    "read",
    "real",
    "reassign",
    "recheck",
    "recursive",
    "ref",
    "references",
    "referencing",
    "refresh",
    "reindex",
    "relative",
    "release",
    "rename",
    "repeatable",
    "replace",
    "replica",
    "reset",
    "restart",
    "restrict",
    "return",
    "returning",
    "returns",
    "revoke",
    "right",
    "role",
    "rollback",
    "rollup",
    "routine",
    "routines",
    "row",
    "rows",
    "rule",
    "savepoint",
    "scalar",
    "schema",
    "schemas",
    "scroll",
    "search",
    "second",
    "security",
    "select",
    "sequence",
    "sequences",
    "serial",
    "serializable",
    "server",
    "session",
    "session_user",
    "set",
    "setof",
    "sets",
    "settings",
    "share",
    "show",
    "similar",
    "simple",
    "skip",
    "smallint",
    "snapshot",
    "some",
    "source",
    "sql",
    "stable",
    "standalone",
    "start",
    "statement",
    "statistics",
    "stdin",
    "stdout",
    "storage",
    "stored",
    "strict",
    "string",
    "strip",
    "subscription",
    "substring",
    "summary",
    "support",
    "symmetric",
    "sysid",
    "system",
    "system_user",
    "table",
    "tables",
    "tablesample",
    "tablespace",
    "target",
    "temp",
    "template",
    "temporary",
    "text",
    "then",
    "ties",
    "time",
    "timestamp",
    "timestamptz",
    "timing",
    "to",
    "trailing",
    "transaction",
    "transform",
    "treat",
    "trigger",
    "trim",
    "true",
    "truncate",
    "trusted",
    "type",
    "types",
    "uescape",
    "unbounded",
    "uncommitted",
    "unconditional",
    "unencrypted",
    "union",
    "unique",
    "unknown",
    "unlisten",
    "unlogged",
    "until",
    "update",
    "user",
    "using",
    "uuid",
    "vacuum",
    "valid",
    "validate",
    "validator",
    "value",
    "values",
    "varchar",
    "variadic",
    "varying",
    "verbose",
    "version",
    "view",
    "views",
    "volatile",
    "wal",
    "when",
    "where",
    "whitespace",
    "window",
    "with",
    "within",
    "without",
    "work",
    "wrapper",
    "write",
    "xml",
    "year",
    "yes",
    "zone",
];

/// Commonly used built-in functions offered by autocompletion.
pub const BUILTIN_FUNCTIONS: &[&str] = &[
    "abs",
    "age",
    "array_agg",
    "array_length",
    "array_position",
    "array_remove",
    "array_to_string",
    "avg",
    "bool_and",
    "bool_or",
    "btrim",
    "ceil",
    "char_length",
    "clock_timestamp",
    "coalesce",
    "concat",
    "concat_ws",
    "count",
    "cume_dist",
    "current_setting",
    "date_bin",
    "date_part",
    "date_trunc",
    "dense_rank",
    "encode",
    "decode",
    "every",
    "exp",
    "first_value",
    "floor",
    "format",
    "gen_random_uuid",
    "generate_series",
    "generate_subscripts",
    "greatest",
    "has_table_privilege",
    "initcap",
    "jsonb_agg",
    "jsonb_array_elements",
    "jsonb_array_length",
    "jsonb_build_array",
    "jsonb_build_object",
    "jsonb_each",
    "jsonb_extract_path",
    "jsonb_object_agg",
    "jsonb_path_query",
    "jsonb_pretty",
    "jsonb_set",
    "jsonb_strip_nulls",
    "jsonb_typeof",
    "json_agg",
    "json_build_object",
    "lag",
    "last_value",
    "lead",
    "least",
    "left",
    "length",
    "ln",
    "log",
    "lower",
    "lpad",
    "ltrim",
    "make_date",
    "make_interval",
    "max",
    "md5",
    "min",
    "mod",
    "now",
    "nth_value",
    "ntile",
    "nullif",
    "percent_rank",
    "percentile_cont",
    "percentile_disc",
    "pg_cancel_backend",
    "pg_column_size",
    "pg_database_size",
    "pg_relation_size",
    "pg_size_pretty",
    "pg_sleep",
    "pg_terminate_backend",
    "pg_total_relation_size",
    "position",
    "power",
    "random",
    "rank",
    "regexp_match",
    "regexp_matches",
    "regexp_replace",
    "regexp_split_to_table",
    "repeat",
    "replace",
    "reverse",
    "right",
    "round",
    "row_number",
    "row_to_json",
    "rpad",
    "rtrim",
    "set_config",
    "sign",
    "split_part",
    "sqrt",
    "starts_with",
    "statement_timestamp",
    "stddev",
    "string_agg",
    "strpos",
    "substr",
    "substring",
    "sum",
    "to_char",
    "to_date",
    "to_json",
    "to_jsonb",
    "to_number",
    "to_timestamp",
    "to_tsquery",
    "to_tsvector",
    "translate",
    "trim",
    "trunc",
    "unnest",
    "upper",
    "variance",
    "version",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_lists_sorted() {
        assert!(KEYWORDS.windows(2).all(|w| w[0] < w[1]), "KEYWORDS must be sorted");
        assert!(RESERVED.windows(2).all(|w| w[0] < w[1]), "RESERVED must be sorted");
    }

    #[test]
    fn split_basic() {
        let s = "select 1; select ';' ; -- c;\n select $$a;b$$; /* x; */";
        let st: Vec<_> = split_statements(s).iter().map(|x| x.text(s).to_string()).collect();
        assert_eq!(st, vec!["select 1", "select ';'", "-- c;\n select $$a;b$$"]);
    }

    #[test]
    fn split_function_body() {
        let s = "CREATE FUNCTION f() RETURNS int LANGUAGE plpgsql AS $fn$ BEGIN RETURN 1; END $fn$; SELECT f();";
        assert_eq!(split_statements(s).len(), 2);
        let s = "CREATE FUNCTION g() RETURNS int BEGIN ATOMIC SELECT 1; SELECT 2; END; SELECT 3";
        assert_eq!(split_statements(s).len(), 2);
    }

    #[test]
    fn statement_at_cursor() {
        let s = "select 1;\nselect 2;\n\nselect 3";
        assert_eq!(statement_at(s, 3).unwrap().text(s), "select 1");
        assert_eq!(statement_at(s, 9).unwrap().text(s), "select 1");
        assert_eq!(statement_at(s, 12).unwrap().text(s), "select 2");
        assert_eq!(statement_at(s, 21).unwrap().text(s), "select 3");
    }

    #[test]
    fn params() {
        let p: Vec<String> = find_params("select * from t where a = :id and b::text = ':x' and c = arr[lo:hi] -- :no").into_iter().map(|p| p.0).collect();
        assert_eq!(p, vec!["id"]);
        assert_eq!(find_params("where d > :start_date").len(), 1);
        let (sql, names) = bind_params("select * from b where c = :customer_id and s = :status or c2 = :customer_id");
        assert_eq!(sql, "select * from b where c = $1 and s = $2 or c2 = $1");
        assert_eq!(names, vec!["customer_id", "status"]);
    }

    #[test]
    fn tokens() {
        let s = "SELECT e'a\\'b', \"Col\"::int, $1, 1.5e3 FROM t";
        let kinds: Vec<_> = tokenize(s).into_iter().filter(|t| t.kind != Tok::Whitespace).map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                Tok::Keyword,
                Tok::String,
                Tok::Punct,
                Tok::QuotedIdent,
                Tok::Operator,
                Tok::Keyword,
                Tok::Punct,
                Tok::Param,
                Tok::Punct,
                Tok::Number,
                Tok::Keyword,
                Tok::Ident
            ]
        );
    }
}
