//! Entity relationship diagram of a database, a schema, or one table and its
//! neighbours. Built from the already loaded catalog, so opening it costs no
//! query at all.
//!
//! Layout: referenced (parent) tables are placed left of the tables that
//! point at them, ordered within each column to reduce crossing lines.
//! Navigation follows design tools: scroll pans, pinch or ⌘ + scroll zooms,
//! tables can be dragged.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use crate::action::{Action, ToastKind};
use crate::db::meta::{Catalog, RelKind, quote_ident};
use crate::ui::theme::{self, icon};
use crate::ui::widgets as w;
use crate::util;

const HEADER_H: f32 = 34.0;
const ROW_H: f32 = 22.0;
const PAD_X: f32 = 12.0;
const RANK_GAP: f32 = 110.0;
const NODE_GAP: f32 = 34.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Columns,
    Keys,
    Names,
}

struct Col {
    name: String,
    ty: String,
    pk: bool,
    fk: bool,
    not_null: bool,
}

struct Node {
    oid: u32,
    schema: String,
    name: String,
    kind: RelKind,
    est_rows: i64,
    comment: Option<String>,
    cols: Vec<Col>,
    pos: Pos2,
    size: Vec2,
}

impl Node {
    fn visible_cols(&self, detail: Detail) -> impl Iterator<Item = (usize, &Col)> {
        self.cols.iter().enumerate().filter(move |(_, c)| match detail {
            Detail::Columns => true,
            Detail::Keys => c.pk || c.fk,
            Detail::Names => false,
        })
    }

    /// World-space y of a column's row centre (header centre when hidden).
    fn col_y(&self, detail: Detail, col: &str) -> f32 {
        match self.visible_cols(detail).position(|(_, c)| c.name == col) {
            Some(i) => self.pos.y + HEADER_H + 4.0 + ROW_H * i as f32 + ROW_H / 2.0,
            None => self.pos.y + HEADER_H / 2.0,
        }
    }

    fn rect(&self) -> Rect {
        Rect::from_min_size(self.pos, self.size)
    }
}

struct Edge {
    from: usize,
    to: usize,
    from_cols: Vec<String>,
    to_cols: Vec<String>,
    name: String,
    /// At least one referencing column is nullable (zero-or-one parent).
    optional: bool,
}

pub struct ErdTab {
    pub profile_id: u64,
    pub database: String,
    pub title: String,
    schema: Option<String>,
    focus: Option<u32>,
    include_views: bool,
    detail: Detail,
    catalog: Option<Arc<Catalog>>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    pan: Vec2,
    zoom: f32,
    search: String,
    selected: Option<usize>,
    dragging: Option<usize>,
    fit_pending: bool,
    needs_rebuild: bool,
    /// Screen rect of the canvas in the last frame.
    view: Rect,
}

impl ErdTab {
    pub fn new(profile_id: u64, database: &str, schema: Option<String>, focus: Option<(u32, String)>) -> ErdTab {
        let title = match (&focus, &schema) {
            (Some((_, name)), _) => format!("ERD · {name}"),
            (None, Some(s)) => format!("ERD · {s}"),
            (None, None) => format!("ERD · {database}"),
        };
        ErdTab {
            profile_id,
            database: database.to_string(),
            title,
            schema,
            focus: focus.map(|f| f.0),
            include_views: false,
            detail: Detail::Columns,
            catalog: None,
            nodes: Vec::new(),
            edges: Vec::new(),
            pan: Vec2::ZERO,
            zoom: 1.0,
            search: String::new(),
            selected: None,
            dragging: None,
            fit_pending: true,
            needs_rebuild: true,
            view: Rect::NOTHING,
        }
    }

    // ---- model ----------------------------------------------------------------

