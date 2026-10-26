//! Trunq's design system: buttons, inputs, segmented controls, menu items,
//! pills and cards with one consistent size scale, icon slot and colours.
//!
//! Sizes: controls are 32 px high (28 px small), corner radius 8, horizontal
//! padding 12 (9 small), icon slot 16 px with a 7 px gap.

use egui::{Align2, Color32, CornerRadius, CursorIcon, FontId, Margin, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

use super::theme::{self, icon};

pub const H: f32 = 32.0;
pub const H_SMALL: f32 = 28.0;
pub const RADIUS: u8 = 8;

/// Shared inspector detail surface, using the same spacing and borders as controls.
pub fn card<R>(ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    let p = theme::p();
    egui::Frame::NONE.fill(p.bg1).stroke(Stroke::new(1.0, p.border)).corner_radius(RADIUS).inner_margin(Margin::same(16)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        body(ui)
    })
}

pub fn empty_state(ui: &mut Ui, ico: &str, title: &str, detail: &str) {
    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(ico).size(28.0).color(theme::p().faint));
        ui.label(RichText::new(title).font(theme::bold(15.0)));
        ui.label(RichText::new(detail).color(theme::p().dim));
    });
    ui.add_space(20.0);
}

/// Two-line inspector choice with the same selection, type and focus scale.
pub fn list_item(ui: &mut Ui, selected: bool, ico: &str, title: &str, detail: &str) -> Response {
    let p = theme::p();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
    let fill = if selected {
        p.accent_soft
    } else if response.hovered() {
        p.control_hover
    } else {
        p.control
    };
    ui.painter().rect(rect, CornerRadius::same(RADIUS), fill, Stroke::new(1.0, if selected { p.accent } else { p.border }), StrokeKind::Inside);
    ui.painter().text(
        pos2(rect.left() + 20.0, rect.center().y),
        Align2::CENTER_CENTER,
        ico,
        FontId::proportional(16.0),
        if selected { p.accent } else { p.dim },
    );
    let title_galley = truncated(ui, title, theme::bold(13.5), p.text, rect.width() - 52.0);
    let detail_galley = truncated(ui, detail, FontId::proportional(12.0), p.dim, rect.width() - 52.0);
    ui.painter().galley(pos2(rect.left() + 40.0, rect.top() + 7.0), title_galley, p.text);
    ui.painter().galley(pos2(rect.left() + 40.0, rect.top() + 27.0), detail_galley, p.dim);
    response.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(format!("{title}\n{detail}"))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The one main action of an area (blue).
    Primary,
    /// Regular action.
    Secondary,
    /// Low-emphasis, transparent until hovered.
    Ghost,
    /// Destructive action.
    Danger,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Regular,
    Small,
}

struct Colors {
    fill: Color32,
    stroke: Color32,
    fg: Color32,
}

fn colors(kind: Kind, hovered: bool, pressed: bool, selected: bool) -> Colors {
    let p = theme::p();
    match kind {
        Kind::Primary => Colors {
            fill: if pressed {
                p.primary_pressed
            } else if hovered {
                p.primary_hover
            } else {
                p.primary
            },
            stroke: Color32::TRANSPARENT,
            fg: Color32::WHITE,
        },
        Kind::Secondary => Colors {
            fill: if selected {
                p.accent_soft
            } else if pressed {
                p.bg3
            } else if hovered {
                p.control_hover
            } else {
                p.control
            },
            stroke: if selected {
                p.accent.gamma_multiply(0.55)
            } else if hovered {
                p.border_strong
            } else {
                p.border
            },
            fg: if selected { p.accent } else { p.text },
        },
        Kind::Ghost => Colors {
            fill: if selected {
                p.accent_soft
            } else if pressed {
                p.bg3
            } else if hovered {
                p.control_hover
            } else {
                Color32::TRANSPARENT
            },
            stroke: Color32::TRANSPARENT,
            fg: if selected {
                p.accent
            } else if hovered {
                p.text
            } else {
                p.dim
            },
        },
        Kind::Danger => Colors {
            fill: if pressed {
                p.danger.gamma_multiply(0.32)
            } else if hovered {
                p.danger.gamma_multiply(0.22)
            } else {
                p.danger.gamma_multiply(0.12)
            },
            stroke: p.danger.gamma_multiply(0.35),
            fg: p.danger,
        },
    }
}

