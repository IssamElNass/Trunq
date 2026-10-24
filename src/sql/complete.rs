//! Context-aware autocompletion: knows which tables are in scope (with their
//! aliases), whether a table, column, schema or keyword is expected, and
//! ranks candidates by match quality.

use std::collections::HashSet;

use super::lexer::{self, Tok, Token};
use crate::db::meta::{Catalog, RelKind, quote_ident};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Column,
    Table,
    View,
    Schema,
    Function,
    Keyword,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub label: String,
    pub insert: String,
    pub kind: Kind,
    pub detail: String,
    score: i32,
}

#[derive(Debug, PartialEq, Eq)]
enum Expect {
    Relation,
    Column,
    Any,
    /// Only keywords make sense (e.g. right after a table name).
    Keyword,
}

/// A table referenced in the statement, with its alias.
#[derive(Debug, Clone)]
pub struct TableRef {
    pub schema: Option<String>,
    pub name: String,
    pub alias: Option<String>,
}

fn unquote(s: &str) -> String {
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 { s[1..s.len() - 1].replace("\"\"", "\"") } else { s.to_lowercase() }
}

fn sig(src: &str, toks: &[Token]) -> Vec<(Tok, String, usize)> {
    toks.iter().filter(|t| !matches!(t.kind, Tok::Whitespace | Tok::Comment)).map(|t| (t.kind, src[t.range.clone()].to_string(), t.range.start)).collect()
}

/// Finds tables referenced after FROM / JOIN / UPDATE / INTO (+ aliases).
pub fn table_refs(stmt: &str) -> Vec<TableRef> {
    let toks = lexer::tokenize(stmt);
    let s = sig(stmt, &toks);
    let mut out = Vec::new();
    let mut i = 0;
    let is_kw = |t: &(Tok, String, usize), w: &str| t.0 == Tok::Keyword && t.1.eq_ignore_ascii_case(w);
    let mut in_from = false;
    let mut depth = 0i32;
    let mut from_depth = 0i32;
    while i < s.len() {
        let t = &s[i];
        if t.1 == "(" {
            depth += 1;
        } else if t.1 == ")" {
            depth -= 1;
            if depth < from_depth {
                in_from = false;
            }
        }
        let starts_ref = is_kw(t, "from")
            || is_kw(t, "join")
            || is_kw(t, "update")
            || is_kw(t, "into")
            || (is_kw(t, "table") && i > 0 && (is_kw(&s[i - 1], "alter") || is_kw(&s[i - 1], "truncate")))
            || (in_from && t.1 == "," && depth == from_depth);
        if is_kw(t, "from") || is_kw(t, "join") {
            in_from = true;
            from_depth = depth;
        } else if t.0 == Tok::Keyword
            && ["where", "group", "order", "having", "limit", "union", "returning", "set", "on", "window", "offset"].iter().any(|w| t.1.eq_ignore_ascii_case(w))
            && depth == from_depth
        {
            in_from = false;
        }
        if starts_ref {
            let mut j = i + 1;
            if j < s.len() && (is_kw(&s[j], "only") || is_kw(&s[j], "lateral")) {
                j += 1;
            }
            if j < s.len() && matches!(s[j].0, Tok::Ident | Tok::QuotedIdent | Tok::Keyword) && s[j].1 != "(" {
                let mut name = unquote(&s[j].1);
                let mut schema = None;
                j += 1;
                if j + 1 < s.len() && s[j].1 == "." && matches!(s[j + 1].0, Tok::Ident | Tok::QuotedIdent | Tok::Keyword) {
                    schema = Some(name);
                    name = unquote(&s[j + 1].1);
                    j += 2;
                }
                let mut alias = None;
                if j < s.len() && is_kw(&s[j], "as") {
                    j += 1;
                }
                if j < s.len() && matches!(s[j].0, Tok::Ident | Tok::QuotedIdent) {
                    alias = Some(unquote(&s[j].1));
                }
                // Skip obvious keywords that are not table names.
                if !["select", "lateral", "set", "values"].contains(&name.as_str()) {
                    out.push(TableRef { schema, name, alias });
                }
            }
        }
        i += 1;
    }
    out
}

pub struct Completion {
    pub items: Vec<Item>,
    /// Byte range in the full text that the accepted item replaces.
    pub replace: std::ops::Range<usize>,
}

