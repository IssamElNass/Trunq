//! EXPLAIN (FORMAT JSON) visualiser: a plan tree with timing / cost bars.

use egui::{RichText, Ui};
use serde_json::Value;

use super::theme;
use crate::db::result::ResultSet;

pub struct PlanNode {
    pub title: String,
    pub detail: String,
    pub depth: usize,
    /// Exclusive time (ms) when analysed, otherwise exclusive cost.
    pub self_value: f64,
    pub rows_est: f64,
    pub rows_actual: Option<f64>,
    pub loops: f64,
}

pub struct PlanView {
    pub nodes: Vec<PlanNode>,
    pub total: f64,
    pub analyzed: bool,
    pub planning_ms: Option<f64>,
    pub execution_ms: Option<f64>,
}

impl PlanView {
    pub fn from_result(rs: &ResultSet) -> Option<PlanView> {
        let raw = rs.get(0, 0)?;
        let v: Value = serde_json::from_str(raw).ok()?;
        let root = v.get(0)?;
        let plan = root.get("Plan")?;
        let analyzed = plan.get("Actual Total Time").is_some();
        let mut nodes = Vec::new();
        walk(plan, 0, analyzed, &mut nodes);
        let total = nodes.iter().map(|n| n.self_value).sum::<f64>().max(1e-9);
        Some(PlanView {
            nodes,
            total,
            analyzed,
            planning_ms: root.get("Planning Time").and_then(Value::as_f64),
            execution_ms: root.get("Execution Time").and_then(Value::as_f64),
        })
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let pal = theme::p();
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            if let Some(p) = self.planning_ms {
                ui.label(RichText::new(format!("Planning {p:.2} ms")).color(pal.dim));
            }
            if let Some(e) = self.execution_ms {
                ui.label(RichText::new(format!("Execution {e:.2} ms")).color(pal.dim));
            }
            if !self.analyzed {
                ui.label(RichText::new("Estimated plan (use Explain Analyze for real timings)").color(pal.faint));
            }
        });
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            for n in &self.nodes {
                let pct = n.self_value / self.total;
                ui.horizontal(|ui| {
                    ui.add_space(8.0 + n.depth as f32 * 18.0);
                    let color = if pct > 0.5 {
                        pal.danger
                    } else if pct > 0.2 {
                        pal.warn
                    } else {
                        pal.success
                    };
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(60.0, 10.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 3.0, pal.bg3);
                    let mut fill = rect;
                    fill.set_width((rect.width() * pct as f32).max(1.0));
                    ui.painter().rect_filled(fill, 3.0, color);
                    ui.label(RichText::new(format!("{:>5.1}%", pct * 100.0)).font(theme::mono(11.5)).color(pal.dim));
                    ui.label(RichText::new(&n.title).font(theme::bold(13.0)));
                    if !n.detail.is_empty() {
                        ui.label(RichText::new(&n.detail).color(pal.dim));
                    }
                    if n.loops > 1.0 {
                        ui.label(RichText::new(format!("× {} loops", n.loops as u64)).small().color(pal.faint));
                    }
                    let rows = match n.rows_actual {
                        Some(a) => {
                            let f = if a > 0.0 && n.rows_est > 0.0 { (a / n.rows_est).max(n.rows_est / a) } else { 1.0 };
                            let txt = format!("rows {} (est {})", a as u64, n.rows_est as u64);
                            if f >= 10.0 { RichText::new(format!("{txt} ⚠ {f:.0}×")).color(pal.warn) } else { RichText::new(txt).color(pal.faint) }
                        }
                        None => RichText::new(format!("est rows {}", n.rows_est as u64)).color(pal.faint),
                    };
                    ui.label(rows.small());
                });
            }
        });
    }
}

fn walk(node: &Value, depth: usize, analyzed: bool, out: &mut Vec<PlanNode>) {
    let s = |k: &str| node.get(k).and_then(Value::as_str).unwrap_or("");
    let f = |k: &str| node.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let loops = f("Actual Loops").max(1.0);
    let mine = if analyzed { f("Actual Total Time") * loops } else { f("Total Cost") };
    let children: Vec<&Value> = node.get("Plans").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let child_sum: f64 = children
        .iter()
        .map(|c| {
            let cl = c.get("Actual Loops").and_then(Value::as_f64).unwrap_or(1.0).max(1.0);
            if analyzed {
                c.get("Actual Total Time").and_then(Value::as_f64).unwrap_or(0.0) * cl
            } else {
                c.get("Total Cost").and_then(Value::as_f64).unwrap_or(0.0)
            }
        })
        .sum();
    let mut title = s("Node Type").to_string();
    if !s("Join Type").is_empty() && title.contains("Join") || title == "Nested Loop" && !s("Join Type").is_empty() {
        title = format!("{} {}", s("Join Type"), title);
    }
    let mut detail = Vec::new();
    if !s("Relation Name").is_empty() {
        detail.push(format!(
            "on {}.{}{}",
            s("Schema"),
            s("Relation Name"),
            if s("Alias") != s("Relation Name") && !s("Alias").is_empty() { format!(" {}", s("Alias")) } else { String::new() }
        ));
    }
    if !s("Index Name").is_empty() {
        detail.push(format!("using {}", s("Index Name")));
    }
    for k in ["Index Cond", "Hash Cond", "Merge Cond", "Filter", "Join Filter"] {
        if !s(k).is_empty() {
            detail.push(format!("{}: {}", k.to_lowercase(), s(k)));
        }
    }
    out.push(PlanNode {
        title,
        detail: detail.join(" · "),
        depth,
        self_value: (mine - child_sum).max(0.0),
        rows_est: f("Plan Rows"),
        rows_actual: analyzed.then(|| f("Actual Rows")),
        loops,
    });
    for c in children {
        walk(c, depth + 1, analyzed, out);
    }
}
