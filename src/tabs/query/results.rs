//! The results area of a query / table tab: result tabs, grid, card view,
//! charts, value / JSON viewer, compare, FK peek, export and the footer.

use super::*;
use crate::ui::grid::{ColFilter, TsMode};
use crate::ui::json_tree::JsonTree;
use crate::ui::widgets as w;

const FOOTER_H: f32 = 34.0;
const VIEWER_W: f32 = 380.0;

impl QueryTab {
    /// Development / screenshot automation hooks (TRUNQ_SCRIPT).
    pub fn dev(&mut self, cmd: &str, arg: &str) {
        let idx = self.active_pane;
        let Some(p) = self.panes.get_mut(idx) else { return };
        match cmd {
            "card" => p.card = !p.card,
            "pin" => p.pinned = !p.pinned,
            "side" => self.side_by_side = !self.side_by_side,
            "search" => p.grid.filter = arg.to_string(),
            "finder" => p.grid.open_finder(),
            "hideempty" => p.grid.hide_empty = !p.grid.hide_empty,
            "ts" => {
                self.ts_mode = match arg {
                    "utc" => TsMode::Utc,
                    "local" => TsMode::Local,
                    _ => TsMode::Raw,
                }
            }
            "compare" => self.compare = Some((idx, vec![0, 1], false)),
            "value" => {
                let (r, c) = arg.split_once(',').map(|(a, b)| (a.parse().unwrap_or(0), b.parse().unwrap_or(0))).unwrap_or((0, 0));
                let mut dummy = Vec::new();
                let empty = HashMap::new();
                let mut hist = crate::config::History { entries: Vec::new() };
                let settings = crate::config::Settings::default();
                let mut ctx = TabCtx {
                    settings: &settings,
                    profiles: &[],
                    catalog: None,
                    actions: &mut dummy,
                    history: &mut hist,
                    passwords: &empty,
                    monitor: None,
                    meta: None,
                    table_prefs: &HashMap::new(),
                    snippets: &[],
                };
                self.handle_grid_action(GridAction::ViewValue(r, c), &mut ctx, idx);
            }
            _ => {}
        }
    }

    fn prefs_key(&self) -> Option<String> {
        let t = self.table.as_ref()?;
        Some(format!("{}/{}/{}.{}", self.profile_id?, self.database, t.schema, t.name))
    }

    /// Loads "remember my table" settings before the first query of a table tab.
    pub(super) fn apply_table_prefs(&mut self, ctx: &TabCtx) {
        if self.prefs_applied || self.table.is_none() {
            return;
        }
        self.prefs_applied = true;
        if let Some(key) = self.prefs_key()
            && let Some(p) = ctx.table_prefs.get(&key)
            && let Some(t) = &mut self.table
        {
            t.filter = p.filter.clone();
            t.order = p.order.clone();
            self.prefs_last = Some(p.clone());
        }
    }

    /// Persists the table layout when it changed (checked at most once a second).
    fn persist_table_prefs(&mut self, ctx: &mut TabCtx) {
        if self.prefs_checked.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.prefs_checked = Instant::now();
        let (Some(key), Some(t), Some(p)) = (self.prefs_key(), &self.table, self.panes.iter().find(|p| !p.pinned)) else { return };
        let prefs = crate::config::TablePrefs { grid: p.grid.prefs(&p.rs), filter: t.filter.clone(), order: t.order.clone(), card: p.card };
        if self.prefs_last.as_ref() != Some(&prefs) {
            self.prefs_last = Some(prefs.clone());
            ctx.actions.push(Action::SaveTablePrefs(key, prefs));
        }
    }