/// Computes suggestions for `text` with the cursor at byte offset `cursor`.
/// `explicit` = user pressed Ctrl+Space (show even with empty prefix).
pub fn complete(text: &str, cursor: usize, catalog: Option<&Catalog>, explicit: bool, upper_kw: bool) -> Option<Completion> {
    let cursor = cursor.min(text.len());
    let before = &text[..cursor];
    // Current word (identifier chars, possibly with an open quote).
    let word_start =
        before.char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '"' || *c == '$').last().map_or(cursor, |(i, _)| i);
    let word = &before[word_start..];
    if word.starts_with('$') {
        return None;
    }
    // Don't complete inside strings / comments.
    let toks = lexer::tokenize(before);
    if let Some(last) = toks.last()
        && last.range.end == cursor
    {
        match last.kind {
            Tok::String | Tok::Comment => return None,
            Tok::Number if !word.is_empty() && word.chars().all(|c| c.is_ascii_digit()) => return None,
            _ => {}
        }
    }
    let after_dot = before[..word_start].ends_with('.');
    if word.is_empty() && !after_dot && !explicit {
        return None;
    }

    // Isolate the current statement.
    let stmt_start = toks.iter().rev().find(|t| t.kind == Tok::Semicolon).map_or(0, |t| t.range.end);
    let stmt_end = text[cursor..].find(';').map_or(text.len(), |p| cursor + p);
    let stmt = &text[stmt_start..stmt_end];
    let refs = table_refs(stmt);

    let stoks = sig(before, &toks);
    let prefix_idx = stoks.iter().rposition(|t| t.2 < word_start || (after_dot && t.2 < word_start - 1));
    let prev: Vec<&(Tok, String, usize)> = match prefix_idx {
        Some(i) => stoks[..=i].iter().rev().take(4).collect(),
        None => vec![],
    };
    let prev_kw = |n: usize| prev.get(n).filter(|t| t.0 == Tok::Keyword).map(|t| t.1.to_ascii_lowercase());

    let mut items: Vec<Item> = Vec::new();
    let word_l = unquote(word.trim_start_matches('"'));
    let push = |items: &mut Vec<Item>, label: String, insert: String, kind: Kind, detail: String| {
        if let Some(score) = score(&label, &word_l) {
            items.push(Item { label, insert, kind, detail, score });
        }
    };

    if after_dot {
        // qualifier.<word>
        let q_end = word_start - 1;
        let q_start = before[..q_end].char_indices().rev().take_while(|(_, c)| c.is_alphanumeric() || *c == '_' || *c == '"').last().map_or(q_end, |(i, _)| i);
        let qualifier = unquote(&before[q_start..q_end]);
        let cat = catalog?;
        // alias or table name?
        let target = refs
            .iter()
            .find(|r| r.alias.as_deref() == Some(qualifier.as_str()))
            .or_else(|| refs.iter().find(|r| r.name == qualifier && r.alias.is_none()))
            .or_else(|| refs.iter().find(|r| r.name == qualifier));
        let rel = match target {
            Some(r) => cat.find_relation(r.schema.as_deref(), &r.name),
            None => cat.find_relation(None, &qualifier).filter(|_| !cat.schemas.iter().any(|s| s.name == qualifier)),
        };
        if let Some(rel) = rel {
            for c in &rel.columns {
                push(&mut items, c.name.clone(), quote_ident(&c.name), Kind::Column, c.data_type.clone());
            }
        }
        if let Some(schema) = cat.schemas.iter().find(|s| s.name == qualifier) {
            for r in &schema.relations {
                let kind = if r.kind.is_table_like() { Kind::Table } else { Kind::View };
                push(&mut items, r.name.clone(), quote_ident(&r.name), kind, r.kind.label().to_string());
            }
            for f in &schema.functions {
                push(&mut items, f.name.clone(), format!("{}(", quote_ident(&f.name)), Kind::Function, format!("({}) → {}", f.args, f.result));
            }
        }
        finish(&mut items);
        return Some(Completion { items, replace: word_start..cursor });
    }

    let expect = match (prev_kw(0).as_deref(), prev.first().map(|t| t.1.as_str())) {
        (Some("from" | "join" | "update" | "into" | "table" | "truncate"), _) => Expect::Relation,
        (
            Some("select" | "where" | "and" | "or" | "on" | "by" | "set" | "having" | "when" | "then" | "else" | "not" | "distinct" | "returning" | "case"),
            _,
        ) => Expect::Column,
        (_, Some("," | "(" | "=" | "<" | ">" | "<=" | ">=" | "<>" | "!=" | "+" | "-" | "*" | "/" | "||")) => {
            // Comma inside FROM list → relation.
            let in_from = stoks[..prefix_idx.map_or(0, |i| i + 1)]
                .iter()
                .rev()
                .find(|t| t.0 == Tok::Keyword && !["as", "on", "and", "or"].contains(&t.1.to_ascii_lowercase().as_str()))
                .is_some_and(|t| ["from", "join"].contains(&t.1.to_ascii_lowercase().as_str()));
            if in_from && prev.first().map(|t| t.1.as_str()) == Some(",") { Expect::Relation } else { Expect::Column }
        }
        (_, Some(_)) if prev.first().is_some_and(|t| matches!(t.0, Tok::Ident | Tok::QuotedIdent)) => Expect::Keyword,
        _ => Expect::Any,
    };

    if let Some(cat) = catalog {
        if matches!(expect, Expect::Column | Expect::Any) {
            let mut seen = HashSet::new();
            for r in &refs {
                if let Some(rel) = cat.find_relation(r.schema.as_deref(), &r.name) {
                    let owner = r.alias.clone().unwrap_or_else(|| rel.name.clone());
                    for c in &rel.columns {
                        if seen.insert(c.name.clone()) {
                            push(&mut items, c.name.clone(), quote_ident(&c.name), Kind::Column, format!("{} · {owner}", c.data_type));
                        } else if let Some(existing) = items.iter_mut().find(|i| i.kind == Kind::Column && i.label == c.name) {
                            existing.detail = format!("{} · ambiguous", c.data_type);
                        }
                    }
                }
            }
            if expect == Expect::Column {
                // Aliases themselves are also useful (alias.col)
                for r in &refs {
                    if let Some(a) = &r.alias {
                        push(&mut items, a.clone(), a.clone(), Kind::Table, format!("alias of {}", r.name));
                    }
                }
                for f in cat.functions().filter(|f| cat.on_search_path(&f.schema)) {
                    push(&mut items, f.name.clone(), format!("{}(", quote_ident(&f.name)), Kind::Function, format!("({})", f.args));
                }
            }
        }
        if matches!(expect, Expect::Relation | Expect::Any) {
            for r in cat.relations() {
                if r.kind == RelKind::Sequence {
                    continue;
                }
                let insert = if cat.on_search_path(&r.schema) { quote_ident(&r.name) } else { r.qualified() };
                let kind = if r.kind.is_table_like() { Kind::Table } else { Kind::View };
                let detail = if r.est_rows >= 0 { format!("{} · ~{} rows", r.schema, crate::util::compact_num(r.est_rows)) } else { r.schema.clone() };
                push(&mut items, r.name.clone(), insert, kind, detail);
            }
            for s in &cat.schemas {
                push(&mut items, s.name.clone(), quote_ident(&s.name), Kind::Schema, "schema".into());
            }
        }
    }
    // Built-ins do not require a connection or a loaded catalog.
    if expect == Expect::Column {
        for f in lexer::BUILTIN_FUNCTIONS {
            push(&mut items, f.to_string(), format!("{f}("), Kind::Function, "built-in".into());
        }
    }
    if expect != Expect::Relation || word.len() >= 2 {
        for k in lexer::KEYWORDS {
            let ins = if upper_kw { k.to_ascii_uppercase() } else { k.to_string() };
            push(&mut items, ins.clone(), ins, Kind::Keyword, String::new());
        }
    }
    finish(&mut items);
    // Hide the popup if the only candidate is exactly what was typed.
    if items.len() == 1 && items[0].insert.eq_ignore_ascii_case(word) {
        return None;
    }
    if items.is_empty() {
        return None;
    }
    Some(Completion { items, replace: word_start..cursor })
}