    fn rebuild(&mut self, ui: &Ui) {
        let Some(cat) = self.catalog.clone() else { return };
        let old: HashMap<u32, Pos2> = self.nodes.iter().map(|n| (n.oid, n.pos)).collect();
        let selected_oid = self.selected.and_then(|i| self.nodes.get(i)).map(|n| n.oid);
        let in_scope = |schema: &str, is_system: bool| match &self.schema {
            Some(s) => s == schema,
            None => !is_system,
        };
        let wanted = |k: RelKind| k.is_table_like() || (self.include_views && matches!(k, RelKind::View | RelKind::MatView));
        let mut rels: Vec<&crate::db::meta::Relation> = cat
            .schemas
            .iter()
            .filter(|s| in_scope(&s.name, s.is_system) || self.focus.is_some())
            .flat_map(|s| s.relations.iter())
            .filter(|r| wanted(r.kind) && !r.is_partition)
            .collect();
        if let Some(f) = self.focus {
            // the table, everything it references and everything referencing it
            let mut keep: HashSet<u32> = HashSet::from([f]);
            if let Some(r) = cat.relation_by_oid(f) {
                keep.extend(r.fks.iter().map(|k| k.ref_oid));
            }
            keep.extend(cat.relations().filter(|r| r.fks.iter().any(|k| k.ref_oid == f)).map(|r| r.oid));
            rels.retain(|r| keep.contains(&r.oid));
        }
        let multi_schema = rels.iter().map(|r| r.schema.as_str()).collect::<HashSet<_>>().len() > 1;
        let index: HashMap<u32, usize> = rels.iter().enumerate().map(|(i, r)| (r.oid, i)).collect();
        let header_font = theme::bold(13.0);
        let row_font = FontId::proportional(12.5);
        let type_font = theme::mono(11.5);
        self.nodes = rels
            .iter()
            .map(|r| {
                let fk_cols: HashSet<&str> = r.fks.iter().flat_map(|k| k.columns.iter().map(String::as_str)).collect();
                let cols: Vec<Col> = r
                    .columns
                    .iter()
                    .map(|c| Col {
                        name: c.name.clone(),
                        ty: short_type(&c.data_type),
                        pk: c.is_pk,
                        fk: fk_cols.contains(c.name.as_str()),
                        not_null: c.not_null,
                    })
                    .collect();
                let title = if multi_schema { format!("{}.{}", r.schema, r.name) } else { r.name.clone() };
                let measure = |t: &str, f: &FontId| ui.fonts_mut(|fo| fo.layout_no_wrap(t.to_string(), f.clone(), Color32::WHITE).size().x);
                let mut width = measure(&title, &header_font) + 70.0;
                for c in &cols {
                    width = width.max(measure(&c.name, &row_font) + measure(&c.ty, &type_font) + 3.0 * PAD_X + 26.0);
                }
                Node {
                    oid: r.oid,
                    schema: r.schema.clone(),
                    name: title,
                    kind: r.kind,
                    est_rows: r.est_rows,
                    comment: r.comment.clone(),
                    cols,
                    pos: Pos2::ZERO,
                    size: vec2(width.clamp(170.0, 380.0), 0.0),
                }
            })
            .collect();
        self.edges = rels
            .iter()
            .enumerate()
            .flat_map(|(i, r)| {
                let index = &index;
                r.fks.iter().filter_map(move |k| {
                    let to = *index.get(&k.ref_oid)?;
                    let optional = k.columns.iter().any(|c| r.columns.iter().any(|x| &x.name == c && !x.not_null));
                    Some(Edge { from: i, to, from_cols: k.columns.clone(), to_cols: k.ref_columns.clone(), name: k.name.clone(), optional })
                })
            })
            .collect();
        self.update_sizes();
        let all_known = !self.nodes.is_empty() && self.nodes.iter().all(|n| old.contains_key(&n.oid));
        if all_known {
            for n in &mut self.nodes {
                n.pos = old[&n.oid];
            }
        } else {
            self.layout();
            self.fit_pending = true;
        }
        self.selected = selected_oid.and_then(|o| self.nodes.iter().position(|n| n.oid == o));
    }

    fn update_sizes(&mut self) {
        let detail = self.detail;
        for n in &mut self.nodes {
            let rows = n.visible_cols(detail).count();
            n.size.y = HEADER_H + if rows > 0 { rows as f32 * ROW_H + 8.0 } else { 0.0 };
        }
    }

