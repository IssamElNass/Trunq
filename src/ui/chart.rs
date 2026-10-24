//! One-click charts of a result set: bars for categories, lines for time /
//! numeric X. Follows the data-viz rules: one y-axis, categorical hues in a
//! fixed validated order, thin marks, recessive grid, hover tooltips, legend
//! for 2+ series with direct labels for up to four.

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, pos2, vec2};

use super::theme::{self, icon};
use super::widgets;
use crate::db::result::ResultSet;
use crate::db::types::Cat;
use crate::util;

type Tooltip = (Pos2, String, Vec<(usize, String)>);

/// Validated categorical palette (fixed order; never cycled).
const LIGHT: [u32; 8] = [0x2a78d6, 0xeb6834, 0x1baf7a, 0xeda100, 0xe87ba4, 0x008300, 0x6250d6, 0xe34948];
const DARK: [u32; 8] = [0x3987e5, 0xd95926, 0x199e70, 0xc98500, 0xd55181, 0x008300, 0x9085e9, 0xe66767];
const MAX_SERIES: usize = 8;
const MAX_BARS: usize = 300;

fn series_color(i: usize) -> Color32 {
    let pal = if theme::p().dark { DARK } else { LIGHT };
    let c = pal[i.min(MAX_SERIES - 1)];
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Auto,
    Bar,
    Line,
}

#[derive(Default)]
pub struct ChartState {
    pub x: Option<usize>,
    pub ys: Vec<usize>,
    pub kind: Kind,
    initialised: bool,
}

/// Parses timestamps / dates / numbers into an f64 x-position.
fn x_value(v: &str, cat: Cat) -> Option<f64> {
    match cat {
        Cat::Number => v.parse().ok(),
        Cat::DateTime => {
            let s = v.trim();
            if let Ok(d) = chrono::DateTime::parse_from_str(&format!("{s}00"), "%Y-%m-%d %H:%M:%S%.f%z") {
                return Some(d.timestamp_millis() as f64);
            }
            if let Ok(d) = chrono::DateTime::parse_from_str(&format!("{s}:00"), "%Y-%m-%d %H:%M:%S%.f%:z") {
                return Some(d.timestamp_millis() as f64);
            }
            if let Ok(d) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
                return Some(d.and_utc().timestamp_millis() as f64);
            }
            if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                return Some(d.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis() as f64);
            }
            None
        }
        _ => None,
    }
}

/// Identifier columns (`id`, `customer_id`) are rarely what you want to plot.
fn is_id_column(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "id" || n.ends_with("_id")
}

fn col_abs_max(rs: &ResultSet, c: usize) -> f64 {
    (0..rs.nrows().min(5000)).filter_map(|r| rs.get(r, c)?.parse::<f64>().ok()).fold(0.0, |m, v| m.max(v.abs()))
}

/// Two measures are plottable on one axis if within 20× of each other.
fn same_scale(a: f64, b: f64) -> bool {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    hi == 0.0 || lo / hi >= 0.05
}

/// "Nice" axis ticks (1-2-5 steps).
fn nice_ticks(min: f64, max: f64, count: usize) -> Vec<f64> {
    if !min.is_finite() || !max.is_finite() || max <= min {
        return vec![min];
    }
    let raw = (max - min) / count.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw).unwrap_or(10.0 * mag);
    let start = (min / step).floor() * step;
    let mut out = Vec::new();
    let mut v = start;
    while v <= max + step * 0.5 && out.len() < 50 {
        out.push(v);
        v += step;
    }
    out
}

