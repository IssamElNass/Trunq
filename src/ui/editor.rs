//! The SQL editor: syntax highlighting, line numbers, current-statement
//! highlight, error squiggles and a cursor-anchored completion popup.

use std::ops::Range;
use std::sync::Arc;

use egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use egui::{Color32, FontId, Galley, Id, Key, Modifiers, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2};

use super::theme::{self, icon};
use crate::db::meta::Catalog;
use crate::sql::complete::{self, Completion, Kind};
use crate::sql::lexer::{self, Tok};
use crate::ui::widgets as w;

pub struct EditorState {
    pub text: String,
    pub id: Id,
    /// Error location to underline (byte range) and its message.
    pub error: Option<(Range<usize>, String)>,
    completion: Option<Completion>,
    selected: usize,
    last_cursor: Option<usize>,
    /// Pending cursor placement (byte offset) for the next frame.
    pending_cursor: Option<Range<usize>>,
    pub request_focus: bool,
    pub font_size: f32,
}

#[derive(Default)]
pub struct EditorOutput {
    pub changed: bool,
    /// Cursor position as byte offset.
    pub cursor: Option<usize>,
    /// Selected byte range (non-empty).
    pub selection: Option<Range<usize>>,
    pub has_focus: bool,
}

impl EditorState {
    pub fn new(id: Id, text: String) -> EditorState {
        EditorState { text, id, error: None, completion: None, selected: 0, last_cursor: None, pending_cursor: None, request_focus: true, font_size: 13.5 }
    }

    /// Replaces a byte range and places the cursor after the insertion.
    pub fn replace_range(&mut self, range: Range<usize>, with: &str) {
        let start = range.start.min(self.text.len());
        let end = range.end.min(self.text.len()).max(start);
        self.text.replace_range(start..end, with);
        let c = start + with.len();
        self.pending_cursor = Some(c..c);
        self.error = None;
        self.completion = None;
    }

    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.pending_cursor = Some(0..0);
        self.error = None;
        self.completion = None;
    }

    fn accept_completion(&mut self, index: usize) -> bool {
        let Some(completion) = self.completion.take() else { return false };
        let Some(item) = completion.items.get(index) else { return false };
        if self.text.get(completion.replace.clone()).is_none() {
            return false;
        }
        let mut insert = item.insert.clone();
        // Mouse and keyboard acceptance must both preserve an existing "(".
        if insert.ends_with('(') && self.text[completion.replace.end..].starts_with('(') {
            insert.pop();
        }
        self.replace_range(completion.replace, &insert);
        self.request_focus = true;
        true
    }
}

fn char_to_byte(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map_or(s.len(), |(b, _)| b)
}

fn byte_to_char(s: &str, b: usize) -> usize {
    s[..s.floor_char_boundary(b.min(s.len()))].chars().count()
}

#[derive(Default)]
struct SqlHighlighter;

impl egui::cache::ComputerMut<(&str, &FontId, bool), LayoutJob> for SqlHighlighter {
    fn compute(&mut self, (text, font, _dark): (&str, &FontId, bool)) -> LayoutJob {
        highlight(text, font)
    }
}

fn highlighted_galley(ui: &Ui, text: &str, font: &FontId, wrap: f32) -> Arc<Galley> {
    // Cache syntax, never glyph meshes: egui invalidates its font atlas when
    // fonts/rendering options change or the atlas fills up. Old galleys then
    // point at unrelated glyphs. FrameCache also evicts earlier edits.
    let mut job = ui.ctx().memory_mut(|m| m.caches.cache::<egui::cache::FrameCache<LayoutJob, SqlHighlighter>>().get((text, font, theme::p().dark)).clone());
    job.wrap.max_width = wrap;
    ui.ctx().fonts_mut(|f| f.layout_job(job))
}

/// Builds a highlighted layout job for `text`.
pub fn highlight(text: &str, font: &FontId) -> LayoutJob {
    let pal = theme::p();
    let mut job = LayoutJob { text: String::new(), ..Default::default() };
    job.text.reserve(text.len());
    for t in lexer::tokenize(text) {
        let s = &text[t.range.clone()];
        let (color, italics) = match t.kind {
            Tok::Keyword => (pal.kw, false),
            Tok::String => (pal.string, false),
            Tok::Number => (pal.number, false),
            Tok::Comment => (pal.comment, true),
            Tok::QuotedIdent => (pal.qident, false),
            Tok::Operator | Tok::Semicolon => (pal.op, false),
            Tok::Param => (pal.param, false),
            Tok::Meta => (pal.warn, false),
            Tok::Punct => (pal.dim, false),
            Tok::Ident | Tok::Whitespace => (pal.ident, false),
        };
        job.append(s, 0.0, TextFormat { font_id: font.clone(), color, italics, ..Default::default() });
    }
    job
}