    /// Layered layout: parents left of children, barycentre ordering per layer.
    fn layout(&mut self) {
        let n = self.nodes.len();
        if n == 0 {
            return;
        }
        let mut parents: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
        for e in &self.edges {
            if e.from != e.to {
                parents[e.from].push(e.to);
                children[e.to].push(e.from);
            }
        }
        // connected components
        let mut comp = vec![usize::MAX; n];
        let mut comps: Vec<Vec<usize>> = Vec::new();
        for start in 0..n {
            if comp[start] != usize::MAX {
                continue;
            }
            let id = comps.len();
            let mut stack = vec![start];
            let mut members = Vec::new();
            comp[start] = id;
            while let Some(v) = stack.pop() {
                members.push(v);
                for &u in parents[v].iter().chain(children[v].iter()) {
                    if comp[u] == usize::MAX {
                        comp[u] = id;
                        stack.push(u);
                    }
                }
            }
            comps.push(members);
        }
        // rank = longest path to a root through references (cycles cut)
        fn rank_of(v: usize, parents: &[Vec<usize>], memo: &mut [Option<usize>], on_stack: &mut [bool]) -> usize {
            if let Some(r) = memo[v] {
                return r;
            }
            on_stack[v] = true;
            let mut r = 0;
            for &p in &parents[v] {
                if !on_stack[p] {
                    r = r.max(rank_of(p, parents, memo, on_stack) + 1);
                }
            }
            on_stack[v] = false;
            memo[v] = Some(r);
            r
        }
        let mut memo = vec![None; n];
        let mut on_stack = vec![false; n];
        let ranks: Vec<usize> = (0..n).map(|v| rank_of(v, &parents, &mut memo, &mut on_stack)).collect();

        let (connected, singles): (Vec<Vec<usize>>, Vec<Vec<usize>>) = comps.into_iter().partition(|c| c.len() > 1);
        let mut connected = connected;
        connected.sort_by_key(|c| std::cmp::Reverse(c.len()));
        let mut y0 = 0.0f32;
        let mut max_w = 0.0f32;
        for members in &connected {
            let max_rank = members.iter().map(|&v| ranks[v]).max().unwrap_or(0);
            let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
            for &v in members {
                layers[ranks[v]].push(v);
            }
            for l in &mut layers {
                l.sort_by(|a, b| self.nodes[*a].name.cmp(&self.nodes[*b].name));
            }
            // barycentre sweeps
            for sweep in 0..6 {
                let down = sweep % 2 == 0;
                let order: Vec<usize> = if down { (1..layers.len()).collect() } else { (0..layers.len().saturating_sub(1)).rev().collect() };
                for li in order {
                    let reference = if down { li - 1 } else { li + 1 };
                    let pos: HashMap<usize, usize> = layers[reference].iter().enumerate().map(|(i, v)| (*v, i)).collect();
                    let mut keyed: Vec<(f32, usize)> = layers[li]
                        .iter()
                        .enumerate()
                        .map(|(i, &v)| {
                            let nbrs: Vec<usize> = parents[v].iter().chain(children[v].iter()).filter_map(|u| pos.get(u).copied()).collect();
                            let key = if nbrs.is_empty() { i as f32 } else { nbrs.iter().sum::<usize>() as f32 / nbrs.len() as f32 };
                            (key, v)
                        })
                        .collect();
                    keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
                    layers[li] = keyed.into_iter().map(|(_, v)| v).collect();
                }
            }
            let heights: Vec<f32> = layers.iter().map(|l| l.iter().map(|&v| self.nodes[v].size.y + NODE_GAP).sum::<f32>() - NODE_GAP).collect();
            let tallest = heights.iter().copied().fold(0.0, f32::max);
            let mut x = 0.0;
            for (li, l) in layers.iter().enumerate() {
                let w = l.iter().map(|&v| self.nodes[v].size.x).fold(0.0, f32::max);
                let mut y = y0 + (tallest - heights[li]) / 2.0;
                for &v in l {
                    self.nodes[v].pos = pos2(x, y);
                    y += self.nodes[v].size.y + NODE_GAP;
                }
                x += w + RANK_GAP;
            }
            max_w = max_w.max(x - RANK_GAP);
            y0 += tallest + RANK_GAP;
        }
        // unrelated tables: packed in rows
        let row_w = max_w.max(1400.0);
        let (mut x, mut row_h) = (0.0f32, 0.0f32);
        let mut singles: Vec<usize> = singles.into_iter().flatten().collect();
        singles.sort_by(|a, b| self.nodes[*a].name.cmp(&self.nodes[*b].name));
        for v in singles {
            let s = self.nodes[v].size;
            if x > 0.0 && x + s.x > row_w {
                x = 0.0;
                y0 += row_h + NODE_GAP;
                row_h = 0.0;
            }
            self.nodes[v].pos = pos2(x, y0);
            x += s.x + NODE_GAP;
            row_h = row_h.max(s.y);
        }
    }

    fn bounds(&self) -> Option<Rect> {
        self.nodes.iter().map(Node::rect).reduce(|a, b| a.union(b))
    }

    fn fit(&mut self, view: Rect) {
        let Some(b) = self.bounds() else { return };
        let margin = 40.0;
        let z = ((view.width() - margin * 2.0) / b.width()).min((view.height() - margin * 2.0) / b.height());
        self.zoom = z.clamp(0.2, 1.2);
        self.pan = view.center() - view.min - b.center().to_vec2() * self.zoom;
    }

    fn center_on(&mut self, i: usize, view: Rect) {
        if self.zoom < 0.7 {
            self.zoom = 0.9;
        }
        let c = self.nodes[i].rect().center();
        self.pan = view.center() - view.min - c.to_vec2() * self.zoom;
        self.selected = Some(i);
    }

    // ---- UI -------------------------------------------------------------------