/// The core button painter used by all variants.
fn paint_button(ui: &mut Ui, kind: Kind, size: Size, ico: &str, label: &str, selected: bool, max_width: f32) -> Response {
    let (h, pad, font_size) = match size {
        Size::Regular => (H, 12.0, 13.5),
        Size::Small => (H_SMALL, 9.0, 12.5),
    };
    let font = FontId::proportional(font_size);
    let label_g = (!label.is_empty()).then(|| ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), Color32::PLACEHOLDER)));
    let icon_g = (!ico.is_empty()).then(|| ui.fonts_mut(|f| f.layout_no_wrap(ico.to_string(), FontId::proportional(font_size + 1.0), Color32::PLACEHOLDER)));
    let gap = if label_g.is_some() && icon_g.is_some() { 7.0 } else { 0.0 };
    let content_w = icon_g.as_ref().map_or(0.0, |g| g.size().x) + gap + label_g.as_ref().map_or(0.0, |g| g.size().x);
    // Bound identifiers to the container width, not the remaining line width,
    // so wrapped toolbars can still move whole controls onto the next line.
    let w = if label.is_empty() { h } else { (content_w + pad * 2.0).min(ui.max_rect().width().max(h)) };
    let w = if max_width > 0.0 { w.min(max_width.max(h)) } else { w };
    let clipped = content_w + pad * 2.0 > w;
    let label_g = label_g
        .map(|g| if clipped { truncated(ui, label, font, Color32::PLACEHOLDER, w - pad * 2.0 - icon_g.as_ref().map_or(0.0, |g| g.size().x) - gap) } else { g });
    let content_w = icon_g.as_ref().map_or(0.0, |g| g.size().x) + gap + label_g.as_ref().map_or(0.0, |g| g.size().x);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    if ui.is_rect_visible(rect) {
        let enabled = ui.is_enabled();
        let hovered = enabled && resp.hovered();
        let pressed = enabled && resp.is_pointer_button_down_on();
        let mut c = colors(kind, hovered, pressed, selected);
        if !enabled {
            c.fill = c.fill.gamma_multiply(0.5);
            c.fg = c.fg.gamma_multiply(0.45);
            c.stroke = c.stroke.gamma_multiply(0.5);
        }
        let painter = ui.painter();
        painter.rect(rect, CornerRadius::same(RADIUS), c.fill, Stroke::new(1.0, c.stroke), StrokeKind::Inside);
        if resp.has_focus() {
            painter.rect_stroke(rect.expand(2.0), CornerRadius::same(RADIUS + 2), Stroke::new(1.5, theme::p().accent.gamma_multiply(0.8)), StrokeKind::Outside);
        }
        let mut x = rect.center().x - content_w / 2.0;
        if let Some(g) = icon_g {
            let gw = g.size().x;
            painter.galley_with_override_text_color(pos2(x, rect.center().y - g.size().y / 2.0), g, c.fg);
            x += gw + gap;
        }
        if let Some(g) = label_g {
            painter.galley_with_override_text_color(pos2(x, rect.center().y - g.size().y / 2.0), g, c.fg);
        }
        if hovered {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
    }
    if clipped { resp.on_hover_text(label) } else { resp }
}

/// Regular 32 px button with an optional icon.
pub fn button(ui: &mut Ui, kind: Kind, ico: &str, label: &str) -> Response {
    paint_button(ui, kind, Size::Regular, ico, label, false, 0.0)
}

/// 28 px button for dense areas (footers, inline actions).
pub fn small_button(ui: &mut Ui, kind: Kind, ico: &str, label: &str) -> Response {
    paint_button(ui, kind, Size::Small, ico, label, false, 0.0)
}

/// Button that shows an on / off state.
pub fn toggle(ui: &mut Ui, selected: bool, ico: &str, label: &str) -> Response {
    paint_button(ui, Kind::Secondary, Size::Regular, ico, label, selected, 0.0)
}