/// Read-only highlighted SQL (for DDL views, history previews...).
pub fn code_view(ui: &mut Ui, text: &str, size: f32) {
    let font = theme::mono(size);
    let job = highlight(text, &font);
    let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(8.0, 8.0), Sense::click());
    ui.painter().galley(rect.min + vec2(4.0, 4.0), galley, theme::p().text);
    resp.context_menu(|ui| {
        if w::menu_item(ui, icon::COPY, "Copy").clicked() {
            ui.ctx().copy_text(text.to_string());
            ui.close();
        }
    });
}

/// Shows the editor. Returns information about cursor/selection.
#[allow(clippy::too_many_arguments)]
pub fn sql_editor(
    ui: &mut Ui,
    st: &mut EditorState,
    catalog: Option<&Catalog>,
    autocomplete: bool,
    upper_kw: bool,
    readonly: bool,
    highlight_stmt: bool,
) -> EditorOutput {
    let pal = theme::p();
    let font = theme::mono(st.font_size);
    let row_h = ui.ctx().fonts_mut(|f| f.row_height(&font));
    let text_id = st.id.with("text");

    // ---- keyboard handling before the TextEdit sees the events ---------------
    let focused = ui.memory(|m| m.has_focus(text_id));
    let mut accept: Option<usize> = None;
    let mut force_complete = false;
    if readonly {
        st.completion = None;
    }
    if focused && !readonly {
        if st.completion.is_some() {
            ui.input_mut(|i| {
                let n = st.completion.as_ref().map_or(0, |c| c.items.len());
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) && n > 0 {
                    st.selected = (st.selected + 1) % n;
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) && n > 0 {
                    st.selected = (st.selected + n - 1) % n;
                }
                if i.consume_key(Modifiers::NONE, Key::PageDown) && n > 0 {
                    st.selected = (st.selected + 8).min(n - 1);
                }
                if i.consume_key(Modifiers::NONE, Key::PageUp) {
                    st.selected = st.selected.saturating_sub(8);
                }
                if i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab) {
                    accept = Some(st.selected);
                }
                if i.consume_key(Modifiers::NONE, Key::Escape) {
                    st.completion = None;
                }
            });
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Space) || i.consume_key(Modifiers::COMMAND, Key::Space)) {
            force_complete = true;
        }
    }

    // ---- accept a completion -------------------------------------------------
    let completion_changed = accept.take().is_some_and(|index| st.accept_completion(index));

    // Indent / outdent with Tab when no completion is open.
    let mut tab_action: Option<bool> = None;
    if focused && st.completion.is_none() && !readonly {
        ui.input_mut(|i| {
            if i.consume_key(Modifiers::SHIFT, Key::Tab) {
                tab_action = Some(false);
            } else if i.consume_key(Modifiers::NONE, Key::Tab) {
                tab_action = Some(true);
            }
        });
    }
    let toggle_comment = focused && !readonly && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Slash));
    let duplicate_line = focused && !readonly && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::D));

    // Apply a pending cursor (from replace/select) before rendering.
    if let Some(r) = st.pending_cursor.take() {
        let mut state = egui::TextEdit::load_state(ui.ctx(), text_id).unwrap_or_default();
        let a = byte_to_char(&st.text, r.start);
        let b = byte_to_char(&st.text, r.end);
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
        state.store(ui.ctx(), text_id);
    }

    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| -> Arc<Galley> { highlighted_galley(ui, buf.as_str(), &font, wrap) };

    let mut out = EditorOutput::default();
    let gutter_digits = (st.text.matches('\n').count() + 1).to_string().len().max(3);
    let gutter_w = gutter_digits as f32 * st.font_size * 0.62 + 18.0;

    let scroll = egui::ScrollArea::both().id_salt(st.id.with("scroll")).auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (gutter_rect, _) = ui.allocate_exact_size(vec2(gutter_w, ui.available_height().max(row_h)), Sense::hover());
            let bg_idx = ui.painter().add(Shape::Noop);
            let before = readonly.then(|| st.text.clone());
            let te = egui::TextEdit::multiline(&mut st.text)
                .id(text_id)
                .font(font.clone())
                .code_editor()
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .desired_rows(8)
                .margin(egui::Margin { left: 6, right: 8, top: 4, bottom: 40 })
                .lock_focus(true)
                .layouter(&mut layouter);
            let output = te.show(ui);
            if let Some(before) = before
                && output.response.changed()
            {
                st.text = before; // read-only: discard edits
            }
            let galley = output.galley.clone();
            let gpos = output.galley_pos;
            out.changed = completion_changed || output.response.changed() && !readonly;
            out.has_focus = output.response.has_focus();
            if st.request_focus {
                output.response.request_focus();
                st.request_focus = false;
            }

            // cursor / selection
            let mut cursor_char = None;
            if let Some(cr) = output.cursor_range {
                let [a, b] = cr.sorted_cursors();
                cursor_char = Some(cr.primary.index.0);
                let ab = char_to_byte(&st.text, a.index.0);
                let bb = char_to_byte(&st.text, b.index.0);
                out.cursor = Some(char_to_byte(&st.text, cr.primary.index.0));
                if ab != bb {
                    out.selection = Some(ab..bb);
                }
            }

            // ---- background: current statement + current line --------------
            let mut bg_shapes = Vec::new();
            if let Some(cur) = out.cursor
                && out.has_focus
            {
                if highlight_stmt
                    && out.selection.is_none()
                    && let Some(stmt) = lexer::statement_at(&st.text, cur)
                {
                    let a = galley.pos_from_cursor(CCursor::new(byte_to_char(&st.text, stmt.range.start)));
                    let b = galley.pos_from_cursor(CCursor::new(byte_to_char(&st.text, stmt.range.end)));
                    let r = Rect::from_min_max(pos2(gutter_rect.right(), gpos.y + a.min.y - 1.0), pos2(ui.clip_rect().right(), gpos.y + b.max.y + 1.0));
                    bg_shapes.push(Shape::rect_filled(r, 0.0, pal.accent.gamma_multiply(if pal.dark { 0.045 } else { 0.06 })));
                    bg_shapes.push(Shape::line_segment(
                        [pos2(r.left() + 1.0, r.top()), pos2(r.left() + 1.0, r.bottom())],
                        Stroke::new(2.0, pal.accent.gamma_multiply(0.7)),
                    ));
                }
                if let Some(cc) = cursor_char {
                    let cr = galley.pos_from_cursor(CCursor::new(cc));
                    let r = Rect::from_min_max(pos2(gutter_rect.right(), gpos.y + cr.min.y), pos2(ui.clip_rect().right(), gpos.y + cr.max.y));
                    bg_shapes.push(Shape::rect_filled(r, 0.0, if pal.dark { Color32::from_white_alpha(6) } else { Color32::from_black_alpha(8) }));
                }
            }
            // error squiggle
            if let Some((range, _)) = &st.error {
                let s = byte_to_char(&st.text, range.start);
                let e = byte_to_char(&st.text, range.end.max(range.start + 1));
                let a = galley.pos_from_cursor(CCursor::new(s));
                let b = galley.pos_from_cursor(CCursor::new(e));
                if (a.min.y - b.min.y).abs() < 1.0 {
                    let y = gpos.y + a.max.y;
                    let x0 = gpos.x + a.min.x;
                    let x1 = (gpos.x + b.min.x).max(x0 + 8.0);
                    let mut pts = Vec::new();
                    let mut x = x0;
                    let mut up = true;
                    while x <= x1 {
                        pts.push(pos2(x, if up { y - 1.0 } else { y + 1.5 }));
                        x += 3.0;
                        up = !up;
                    }
                    bg_shapes.push(Shape::line(pts, Stroke::new(1.3, pal.danger)));
                }
                // gutter marker
                bg_shapes.push(Shape::circle_filled(pos2(gutter_rect.left() + 7.0, gpos.y + a.center().y), 3.5, pal.danger));
            }
            ui.painter().set(bg_idx, Shape::Vec(bg_shapes));

            // ---- line numbers -----------------------------------------------
            let painter = ui.painter_at(gutter_rect.expand2(vec2(0.0, 2000.0)));
            let visible = ui.clip_rect();
            let cur_line = out.cursor.map(|c| st.text[..c.min(st.text.len())].matches('\n').count());
            let mut line = 0usize;
            let mut new_line = true;
            let num_font = theme::mono(st.font_size * 0.86);
            for row in galley.rows.iter() {
                let y = gpos.y + row.pos.y;
                if new_line && y + row_h >= visible.top() && y <= visible.bottom() {
                    let color = if Some(line) == cur_line { pal.dim } else { pal.faint };
                    painter.text(
                        pos2(gutter_rect.right() - 8.0, y + row.row.size.y * 0.5),
                        egui::Align2::RIGHT_CENTER,
                        (line + 1).to_string(),
                        num_font.clone(),
                        color,
                    );
                }
                if row.ends_with_newline {
                    line += 1;
                    new_line = true;
                } else {
                    new_line = false;
                }
            }
            painter.vline(gutter_rect.right() - 1.0, visible.y_range(), Stroke::new(1.0, pal.border.gamma_multiply(0.6)));

            // ---- completion -------------------------------------------------
            let cursor_byte = out.cursor;
            let typed = out.changed
                && ui.input(|i| {
                    i.events.iter().any(|e| matches!(e, egui::Event::Text(t) if t.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.' || c == '"')))
                });
            if !readonly && ((typed && autocomplete) || force_complete) {
                if let Some(c) = cursor_byte {
                    st.completion = complete::complete(&st.text, c, catalog, force_complete, upper_kw);
                    st.selected = 0;
                }
            } else if st.completion.is_some() && (cursor_byte != st.last_cursor || !out.has_focus) {
                // cursor moved by other means (click, arrows, deletion)
                if let (true, Some(c)) = (out.changed, cursor_byte) {
                    st.completion = complete::complete(&st.text, c, catalog, false, upper_kw);
                    st.selected = 0;
                } else {
                    st.completion = None;
                }
            }
            st.last_cursor = cursor_byte;

            if let (Some(comp), Some(cc)) = (&st.completion, cursor_char) {
                let cr = galley.pos_from_cursor(CCursor::new(cc));
                let anchor = gpos + vec2(cr.min.x, cr.max.y + 2.0);
                if let Some(i) = completion_popup(ui, st.id, comp, st.selected, anchor) {
                    accept = Some(i);
                }
            }
            if let Some(i) = accept.take() {
                out.changed |= st.accept_completion(i);
            }
        });
    });
    let _ = scroll;

    // ---- edit commands (applied after layout; take effect next frame) ----------
    if let Some(indent) = tab_action {
        let sel = out.selection.clone().unwrap_or_else(|| out.cursor.unwrap_or(0)..out.cursor.unwrap_or(0));
        let multi_line = st.text[sel.clone()].contains('\n');
        if indent && !multi_line {
            st.replace_range(sel, "    ");
        } else {
            let (new_text, new_sel) = indent_lines(&st.text, sel, indent);
            st.text = new_text;
            st.pending_cursor = Some(new_sel);
        }
        out.changed = true;
    }
    if toggle_comment {
        let sel = out.selection.clone().unwrap_or_else(|| out.cursor.unwrap_or(0)..out.cursor.unwrap_or(0));
        let (new_text, new_sel) = toggle_line_comment(&st.text, sel);
        st.text = new_text;
        st.pending_cursor = Some(new_sel);
        out.changed = true;
    }
    if duplicate_line {
        let cur = out.cursor.unwrap_or(0).min(st.text.len());
        let ls = st.text[..cur].rfind('\n').map_or(0, |p| p + 1);
        let le = st.text[cur..].find('\n').map_or(st.text.len(), |p| cur + p);
        let line = st.text[ls..le].to_string();
        st.text.insert_str(le, &format!("\n{line}"));
        let c = cur + line.len() + 1;
        st.pending_cursor = Some(c..c);
        out.changed = true;
    }
    if out.changed {
        st.error = None;
    }
    if tab_action.is_some() || toggle_comment || duplicate_line {
        st.completion = None;
    }
    out
}