fn fmt_num(v: f64) -> String {
    let a = v.abs();
    if a >= 1e9 {
        format!("{:.1}B", v / 1e9)
    } else if a >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if a >= 1e4 {
        format!("{:.1}K", v / 1e3)
    } else if v.fract() == 0.0 {
        format!("{v:.0}")
    } else if a >= 100.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn fmt_time(ms: f64, span_ms: f64) -> String {
    let Some(d) = chrono::DateTime::from_timestamp_millis(ms as i64) else { return String::new() };
    let d = d.with_timezone(&chrono::Local);
    if span_ms < 2.0 * 86_400_000.0 {
        d.format("%H:%M").to_string()
    } else if span_ms < 400.0 * 86_400_000.0 {
        d.format("%b %d").to_string()
    } else {
        d.format("%Y-%m").to_string()
    }
}

pub fn ui(ui: &mut Ui, st: &mut ChartState, rs: &ResultSet, cats: &[Cat]) {
    let pal = theme::p();
    let ncols = rs.ncols();
    let numeric: Vec<usize> = (0..ncols).filter(|&c| cats.get(c) == Some(&Cat::Number)).collect();
    if !st.initialised {
        st.initialised = true;
        // X: first date/time column, else first non-numeric, else first column.
        st.x = (0..ncols).find(|&c| cats.get(c) == Some(&Cat::DateTime)).or_else(|| (0..ncols).find(|&c| !numeric.contains(&c))).or(Some(0));
        let candidates: Vec<usize> = numeric.iter().copied().filter(|c| Some(*c) != st.x && !is_id_column(&rs.columns[*c].name)).collect();
        let candidates = if candidates.is_empty() { numeric.iter().copied().filter(|c| Some(*c) != st.x).collect() } else { candidates };
        // Only combine measures of a similar magnitude (one axis, never two).
        let first_max = candidates.first().map_or(0.0, |&c| col_abs_max(rs, c));
        st.ys = candidates.iter().copied().filter(|&c| same_scale(first_max, col_abs_max(rs, c))).take(3).collect();
    }

    // ---- controls (one row above the chart) ----
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.add_space(8.0);
        ui.label(RichText::new("X axis").color(pal.dim));
        let xs: Vec<(Option<usize>, &str)> = (0..ncols).map(|c| (Some(c), rs.columns[c].name.as_str())).collect();
        widgets::dropdown(ui, &mut st.x, &xs, 180.0);
        ui.add_space(10.0);
        ui.label(RichText::new("Series").color(pal.dim));
        for &c in &numeric {
            if Some(c) == st.x {
                continue;
            }
            let on = st.ys.contains(&c);
            let dot = st.ys.iter().position(|y| *y == c).map(series_color).unwrap_or(pal.faint);
            if widgets::choice_chip(ui, on, &rs.columns[c].name, Some(dot)).clicked() {
                if on {
                    st.ys.retain(|y| *y != c);
                } else if st.ys.len() < MAX_SERIES {
                    st.ys.push(c);
                }
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            widgets::segmented(
                ui,
                &mut st.kind,
                &[(Kind::Auto, icon::SPARKLES, "Auto"), (Kind::Bar, icon::CHART_BAR, "Bars"), (Kind::Line, icon::CHART_LINE, "Lines")],
            );
        });
    });
    ui.add_space(4.0);

    let Some(xc) = st.x else { return };
    if st.ys.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("Pick at least one numeric column for Y").color(pal.faint)));
        return;
    }
    let xcat = cats.get(xc).copied().unwrap_or(Cat::Other);
    let continuous = matches!(xcat, Cat::DateTime | Cat::Number);
    let line = match st.kind {
        Kind::Line => true,
        Kind::Bar => false,
        Kind::Auto => continuous,
    };

    // ---- data ----
    let n = rs.nrows();
    let mut rows: Vec<(f64, usize)> = Vec::with_capacity(n);
    if line && continuous {
        for r in 0..n {
            if let Some(x) = rs.get(r, xc).and_then(|v| x_value(v, xcat)) {
                rows.push((x, r));
            }
        }
        rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    } else {
        rows = (0..n.min(if line { 20_000 } else { MAX_BARS })).map(|r| (r as f64, r)).collect();
    }
    let yv = |r: usize, c: usize| rs.get(r, c).and_then(|v| v.parse::<f64>().ok());
    let (mut ymin, mut ymax) = (0f64, f64::NEG_INFINITY);
    for &(_, r) in &rows {
        for &c in &st.ys {
            if let Some(v) = yv(r, c) {
                ymin = ymin.min(v);
                ymax = ymax.max(v);
            }
        }
    }
    if !ymax.is_finite() || rows.is_empty() {
        ui.add_space(30.0);
        ui.vertical_centered(|ui| ui.label(RichText::new("No numeric data to plot").color(pal.faint)));
        return;
    }
    if ymax <= ymin {
        ymax = ymin + 1.0;
    }
    let ticks = nice_ticks(ymin, ymax, 5);
    let ylo = ticks.first().copied().unwrap_or(ymin).min(ymin);
    let yhi = ticks.last().copied().unwrap_or(ymax).max(ymax);

    // One axis only: warn when selected measures differ wildly in scale.
    let maxes: Vec<f64> = st.ys.iter().map(|&c| col_abs_max(rs, c)).collect();
    let (mn, mx) = maxes.iter().fold((f64::INFINITY, 0f64), |(a, b), &v| (a.min(v), b.max(v)));
    if st.ys.len() > 1 && !same_scale(mn, mx) {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                RichText::new(format!(
                    "{}  These measures differ ~{}× in scale, the smaller one will look flat. Plot them separately.",
                    super::theme::icon::INFO,
                    fmt_num((mx / mn.max(1e-9)).round())
                ))
                .small()
                .color(pal.warn),
            );
        });
    }

    // legend (2+ series)
    if st.ys.len() >= 2 {
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            for (i, &c) in st.ys.iter().enumerate() {
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 10.0), Sense::hover());
                if line {
                    ui.painter().line_segment([r.left_center(), r.right_center()], Stroke::new(2.0, series_color(i)));
                } else {
                    ui.painter().rect_filled(r.shrink(1.0), 2.0, series_color(i));
                }
                ui.label(RichText::new(&rs.columns[c].name).small().color(pal.dim));
                ui.add_space(8.0);
            }
        });
    }

    let avail = ui.available_rect_before_wrap();
    let (rect, resp) = ui.allocate_exact_size(avail.size(), Sense::hover());
    let p = ui.painter_at(rect);
    let axis_font = FontId::proportional(11.0);
    let left_w = ticks.iter().map(|t| fmt_num(*t).len()).max().unwrap_or(3) as f32 * 7.0 + 14.0;
    let right_pad = if line && st.ys.len() <= 4 && st.ys.len() >= 2 { 90.0 } else { 16.0 };
    let plot = Rect::from_min_max(pos2(rect.left() + left_w, rect.top() + 10.0), pos2(rect.right() - right_pad, rect.bottom() - 28.0));
    if plot.width() < 40.0 || plot.height() < 40.0 {
        return;
    }
    let ymap = |v: f64| plot.bottom() - ((v - ylo) / (yhi - ylo)) as f32 * plot.height();

    // recessive grid + y labels
    for t in &ticks {
        let y = ymap(*t);
        p.hline(plot.x_range(), y, Stroke::new(1.0, pal.border.gamma_multiply(if *t == 0.0 { 1.0 } else { 0.45 })));
        p.text(pos2(plot.left() - 8.0, y), Align2::RIGHT_CENTER, fmt_num(*t), axis_font.clone(), pal.faint);
    }

    let hover = resp.hover_pos().filter(|h| plot.expand2(vec2(0.0, 10.0)).contains(*h));
    // (anchor, title, (series index, text) lines)
    let mut tooltip: Option<Tooltip> = None;

    if line {
        let (xmin, xmax) = (rows.first().map(|r| r.0).unwrap_or(0.0), rows.last().map(|r| r.0).unwrap_or(1.0));
        let xspan = (xmax - xmin).max(1e-9);
        let xmap = |x: f64| plot.left() + ((x - xmin) / xspan) as f32 * plot.width();
        // x axis labels
        if continuous {
            for t in nice_ticks(xmin, xmax, (plot.width() / 110.0) as usize) {
                if t < xmin || t > xmax {
                    continue;
                }
                let label = if xcat == Cat::DateTime { fmt_time(t, xspan) } else { fmt_num(t) };
                p.text(pos2(xmap(t), plot.bottom() + 8.0), Align2::CENTER_TOP, label, axis_font.clone(), pal.faint);
            }
        }
        // series (decimated to ~2 points per pixel column)
        let px = plot.width().max(1.0) as usize;
        let mut end_labels: Vec<(f32, String)> = Vec::new();
        for (i, &c) in st.ys.iter().enumerate() {
            let color = series_color(i);
            let mut pts: Vec<Pos2> = Vec::with_capacity(px * 2);
            let mut last_bucket = usize::MAX;
            let (mut bmin, mut bmax): (Option<Pos2>, Option<Pos2>) = (None, None);
            for &(x, r) in &rows {
                let Some(v) = yv(r, c) else { continue };
                let pt = pos2(xmap(x), ymap(v));
                let bucket = ((pt.x - plot.left()) as usize).min(px);
                if bucket != last_bucket {
                    if let Some(a) = bmin {
                        pts.push(a);
                    }
                    if let Some(b) = bmax
                        && Some(b) != bmin
                    {
                        pts.push(b);
                    }
                    bmin = Some(pt);
                    bmax = Some(pt);
                    last_bucket = bucket;
                } else {
                    if pt.y > bmin.map_or(f32::MIN, |p| p.y) {
                        bmin = Some(pt);
                    }
                    if pt.y < bmax.map_or(f32::MAX, |p| p.y) {
                        bmax = Some(pt);
                    }
                }
            }
            pts.extend(bmin);
            if bmax != bmin {
                pts.extend(bmax);
            }
            if pts.len() == 1 {
                p.circle_filled(pts[0], 4.0, color);
            }
            p.add(egui::Shape::line(pts.clone(), Stroke::new(2.0, color)));
            // direct label at the line end (2-4 series)
            if (2..=4).contains(&st.ys.len())
                && let Some(end) = pts.last()
            {
                end_labels.push((end.y, rs.columns[c].name.clone()));
            }
        }
        // direct labels, nudged apart so they never overlap
        end_labels.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut last_y = f32::NEG_INFINITY;
        for (y, name) in end_labels {
            let y = y.max(last_y + 15.0);
            p.text(pos2(plot.right() + 6.0, y), Align2::LEFT_CENTER, name, axis_font.clone(), pal.dim);
            last_y = y;
        }
        // crosshair + tooltip on nearest x
        if let Some(h) = hover {
            let target = xmin + ((h.x - plot.left()) / plot.width()) as f64 * xspan;
            let idx = rows.partition_point(|r| r.0 < target).min(rows.len() - 1);
            let idx = if idx > 0 && (rows[idx - 1].0 - target).abs() < (rows[idx].0 - target).abs() { idx - 1 } else { idx };
            let (x, r) = rows[idx];
            let xx = xmap(x);
            p.vline(xx, plot.y_range(), Stroke::new(1.0, pal.dim.gamma_multiply(0.6)));
            let mut vals = Vec::new();
            for (i, &c) in st.ys.iter().enumerate() {
                if let Some(v) = yv(r, c) {
                    p.circle_filled(pos2(xx, ymap(v)), 4.5, pal.bg0);
                    p.circle_filled(pos2(xx, ymap(v)), 3.5, series_color(i));
                    vals.push((i, format!("{}: {}", rs.columns[c].name, rs.s(r, c))));
                }
            }
            tooltip = Some((pos2(xx, h.y), rs.get(r, xc).unwrap_or("NULL").to_string(), vals));
        }
    } else {
        // grouped bars
        let groups = rows.len().max(1);
        let gw = plot.width() / groups as f32;
        let ns = st.ys.len();
        let gap = 2.0;
        let bw = ((gw - 6.0).max(1.0) / ns as f32 - gap).clamp(1.0, 56.0);
        let group_w = ns as f32 * (bw + gap) - gap;
        let y0 = ymap(0f64.max(ylo));
        for (gi, &(_, r)) in rows.iter().enumerate() {
            let gx = plot.left() + gi as f32 * gw + 3.0 + ((gw - 6.0) - group_w).max(0.0) / 2.0;
            for (i, &c) in st.ys.iter().enumerate() {
                let Some(v) = yv(r, c) else { continue };
                let x0 = gx + i as f32 * (bw + gap);
                let y = ymap(v);
                let bar = Rect::from_min_max(pos2(x0, y.min(y0)), pos2(x0 + bw, y.max(y0)));
                let radius = if bw >= 6.0 { 4u8 } else { 0 };
                let cr = if v >= 0.0 { CornerRadius { nw: radius, ne: radius, sw: 0, se: 0 } } else { CornerRadius { nw: 0, ne: 0, sw: radius, se: radius } };
                let hovered = hover.is_some_and(|h| h.x >= gx && h.x < gx + group_w);
                let color = series_color(i);
                p.rect_filled(bar, cr, if hover.is_some() && !hovered { color.gamma_multiply(0.55) } else { color });
            }
            // x labels: skip to avoid collisions
            let every = ((70.0 / gw).ceil() as usize).max(1);
            if gi % every == 0 {
                let label: String = rs.get(r, xc).unwrap_or("NULL").chars().take(14).collect();
                p.text(pos2(gx + group_w / 2.0, plot.bottom() + 8.0), Align2::CENTER_TOP, label, axis_font.clone(), pal.faint);
            }
            let slot_x0 = plot.left() + gi as f32 * gw;
            if let Some(h) = hover
                && h.x >= slot_x0
                && h.x < slot_x0 + gw
            {
                let vals = st.ys.iter().enumerate().map(|(i, &c)| (i, format!("{}: {}", rs.columns[c].name, rs.s(r, c)))).collect();
                tooltip = Some((pos2(gx + group_w / 2.0, h.y), rs.get(r, xc).unwrap_or("NULL").to_string(), vals));
            }
        }
        if n > MAX_BARS {
            p.text(
                plot.right_top() + vec2(0.0, -2.0),
                Align2::RIGHT_TOP,
                format!("first {MAX_BARS} of {} rows", util::thousands(n)),
                axis_font.clone(),
                pal.faint,
            );
        }
    }
    // x axis line
    p.hline(plot.x_range(), plot.bottom(), Stroke::new(1.0, pal.border));

    if let Some((at, title, vals)) = tooltip {
        egui::Area::new(egui::Id::unique("chart_tip")).order(egui::Order::Tooltip).fixed_pos(at + vec2(14.0, 10.0)).constrain(true).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).fill(pal.bg2).show(ui, |ui| {
                ui.label(RichText::new(title).strong().color(pal.text));
                for (i, v) in vals {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                        ui.painter().rect_filled(r, 2.0, series_color(i));
                        ui.label(RichText::new(v).font(theme::mono(12.0)).color(pal.text));
                    });
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_nice() {
        assert_eq!(nice_ticks(0.0, 100.0, 5), vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0]);
        assert!(nice_ticks(0.0, 7.3, 5).windows(2).all(|w| (w[1] - w[0] - 2.0).abs() < 1e-9));
    }

    #[test]
    fn id_columns() {
        assert!(is_id_column("id") && is_id_column("customer_id"));
        assert!(!is_id_column("paid") && !is_id_column("valid"));
    }

    #[test]
    fn parses_timestamps() {
        assert!(x_value("2026-10-22 00:11:51.411675+02", Cat::DateTime).is_some());
        assert!(x_value("2026-10-22", Cat::DateTime).is_some());
        assert!(x_value("2026-10-22 00:11:51", Cat::DateTime).is_some());
    }
}