/// Square icon-only button (ghost), with a tooltip.
pub fn icon_button(ui: &mut Ui, ico: &str, tooltip: &str) -> Response {
    paint_button(ui, Kind::Ghost, Size::Regular, ico, "", false, 0.0).on_hover_text(tooltip)
}

/// Square icon-only button that can be active.
pub fn icon_toggle(ui: &mut Ui, selected: bool, ico: &str, tooltip: &str) -> Response {
    paint_button(ui, Kind::Ghost, Size::Regular, ico, "", selected, 0.0).on_hover_text(tooltip)
}

/// A drop-down menu opened from a styled button.
pub fn menu_button<R>(ui: &mut Ui, kind: Kind, ico: &str, label: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> Response {
    menu_button_width(ui, kind, ico, label, 0.0, add_contents)
}

pub fn menu_button_width<R>(ui: &mut Ui, kind: Kind, ico: &str, label: &str, width: f32, add_contents: impl FnOnce(&mut Ui) -> R) -> Response {
    let text = if label.is_empty() { ico.to_string() } else { format!("{label}  {}", icon::CHEVRON_DOWN) };
    let resp = paint_button(ui, kind, Size::Regular, if label.is_empty() { "" } else { ico }, &text, false, width);
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(220.0);
        add_contents(ui)
    });
    resp
}

/// Styled single-line text input with an optional leading icon.
pub fn input(ui: &mut Ui, text: &mut String, hint: &str, ico: Option<&str>, width: f32) -> Response {
    input_impl(ui, text, hint, ico, width, false, false)
}

/// Monospace input (SQL fragments, identifiers).
pub fn code_input(ui: &mut Ui, text: &mut String, hint: &str, ico: Option<&str>, width: f32) -> Response {
    input_impl(ui, text, hint, ico, width, true, false)
}

/// Password input.
pub fn password_input(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> Response {
    input_impl(ui, text, hint, Some(icon::KEY), width, false, true)
}

fn input_impl(ui: &mut Ui, text: &mut String, hint: &str, ico: Option<&str>, width: f32, mono: bool, password: bool) -> Response {
    let p = theme::p();
    let width = if width <= 0.0 { ui.available_width() } else { width.min(ui.available_width().max(60.0)) };
    let (rect, _) = ui.allocate_exact_size(vec2(width, H), Sense::hover());
    let frame_idx = ui.painter().add(egui::Shape::Noop);
    let left = if ico.is_some() { 32.0 } else { 11.0 };
    let inner = egui::Rect::from_min_max(pos2(rect.left() + left, rect.top()), pos2(rect.right() - 10.0, rect.bottom()));
    let font = if mono { theme::mono(13.0) } else { FontId::proportional(13.5) };
    let te = egui::TextEdit::singleline(text)
        .frame(egui::Frame::NONE)
        .margin(Margin::ZERO)
        .font(font)
        .password(password)
        .align(egui::Align2::LEFT_CENTER)
        .desired_width(inner.width())
        .hint_text(RichText::new(hint).color(p.faint));
    // A detached child UI: placing the TextEdit must not move the parent's
    // layout cursor back inside the frame we already allocated.
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let resp = child.add(te);
    let focused = resp.has_focus();
    let hovered = ui.rect_contains_pointer(rect);
    let stroke = if focused {
        Stroke::new(1.5, p.accent)
    } else if hovered {
        Stroke::new(1.0, p.border_strong)
    } else {
        Stroke::new(1.0, p.border)
    };
    let mut shapes = vec![egui::Shape::Rect(egui::epaint::RectShape::new(rect, CornerRadius::same(RADIUS), p.input, stroke, StrokeKind::Inside))];
    if focused {
        shapes.push(egui::Shape::Rect(egui::epaint::RectShape::stroke(
            rect.expand(2.5),
            CornerRadius::same(RADIUS + 3),
            Stroke::new(3.0, p.accent.gamma_multiply(0.18)),
            StrokeKind::Outside,
        )));
    }
    if let Some(i) = ico {
        let g = ui.fonts_mut(|f| f.layout_no_wrap(i.to_string(), FontId::proportional(14.0), if focused { p.accent } else { p.faint }));
        shapes.push(egui::Shape::galley(pos2(rect.left() + 11.0, rect.center().y - g.size().y / 2.0), g, p.faint));
    }
    ui.painter().set(frame_idx, egui::Shape::Vec(shapes));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
    }
    resp
}