    pub fn ui(&mut self, ui: &mut Ui, catalog: Option<Arc<Catalog>>, actions: &mut Vec<Action>) {
        let pal = theme::p();
        let changed_catalog = match (&self.catalog, &catalog) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, Some(_)) => true,
            _ => false,
        };
        if changed_catalog {
            self.catalog = catalog;
            self.needs_rebuild = true;
        }
        if self.needs_rebuild && self.catalog.is_some() {
            self.needs_rebuild = false;
            self.rebuild(ui);
        }

        let mut view_rect = ui.max_rect();
        let mut do_fit = false;
        let mut do_layout = false;
        let mut find: Option<String> = None;
        egui::Frame::NONE.fill(pal.bg1).inner_margin(egui::Margin { left: 14, right: 14, top: 8, bottom: 8 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if let Some(cat) = &self.catalog {
                    let mut names: Vec<(Option<String>, String)> = vec![(None, "All schemas".to_string())];
                    names.extend(cat.schemas.iter().filter(|s| !s.is_system).map(|s| (Some(s.name.clone()), s.name.clone())));
                    let mut idx = names.iter().position(|(s, _)| *s == self.schema).unwrap_or(0);
                    let view: Vec<(usize, &str)> = names.iter().enumerate().map(|(i, (_, l))| (i, l.as_str())).collect();
                    if w::dropdown(ui, &mut idx, &view, 170.0) {
                        self.schema = names[idx].0.clone();
                        self.focus = None;
                        self.title = format!("ERD · {}", self.schema.clone().unwrap_or_else(|| self.database.clone()));
                        self.nodes.clear();
                        self.needs_rebuild = true;
                    }
                }
                if w::segmented(
                    ui,
                    &mut self.detail,
                    &[(Detail::Columns, icon::ROWS, "Columns"), (Detail::Keys, icon::KEY, "Keys"), (Detail::Names, icon::TABLE, "Tables")],
                ) {
                    self.update_sizes();
                    do_layout = true;
                }
                if w::toggle(ui, self.include_views, icon::EYE, "Views").on_hover_text("Include views and materialized views").clicked() {
                    self.include_views = !self.include_views;
                    self.nodes.clear();
                    self.needs_rebuild = true;
                }
                if self.focus.is_some() {
                    let name = self.focus.and_then(|f| self.nodes.iter().find(|n| n.oid == f)).map(|n| n.name.clone()).unwrap_or_default();
                    if w::chip(ui, &format!("Neighbours of {name}"), pal.accent) {
                        self.focus = None;
                        self.nodes.clear();
                        self.needs_rebuild = true;
                    }
                }
                let r = w::input(ui, &mut self.search, "Find table", Some(icon::SEARCH), 200.0);
                if (r.changed() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))) && !self.search.trim().is_empty() {
                    find = Some(self.search.trim().to_lowercase());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    w::menu_button(ui, w::Kind::Secondary, icon::DOWNLOAD, "Export", |ui| {
                        if w::menu_item(ui, icon::FILE_CODE, "Save as SVG…").clicked()
                            && let Some(path) =
                                rfd::FileDialog::new().set_file_name(format!("{}.svg", self.file_stem())).add_filter("SVG", &["svg"]).save_file()
                        {
                            let msg = match std::fs::write(&path, self.to_svg()) {
                                Ok(()) => (ToastKind::Success, format!("Saved {}", path.display())),
                                Err(e) => (ToastKind::Error, format!("Could not save: {e}")),
                            };
                            actions.push(Action::Toast(msg.0, msg.1));
                        }
                        if w::menu_item(ui, icon::COPY, "Copy as Mermaid").clicked() {
                            ui.ctx().copy_text(self.to_mermaid());
                            actions.push(Action::Toast(ToastKind::Success, "Mermaid diagram copied".into()));
                        }
                        if w::menu_item(ui, icon::COPY, "Copy as DBML").clicked() {
                            ui.ctx().copy_text(self.to_dbml());
                            actions.push(Action::Toast(ToastKind::Success, "DBML copied".into()));
                        }
                    });
                    if w::icon_button(ui, icon::WORKFLOW, "Tidy up: recompute the layout").clicked() {
                        do_layout = true;
                    }
                    if w::icon_button(ui, icon::EXPAND, "Fit to screen").clicked() {
                        do_fit = true;
                    }
                    if w::icon_button(ui, icon::PLUS, "Zoom in").clicked() {
                        self.zoom_at(1.2, None);
                    }
                    ui.label(egui::RichText::new(format!("{:.0}%", self.zoom * 100.0)).color(pal.dim).font(theme::mono(12.0)));
                    if w::icon_button(ui, icon::MINUS, "Zoom out").clicked() {
                        self.zoom_at(1.0 / 1.2, None);
                    }
                    ui.label(egui::RichText::new(format!("{} tables · {} relationships", self.nodes.len(), self.edges.len())).size(12.0).color(pal.faint));
                });
            });
        });
        let top = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), top, Stroke::new(1.0, pal.border));
        view_rect.min.y = top;

        let (rect, resp) = ui.allocate_exact_size(view_rect.size(), Sense::click_and_drag());
        self.view = rect;
        if self.catalog.is_none() {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "Loading schema…", FontId::proportional(14.0), pal.faint);
            return;
        }
        if do_layout {
            self.layout();
            do_fit = true;
        }
        if (self.fit_pending || do_fit) && rect.width() > 50.0 {
            self.fit_pending = false;
            self.fit(rect);
        }
        if let Some(q) = find
            && let Some(i) = self.nodes.iter().position(|n| n.name.to_lowercase().contains(&q))
        {
            self.center_on(i, rect);
        }

        // ---- input ----
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p));
        let hovered = pointer.and_then(|p| self.nodes.iter().rposition(|n| self.to_screen(n.rect()).contains(p)));
        if resp.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            if zoom != 1.0 {
                self.zoom_at(zoom, pointer);
            } else if scroll != Vec2::ZERO {
                self.pan += scroll;
            }
        }
        if resp.drag_started() {
            self.dragging = hovered;
            if hovered.is_some() {
                self.selected = hovered;
            }
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            match self.dragging {
                Some(i) => self.nodes[i].pos += d / self.zoom,
                None => self.pan += d,
            }
        }
        if resp.drag_stopped() {
            self.dragging = None;
        }
        if resp.clicked() {
            self.selected = hovered;
        }
        if resp.double_clicked()
            && let Some(i) = hovered
        {
            let n = &self.nodes[i];
            actions.push(Action::OpenTable { profile: self.profile_id, db: self.database.clone(), schema: n.schema.clone(), name: raw_name(n) });
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.selected = None;
        }
        let ctx_target = hovered;
        resp.context_menu(|ui| match ctx_target.or(self.selected) {
            Some(i) => {
                let n = &self.nodes[i];
                let (schema, name, oid) = (n.schema.clone(), raw_name(n), n.oid);
                if n.kind.has_data() && w::menu_item(ui, icon::TABLE, "Open data").clicked() {
                    actions.push(Action::OpenTable { profile: self.profile_id, db: self.database.clone(), schema: schema.clone(), name: name.clone() });
                }
                if w::menu_item(ui, icon::TABLE_PROPS, "Structure & health").clicked() {
                    actions.push(Action::OpenStructure { profile: self.profile_id, db: self.database.clone(), oid, title: name.clone(), ddl: false });
                }
                if w::menu_item(ui, icon::NETWORK, "Show only this table and its neighbours").clicked() {
                    self.focus = Some(oid);
                    self.title = format!("ERD · {name}");
                    self.nodes.clear();
                    self.needs_rebuild = true;
                }
                if w::menu_item(ui, icon::COPY, "Copy name").clicked() {
                    ui.ctx().copy_text(format!("{}.{}", quote_ident(&schema), quote_ident(&name)));
                }
            }
            None => {
                if w::menu_item(ui, icon::EXPAND, "Fit to screen").clicked() {
                    self.fit_pending = true;
                }
                if w::menu_item(ui, icon::WORKFLOW, "Tidy up layout").clicked() {
                    self.layout();
                    self.fit_pending = true;
                }
            }
        });
        if let Some(i) = hovered
            && self.dragging.is_none()
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            let n = &self.nodes[i];
            let tip = format!(
                "{} {}.{}{}{}\n\nDouble-click to open data · right-click for more",
                n.kind.label(),
                n.schema,
                raw_name(n),
                if n.est_rows >= 0 { format!("\n~{} rows", util::thousands(n.est_rows as usize)) } else { String::new() },
                n.comment.as_ref().map(|c| format!("\n\n{c}")).unwrap_or_default()
            );
            resp.clone().on_hover_text_at_pointer(tip);
        } else if self.dragging.is_some() || (resp.dragged() && self.dragging.is_none()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }

        // ---- paint ----
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, pal.bg0);
        let step = 24.0 * self.zoom;
        if step >= 10.0 {
            let dot = pal.border.gamma_multiply(0.9);
            let ox = (rect.left() + self.pan.x).rem_euclid(step);
            let oy = (rect.top() + self.pan.y).rem_euclid(step);
            let mut y = rect.top() + oy;
            while y < rect.bottom() {
                let mut x = rect.left() + ox;
                while x < rect.right() {
                    painter.rect_filled(Rect::from_center_size(pos2(x, y), vec2(1.5, 1.5)), 0.0, dot);
                    x += step;
                }
                y += step;
            }
        }
        let focus_node = hovered.or(self.selected);
        let related: HashSet<usize> = match focus_node {
            Some(f) => self.edges.iter().filter(|e| e.from == f || e.to == f).flat_map(|e| [e.from, e.to]).chain([f]).collect(),
            None => HashSet::new(),
        };
        for e in &self.edges {
            let hot = focus_node.is_some_and(|f| e.from == f || e.to == f);
            let color = if hot {
                pal.accent
            } else if focus_node.is_some() {
                pal.border_strong.gamma_multiply(0.45)
            } else {
                pal.border_strong.gamma_multiply(1.3)
            };
            self.paint_edge(&painter, e, color, if hot { 2.0 } else { 1.4 });
        }
        for (i, n) in self.nodes.iter().enumerate() {
            let r = self.to_screen(n.rect());
            if !r.intersects(rect) {
                continue;
            }
            let dim = focus_node.is_some() && !related.contains(&i);
            self.paint_node(&painter, n, r, self.selected == Some(i), hovered == Some(i), dim);
        }
        if self.nodes.is_empty() {
            painter.text(rect.center(), Align2::CENTER_CENTER, "No tables in this scope", FontId::proportional(14.0), pal.faint);
        }
        // hint
        painter.text(
            rect.left_bottom() + vec2(14.0, -12.0),
            Align2::LEFT_BOTTOM,
            "Scroll to pan · pinch or ⌘ + scroll to zoom · drag tables · double-click to open",
            FontId::proportional(11.5),
            pal.faint,
        );
    }

    fn file_stem(&self) -> String {
        self.title.trim_start_matches("ERD · ").replace(['/', ' '], "_")
    }

    fn zoom_at(&mut self, factor: f32, anchor: Option<Pos2>) {
        let old = self.zoom;
        self.zoom = (self.zoom * factor).clamp(0.15, 2.5);
        // keep the point under the cursor (or the view centre) fixed
        let a = anchor.unwrap_or(self.view.center()) - self.view.min;
        self.pan = a - (a - self.pan) * (self.zoom / old);
    }

    fn to_screen(&self, r: Rect) -> Rect {
        Rect::from_min_size(self.view.min + self.pan + r.min.to_vec2() * self.zoom, r.size() * self.zoom)
    }

    fn sp(&self, p: Pos2) -> Pos2 {
        self.view.min + self.pan + p.to_vec2() * self.zoom
    }

    fn paint_edge(&self, painter: &egui::Painter, e: &Edge, color: Color32, width: f32) {
        let (a, b) = (&self.nodes[e.from], &self.nodes[e.to]);
        let ya = a.col_y(self.detail, e.from_cols.first().map_or("", String::as_str));
        let yb = b.col_y(self.detail, e.to_cols.first().map_or("", String::as_str));
        let (ar, br) = (a.rect(), b.rect());
        // pick sides: child usually right of its parent
        let (xa, da, xb, db) = if e.from == e.to {
            (ar.right(), 1.0, ar.right(), 1.0)
        } else if ar.left() >= br.right() {
            (ar.left(), -1.0, br.right(), 1.0)
        } else if br.left() >= ar.right() {
            (ar.right(), 1.0, br.left(), -1.0)
        } else {
            (ar.right(), 1.0, br.right(), 1.0)
        };
        let z = self.zoom;
        let p0 = self.sp(pos2(xa, ya));
        let p3 = self.sp(pos2(xb, yb));
        let reach = ((p3.x - p0.x).abs() / 2.0).max(50.0 * z) + if e.from == e.to { 40.0 * z } else { 0.0 };
        let p1 = p0 + vec2(da * reach, 0.0);
        let p2 = p3 + vec2(db * reach, 0.0);
        let stroke = Stroke::new(width, color);
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke([p0, p1, p2, p3], false, Color32::TRANSPARENT, stroke));
        // crow's foot ("many") at the referencing table
        let s = 9.0 * z;
        let tip = p0 + vec2(da * s, 0.0);
        for dy in [-s * 0.7, 0.0, s * 0.7] {
            painter.line_segment([tip, p0 + vec2(0.0, dy)], stroke);
        }
        // "one" (or zero-or-one) at the referenced table
        let bar = p3 + vec2(db * s * 0.8, 0.0);
        painter.line_segment([bar + vec2(0.0, -s * 0.7), bar + vec2(0.0, s * 0.7)], stroke);
        if e.optional {
            painter.circle(p3 + vec2(db * s * 1.6, 0.0), s * 0.38, theme::p().bg0, stroke);
        } else {
            let bar2 = p3 + vec2(db * s * 1.3, 0.0);
            painter.line_segment([bar2 + vec2(0.0, -s * 0.7), bar2 + vec2(0.0, s * 0.7)], stroke);
        }
    }

    fn paint_node(&self, painter: &egui::Painter, n: &Node, r: Rect, selected: bool, hovered: bool, dim: bool) {
        let pal = theme::p();
        let z = self.zoom;
        let fade = |c: Color32| if dim { c.gamma_multiply(0.35) } else { c };
        let radius = CornerRadius::same((8.0 * z).clamp(2.0, 10.0) as u8);
        if !dim {
            painter.add(
                egui::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(if pal.dark { 90 } else { 25 }) }.as_shape(r, radius),
            );
        }
        let accent = match n.kind {
            RelKind::View | RelKind::MatView => pal.qident,
            RelKind::Foreign => pal.number,
            _ => pal.accent,
        };
        painter.rect(
            r,
            radius,
            fade(pal.bg1),
            Stroke::new(
                if selected { 2.0 } else { 1.0 },
                fade(if selected {
                    accent
                } else if hovered {
                    pal.border_strong
                } else {
                    pal.border
                }),
            ),
            StrokeKind::Inside,
        );
        let header = Rect::from_min_size(r.min, vec2(r.width(), HEADER_H * z));
        painter.rect_filled(
            header.shrink(1.0),
            CornerRadius { nw: radius.nw.saturating_sub(1), ne: radius.ne.saturating_sub(1), sw: 0, se: 0 },
            fade(accent.gamma_multiply(if pal.dark { 0.16 } else { 0.12 })),
        );
        painter.rect_filled(Rect::from_min_size(header.min + vec2(0.0, 1.0), vec2(3.0 * z.max(0.6), header.height() - 2.0)), 0.0, fade(accent));
        let fs = |px: f32| (px * z).round().max(4.0);
        let show_text = z >= 0.45;
        let ico = match n.kind {
            RelKind::View => icon::EYE,
            RelKind::MatView => icon::LAYERS,
            RelKind::Foreign => icon::GLOBE,
            _ => icon::TABLE,
        };
        if z >= 0.3 {
            painter.text(header.left_center() + vec2(PAD_X * z + 4.0 * z, 0.0), Align2::LEFT_CENTER, ico, FontId::proportional(fs(13.0)), fade(accent));
            painter.text(
                header.left_center() + vec2((PAD_X + 22.0) * z, 0.0),
                Align2::LEFT_CENTER,
                &n.name,
                theme::bold(fs(if show_text { 13.0 } else { 18.0 })),
                fade(pal.text),
            );
        }
        if show_text && n.est_rows >= 0 && n.kind.has_data() {
            painter.text(
                header.right_center() - vec2(PAD_X * z, 0.0),
                Align2::RIGHT_CENTER,
                util::compact_num(n.est_rows),
                FontId::proportional(fs(11.0)),
                fade(pal.faint),
            );
        }
        if !show_text {
            return;
        }
        for (row, (_, c)) in n.visible_cols(self.detail).enumerate() {
            let y = r.top() + (HEADER_H + 4.0 + ROW_H * row as f32 + ROW_H / 2.0) * z;
            let x = r.left() + PAD_X * z;
            let (ic, col) = if c.pk {
                (icon::KEY, pal.warn)
            } else if c.fk {
                (icon::LINK, pal.accent)
            } else {
                ("", pal.faint)
            };
            if ic.is_empty() {
                painter.circle_filled(pos2(x + 6.0 * z, y), 2.0 * z, fade(pal.border_strong));
            } else {
                painter.text(pos2(x + 6.0 * z, y), Align2::CENTER_CENTER, ic, FontId::proportional(fs(11.5)), fade(col));
            }
            let name_color = if c.pk || c.not_null { pal.text } else { pal.dim };
            painter.text(pos2(x + 20.0 * z, y), Align2::LEFT_CENTER, &c.name, FontId::proportional(fs(12.5)), fade(name_color));
            painter.text(pos2(r.right() - PAD_X * z, y), Align2::RIGHT_CENTER, &c.ty, theme::mono(fs(11.5)), fade(pal.faint));
        }
    }

    // ---- export ---------------------------------------------------------------

    fn to_mermaid(&self) -> String {
        let id = |n: &Node| mermaid_ident(&format!("{}_{}", n.schema, raw_name(n)));
        let mut out = String::from("erDiagram\n");
        for n in &self.nodes {
            out.push_str(&format!("    {} {{\n", id(n)));
            for c in &n.cols {
                let key = match (c.pk, c.fk) {
                    (true, true) => " PK, FK",
                    (true, false) => " PK",
                    (false, true) => " FK",
                    _ => "",
                };
                out.push_str(&format!("        {} {}{key}\n", mermaid_ident(&c.ty), mermaid_ident(&c.name)));
            }
            out.push_str("    }\n");
        }
        for e in &self.edges {
            let card = if e.optional { "|o--o{" } else { "||--o{" };
            out.push_str(&format!("    {} {card} {} : \"{}\"\n", id(&self.nodes[e.to]), id(&self.nodes[e.from]), e.name.replace('"', "'")));
        }
        out
    }

    fn to_dbml(&self) -> String {
        let q = |s: &str| if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { s.to_string() } else { format!("\"{}\"", s.replace('"', "\\\"")) };
        let mut out = String::new();
        for n in &self.nodes {
            out.push_str(&format!("Table {}.{} {{\n", q(&n.schema), q(&raw_name(n))));
            for c in &n.cols {
                let mut attrs = Vec::new();
                if c.pk {
                    attrs.push("pk".to_string());
                }
                if c.not_null && !c.pk {
                    attrs.push("not null".to_string());
                }
                let attrs = if attrs.is_empty() { String::new() } else { format!(" [{}]", attrs.join(", ")) };
                out.push_str(&format!("  {} {}{attrs}\n", q(&c.name), q(&c.ty)));
            }
            if let Some(cm) = &n.comment {
                out.push_str(&format!("  Note: '{}'\n", cm.replace('\'', "\\'")));
            }
            out.push_str("}\n\n");
        }
        for e in &self.edges {
            let (a, b) = (&self.nodes[e.from], &self.nodes[e.to]);
            let cols = |c: &[String]| if c.len() == 1 { q(&c[0]) } else { format!("({})", c.iter().map(|x| q(x)).collect::<Vec<_>>().join(", ")) };
            out.push_str(&format!(
                "Ref {}: {}.{}.{} > {}.{}.{}\n",
                q(&e.name),
                q(&a.schema),
                q(&raw_name(a)),
                cols(&e.from_cols),
                q(&b.schema),
                q(&raw_name(b)),
                cols(&e.to_cols)
            ));
        }
        out
    }

    fn to_svg(&self) -> String {
        let pal = theme::p();
        let hex = |c: Color32| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
        let b = self.bounds().unwrap_or(Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0))).expand(40.0);
        let mut out = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" width=\"{}\" height=\"{}\" font-family=\"Inter, -apple-system, Segoe UI, sans-serif\">\n<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>\n",
            b.left(),
            b.top(),
            b.width(),
            b.height(),
            b.width().round(),
            b.height().round(),
            b.left(),
            b.top(),
            b.width(),
            b.height(),
            hex(pal.bg0)
        );
        for e in &self.edges {
            let (a, t) = (&self.nodes[e.from], &self.nodes[e.to]);
            let ya = a.col_y(self.detail, e.from_cols.first().map_or("", String::as_str));
            let yb = t.col_y(self.detail, e.to_cols.first().map_or("", String::as_str));
            let (ar, br) = (a.rect(), t.rect());
            let (xa, da, xb, db) = if ar.left() >= br.right() {
                (ar.left(), -1.0, br.right(), 1.0)
            } else if br.left() >= ar.right() {
                (ar.right(), 1.0, br.left(), -1.0)
            } else {
                (ar.right(), 1.0, br.right(), 1.0)
            };
            let reach = ((xb - xa).abs() / 2.0).max(50.0);
            out.push_str(&format!(
                "<path d=\"M{xa} {ya} C{} {ya} {} {yb} {xb} {yb}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.5\"><title>{}</title></path>\n",
                xa + da * reach,
                xb + db * reach,
                hex(pal.border_strong),
                esc(&e.name)
            ));
        }
        for n in &self.nodes {
            let r = n.rect();
            out.push_str(&format!(
                "<g><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"8\" fill=\"{}\" stroke=\"{}\"/>\n<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{HEADER_H}\" rx=\"8\" fill=\"{}\" fill-opacity=\"0.18\"/>\n<text x=\"{}\" y=\"{}\" font-size=\"13\" font-weight=\"700\" fill=\"{}\">{}</text>\n",
                r.left(),
                r.top(),
                r.width(),
                r.height(),
                hex(pal.bg1),
                hex(pal.border),
                r.left(),
                r.top(),
                r.width(),
                hex(pal.accent),
                r.left() + PAD_X,
                r.top() + HEADER_H / 2.0 + 4.5,
                hex(pal.text),
                esc(&n.name)
            ));
            for (row, (_, c)) in n.visible_cols(self.detail).enumerate() {
                let y = r.top() + HEADER_H + 4.0 + ROW_H * row as f32 + ROW_H / 2.0 + 4.0;
                let mark = if c.pk {
                    "PK "
                } else if c.fk {
                    "FK "
                } else {
                    ""
                };
                out.push_str(&format!(
                    "<text x=\"{}\" y=\"{y}\" font-size=\"12\" fill=\"{}\"><tspan fill=\"{}\" font-size=\"10\">{mark}</tspan>{}</text>\n<text x=\"{}\" y=\"{y}\" font-size=\"11\" text-anchor=\"end\" font-family=\"JetBrains Mono, monospace\" fill=\"{}\">{}</text>\n",
                    r.left() + PAD_X,
                    hex(pal.text),
                    hex(if c.pk { pal.warn } else { pal.accent }),
                    esc(&c.name),
                    r.right() - PAD_X,
                    hex(pal.faint),
                    esc(&c.ty)
                ));
            }
            out.push_str("</g>\n");
        }
        out.push_str("</svg>\n");
        out
    }
}

/// The relation name without the schema prefix added for multi-schema views.
fn raw_name(n: &Node) -> String {
    n.name.strip_prefix(&format!("{}.", n.schema)).unwrap_or(&n.name).to_string()
}

/// Shortens verbose type names for the diagram.
fn short_type(t: &str) -> String {
    t.replace("timestamp with time zone", "timestamptz")
        .replace("timestamp without time zone", "timestamp")
        .replace("character varying", "varchar")
        .replace("double precision", "float8")
        .replace("time with time zone", "timetz")
        .replace("time without time zone", "time")
}

fn mermaid_ident(s: &str) -> String {
    let out: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) { format!("_{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_identifiers() {
        assert_eq!(mermaid_ident("numeric(10,2)"), "numeric_10_2_");
        assert_eq!(mermaid_ident("Mixed Case"), "Mixed_Case");
        assert_eq!(mermaid_ident("1st"), "_1st");
        assert_eq!(short_type("timestamp with time zone"), "timestamptz");
    }
}
