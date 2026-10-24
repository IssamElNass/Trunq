//! Interactive JSON tree for JSON / JSONB values: lazy expansion (fast on
//! multi-megabyte documents), search, copy path / PostgreSQL accessor / value.

use std::collections::HashSet;

use egui::{RichText, Ui};
use serde_json::Value;

use super::theme::{self, icon};
use crate::ui::widgets as w;

pub struct JsonTree {
    root: Value,
    /// Column name, for PostgreSQL accessors (`col->'a'->>'b'`).
    column: String,
    pub search: String,
    /// Paths (as strings) that match the search, plus their ancestors.
    hits: HashSet<String>,
    hit_count: usize,
    searched: String,
}

#[derive(Clone)]
enum Seg {
    Key(String),
    Index(usize),
}

fn path_string(path: &[Seg]) -> String {
    let mut s = String::from("$");
    for p in path {
        match p {
            Seg::Key(k) if k.chars().all(|c| c.is_alphanumeric() || c == '_') && !k.is_empty() => {
                s.push('.');
                s.push_str(k);
            }
            Seg::Key(k) => s.push_str(&format!("[{}]", serde_json::to_string(k).unwrap_or_default())),
            Seg::Index(i) => s.push_str(&format!("[{i}]")),
        }
    }
    s
}

fn pg_accessor(column: &str, path: &[Seg], text: bool) -> String {
    let mut s = crate::db::meta::quote_ident(column);
    for (i, p) in path.iter().enumerate() {
        let op = if text && i + 1 == path.len() { "->>" } else { "->" };
        match p {
            Seg::Key(k) => s.push_str(&format!("{op}{}", crate::db::meta::quote_literal(k))),
            Seg::Index(n) => s.push_str(&format!("{op}{n}")),
        }
    }
    s
}

fn scalar(v: &Value) -> (String, egui::Color32) {
    let pal = theme::p();
    match v {
        Value::Null => ("null".into(), pal.null),
        Value::Bool(b) => (b.to_string(), pal.bool_c),
        Value::Number(n) => (n.to_string(), pal.number),
        Value::String(s) => {
            let short: String = s.chars().take(300).collect();
            (format!("\"{}{}\"", short, if s.chars().count() > 300 { "…" } else { "" }), pal.string)
        }
        Value::Array(a) => (format!("[{} items]", a.len()), pal.dim),
        Value::Object(o) => (format!("{{{} keys}}", o.len()), pal.dim),
    }
}

impl JsonTree {
    pub fn parse(text: &str, column: &str) -> Option<JsonTree> {
        let root: Value = serde_json::from_str(text).ok()?;
        if !(root.is_object() || root.is_array()) {
            return None;
        }
        Some(JsonTree { root, column: column.to_string(), search: String::new(), hits: HashSet::new(), hit_count: 0, searched: String::new() })
    }