fn line_bounds(text: &str, sel: &Range<usize>) -> Range<usize> {
    let s = text[..sel.start.min(text.len())].rfind('\n').map_or(0, |p| p + 1);
    let mut end = sel.end.min(text.len());
    // A selection ending at column 0 doesn't include that line.
    if end > sel.start && text[..end].ends_with('\n') {
        end -= 1;
    }
    let e = text[end..].find('\n').map_or(text.len(), |p| end + p);
    s..e
}

fn indent_lines(text: &str, sel: Range<usize>, indent: bool) -> (String, Range<usize>) {
    let lb = line_bounds(text, &sel);
    let block = &text[lb.clone()];
    let new_block: String = block
        .split('\n')
        .map(|l| {
            if indent {
                format!("    {l}")
            } else {
                let strip = l.chars().take(4).take_while(|c| *c == ' ').count();
                let strip = if strip == 0 && l.starts_with('\t') { 1 } else { strip };
                l[strip..].to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = String::with_capacity(text.len() + 64);
    out.push_str(&text[..lb.start]);
    out.push_str(&new_block);
    out.push_str(&text[lb.end..]);
    let new_sel = lb.start..lb.start + new_block.len();
    (out, new_sel)
}

fn toggle_line_comment(text: &str, sel: Range<usize>) -> (String, Range<usize>) {
    let lb = line_bounds(text, &sel);
    let block = &text[lb.clone()];
    let lines: Vec<&str> = block.split('\n').collect();
    let all_commented = lines.iter().filter(|l| !l.trim().is_empty()).all(|l| l.trim_start().starts_with("--"));
    let min_indent = lines.iter().filter(|l| !l.trim().is_empty()).map(|l| l.chars().take_while(|c| c.is_whitespace()).count()).min().unwrap_or(0);
    let new_block: String = lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                l.to_string()
            } else if all_commented {
                let i = l.find("--").unwrap_or(0);
                let rest = &l[i + 2..];
                let rest = rest.strip_prefix(' ').unwrap_or(rest);
                format!("{}{}", &l[..i], rest)
            } else {
                let indent_end = char_to_byte(l, min_indent);
                format!("{}-- {}", &l[..indent_end], &l[indent_end..])
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = String::with_capacity(text.len() + 64);
    out.push_str(&text[..lb.start]);
    out.push_str(&new_block);
    out.push_str(&text[lb.end..]);
    let new_sel = if sel.is_empty() {
        let delta = new_block.len() as isize - block.len() as isize;
        let c = (sel.start as isize + delta).max(lb.start as isize) as usize;
        c..c
    } else {
        lb.start..lb.start + new_block.len()
    };
    (out, new_sel)
}

fn completion_popup(ui: &Ui, id: Id, comp: &Completion, selected: usize, anchor: Pos2) -> Option<usize> {
    let pal = theme::p();
    let mut clicked = None;
    let area = egui::Area::new(id.with("completion")).order(egui::Order::Tooltip).fixed_pos(anchor).constrain(true).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(4)).fill(pal.bg1).show(ui, |ui| {
            ui.set_width(420.0);
            egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, item) in comp.items.iter().enumerate() {
                    let is_sel = i == selected;
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                    if is_sel {
                        ui.painter().rect_filled(rect, 5.0, pal.sel);
                        resp.scroll_to_me(None);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 5.0, pal.bg3);
                    }
                    let (ico, color) = match item.kind {
                        Kind::Column => (icon::COLUMNS, pal.accent),
                        Kind::Table => (icon::TABLE, pal.success),
                        Kind::View => (icon::EYE, pal.qident),
                        Kind::Schema => (icon::FOLDER, pal.warn),
                        Kind::Function => (icon::FUNCTION, pal.number),
                        Kind::Keyword => (icon::TEXT, pal.kw),
                    };
                    let p = ui.painter();
                    p.text(rect.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, ico, FontId::proportional(13.0), color);
                    p.text(rect.left_center() + vec2(30.0, 0.0), egui::Align2::LEFT_CENTER, &item.label, theme::mono(13.0), pal.text);
                    let detail: String = item.detail.chars().take(48).collect();
                    p.text(rect.right_center() - vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, detail, FontId::proportional(11.5), pal.faint);
                    if resp.clicked() {
                        clicked = Some(i);
                    }
                }
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("↑↓ navigate · ⏎/Tab accept · Esc close · Ctrl+Space show").small().color(pal.faint));
            });
        });
    });
    let _ = area;
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_offsets_inside_unicode_characters_do_not_panic() {
        let text = "é世界";
        assert_eq!(byte_to_char(text, 1), 0);
        assert_eq!(byte_to_char(text, 3), 1);
        assert_eq!(byte_to_char(text, 7), 2);
        assert_eq!(byte_to_char(text, usize::MAX), 3);
    }

    #[test]
    fn highlighted_text_refreshes_after_font_changes() {
        let ctx = egui::Context::default();
        let text = "SELECT 'héllo 世界' AS greeting, 42;";
        let font = theme::mono(13.5);
        let mut before = None;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            before = Some(highlighted_galley(ui, text, &font, 800.0));
        });
        output.textures_delta.clear();
        theme::install_fonts(&ctx);
        let mut actual = None;
        let mut expected = None;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            actual = Some(highlighted_galley(ui, text, &font, 800.0));
            let mut job = highlight(text, &font);
            job.wrap.max_width = 800.0;
            expected = Some(ui.fonts_mut(|f| f.layout_job(job)));
        });
        output.textures_delta.clear();
        assert!(!Arc::ptr_eq(before.as_ref().unwrap(), expected.as_ref().unwrap()));
        assert!(Arc::ptr_eq(actual.as_ref().unwrap(), expected.as_ref().unwrap()), "editor reused glyphs from the old font atlas");
    }

    #[test]
    fn comment_toggle() {
        let (t, _) = toggle_line_comment("select 1\nfrom t", 0..12);
        assert_eq!(t, "-- select 1\n-- from t");
        let (t, _) = toggle_line_comment(&t, 0..5);
        assert_eq!(t, "select 1\n-- from t");
    }

    #[test]
    fn comment_toggle_preserves_unicode_indentation() {
        let sql = "\u{3000}SELECT 'café';\n  SELECT '世界';";
        let (commented, _) = toggle_line_comment(sql, 0..sql.len());
        assert_eq!(commented, "\u{3000}-- SELECT 'café';\n --  SELECT '世界';");
        let (restored, _) = toggle_line_comment(&commented, 0..commented.len());
        assert_eq!(restored, sql);
    }

    #[test]
    fn function_completion_preserves_existing_parentheses() {
        for (text, expected) in [("SELECT cou", "SELECT count("), ("SELECT cou()", "SELECT count()")] {
            let mut state = EditorState::new(Id::unique("completion-test"), text.into());
            let completion = complete::complete(text, "SELECT cou".len(), None, true, false).unwrap();
            let index = completion.items.iter().position(|item| item.insert == "count(").unwrap();
            state.completion = Some(completion);
            assert!(state.accept_completion(index));
            assert_eq!(state.text, expected);
            assert!(state.completion.is_none());
            assert!(state.request_focus);
        }
    }

    #[test]
    fn completion_is_discarded_after_external_replacement() {
        let mut state = EditorState::new(Id::unique("replacement-completion"), "SELECT cou".into());
        state.completion = complete::complete(&state.text, state.text.len(), None, true, false);
        assert!(state.completion.is_some());
        state.replace_range(0..state.text.len(), "SELECT 1");
        assert!(state.completion.is_none());
    }

    #[test]
    fn stale_completion_range_does_not_panic() {
        let mut state = EditorState::new(Id::unique("stale-completion"), "SELECT cou".into());
        state.completion = complete::complete(&state.text, state.text.len(), None, true, false);
        state.text = "短".into();
        assert!(!state.accept_completion(0));
        assert_eq!(state.text, "短");
    }

    #[test]
    fn comment_shortcut_closes_the_old_completion() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut state = EditorState::new(Id::unique("comment-completion"), "SELECT cou".into());
        let mut output = ctx.run_ui(Default::default(), |ui| {
            sql_editor(ui, &mut state, None, true, true, false, false);
        });
        output.textures_delta.clear();
        state.completion = complete::complete(&state.text, state.text.len(), None, true, false);
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::ModifiersChanged(Modifiers::COMMAND));
        input.events.push(egui::Event::Key { key: Key::Slash, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
        let mut output = ctx.run_ui(input, |ui| {
            ui.memory_mut(|memory| memory.request_focus(state.id.with("text")));
            sql_editor(ui, &mut state, None, true, true, false, false);
        });
        output.textures_delta.clear();
        assert_eq!(state.text, "-- SELECT cou");
        assert!(state.completion.is_none());
    }

    #[test]
    fn indent() {
        let (t, _) = indent_lines("a\nb", 0..3, true);
        assert_eq!(t, "    a\n    b");
        let (t, _) = indent_lines(&t, 0..5, false);
        assert_eq!(t, "a\n    b");
    }
}
