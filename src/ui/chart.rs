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

#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
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
    data: Option<ChartData>,
    lines: Option<LineData>,
}

struct ChartData {
    key: u64,
    rows: Vec<(f64, usize)>,
    /// Finite Y values by series/source row; NaN marks NULL or invalid data.
    values: Vec<Vec<f64>>,
    ymin: f64,
    ymax: f64,
    maxes: Vec<f64>,
}

struct LineData {
    key: (u64, usize),
    series: Vec<Vec<(f64, f64)>>,
}

impl ChartState {
    fn prepare(&mut self, rs: &ResultSet, cats: &[Cat], xc: usize, line: bool) {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (rs.generation, rs.nrows(), rs.byte_size(), xc, &self.ys, line, cats).hash(&mut hash);
        let key = hash.finish();
        if self.data.as_ref().is_some_and(|d| d.key == key) {
            return;
        }
        let xcat = cats.get(xc).copied().unwrap_or(Cat::Other);
        let continuous = matches!(xcat, Cat::DateTime | Cat::Number);
        let rows = if line && continuous {
            let mut rows: Vec<_> = (0..rs.nrows()).filter_map(|r| Some((x_value(rs.get(r, xc)?, xcat)?, r))).collect();
            rows.sort_by(|a, b| a.0.total_cmp(&b.0));
            rows
        } else {
            (0..rs.nrows().min(if line { 20_000 } else { MAX_BARS })).map(|r| (r as f64, r)).collect()
        };
        // Bars only need their displayed rows. Continuous lines retain all
        // finite X rows for exact hover lookup; no strings are copied.
        let count = rows.iter().map(|r| r.1 + 1).max().unwrap_or(0);
        let values: Vec<Vec<f64>> = self.ys.iter().map(|&c| (0..count).map(|r| rs.get(r, c).and_then(finite_number).unwrap_or(f64::NAN)).collect()).collect();
        let (mut ymin, mut ymax) = (0f64, f64::NEG_INFINITY);
        for &(_, r) in &rows {
            for series in &values {
                let v = series[r];
                if v.is_finite() {
                    ymin = ymin.min(v);
                    ymax = ymax.max(v);
                }
            }
        }
        let maxes = self.ys.iter().map(|&c| col_abs_max(rs, c)).collect();
        self.data = Some(ChartData { key, rows, values, ymin, ymax, maxes });
        self.lines = None;
    }

    fn prepare_lines(&mut self, width: usize) {
        let data = self.data.as_ref().unwrap();
        let width = width.max(1);
        let key = (data.key, width);
        if self.lines.as_ref().is_some_and(|lines| lines.key == key) {
            return;
        }
        let xmin = data.rows.first().map_or(0.0, |r| r.0);
        let xmax = data.rows.last().map_or(1.0, |r| r.0);
        let series = data.values.iter().map(|values| decimate(&data.rows, values, xmin, xmax, width)).collect();
        self.lines = Some(LineData { key, series });
    }
}

fn finite_number(value: &str) -> Option<f64> {
    value.parse::<f64>().ok().filter(|v| v.is_finite())
}

// Preserve the source order of each bucket's extrema: always drawing the min
// before the max invents reversals in a line that actually trends downward.
fn decimate(rows: &[(f64, usize)], values: &[f64], xmin: f64, xmax: f64, width: usize) -> Vec<(f64, f64)> {
    let mut out = Vec::with_capacity(width.saturating_add(1) * 2);
    let mut bucket = usize::MAX;
    let (mut lo, mut hi): (Option<usize>, Option<usize>) = (None, None);
    let flush = |out: &mut Vec<_>, lo: Option<usize>, hi: Option<usize>| {
        if let (Some(a), Some(b)) = (lo, hi) {
            let (a, b) = (a.min(b), a.max(b));
            out.push((rows[a].0, values[rows[a].1]));
            if a != b {
                out.push((rows[b].0, values[rows[b].1]));
            }
        }
    };
    for (i, &(x, r)) in rows.iter().enumerate() {
        let v = values[r];
        if !v.is_finite() {
            continue;
        }
        let next = (normalise(x, xmin, xmax) * width as f64) as usize;
        if next != bucket {
            flush(&mut out, lo, hi);
            lo = Some(i);
            hi = Some(i);
            bucket = next;
        } else {
            if v < values[rows[lo.unwrap()].1] {
                lo = Some(i);
            }
            if v > values[rows[hi.unwrap()].1] {
                hi = Some(i);
            }
        }
    }
    flush(&mut out, lo, hi);
    // Extrema alone can omit the actual endpoints within their buckets.
    if let Some(&(x, r)) = rows.iter().find(|&&(_, r)| values[r].is_finite()) {
        let first = (x, values[r]);
        if out.first() != Some(&first) {
            out.insert(0, first);
        }
    }
    if let Some(&(x, r)) = rows.iter().rev().find(|&&(_, r)| values[r].is_finite()) {
        let last = (x, values[r]);
        if out.last() != Some(&last) {
            out.push(last);
        }
    }
    out
}