/// Segmented control: one of several options.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str, &str)]) -> bool {
    let p = theme::p();
    let mut changed = false;
    let font = FontId::proportional(13.0);
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, ico, label)| {
            let t = if ico.is_empty() {
                label.to_string()
            } else if label.is_empty() {
                ico.to_string()
            } else {
                format!("{ico}  {label}")
            };
            ui.fonts_mut(|f| f.layout_no_wrap(t, font.clone(), Color32::WHITE).size().x) + 22.0
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0;
    let (rect, base) = ui.allocate_exact_size(vec2(total, H), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(RADIUS), p.control, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let mut x = rect.left() + 3.0;
    for (i, ((v, ico, label), w)) in options.iter().zip(widths).enumerate() {
        let r = egui::Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(w, H - 6.0));
        let resp = ui.interact(r, base.id.with(("seg", i)), Sense::click());
        let selected = *value == *v;
        if selected {
            ui.painter().rect(r, CornerRadius::same(RADIUS - 2), p.bg3, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(RADIUS - 2), p.control_hover);
        }
        let t = if ico.is_empty() {
            label.to_string()
        } else if label.is_empty() {
            ico.to_string()
        } else {
            format!("{ico}  {label}")
        };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, t, font.clone(), if selected { p.text } else { p.dim });
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if resp.clicked() && !selected {
            *value = *v;
            changed = true;
        }
        x += w;
    }
    changed
}

/// Menu row with a fixed icon column, label and optional shortcut.
pub fn menu_item(ui: &mut Ui, ico: &str, label: &str) -> Response {
    menu_item_impl(ui, ico, label, "", false, None)
}

pub fn menu_item_shortcut(ui: &mut Ui, ico: &str, label: &str, shortcut: &str) -> Response {
    menu_item_impl(ui, ico, label, shortcut, false, None)
}

pub fn menu_item_danger(ui: &mut Ui, ico: &str, label: &str) -> Response {
    menu_item_impl(ui, ico, label, "", false, Some(theme::p().danger))
}

/// Menu row with a check mark when `checked`.
pub fn menu_check(ui: &mut Ui, checked: bool, label: &str) -> Response {
    menu_item_impl(ui, if checked { icon::CHECK } else { "" }, label, "", checked, None)
}

fn menu_item_impl(ui: &mut Ui, ico: &str, label: &str, shortcut: &str, emphasized: bool, color: Option<Color32>) -> Response {
    let p = theme::p();
    let w = ui.available_width().max(200.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    let enabled = ui.is_enabled();
    let hovered = enabled && resp.hovered();
    if hovered {
        ui.painter().rect_filled(rect, CornerRadius::same(6), p.control_hover);
    }
    let fg = color.unwrap_or(if emphasized { p.accent } else { p.text });
    let fg = if enabled { fg } else { fg.gamma_multiply(0.45) };
    if !ico.is_empty() {
        ui.painter().text(
            pos2(rect.left() + 18.0, rect.center().y),
            Align2::CENTER_CENTER,
            ico,
            FontId::proportional(14.0),
            color.unwrap_or(if hovered { p.accent } else { p.dim }),
        );
    }
    let shortcut_w =
        if shortcut.is_empty() { 0.0 } else { ui.fonts_mut(|f| f.layout_no_wrap(shortcut.to_string(), theme::mono(11.5), p.faint).size().x) + 16.0 };
    let g = truncated(ui, label, FontId::proportional(13.5), fg, rect.width() - 48.0 - shortcut_w);
    ui.painter().galley(pos2(rect.left() + 36.0, rect.center().y - g.size().y / 2.0), g, fg);
    if !shortcut.is_empty() {
        ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, shortcut, theme::mono(11.5), p.faint);
    }
    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if resp.clicked() {
        ui.close();
    }
    resp.on_hover_text(label)
}