    fn update_search(&mut self) {
        if self.search == self.searched {
            return;
        }
        self.searched = self.search.clone();
        self.hits.clear();
        self.hit_count = 0;
        let q = self.search.trim().to_lowercase();
        if q.is_empty() {
            return;
        }
        fn walk(v: &Value, path: &mut Vec<Seg>, q: &str, hits: &mut HashSet<String>, count: &mut usize) -> bool {
            let mut any = false;
            match v {
                Value::Object(o) => {
                    for (k, child) in o {
                        path.push(Seg::Key(k.clone()));
                        let self_hit = k.to_lowercase().contains(q);
                        if walk(child, path, q, hits, count) || self_hit {
                            if self_hit {
                                *count += 1;
                            }
                            hits.insert(path_string(path));
                            any = true;
                        }
                        path.pop();
                    }
                }
                Value::Array(a) => {
                    for (i, child) in a.iter().enumerate() {
                        path.push(Seg::Index(i));
                        if walk(child, path, q, hits, count) {
                            hits.insert(path_string(path));
                            any = true;
                        }
                        path.pop();
                    }
                }
                other => {
                    let text = match other {
                        Value::String(s) => s.to_lowercase(),
                        o => o.to_string().to_lowercase(),
                    };
                    if text.contains(q) {
                        *count += 1;
                        any = true;
                    }
                }
            }
            any
        }
        let mut path = Vec::new();
        let mut count = 0;
        walk(&self.root, &mut path, &q, &mut self.hits, &mut count);
        self.hit_count = count;
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let pal = theme::p();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text(format!("{}  Search keys & values", icon::SEARCH)).desired_width(200.0));
            if !self.search.trim().is_empty() {
                ui.label(RichText::new(format!("{} matches", self.hit_count)).small().color(if self.hit_count > 0 { pal.success } else { pal.faint }));
            }
        });
        self.update_search();
        ui.add_space(4.0);
        let root = std::mem::take(&mut self.root);
        let mut path = Vec::new();
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            self.node(ui, None, &root, &mut path, 0);
        });
        self.root = root;
    }

    fn node(&self, ui: &mut Ui, label: Option<String>, v: &Value, path: &mut Vec<Seg>, depth: usize) {
        let pal = theme::p();
        let key_text = |ui: &mut Ui, label: &Option<String>| {
            if let Some(l) = label {
                let hit = !self.search.trim().is_empty() && l.to_lowercase().contains(&self.search.trim().to_lowercase());
                let t = RichText::new(format!("{l}:")).font(theme::mono(12.5)).color(pal.qident);
                ui.label(if hit { t.background_color(pal.warn.gamma_multiply(0.3)) } else { t });
            }
        };
        let menu = |resp: egui::Response, path: &[Seg], v: &Value| {
            resp.context_menu(|ui| {
                if w::menu_item(ui, icon::COPY, &format!("Copy path  {}", path_string(path))).clicked() {
                    ui.ctx().copy_text(path_string(path));
                    ui.close();
                }
                if !path.is_empty() {
                    let acc = pg_accessor(&self.column, path, !(v.is_object() || v.is_array()));
                    if w::menu_item(ui, icon::CODE, &format!("Copy as SQL  {}", acc)).clicked() {
                        ui.ctx().copy_text(acc);
                        ui.close();
                    }
                }
                if w::menu_item(ui, icon::CLIPBOARD, "Copy value").clicked() {
                    let text = match v {
                        Value::String(s) => s.clone(),
                        other => serde_json::to_string_pretty(other).unwrap_or_default(),
                    };
                    ui.ctx().copy_text(text);
                    ui.close();
                }
            });
        };
        match v {
            Value::Object(_) | Value::Array(_) => {
                let id = ui.make_persistent_id(("jt", path_string(path)));
                let force_open = !self.search.trim().is_empty() && (path.is_empty() || self.hits.contains(&path_string(path)));
                let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, depth < 1);
                if force_open {
                    state.set_open(true);
                }
                let (summary, color) = scalar(v);
                let header = state.show_header(ui, |ui| {
                    key_text(ui, &label);
                    let r = ui.add(egui::Label::new(RichText::new(summary).font(theme::mono(12.0)).color(color)).sense(egui::Sense::click()));
                    menu(r, path, v);
                });
                header.body(|ui| match v {
                    Value::Object(o) => {
                        for (k, child) in o {
                            path.push(Seg::Key(k.clone()));
                            self.node(ui, Some(k.clone()), child, path, depth + 1);
                            path.pop();
                        }
                    }
                    Value::Array(a) => {
                        // Very long arrays: render in pages to stay fast.
                        for (i, child) in a.iter().enumerate().take(2000) {
                            path.push(Seg::Index(i));
                            self.node(ui, Some(i.to_string()), child, path, depth + 1);
                            path.pop();
                        }
                        if a.len() > 2000 {
                            ui.label(RichText::new(format!("… {} more items (copy the value to see all)", a.len() - 2000)).small().color(pal.faint));
                        }
                    }
                    _ => {}
                });
            }
            scalar_value => {
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    key_text(ui, &label);
                    let (text, color) = scalar(scalar_value);
                    let hit = !self.search.trim().is_empty() && text.to_lowercase().contains(&self.search.trim().to_lowercase());
                    let t = RichText::new(text).font(theme::mono(12.5)).color(color);
                    let r = ui.add(egui::Label::new(if hit { t.background_color(pal.warn.gamma_multiply(0.3)) } else { t }).sense(egui::Sense::click()).wrap());
                    menu(r, path, scalar_value);
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        let p = vec![Seg::Key("a".into()), Seg::Index(2), Seg::Key("b c".into())];
        assert_eq!(path_string(&p), "$.a[2][\"b c\"]");
        assert_eq!(pg_accessor("payload", &p, true), "payload->'a'->2->>'b c'");
        let mut t = JsonTree::parse(r#"{"user": {"name": "Ana", "tags": ["x", "needle"]}}"#, "payload").unwrap();
        t.search = "needle".into();
        t.update_search();
        assert_eq!(t.hit_count, 1);
        assert!(t.hits.contains("$.user.tags[1]"));
    }
}