fn normalise(value: f64, min: f64, max: f64) -> f64 {
    let scale = min.abs().max(max.abs()).max(f64::MIN_POSITIVE);
    let span = max / scale - min / scale;
    if span <= 0.0 { 0.0 } else { (value / scale - min / scale) / span }
}

/// Parses timestamps / dates / numbers into an f64 x-position.
fn x_value(v: &str, cat: Cat) -> Option<f64> {
    match cat {
        Cat::Number => finite_number(v),
        Cat::DateTime => {
            let s = v.trim();
            if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
                return Some(d.timestamp_millis() as f64);
            }
            if let Ok(d) = chrono::DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f%:z") {
                return Some(d.timestamp_millis() as f64);
            }
            if let Ok(d) = chrono::DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f%z") {
                return Some(d.timestamp_millis() as f64);
            }
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
    (0..rs.nrows().min(5000)).filter_map(|r| finite_number(rs.get(r, c)?)).fold(0.0, |m, v| m.max(v.abs()))
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
    if !(max - min).is_finite() {
        let scale = min.abs().max(max.abs());
        return nice_ticks(min / scale, max / scale, count).into_iter().map(|v| v * scale).filter(|v| v.is_finite()).collect();
    }
    let raw = (max - min) / count.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    if mag == 0.0 || !mag.is_finite() {
        return vec![min, max];
    }
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
    st.prepare(rs, cats, xc, line);
    let data = st.data.as_ref().unwrap();
    let rows = &data.rows;
    let (ymin, mut ymax) = (data.ymin, data.ymax);
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
    let (mn, mx) = data.maxes.iter().fold((f64::INFINITY, 0f64), |(a, b), &v| (a.min(v), b.max(v)));
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
    let ymap = |v: f64| plot.bottom() - normalise(v, ylo, yhi) as f32 * plot.height();

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
        st.prepare_lines(plot.width().max(1.0) as usize);
    }
    let data = st.data.as_ref().unwrap();
    let rows = &data.rows;
    let yv = |r: usize, c: usize| st.ys.iter().position(|&y| y == c).map(|i| data.values[i][r]).filter(|v| v.is_finite());

    if line {
        let (xmin, xmax) = (rows.first().map(|r| r.0).unwrap_or(0.0), rows.last().map(|r| r.0).unwrap_or(1.0));
        let xspan = (xmax - xmin).max(1e-9);
        let xmap = |x: f64| plot.left() + normalise(x, xmin, xmax) as f32 * plot.width();
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
        let mut end_labels: Vec<(f32, String)> = Vec::new();
        for (i, &c) in st.ys.iter().enumerate() {
            let color = series_color(i);
            let pts: Vec<Pos2> = st.lines.as_ref().unwrap().series[i].iter().map(|&(x, y)| pos2(xmap(x), ymap(y))).collect();
            if pts.len() == 1 {
                p.circle_filled(pts[0], 4.0, color);
            }
            // direct label at the line end (2-4 series)
            if (2..=4).contains(&st.ys.len())
                && let Some(end) = pts.last()
            {
                end_labels.push((end.y, rs.columns[c].name.clone()));
            }
            p.add(egui::Shape::line(pts, Stroke::new(2.0, color)));
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
            let ratio = ((h.x - plot.left()) / plot.width()).clamp(0.0, 1.0) as f64;
            let target = xmin * (1.0 - ratio) + xmax * ratio;
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
mod cache_tests {
    use super::*;
    use crate::db::result::Column;

    fn result(rows: &[[&str; 2]]) -> ResultSet {
        let mut rs = ResultSet::new(vec![Column::text("x"), Column::text("y")]);
        for row in rows {
            rs.push_text_row(&[Some(row[0]), Some(row[1])]);
        }
        rs
    }

    #[test]
    fn chart_cache_refreshes_for_append_replacement_axes_and_kind() {
        let cats = [Cat::Number, Cat::Number];
        let mut rs = result(&[["2", "4"], ["1", "3"]]);
        let mut state = ChartState { ys: vec![1], ..Default::default() };
        state.prepare(&rs, &cats, 0, true);
        state.prepare_lines(100);
        assert_eq!(state.data.as_ref().unwrap().rows, [(1.0, 1), (2.0, 0)]);
        let key = state.data.as_ref().unwrap().key;
        let ptr = state.data.as_ref().unwrap().rows.as_ptr();
        state.prepare(&rs, &cats, 0, true);
        assert_eq!(state.data.as_ref().unwrap().rows.as_ptr(), ptr);
        assert!(state.lines.is_some());
        rs.append(result(&[["0", "9"]]));
        state.prepare(&rs, &cats, 0, true);
        assert_ne!(state.data.as_ref().unwrap().key, key);
        assert!(state.lines.is_none());
        assert_eq!(state.data.as_ref().unwrap().ymax, 9.0);
        // Same shape and bytes, different content and result identity.
        let rs = result(&[["2", "6"], ["1", "5"], ["0", "7"]]);
        state.prepare(&rs, &cats, 0, true);
        assert_eq!(state.data.as_ref().unwrap().ymax, 7.0);
        state.prepare(&rs, &cats, 0, false);
        assert_eq!(state.data.as_ref().unwrap().rows[0].1, 0);
        state.ys = vec![0];
        state.prepare(&rs, &cats, 1, true);
        assert_eq!(state.data.as_ref().unwrap().ymax, 2.0);
    }

    #[test]
    fn timestamp_offsets_and_nonfinite_values() {
        let reference = x_value("2026-10-25 12:30:00+00", Cat::DateTime).unwrap();
        for value in ["2026-10-25 14:00:00+01:30", "2026-10-25 07:30:00-05", "2026-10-25T12:30:00Z", "2026-10-25 14:00:00+0130"] {
            assert_eq!(x_value(value, Cat::DateTime), Some(reference), "{value}");
        }
        let rs = result(&[["NaN", "2"], ["Infinity", "3"], ["1", "Infinity"], ["2", "4"]]);
        let mut state = ChartState { ys: vec![1], ..Default::default() };
        state.prepare(&rs, &[Cat::Number, Cat::Number], 0, true);
        let data = state.data.unwrap();
        assert_eq!(data.rows.len(), 2);
        assert_eq!(data.ymax, 4.0);
        assert!(data.values[0][2].is_nan());
        assert_eq!(normalise(0.0, -1e308, 1e308), 0.5);
        assert!(nice_ticks(-1e308, 1e308, 5).iter().all(|v| v.is_finite()));
    }

    #[test]
    fn decimation_preserves_extrema_order_and_bounds_geometry() {
        let rows = vec![(0.0, 0), (1.0, 1), (2.0, 2), (3.0, 3)];
        assert_eq!(decimate(&rows, &[3.0, 1.0, 2.0, 0.0], 0.0, 3.0, 1), [(0.0, 3.0), (1.0, 1.0), (3.0, 0.0)]);
        let rows: Vec<_> = (0..100_000).map(|i| (i as f64, i)).collect();
        let values: Vec<_> = (0..100_000).map(|i| (i as f64).sin()).collect();
        let points = decimate(&rows, &values, 0.0, 99_999.0, 500);
        assert!(points.len() <= 1004);
        assert!(points.windows(2).all(|p| p[0].0 <= p[1].0));
        assert_eq!(points.first(), Some(&(0.0, 0.0)));
        assert_eq!(points.last(), Some(&(99_999.0, 99_999f64.sin())));
    }

    /// Preparation/cache benchmark, excludes fixture construction and painting.
    #[test]
    #[ignore]
    fn bench_chart_million() {
        let mut rs = ResultSet::new(vec![Column::text("x"), Column::text("y")]);
        for i in 0..1_000_000 {
            rs.push_text_row(&[Some(&(1_000_000 - i).to_string()), Some(&(i % 997).to_string())]);
        }
        let mut state = ChartState { ys: vec![1], ..Default::default() };
        let start = std::time::Instant::now();
        state.prepare(&rs, &[Cat::Number, Cat::Number], 0, true);
        state.prepare_lines(1000);
        let cold = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            state.prepare(&rs, &[Cat::Number, Cat::Number], 0, true);
            state.prepare_lines(1000);
            std::hint::black_box(&state);
        }
        println!(
            "TRUNQ_BENCH chart rows=1000000 cold_ms={:.3} cached_1000_ms={:.3} points={}",
            cold.as_secs_f64() * 1000.0,
            start.elapsed().as_secs_f64() * 1000.0,
            state.lines.as_ref().unwrap().series[0].len()
        );
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