/// Small rounded label (counts, states, environment).
pub fn pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let font = FontId::proportional(11.5);
    let g = ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, color));
    let size = g.size() + vec2(16.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(255), color.gamma_multiply(0.14));
    ui.painter().galley(rect.center() - g.size() / 2.0, g, color);
    resp
}

/// Solid badge (e.g. PRODUCTION).
pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let font = theme::bold(11.0);
    let g = ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, Color32::BLACK));
    let size = g.size() + vec2(12.0, 4.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(5), color);
    ui.painter().galley(rect.center() - g.size() / 2.0, g, Color32::BLACK);
    resp
}

/// Upper-case section label.
pub fn section(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).font(theme::bold(11.0)).color(theme::p().faint));
}

/// Title row of a modal: icon, title and an optional subtitle.
pub fn dialog_header(ui: &mut Ui, ico: &str, color: Color32, title: &str, subtitle: &str) {
    let p = theme::p();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(9), color.gamma_multiply(0.14));
        ui.painter().text(r.center(), Align2::CENTER_CENTER, ico, FontId::proportional(18.0), color);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.add_space(1.0);
            ui.label(RichText::new(title).font(theme::bold(17.0)).color(p.text));
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).color(p.dim));
            }
        });
    });
    ui.add_space(12.0);
}

/// Dialog title with a close button on the right. Returns true when closed.
pub fn dialog_title(ui: &mut Ui, ico: &str, color: Color32, title: &str, subtitle: &str) -> bool {
    let mut closed = false;
    let top = ui.cursor().min;
    let right = ui.max_rect().right();
    dialog_header(ui, ico, color, title, subtitle);
    let r = egui::Rect::from_min_size(pos2(right - H, top.y), Vec2::splat(H));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
    if icon_button(&mut child, icon::X, "Close (Esc)").clicked() {
        closed = true;
    }
    closed
}

/// Standard frame for modals.
pub fn modal_frame() -> egui::Frame {
    let p = theme::p();
    egui::Frame::NONE
        .fill(p.bg1)
        .stroke(Stroke::new(1.0, p.border_strong))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(22))
        .shadow(egui::Shadow { offset: [0, 16], blur: 40, spread: 0, color: Color32::from_black_alpha(if p.dark { 150 } else { 60 }) })
}

/// Right-aligned row of dialog buttons.
pub fn dialog_footer(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.add_space(14.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        add(ui);
    });
}

/// A labelled form row: label column + control.
pub fn form_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(118.0, H), Sense::hover());
        ui.painter().text(pos2(r.left(), r.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.0), theme::p().dim);
        add(ui);
    });
    ui.add_space(6.0);
}