    pub(super) fn results(&mut self, ui: &mut Ui, ctx: &mut TabCtx) {
        let pal = theme::p();
        self.poll_background(ctx);
        self.persist_table_prefs(ctx);
        self.result_tabs(ui, ctx);
        ui.painter().hline(ui.available_rect_before_wrap().x_range(), ui.cursor().top(), egui::Stroke::new(1.0, pal.border));

        if self.active_pane >= self.panes.len() {
            self.messages_view(ui);
            return;
        }
        let idx = self.active_pane;

        // plan / chart take the whole area
        {
            let pane = &mut self.panes[idx];
            if let Some(plan) = &mut pane.plan {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.selectable_value(&mut pane.show_plan_raw, false, "Visual");
                    ui.selectable_value(&mut pane.show_plan_raw, true, "Raw JSON");
                });
                if pane.show_plan_raw {
                    egui::ScrollArea::both().show(ui, |ui| {
                        let raw = pane.rs.get(0, 0).unwrap_or("");
                        ui.add(egui::Label::new(RichText::new(crate::ui::grid::json_preview(raw, 100_000)).font(theme::mono(12.0))).selectable(true));
                    });
                } else {
                    plan.ui(ui);
                }
                return;
            }
            if let Some(chart) = &mut pane.chart {
                ui.add_space(6.0);
                crate::ui::chart::ui(ui, chart, &pane.rs, &pane.cats);
                return;
            }
        }

        self.filter_chips(ui);
        let footer_h =
            if ui.available_width() < 1000.0 && (self.panes[idx].more || self.panes[idx].grid.selected_count() > 1) { FOOTER_H * 2.0 } else { FOOTER_H };
        let grid_h = (ui.available_height() - footer_h).max(0.0);
        let viewer_w = if self.viewer.is_some() { VIEWER_W.min(ui.available_width() * 0.45) } else { 0.0 };
        let pinned_idx = self.panes.iter().position(|p| p.pinned).filter(|&i| i != idx && self.side_by_side);
        let mut action = None;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let main_w = (ui.available_width() - viewer_w).max(0.0);
            let (left_w, right_w) = if pinned_idx.is_some() { (main_w / 2.0 - 3.0, main_w / 2.0 - 3.0) } else { (main_w, 0.0) };
            ui.allocate_ui_with_layout(vec2(left_w, grid_h), Layout::top_down(Align::Min), |ui| {
                ui.set_min_size(ui.available_size());
                if pinned_idx.is_some() {
                    let p = &self.panes[idx];
                    pane_label(ui, icon::TABLE, &p.title, p.rs.nrows(), pal.text);
                }
                action = self.pane_body(ui, ctx, idx);
            });
            if let Some(pi) = pinned_idx {
                let (sep, _) = ui.allocate_exact_size(vec2(6.0, grid_h), egui::Sense::hover());
                ui.painter().vline(sep.center().x, sep.y_range(), egui::Stroke::new(1.0, pal.border));
                ui.allocate_ui_with_layout(vec2(right_w, grid_h), Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(ui.available_size());
                    let p = &self.panes[pi];
                    pane_label(ui, icon::PIN, &p.title, p.rs.nrows(), pal.accent);
                    let _ = self.pane_body(ui, ctx, pi);
                });
            }
            self.viewer_ui(ui, grid_h, viewer_w);
        });
        self.footer(ui, ctx, idx);
        self.fk_peek(ui, ctx, idx);
        self.compare_window(ui);
        if let Some(a) = action {
            self.handle_grid_action(a, ctx, idx);
        }
    }

    /// Promise polling for FK peek and export jobs.
    fn poll_background(&mut self, ctx: &mut TabCtx) {
        let done: Vec<String> = self.fk_pending.iter().filter(|(_, p)| p.is_ready()).map(|(k, _)| k.clone()).collect();
        for k in done {
            if let Some(p) = self.fk_pending.remove(&k)
                && let Some(r) = p.take()
            {
                if self.fk_cache.len() > 300 {
                    self.fk_cache.clear();
                }
                self.fk_cache.insert(k, r);
            }
        }
        if let Some(job) = &mut self.export_job {
            for e in job.session.poll() {
                if let Event::ConnectFailed { error, .. } = e {
                    job.promise.set(Err(error));
                }
            }
            if let Some(r) = job.promise.take() {
                let path = job.path.display().to_string();
                match r {
                    Ok(n) => ctx.actions.push(Action::Toast(ToastKind::Success, format!("Exported {} rows to {path}", util::thousands(n as usize)))),
                    Err(e) => ctx.actions.push(Action::Toast(ToastKind::Error, format!("Export failed: {e}"))),
                }
                self.export_job = None; // closes its connection
            }
        }
    }

    fn result_tabs(&mut self, ui: &mut Ui, ctx: &mut TabCtx) {
        let pal = theme::p();
        let compact = ui.available_width() < 760.0;
        egui::Frame::NONE.fill(pal.bg1).inner_margin(egui::Margin { left: 12, right: 12, top: 7, bottom: 7 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let mut close: Option<usize> = None;
                let mut toggle_pin: Option<usize> = None;
                for i in 0..self.panes.len() {
                    if let Some((ri, buf)) = &mut self.renaming
                        && *ri == i
                    {
                        let r = w::input(ui, buf, "Result name", Some(icon::SQUARE_PEN), 180.0);
                        r.request_focus();
                        if r.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            let name = buf.trim().to_string();
                            if !name.is_empty() {
                                self.panes[i].title = name;
                            }
                            self.renaming = None;
                        }
                        continue;
                    }
                    let p = &self.panes[i];
                    let ico = if p.plan.is_some() {
                        icon::WORKFLOW
                    } else if p.pinned {
                        icon::PIN
                    } else {
                        icon::TABLE
                    };
                    let count = (p.plan.is_none()).then(|| util::thousands(p.rs.nrows()));
                    let r = w::tab(ui, self.active_pane == i, ico, &p.title, count.as_deref(), p.pinned.then_some(pal.accent));
                    if r.clicked() {
                        self.active_pane = i;
                    }
                    if r.double_clicked() {
                        self.renaming = Some((i, p.title.clone()));
                    }
                    let sql = p.sql.clone();
                    let pinned = p.pinned;
                    r.on_hover_text(format!("{sql}\n\nDouble-click to rename · right-click for more")).context_menu(|ui| {
                        if w::menu_item(ui, icon::PIN, if pinned { "Unpin" } else { "Pin (keep when re-running)" }).clicked() {
                            toggle_pin = Some(i);
                        }
                        if w::menu_item(ui, icon::SQUARE_PEN, "Rename").clicked() {
                            self.renaming = Some((i, self.panes[i].title.clone()));
                        }
                        if w::menu_item(ui, icon::COPY, "Copy SQL").clicked() {
                            ui.ctx().copy_text(sql.clone());
                        }
                        ui.separator();
                        if w::menu_item(ui, icon::X, "Close result").clicked() {
                            close = Some(i);
                        }
                    });
                }
                if let Some(i) = toggle_pin {
                    self.panes[i].pinned = !self.panes[i].pinned;
                }
                if let Some(i) = close {
                    self.panes.remove(i);
                    self.active_pane = self.active_pane.min(self.panes.len().saturating_sub(1));
                    if self.panes.is_empty() {
                        self.active_pane = usize::MAX;
                    }
                }
                let errors = self.messages.iter().filter(|m| m.kind == MsgKind::Error).count();
                let err_count = (errors > 0).then(|| errors.to_string());
                if w::tab(ui, self.active_pane >= self.panes.len(), icon::BELL, "Messages", err_count.as_deref(), (errors > 0).then_some(pal.danger)).clicked()
                {
                    self.active_pane = usize::MAX;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                let idx = self.active_pane;
                if idx >= self.panes.len() || self.panes[idx].plan.is_some() {
                    return;
                }
                {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    self.export_menu(ui, ctx, idx, compact);
                    let has_pinned = self.panes.iter().any(|p| p.pinned);
                    let p = &mut self.panes[idx];
                    let charting = p.chart.is_some();
                    if w::toggle(ui, charting, icon::CHART_BAR, "Chart").on_hover_text("Plot this result").clicked() {
                        p.chart = if charting { None } else { Some(Default::default()) };
                    }
                    if charting {
                        return;
                    }
                    // columns
                    w::menu_button(ui, w::Kind::Secondary, icon::COLUMNS, if compact { "" } else { "Columns" }, |ui| {
                        ui.set_min_width(250.0);
                        if w::menu_item_shortcut(ui, icon::SEARCH, "Find column", "⌘F").clicked() {
                            p.grid.open_finder();
                        }
                        if w::menu_item(
                            ui,
                            if p.grid.hide_empty { icon::EYE } else { icon::EYE_OFF },
                            if p.grid.hide_empty { "Show empty columns" } else { "Hide empty columns" },
                        )
                        .clicked()
                        {
                            p.grid.hide_empty = !p.grid.hide_empty;
                        }
                        if !p.grid.hidden.is_empty() && w::menu_item(ui, icon::EYE, "Show all columns").clicked() {
                            p.grid.hidden.clear();
                        }
                        ui.separator();
                        w::section(ui, "Visible columns");
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            for c in 0..p.rs.ncols() {
                                let visible = !p.grid.hidden.contains(&c);
                                // keep the menu open while toggling columns
                                let r = column_toggle(ui, visible, &p.rs.columns[c].name);
                                if r.clicked() {
                                    if visible {
                                        p.grid.hidden.insert(c);
                                    } else {
                                        p.grid.hidden.remove(&c);
                                    }
                                }
                            }
                        });
                    });
                    // view options
                    w::menu_button(ui, w::Kind::Secondary, icon::EYE, if compact { "" } else { "View" }, |ui| {
                        ui.set_min_width(260.0);
                        w::section(ui, "Layout");
                        if w::menu_check(ui, !p.card, "Grid").clicked() {
                            p.card = false;
                        }
                        if w::menu_check(ui, p.card, "Row card (one record, vertical)").clicked() {
                            p.card = true;
                        }
                        if has_pinned && !p.pinned && w::menu_check(ui, self.side_by_side, "Side by side with pinned result").clicked() {
                            self.side_by_side = !self.side_by_side;
                        }
                        ui.separator();
                        w::section(ui, "Timestamps (timestamptz)");
                        for m in [TsMode::Raw, TsMode::Local, TsMode::Utc] {
                            if w::menu_check(ui, self.ts_mode == m, m.label()).clicked() {
                                self.ts_mode = m;
                            }
                        }
                    });
                    let p = &mut self.panes[idx];
                    w::input(ui, &mut p.grid.filter, "Search loaded rows", Some(icon::SEARCH), if compact { 180.0 } else { 220.0 });
                }
            });
        });
    }

    fn export_menu(&mut self, ui: &mut Ui, ctx: &mut TabCtx, idx: usize, compact: bool) {
        let pal = theme::p();
        let busy = self.export_job.is_some();
        ui.add_enabled_ui(!busy, |ui| {
            w::menu_button(ui, w::Kind::Secondary, icon::DOWNLOAD, if compact { "" } else { "Export" }, |ui| {
                ui.set_min_width(340.0);
                let p = &self.panes[idx];
                let visible = p.grid.visible_rows();
                let loaded = p.rs.nrows();
                let table = self.table.as_ref().map(|t| format!("{}.{}", quote_ident(&t.schema), quote_ident(&t.name))).unwrap_or_default();
                let base = self.table.as_ref().map(|t| t.name.clone()).unwrap_or_else(|| p.title.to_lowercase().replace(' ', "_"));
                let mut choice: Option<(u8, Format)> = None;
                let section = |ui: &mut Ui, title: String, note: String, which: u8, choice: &mut Option<(u8, Format)>| {
                    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                        ui.label(RichText::new(title).font(theme::bold(13.5)));
                        ui.label(RichText::new(note).small().color(pal.dim));
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                            for f in Format::FILE_FORMATS {
                                if w::small_button(ui, w::Kind::Secondary, "", f.label()).clicked() {
                                    *choice = Some((which, f));
                                }
                            }
                        });
                    });
                };
                if visible != loaded {
                    section(
                        ui,
                        format!("Visible rows ({})", util::thousands(visible)),
                        "Exactly what you see: local search, filters and sort applied.".into(),
                        0,
                        &mut choice,
                    );
                }
                section(
                    ui,
                    format!("Loaded rows ({})", util::thousands(loaded)),
                    if p.more { "Only the rows fetched so far. More rows exist on the server.".into() } else { "This is the complete result.".into() },
                    1,
                    &mut choice,
                );
                section(
                    ui,
                    "Entire result from the server".into(),
                    "Runs the query again on a separate connection and streams every row to the file. Nothing is loaded into memory.".into(),
                    2,
                    &mut choice,
                );
                if let Some((which, f)) = choice {
                    ui.close();
                    if let Some(path) = rfd::FileDialog::new().set_file_name(format!("{base}.{}", f.extension())).save_file() {
                        match which {
                            0 | 1 => {
                                let rows: Vec<usize> =
                                    if which == 0 { p.grid.view_rows().iter().copied().filter(|&r| r < p.rs.nrows()).collect() } else { (0..loaded).collect() };
                                match export::write_file(&path, &p.rs, &rows, &p.cats, f, &table) {
                                    Ok(n) => ctx.actions.push(Action::Toast(
                                        ToastKind::Success,
                                        format!("Exported {} rows to {}", util::thousands(n as usize), path.display()),
                                    )),
                                    Err(e) => ctx.actions.push(Action::Toast(ToastKind::Error, format!("Export failed: {e}"))),
                                }
                            }
                            _ => self.start_export(ctx, ui, idx, path, f, table.clone()),
                        }
                    }
                }
            });
        });
        if let Some(job) = &self.export_job {
            let n = job.progress.load(std::sync::atomic::Ordering::Relaxed);
            if w::small_button(ui, w::Kind::Ghost, icon::X, "Cancel").clicked() {
                job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            ui.spinner();
            ui.label(RichText::new(format!("Exporting… {} rows", util::thousands(n as usize))).small().color(pal.dim));
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
    }

    fn start_export(&mut self, ctx: &TabCtx, ui: &Ui, idx: usize, path: PathBuf, format: Format, table: String) {
        let Some(profile) = self.profile(ctx).cloned() else { return };
        let p = &self.panes[idx];
        let session = Session::connect(&profile, &self.database, "Export", ctx.passwords.get(&profile.id).cloned(), ui.ctx());
        let progress = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let promise = session.export(p.sql.clone(), p.params.clone(), path.clone(), format, p.cats.clone(), table, progress.clone(), cancel.clone());
        self.export_job = Some(ExportJob { session, promise, progress, cancel, path });
    }

    fn filter_chips(&mut self, ui: &mut Ui) {
        let pal = theme::p();
        let Some(p) = self.panes.get_mut(self.active_pane) else { return };
        if p.grid.col_filters.is_empty() {
            return;
        }
        let mut remove = None;
        let mut clear = false;
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                ui.label(RichText::new(icon::FILTER).color(pal.warn));
                for (i, f) in p.grid.col_filters.iter().enumerate() {
                    if w::chip(ui, &f.label(), pal.warn) {
                        remove = Some(i);
                    }
                }
                if w::small_button(ui, w::Kind::Ghost, icon::X, "Clear all").clicked() {
                    clear = true;
                }
                ui.label(RichText::new("Filters apply to the loaded rows").small().color(pal.faint));
            });
        });
        if let Some(i) = remove {
            p.grid.col_filters.remove(i);
        }
        if clear {
            p.grid.col_filters.clear();
        }
    }

    /// Grid or card view of one pane; returns a grid action.
    fn pane_body(&mut self, ui: &mut Ui, ctx: &mut TabCtx, idx: usize) -> Option<GridAction> {
        let font = ctx.settings.grid_font_size;
        let max_chars = ctx.settings.max_cell_chars;
        let ts_mode = self.ts_mode;
        let is_main = idx == self.active_pane;
        let pane = &mut self.panes[idx];
        if pane.card {
            return card_view(ui, pane, ts_mode, is_main);
        }
        let pk_cols: Vec<usize> =
            if is_main { self.table.as_ref().map(|t| t.pk.iter().filter_map(|c| pane.rs.col_index(c)).collect()).unwrap_or_default() } else { vec![] };
        let table_name = self.table.as_ref().map(|t| format!("{}.{}", quote_ident(&t.schema), quote_ident(&t.name))).unwrap_or_default();
        let editable = is_main && self.table.as_ref().is_some_and(|t| !t.pk.is_empty());
        let fk_cols: Vec<usize> = pane.fks.iter().enumerate().filter_map(|(i, f)| f.as_ref().map(|_| i)).collect();
        let edits = if editable { self.table.as_mut().map(|t| &mut t.edits) } else { None };
        let g = grid::GridCtx {
            rs: &pane.rs,
            cats: &pane.cats,
            type_names: &pane.type_names,
            edits,
            font_size: font,
            max_chars,
            table_name: &table_name,
            pk_cols: &pk_cols,
            fk_cols: &fk_cols,
            ts_mode,
        };
        grid::show(ui, egui::Id::unique(("grid", self.uid, idx)), &mut pane.grid, g)
    }

    fn viewer_ui(&mut self, ui: &mut Ui, h: f32, width: f32) {
        let pal = theme::p();
        let Some(v) = &mut self.viewer else { return };
        let mut close = false;
        ui.allocate_ui_with_layout(vec2(width, h), Layout::top_down(Align::Min), |ui| {
            egui::Frame::NONE.fill(pal.bg1).inner_margin(egui::Margin::same(12)).stroke(egui::Stroke::new(1.0, pal.border)).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon::EYE).color(pal.accent));
                    ui.label(RichText::new(&v.title).font(theme::bold(13.5)));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if w::icon_button(ui, icon::X, "Close (Esc)").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            close = true;
                        }
                        if let Some(val) = &v.value
                            && w::icon_button(ui, icon::COPY, "Copy value").clicked()
                        {
                            ui.ctx().copy_text(val.clone());
                        }
                    });
                });
                ui.add_space(4.0);
                if v.tree.is_some() {
                    ui.horizontal(|ui| {
                        w::segmented(ui, &mut v.as_tree, &[(true, icon::LIST_TREE, "Tree"), (false, icon::TEXT, "Text")]);
                        if !v.as_tree {
                            let mut pretty = v.pretty;
                            if w::segmented(ui, &mut pretty, &[(true, "", "Pretty"), (false, "", "Raw")]) {
                                v.pretty = pretty;
                            }
                        }
                    });
                    ui.add_space(4.0);
                }
                if let Some(val) = &v.value {
                    ui.label(RichText::new(format!("{} chars · {}", util::thousands(val.chars().count()), util::bytes(val.len()))).small().color(pal.faint));
                }
                ui.separator();
                match (&v.value, &mut v.tree) {
                    (None, _) => {
                        ui.label(RichText::new("NULL").italics().color(pal.null));
                    }
                    (Some(_), Some(tree)) if v.as_tree => tree.ui(ui),
                    (Some(val), _) => {
                        let text = if v.pretty && v.tree.is_some() { crate::ui::grid::json_preview(val, usize::MAX) } else { val.clone() };
                        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(text).font(theme::mono(12.5))).selectable(true).wrap());
                        });
                    }
                }
            });
        });
        if close {
            self.viewer = None;
        }
    }

    fn footer(&mut self, ui: &mut Ui, ctx: &mut TabCtx, idx: usize) {
        let pal = theme::p();
        let mut fetch: Option<bool> = None;
        let pane = &mut self.panes[idx];
        ui.painter().hline(ui.available_rect_before_wrap().x_range(), ui.cursor().top(), egui::Stroke::new(1.0, pal.border));
        egui::Frame::NONE.fill(pal.bg1).inner_margin(egui::Margin::symmetric(12, 3)).show(ui, |ui| {
            ui.set_min_height(FOOTER_H - 6.0);
            ui.horizontal_wrapped(|ui| {
                let shown = pane.grid.visible_rows();
                let total = pane.rs.nrows();
                let mut txt = format!("{} rows", util::thousands(total));
                if shown != total + self.table.as_ref().map_or(0, |t| t.edits.inserted.len()) {
                    txt = format!("{} of {txt}", util::thousands(shown));
                }
                ui.label(RichText::new(txt).small().color(pal.dim));
                // timing breakdown
                let mut timing = Vec::new();
                if let Some(e) = pane.exec_ms {
                    timing.push(format!("query {}", ms(e)));
                }
                if let Some(f) = pane.fetch_ms.filter(|f| *f >= 0.05) {
                    timing.push(format!("fetch {}", ms(f)));
                }
                timing.push(format!("render {}", ms(pane.grid.render_ms)));
                ui.label(RichText::new(format!("· {}", timing.join(" · "))).small().color(pal.faint)).on_hover_text(
                    "query: until the server returned the first rows (planning + execution)\nfetch: transferring the remaining rows\nrender: drawing the last frame of this grid",
                );
                ui.label(RichText::new(format!("· {}", util::bytes(pane.rs.byte_size()))).small().color(pal.faint));
                let sel = pane.grid.selected_count();
                if sel > 1 {
                    ui.label(RichText::new(format!("· {} selected", util::thousands(sel))).small().color(pal.accent));
                    if let Some(stats) = pane.grid.selection_stats(&pane.rs, &pane.cats, self.table.as_ref().map(|t| &t.edits)) {
                        ui.label(RichText::new(stats).small().color(pal.dim));
                    }
                }
                if pane.more {
                    ui.separator();
                    if pane.fetching {
                        ui.spinner();
                        ui.label(RichText::new("loading…").small());
                    } else {
                        ui.label(RichText::new("More rows on the server").small().color(pal.warn));
                        if w::small_button(ui, w::Kind::Secondary, icon::CHEVRONS_DOWN, &format!("Load {} more", util::thousands(ctx.settings.page_size.max(1000)))).clicked() {
                            fetch = Some(false);
                        }
                        if w::small_button(ui, w::Kind::Ghost, icon::DOWNLOAD, "Load all").clicked() {
                            fetch = Some(true);
                        }
                    }
                }
            });
        });
        let pane = &self.panes[idx];
        if pane.more
            && !pane.fetching
            && !pane.card
            && pane.grid.near_end
            && pane.grid.visible_rows() > 0
            && fetch.is_none()
            && pane.grid.filter.is_empty()
            && pane.grid.col_filters.is_empty()
        {
            fetch = Some(false);
        }
        if let Some(all) = fetch {
            self.fetch_more(ctx, all);
        }
    }

    fn fk_peek(&mut self, ui: &mut Ui, ctx: &TabCtx, idx: usize) {
        let pal = theme::p();
        let p = &self.panes[idx];
        let Some((src, sc, pos)) = p.grid.hover_fk else { return };
        let Some(Some((schema, table, col))) = p.fks.get(sc) else { return };
        let value = self.table.as_ref().and_then(|t| t.edits.cells.get(&(src, sc)).cloned()).unwrap_or_else(|| p.rs.get(src, sc).map(str::to_string));
        let Some(value) = value else { return };
        let key = format!("{schema}.{table}.{col}={value}");
        if !self.fk_cache.contains_key(&key)
            && !self.fk_pending.contains_key(&key)
            && let Some(h) = &ctx.meta
            && let Some(promise) = h.query(
                format!("SELECT * FROM {}.{} WHERE {} = $1 LIMIT 1", quote_ident(schema), quote_ident(table), quote_ident(col)),
                vec![Some(value.clone())],
            )
        {
            self.fk_pending.insert(key.clone(), promise);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(60));
        egui::Area::new(egui::Id::unique(("fk_peek", self.uid)))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos + vec2(0.0, 2.0))
            .constrain(true)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).fill(pal.bg2).show(ui, |ui| {
                    ui.set_max_width(460.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(icon::LINK).color(pal.accent));
                        ui.label(RichText::new(format!("{schema}.{table}")).strong());
                        ui.label(RichText::new(format!("{col} = {value}")).small().color(pal.faint));
                    });
                    ui.separator();
                    match self.fk_cache.get(&key) {
                        None if ctx.meta.is_none() => {
                            ui.label(RichText::new("Not connected").color(pal.faint));
                        }
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Loading…").color(pal.faint));
                            });
                        }
                        Some(Err(e)) => {
                            ui.label(RichText::new(e).color(pal.danger));
                        }
                        Some(Ok(rs)) if rs.nrows() == 0 => {
                            ui.label(RichText::new("No matching row (dangling reference)").color(pal.warn));
                        }
                        Some(Ok(rs)) => {
                            egui::Grid::new("fk_peek_grid").num_columns(2).spacing(vec2(12.0, 2.0)).show(ui, |ui| {
                                for c in 0..rs.ncols().min(16) {
                                    ui.label(RichText::new(&rs.columns[c].name).small().color(pal.dim));
                                    let v = rs.get(0, c);
                                    let text: String = v.map(|v| v.chars().take(60).collect()).unwrap_or_else(|| "NULL".into());
                                    ui.label(RichText::new(text).font(theme::mono(12.0)).color(if v.is_some() { pal.text } else { pal.null }));
                                    ui.end_row();
                                }
                            });
                            if rs.ncols() > 16 {
                                ui.label(RichText::new(format!("+ {} more columns", rs.ncols() - 16)).small().color(pal.faint));
                            }
                        }
                    }
                    ui.label(RichText::new("⌘-click to open the row").small().color(pal.faint));
                });
            });
    }

    fn compare_window(&mut self, ui: &mut Ui) {
        let pal = theme::p();
        let Some((pi, rows, diff_only)) = &mut self.compare else { return };
        let Some(p) = self.panes.get(*pi) else {
            self.compare = None;
            return;
        };
        let mut open = true;
        let ncols = p.rs.ncols();
        let differs: Vec<bool> = (0..ncols).map(|c| rows.iter().any(|&r| p.rs.get(r, c) != p.rs.get(rows[0], c))).collect();
        let ndiff = differs.iter().filter(|d| **d).count();
        egui::Window::new(format!("{}  Compare rows", icon::COLUMNS))
            .open(&mut open)
            .default_size(vec2(640.0, 480.0))
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ui.ctx().content_rect().center())
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{} difference{} in {} columns", ndiff, if ndiff == 1 { "" } else { "s" }, ncols)).strong().color(if ndiff > 0 {
                            pal.warn
                        } else {
                            pal.success
                        }),
                    );
                    ui.checkbox(diff_only, "Differences only");
                    if ui.small_button(format!("{}  Copy as Markdown", icon::COPY)).clicked() {
                        let mut md = String::from("| Column |");
                        for &r in rows.iter() {
                            md.push_str(&format!(" Row {} |", r + 1));
                        }
                        md.push_str("\n|---|");
                        md.push_str(&"---|".repeat(rows.len()));
                        md.push('\n');
                        for c in (0..ncols).filter(|&c| !*diff_only || differs[c]) {
                            md.push_str(&format!("| {} |", p.rs.columns[c].name));
                            for &r in rows.iter() {
                                md.push_str(&format!(" {} |", p.rs.get(r, c).unwrap_or("NULL").replace('|', "\\|")));
                            }
                            md.push('\n');
                        }
                        ui.ctx().copy_text(md);
                    }
                });
                ui.separator();
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Grid::new("compare_grid").striped(true).spacing(vec2(16.0, 4.0)).show(ui, |ui| {
                        ui.label(RichText::new("Column").small().strong().color(pal.dim));
                        for &r in rows.iter() {
                            ui.label(RichText::new(format!("Row {}", r + 1)).small().strong().color(pal.dim));
                        }
                        ui.end_row();
                        for (c, &differ) in differs.iter().enumerate() {
                            if *diff_only && !differ {
                                continue;
                            }
                            let name = RichText::new(&p.rs.columns[c].name).font(theme::bold(12.5));
                            ui.label(if differ { name.color(pal.warn) } else { name.color(pal.dim) });
                            for &r in rows.iter() {
                                let v = p.rs.get(r, c);
                                let text: String = v.map(|v| v.chars().take(80).collect()).unwrap_or_else(|| "NULL".into());
                                let rt = RichText::new(text).font(theme::mono(12.0)).color(if v.is_none() { pal.null } else { pal.text });
                                ui.label(if differ { rt.background_color(pal.warn.gamma_multiply(0.15)) } else { rt });
                            }
                            ui.end_row();
                        }
                    });
                });
            });
        if !open {
            self.compare = None;
        }
    }

    fn handle_grid_action(&mut self, action: GridAction, ctx: &mut TabCtx, idx: usize) {
        match action {
            GridAction::ViewValue(r, c) => {
                let p = &self.panes[idx];
                let value = if let Some(t) = &self.table {
                    if r >= p.rs.nrows() {
                        t.edits.inserted.get(r - p.rs.nrows()).and_then(|row| row.get(c).cloned().flatten())
                    } else {
                        t.edits.cells.get(&(r, c)).cloned().unwrap_or_else(|| p.rs.get(r, c).map(str::to_string))
                    }
                } else {
                    p.rs.get(r, c).map(str::to_string)
                };
                let col = p.rs.columns[c].name.clone();
                let looks_json = p.cats.get(c) == Some(&Cat::Json) || value.as_deref().is_some_and(|v| v.starts_with('{') || v.starts_with('['));
                let tree = if looks_json { value.as_deref().and_then(|v| JsonTree::parse(v, &col)) } else { None };
                let as_tree = tree.is_some();
                self.viewer = Some(Viewer { title: format!("{col} · row {}", r + 1), value, pretty: true, tree, as_tree });
            }
            GridAction::Filter(c, op) => {
                let column = self.panes[idx].rs.columns[c].name.clone();
                let f = ColFilter { column, op };
                if let Some(t) = &mut self.table {
                    // table view: filter on the server so every row is considered
                    let cond = f.sql();
                    t.filter = if t.filter.trim().is_empty() { cond } else { format!("{} AND {cond}", t.filter.trim()) };
                    self.pending_run = true;
                } else {
                    self.panes[idx].grid.add_filter(f);
                }
            }
            GridAction::CompareRows(rows) => self.compare = Some((idx, rows, false)),
            GridAction::DeleteRows(rows) => {
                if let Some(t) = &mut self.table {
                    let n = self.panes[idx].rs.nrows();
                    let mut inserted_remove: Vec<usize> = Vec::new();
                    for r in rows {
                        if r >= n {
                            inserted_remove.push(r - n);
                        } else if !t.edits.deleted.insert(r) {
                            t.edits.deleted.remove(&r);
                        }
                    }
                    inserted_remove.sort_unstable();
                    for i in inserted_remove.into_iter().rev() {
                        if i < t.edits.inserted.len() {
                            t.edits.inserted.remove(i);
                        }
                    }
                }
            }
            GridAction::FollowFk(r, c) => {
                let p = &self.panes[idx];
                let value = self.table.as_ref().and_then(|t| t.edits.cells.get(&(r, c)).cloned()).unwrap_or_else(|| p.rs.get(r, c).map(str::to_string));
                if let (Some(Some((schema, table, col))), Some(v), Some(pid)) = (p.fks.get(c), value, self.profile_id) {
                    ctx.actions.push(Action::OpenTableFiltered {
                        profile: pid,
                        db: self.database.clone(),
                        schema: schema.clone(),
                        name: table.clone(),
                        filter: format!("{} = {}", quote_ident(col), quote_literal(&v)),
                    });
                }
            }
            GridAction::DuplicateRow(r) => {
                // Leave out primary keys and generated / identity columns so
                // the database assigns fresh values.
                let skip: Vec<String> = ctx
                    .catalog
                    .as_deref()
                    .and_then(|cat| {
                        let t = self.table.as_ref()?;
                        let rel = cat.find_relation(Some(&t.schema), &t.name)?;
                        Some(rel.columns.iter().filter(|c| c.is_pk || c.identity || c.generated).map(|c| c.name.clone()).collect())
                    })
                    .unwrap_or_default();
                if let Some(t) = &mut self.table {
                    let p = &self.panes[idx];
                    let src = r.min(p.rs.nrows().saturating_sub(1));
                    let row: Vec<Option<String>> = (0..p.rs.ncols())
                        .map(|c| {
                            let col = &p.rs.columns[c].name;
                            if t.pk.contains(col) || skip.contains(col) { None } else { p.rs.get(src, c).map(str::to_string) }
                        })
                        .collect();
                    t.edits.inserted.push(row);
                }
            }
        }
    }
}