fn finish(items: &mut Vec<Item>) {
    items.sort_by(|a, b| b.score.cmp(&a.score).then(a.kind.cmp(&b.kind)).then(a.label.len().cmp(&b.label.len())).then(a.label.cmp(&b.label)));
    let mut seen = HashSet::new();
    items.retain(|i| seen.insert((i.label.clone(), i.kind)));
    items.truncate(80);
}

/// Match score: exact > prefix > word-boundary > subsequence; None = no match.
fn score(label: &str, word: &str) -> Option<i32> {
    if word.is_empty() {
        return Some(0);
    }
    let l = label.to_lowercase();
    if l == word {
        return Some(1000);
    }
    if l.starts_with(word) {
        return Some(800 - (l.len() as i32 - word.len() as i32).min(100));
    }
    // matches after an underscore: "uid" -> "user_id" ... or segment prefix
    if l.split('_').any(|seg| seg.starts_with(word)) {
        return Some(500);
    }
    if l.contains(word) {
        return Some(300);
    }
    // subsequence (fuzzy) – only for 2+ chars to avoid noise
    if word.len() >= 2 {
        let mut it = l.chars();
        if word.chars().all(|c| it.any(|x| x == c)) {
            return Some(100);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_with_aliases() {
        let r = table_refs("select * from public.users u, items join orders as o on o.uid = u.id where 1=1");
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].schema.as_deref(), Some("public"));
        assert_eq!(r[0].alias.as_deref(), Some("u"));
        assert_eq!(r[1].name, "items");
        assert_eq!(r[2].name, "orders");
        assert_eq!(r[2].alias.as_deref(), Some("o"));
    }

    #[test]
    fn scoring() {
        assert!(score("user_id", "user").unwrap() > score("auser", "user").unwrap());
        assert!(score("created_at", "cat").is_some());
        assert!(score("abc", "xyz").is_none());
    }

    #[test]
    fn keyword_completion_without_catalog() {
        let c = complete("sel", 3, None, false, true).unwrap();
        assert_eq!(c.items[0].insert, "SELECT");
        assert!(complete("select 'fr", 10, None, false, true).is_none());
    }
}