/// Result / section tab: icon, label and an optional count pill.
pub fn tab(ui: &mut Ui, selected: bool, ico: &str, label: &str, count: Option<&str>, tint: Option<Color32>) -> Response {
    let p = theme::p();
    let font = FontId::proportional(13.0);
    let fg = tint.unwrap_or(if selected { p.text } else { p.dim });
    let label_g = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), font.clone(), fg));
    let count_g = count.map(|c| ui.fonts_mut(|f| f.layout_no_wrap(c.to_string(), FontId::proportional(11.0), p.dim)));
    let icon_w = 20.0;
    let count_w = count_g.as_ref().map_or(0.0, |g| g.size().x + 14.0);
    let w = 12.0 + icon_w + label_g.size().x + count_w + 12.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    let hovered = resp.hovered();
    if selected {
        ui.painter().rect(rect, CornerRadius::same(7), p.control, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
    } else if hovered {
        ui.painter().rect_filled(rect, CornerRadius::same(7), p.control_hover);
    }
    let y = rect.center().y;
    ui.painter().text(
        pos2(rect.left() + 12.0 + 7.0, y),
        Align2::CENTER_CENTER,
        ico,
        FontId::proportional(13.5),
        tint.unwrap_or(if selected { p.accent } else { p.faint }),
    );
    let lx = rect.left() + 12.0 + icon_w;
    let lw = label_g.size().x;
    ui.painter().galley(pos2(lx, y - label_g.size().y / 2.0), label_g, fg);
    if let Some(g) = count_g {
        let r = egui::Rect::from_center_size(pos2(lx + lw + 8.0 + (g.size().x + 10.0) / 2.0, y), g.size() + vec2(10.0, 4.0));
        ui.painter().rect_filled(r, CornerRadius::same(255), if selected { p.bg3 } else { p.bg2 });
        ui.painter().galley(r.center() - g.size() / 2.0, g, p.dim);
    }
    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

/// Removable filter chip.
pub fn chip(ui: &mut Ui, text: &str, color: Color32) -> bool {
    let p = theme::p();
    let g = ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), FontId::proportional(12.5), p.text));
    let size = vec2(g.size().x + 40.0, 26.0);
    let (rect, base) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(255), color.gamma_multiply(0.14), Stroke::new(1.0, color.gamma_multiply(0.35)), StrokeKind::Inside);
    ui.painter().galley(pos2(rect.left() + 12.0, rect.center().y - g.size().y / 2.0), g, p.text);
    let x_rect = egui::Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(18.0, 18.0));
    let x = ui.interact(x_rect, base.id.with("chip_x"), Sense::click());
    if x.hovered() {
        ui.painter().circle_filled(x_rect.center(), 9.0, color.gamma_multiply(0.3));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    ui.painter().text(x_rect.center(), Align2::CENTER_CENTER, icon::X, FontId::proportional(11.0), p.text);
    x.on_hover_text("Remove").clicked()
}

/// Select box: shows the current option and opens a list of choices.
pub fn dropdown<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)], width: f32) -> bool {
    let p = theme::p();
    let width = if width <= 0.0 { ui.available_width() } else { width.min(ui.available_width().max(80.0)) };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, H), Sense::click());
    let resp = resp.on_hover_cursor(CursorIcon::PointingHand);
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    let stroke = if open {
        Stroke::new(1.5, p.accent)
    } else if resp.hovered() {
        Stroke::new(1.0, p.border_strong)
    } else {
        Stroke::new(1.0, p.border)
    };
    let fill = if resp.hovered() { p.control_hover } else { p.control };
    ui.painter().rect(rect, CornerRadius::same(RADIUS), fill, stroke, StrokeKind::Inside);
    let current = options.iter().find(|(v, _)| v == value).map_or("", |(_, l)| *l);
    let fg = if ui.is_enabled() { p.text } else { p.faint };
    let g = truncated(ui, current, FontId::proportional(13.5), fg, rect.width() - 38.0);
    ui.painter().galley(pos2(rect.left() + 11.0, rect.center().y - g.size().y / 2.0), g, fg);
    ui.painter().text(pos2(rect.right() - 14.0, rect.center().y), Align2::CENTER_CENTER, icon::CHEVRON_DOWN, FontId::proportional(13.0), p.dim);
    let mut changed = false;
    egui::Popup::menu(&resp).width(width).show(|ui| {
        ui.set_min_width(width - 12.0);
        for (v, label) in options {
            if menu_check(ui, v == value, label).clicked() && v != value {
                *value = *v;
                changed = true;
            }
        }
    });
    changed
}