/// Header row above each grid in side-by-side mode (keeps both grids aligned).
fn pane_label(ui: &mut Ui, ico: &str, title: &str, rows: usize, color: egui::Color32) {
    let pal = theme::p();
    ui.horizontal(|ui| {
        ui.set_height(24.0);
        ui.add_space(8.0);
        ui.label(RichText::new(format!("{ico}  {title}")).strong().color(color));
        ui.label(RichText::new(format!("{} rows", util::thousands(rows))).small().color(pal.faint));
    });
}

/// Column visibility row for the Columns menu (stays open while toggling).
fn column_toggle(ui: &mut Ui, visible: bool, name: &str) -> egui::Response {
    let pal = theme::p();
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(220.0), 28.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, pal.control_hover);
    }
    let box_rect = egui::Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.center().y), vec2(16.0, 16.0));
    ui.painter().rect(
        box_rect,
        4.0,
        if visible { pal.primary } else { pal.input },
        egui::Stroke::new(1.0, if visible { pal.primary } else { pal.border_strong }),
        egui::StrokeKind::Inside,
    );
    if visible {
        ui.painter().text(box_rect.center(), egui::Align2::CENTER_CENTER, icon::CHECK, egui::FontId::proportional(11.0), egui::Color32::WHITE);
    }
    ui.painter().text(
        egui::pos2(rect.left() + 36.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        theme::mono(12.5),
        if visible { pal.text } else { pal.dim },
    );
    resp
}