/// Styled checkbox with a label; the whole row is clickable.
pub fn checkbox(ui: &mut Ui, checked: &mut bool, label: &str) -> Response {
    let p = theme::p();
    let font = FontId::proportional(13.5);
    let g = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), font, p.text));
    let size = vec2(18.0 + if label.is_empty() { 0.0 } else { 9.0 + g.size().x }, 26.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    let enabled = ui.is_enabled();
    if resp.clicked() {
        *checked = !*checked;
        resp.mark_changed();
    }
    let b = egui::Rect::from_min_size(pos2(rect.left(), rect.center().y - 9.0), Vec2::splat(18.0));
    if *checked {
        ui.painter().rect_filled(b, CornerRadius::same(5), if enabled { p.primary } else { p.primary.gamma_multiply(0.5) });
        ui.painter().text(b.center(), Align2::CENTER_CENTER, icon::CHECK, FontId::proportional(12.5), Color32::WHITE);
    } else {
        let s = if resp.hovered() && enabled { p.accent } else { p.border_strong };
        ui.painter().rect(b, CornerRadius::same(5), p.input, Stroke::new(1.0, s), StrokeKind::Inside);
    }
    if !label.is_empty() {
        let fg = if enabled { p.text } else { p.faint };
        ui.painter().galley_with_override_text_color(pos2(b.right() + 9.0, rect.center().y - g.size().y / 2.0), g, fg);
    }
    if resp.hovered() && enabled {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    resp
}

/// Numeric input in the same style as [`input`]; clamps to `min..=max`.
pub fn number<T>(ui: &mut Ui, id_salt: &str, value: &mut T, min: T, max: T, width: f32) -> Response
where
    T: std::str::FromStr + ToString + Copy + PartialOrd,
{
    let id = ui.scope_id().with(("num", id_salt));
    let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| value.to_string());
    let mut resp = input_impl(ui, &mut text, "", None, width, true, false);
    if resp.changed()
        && let Ok(v) = text.trim().parse::<T>()
    {
        let v = if v < min {
            min
        } else if v > max {
            max
        } else {
            v
        };
        if v != *value {
            *value = v;
            resp.mark_changed();
        }
    }
    if resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    resp
}

/// Collapsible section with a full-width header row. Returns whether it is open.
pub fn disclosure(ui: &mut Ui, id_salt: &str, ico: &str, title: &str, detail: &str, default_open: bool, body: impl FnOnce(&mut Ui)) -> bool {
    let p = theme::p();
    let id = ui.scope_id().with(("disclosure", id_salt));
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(default_open);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    let hovered = resp.hovered();
    let radius = if open { CornerRadius { nw: RADIUS, ne: RADIUS, sw: 0, se: 0 } } else { CornerRadius::same(RADIUS) };
    ui.painter().rect(rect, radius, if hovered { p.control_hover } else { p.control }, Stroke::new(1.0, p.border), StrokeKind::Inside);
    let y = rect.center().y;
    ui.painter().text(
        pos2(rect.left() + 16.0, y),
        Align2::CENTER_CENTER,
        if open { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT },
        FontId::proportional(13.0),
        p.faint,
    );
    ui.painter().text(pos2(rect.left() + 38.0, y), Align2::CENTER_CENTER, ico, FontId::proportional(14.5), p.accent);
    ui.painter().text(pos2(rect.left() + 54.0, y), Align2::LEFT_CENTER, title, theme::bold(13.5), p.text);
    if !detail.is_empty() {
        ui.painter().text(pos2(rect.right() - 14.0, y), Align2::RIGHT_CENTER, detail, FontId::proportional(12.5), p.dim);
    }
    if hovered {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    if open {
        egui::Frame::NONE
            .fill(p.bg1)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius { nw: 0, ne: 0, sw: RADIUS, se: RADIUS })
            .inner_margin(Margin { left: 16, right: 16, top: 14, bottom: 10 })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                body(ui);
            });
    }
    ui.add_space(8.0);
    open
}

/// Tinted message box (info, success, warning, error).
pub fn callout(ui: &mut Ui, color: Color32, ico: &str, text: &str) {
    let p = theme::p();
    egui::Frame::NONE
        .fill(color.gamma_multiply(0.10))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.35)))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::symmetric(12, 9))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(ico).size(14.5).color(color));
                ui.add_space(2.0);
                ui.add(egui::Label::new(RichText::new(text).color(p.text)).wrap());
            });
        });
}

/// Small grey helper text under a control.
pub fn hint(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme::p().faint)).wrap());
}

/// [`dropdown`] for string values.
pub fn dropdown_str(ui: &mut Ui, value: &mut String, options: &[&str], width: f32) -> bool {
    let mut idx = options.iter().position(|o| *o == value.as_str()).unwrap_or(usize::MAX);
    let mut opts: Vec<(usize, &str)> = options.iter().copied().enumerate().collect();
    if idx == usize::MAX {
        opts.insert(0, (usize::MAX, value.as_str()));
    }
    let opts_owned: Vec<(usize, String)> = opts.iter().map(|(i, s)| (*i, s.to_string())).collect();
    let view: Vec<(usize, &str)> = opts_owned.iter().map(|(i, s)| (*i, s.as_str())).collect();
    let changed = dropdown(ui, &mut idx, &view, width);
    if changed && let Some(s) = options.get(idx) {
        *value = s.to_string();
    }
    changed
}

/// One-line galley cut to `max_w` with an ellipsis.
pub fn truncated(ui: &Ui, text: &str, font: FontId, color: Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: max_w.max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    ui.fonts_mut(|f| f.layout_job(job))
}

/// Header of a full page (sessions, health, structure): icon tile, title,
/// subtitle and right-aligned actions.
pub fn page_header(ui: &mut Ui, ico: &str, color: Color32, title: &str, subtitle: &str, actions: impl FnOnce(&mut Ui)) {
    let p = theme::p();
    if ui.available_width() < 740.0 {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(10), color.gamma_multiply(0.14));
            ui.painter().text(r.center(), Align2::CENTER_CENTER, ico, FontId::proportional(18.0), color);
            ui.label(RichText::new(title).font(theme::bold(18.0)).color(p.text));
        });
        if !subtitle.is_empty() {
            ui.add(egui::Label::new(RichText::new(subtitle).size(12.5).color(p.dim)).wrap());
        }
        ui.horizontal_wrapped(actions);
        return;
    }
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(42.0), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(10), color.gamma_multiply(0.14));
        ui.painter().text(r.center(), Align2::CENTER_CENTER, ico, FontId::proportional(20.0), color);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(title).font(theme::bold(18.0)).color(p.text));
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).size(12.5).color(p.dim));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            actions(ui);
        });
    });
}

/// Small metric tile: a big number over a label.
pub fn stat(ui: &mut Ui, value: &str, label: &str, color: Color32) -> Response {
    let p = theme::p();
    egui::Frame::NONE
        .fill(p.bg1)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin { left: 14, right: 16, top: 8, bottom: 8 })
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(value).font(theme::bold(18.0)).color(color));
                ui.label(RichText::new(label).size(12.0).color(p.faint));
            });
        })
        .response
}

/// Selectable pill with an optional colour dot (environments, chart series).
pub fn choice_chip(ui: &mut Ui, selected: bool, label: &str, dot: Option<Color32>) -> Response {
    let p = theme::p();
    let g = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), FontId::proportional(12.5), p.text));
    let dot_w = if dot.is_some() { 14.0 } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(g.size().x + 22.0 + dot_w, H_SMALL), Sense::click());
    let tint = dot.unwrap_or(p.accent);
    let (fill, stroke, fg) = if selected {
        (tint.gamma_multiply(0.16), tint.gamma_multiply(0.7), p.text)
    } else if resp.hovered() {
        (p.control_hover, p.border_strong, p.text)
    } else {
        (p.control, p.border, p.dim)
    };
    ui.painter().rect(rect, CornerRadius::same(255), fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let mut x = rect.left() + 11.0;
    if let Some(c) = dot {
        ui.painter().circle_filled(pos2(x + 4.0, rect.center().y), 4.0, if selected { c } else { c.gamma_multiply(0.6) });
        x += dot_w;
    }
    ui.painter().galley_with_override_text_color(pos2(x, rect.center().y - g.size().y / 2.0), g, fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_identifiers_fit_buttons_menus_and_dropdowns() {
        let context = egui::Context::default();
        let label = "a_very_long_database_or_connection_identifier_".repeat(8);
        let mut output =
            context.run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(280.0, 240.0))), ..Default::default() }, |ui| {
                let width = ui.available_width();
                assert!(button(ui, Kind::Secondary, icon::SERVER, &label).rect.width() <= width);
                assert!(menu_item(ui, icon::DATABASE, &label).rect.width() <= width);
                dropdown(ui, &mut 0, &[(0, label.as_str())], width);
            });
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(labels.len(), 3);
        for text in labels {
            assert!(text.galley.elided);
            assert!(text.pos.x >= 0.0);
            assert!(text.pos.x + text.galley.size().x <= 280.0);
        }
        output.textures_delta.clear();
    }
}