fn ms(v: f32) -> String {
    if v < 1.0 {
        format!("{v:.2} ms")
    } else if v < 1000.0 {
        format!("{v:.0} ms")
    } else {
        format!("{:.2} s", v / 1000.0)
    }
}

/// One record shown vertically (field / value), with previous / next.
fn card_view(ui: &mut Ui, pane: &mut Pane, ts_mode: TsMode, keys: bool) -> Option<GridAction> {
    let pal = theme::p();
    pane.grid.refresh_view(&pane.rs, &pane.cats, &pane.type_names);
    let n = pane.grid.visible_rows();
    let mut action = None;
    if n == 0 {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("No rows").color(pal.faint)));
        return None;
    }
    let cur = pane.grid.cursor_view_row().unwrap_or(0).min(n - 1);
    let mut go: Option<usize> = None;
    if keys && !ui.memory(|m| m.focused().is_some()) {
        ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::J) {
                go = Some((cur + 1).min(n - 1));
            }
            if i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::K) {
                go = Some(cur.saturating_sub(1));
            }
        });
    }
    let src = pane.grid.view_rows()[cur];
    egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.add_enabled(cur > 0, egui::Button::new(icon::CHEVRON_LEFT)).on_hover_text("Previous (←)").clicked() {
                go = Some(cur - 1);
            }
            ui.label(RichText::new(format!("Row {} of {}", util::thousands(cur + 1), util::thousands(n))).strong());
            if ui.add_enabled(cur + 1 < n, egui::Button::new(icon::CHEVRON_RIGHT)).on_hover_text("Next (→)").clicked() {
                go = Some(cur + 1);
            }
            ui.label(RichText::new("· source row").small().color(pal.faint));
            ui.label(RichText::new((src + 1).to_string()).small().color(pal.faint));
        });
    });
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
            let value_w = (ui.available_width() - 220.0).max(200.0);
            egui::Grid::new(("card", src)).num_columns(2).striped(true).spacing(vec2(18.0, 8.0)).min_col_width(180.0).show(ui, |ui| {
                for &c in pane.grid.display_cols().to_vec().iter() {
                    let col = &pane.rs.columns[c];
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&col.name).font(theme::bold(12.5)));
                        ui.label(RichText::new(pane.type_names.get(c).cloned().unwrap_or_default()).small().color(pal.faint));
                    });
                    let v = pane.rs.get(src, c);
                    let cat = pane.cats.get(c).copied().unwrap_or(Cat::Other);
                    let text = match v {
                        None => "NULL".to_string(),
                        Some(v) if cat == Cat::Json => crate::ui::grid::json_preview(v, 30),
                        Some(v) if cat == Cat::DateTime && pane.type_names.get(c).is_some_and(|t| t == "timestamptz") => {
                            crate::ui::grid::format_ts(v, ts_mode).unwrap_or_else(|| v.to_string())
                        }
                        Some(v) => v.chars().take(5000).collect(),
                    };
                    let color = if v.is_none() {
                        pal.null
                    } else if cat == Cat::Json {
                        pal.json
                    } else if cat == Cat::Number {
                        pal.number
                    } else {
                        pal.text
                    };
                    let r = ui
                        .scope(|ui| {
                            ui.set_min_width(value_w);
                            ui.set_max_width(value_w);
                            ui.add(
                                egui::Label::new(RichText::new(text).font(theme::mono(12.5)).color(color)).selectable(true).wrap().sense(egui::Sense::click()),
                            )
                        })
                        .inner;
                    if r.double_clicked() {
                        action = Some(GridAction::ViewValue(src, c));
                    }
                    ui.end_row();
                }
            });
        });
    });
    if let Some(row) = go {
        pane.grid.focus_row(row);
    }
    action
}
