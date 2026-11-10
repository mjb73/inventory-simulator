//! Hand-painted stock-cover chart: closing stock, trigger levels and receipts.
//!
//! The plot is drawn one column per week — as wide as the weekly table's
//! columns — and is scrolled sideways by the caller so the graph always shows
//! the same weeks as the table below it. The value axis stays pinned on the
//! left, outside the scrolling part.

use crate::engine::{Results, PERIODS};
use crate::fmt;
use crate::theme::{self, Theme};
use egui::{Align2, Color32, Mesh, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct ChartOptions {
    pub safety: bool,
    pub lower: bool,
    pub upper: bool,
    pub receipts: bool,
}

impl Default for ChartOptions {
    fn default() -> Self {
        Self {
            safety: true,
            lower: true,
            upper: true,
            receipts: true,
        }
    }
}

/// What the pinned axis needs in order to label the plot it sits beside.
struct Span {
    y0: f64,
    y1: f64,
    step: f64,
    top: f32,
    bottom: f32,
}

fn nice_step(range: f64, target: f64) -> f64 {
    if range <= 0.0 {
        return 1.0;
    }
    let raw = range / target;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 2.5 {
        2.5
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    };
    step * mag
}

fn dashed(painter: &egui::Painter, pts: &[Pos2], stroke: Stroke, dash: f32, gap: f32) {
    for pair in pts.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let v = b - a;
        let len = v.length();
        if len <= 0.01 {
            continue;
        }
        let dir = v / len;
        let mut t = 0.0;
        while t < len {
            let end = (t + dash).min(len);
            painter.line_segment([a + dir * t, a + dir * end], stroke);
            t = end + gap;
        }
    }
}

/// The one series the graph draws: closing stock — the inventory left at the
/// end of each week, which is the workbook's "Closing stock" row. The
/// start-of-week stock balance is deliberately never plotted.
pub fn plotted(res: &Results) -> &[f64] {
    &res.end_stock
}

/// Draws the chart: a pinned value axis of `gutter_w`, then the scrolling plot.
/// `offset` is the shared sideways position (in pixels) and is written back
/// with wherever the user scrolled this frame.
pub fn draw(
    ui: &mut Ui,
    theme: &Theme,
    res: &Results,
    opt: &ChartOptions,
    frozen: usize,
    week_w: f32,
    height: f32,
    gutter_w: f32,
    offset: &mut f32,
    first_week: u32,
    first_year: u32,
) {
    let content_w = (PERIODS as f32) * week_w;
    let mut span: Option<Span> = None;
    let mut gutter = Rect::NOTHING;
    let mut viewport = Rect::NOTHING;

    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let (_, g) = ui.allocate_space(Vec2::new(gutter_w, height));
        gutter = g;

        let sa = egui::ScrollArea::horizontal()
            .id_salt("cover_chart")
            .auto_shrink([false, true])
            .horizontal_scroll_offset(*offset)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                span = paint_body(
                    ui,
                    theme,
                    res,
                    opt,
                    frozen,
                    week_w,
                    height,
                    content_w,
                    first_week,
                    first_year,
                );
            });

        *offset = sa.state.clamped_offset().x;
        viewport = sa.inner_rect;
    });

    if !viewport.is_positive() {
        return;
    }
    let Some(s) = span else { return };

    // Pinned value axis, painted in the strip left of the scrolling plot.
    let painter = ui.painter_at(gutter);
    painter.text(
        Pos2::new(gutter.left() + 18.0, gutter.top() + 12.0),
        Align2::LEFT_CENTER,
        "STOCK",
        theme::heading(10.0),
        theme.faint,
    );
    let mut tick = s.y0;
    let mut guard = 0;
    while tick <= s.y1 + s.step * 0.001 && guard < 40 {
        let t = (tick - s.y0) / (s.y1 - s.y0);
        let y = s.bottom - (t as f32) * (s.bottom - s.top);
        painter.text(
            Pos2::new(gutter.right() - 10.0, y),
            Align2::RIGHT_CENTER,
            fmt::compact(tick),
            theme::mono(10.0),
            theme.faint,
        );
        tick += s.step;
        guard += 1;
    }
}

fn paint_body(
    ui: &mut Ui,
    theme: &Theme,
    res: &Results,
    opt: &ChartOptions,
    frozen: usize,
    week_w: f32,
    height: f32,
    content_w: f32,
    first_week: u32,
    first_year: u32,
) -> Option<Span> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(content_w, height), Sense::hover());
    if rect.width() < 40.0 || rect.height() < 40.0 {
        return None;
    }
    let painter = ui.painter_at(rect);
    let view = ui.clip_rect().intersect(rect);

    let top = rect.top() + 10.0;
    // Leave room under the plot for the week numbers and the floating bar.
    let bottom = rect.bottom() - 26.0;
    if bottom - top < 30.0 {
        return None;
    }

    // Vertical range across the whole plan, so sliding sideways never makes
    // the lines jump up and down.
    let mut lo = 0.0f64;
    let mut hi = 0.0f64;
    // The solid line is closing stock — the stock left at the end of each week.
    // The opening balance is not drawn.
    let close = plotted(res);
    for v in close {
        hi = hi.max(*v);
        lo = lo.min(*v);
    }
    if opt.safety {
        for v in &res.safety_stock {
            hi = hi.max(*v);
        }
    }
    if opt.lower {
        for v in &res.lower_threshold {
            lo = lo.min(*v);
        }
    }
    if opt.upper {
        for v in &res.upper_threshold {
            hi = hi.max(*v);
            lo = lo.min(*v);
        }
    }
    if opt.receipts {
        for v in &res.receipts {
            hi = hi.max(*v);
        }
    }
    if hi <= lo {
        hi = lo + 1.0;
    }
    let step = nice_step(hi - lo, 4.5);
    let y0 = (lo / step).floor() * step;
    let y1 = (hi / step).ceil() * step + step * 0.15;

    let x_at = |i: usize| rect.left() + (i as f32 + 0.5) * week_w;
    let y_at = |v: f64| {
        let t = (v - y0) / (y1 - y0);
        bottom - (t as f32) * (bottom - top)
    };
    let in_view = |i: usize| x_at(i) > view.left() - week_w && x_at(i) < view.right() + week_w;

    // Frozen period shading.
    if frozen > 0 {
        let right = x_at(frozen.min(PERIODS) - 1) + week_w / 2.0;
        let band = Rect::from_min_max(Pos2::new(rect.left(), top), Pos2::new(right, bottom));
        painter.rect_filled(band, 3.0, theme.warn.gamma_multiply(0.07));
        if band.center().x > view.left() - 40.0 && band.left() < view.right() {
            painter.text(
                Pos2::new(
                    band.center().x.clamp(view.left() + 26.0, view.right() - 26.0),
                    top + 9.0,
                ),
                Align2::CENTER_CENTER,
                "frozen",
                theme::mono(9.5),
                theme.warn.gamma_multiply(0.85),
            );
        }
    }

    // Horizontal grid + value labels (the labels live in the pinned axis).
    let mut tick = y0;
    let mut guard = 0;
    while tick <= y1 + step * 0.001 && guard < 40 {
        let y = y_at(tick);
        let is_zero = tick.abs() < step * 0.001;
        painter.line_segment(
            [Pos2::new(view.left(), y), Pos2::new(view.right(), y)],
            Stroke::new(
                1.0,
                if is_zero {
                    theme.stroke.gamma_multiply(1.5)
                } else {
                    theme.stroke.gamma_multiply(0.75)
                },
            ),
        );
        tick += step;
        guard += 1;
    }

    // One faint line per week column, so the graph lines up with the table.
    let every = if week_w >= 40.0 { 1 } else { 13 };
    for i in (0..=PERIODS).step_by(every) {
        let x = rect.left() + i as f32 * week_w;
        if x < view.left() - 1.0 || x > view.right() + 1.0 {
            continue;
        }
        painter.line_segment(
            [Pos2::new(x, top), Pos2::new(x, bottom)],
            Stroke::new(1.0, theme.stroke.gamma_multiply(if i % 13 == 0 { 1.0 } else { 0.45 })),
        );
    }

    // Week labels under the plot, written the way the sheet writes them.
    for i in (0..PERIODS).step_by(if week_w >= 40.0 { 1 } else { 13 }) {
        if !in_view(i) {
            continue;
        }
        let quarter = i == 0 || (i + 1) % 13 == 1 || i + 1 == PERIODS;
        painter.text(
            Pos2::new(x_at(i), bottom + 12.0),
            Align2::CENTER_CENTER,
            fmt::period(first_week, first_year, i),
            theme::mono(9.5),
            if quarter { theme.text } else { theme.faint },
        );
    }

    // Receipts as bars from zero.
    if opt.receipts {
        let zero = y_at(0.0f64.max(y0));
        let bw = (week_w * 0.45).min(26.0).max(1.5);
        for (i, v) in res.receipts.iter().enumerate() {
            if *v <= 0.0 || !in_view(i) {
                continue;
            }
            let x = x_at(i);
            let bar = Rect::from_min_max(
                Pos2::new(x - bw / 2.0, y_at(*v).min(zero)),
                Pos2::new(x + bw / 2.0, zero),
            );
            painter.rect_filled(bar, 1.5, theme.good.gamma_multiply(0.45));
        }
    }

    // Closing stock: the inventory left at the end of each week. This is the
    // only line on the plot. It is painted in the theme's brightest neutral —
    // deliberately not the accent blue, which the table uses for the
    // start-of-week balance row — so the line is never mistaken for that row.
    let zero_y = y_at(0.0f64.max(y0));
    let mut mesh = Mesh::default();
    for i in 0..PERIODS {
        let x = x_at(i);
        mesh.colored_vertex(Pos2::new(x, y_at(close[i])), theme.text.gamma_multiply(0.08));
        mesh.colored_vertex(Pos2::new(x, zero_y), Color32::TRANSPARENT);
    }
    for i in 0..PERIODS - 1 {
        let a = (i * 2) as u32;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    painter.add(Shape::mesh(mesh));

    if opt.safety {
        let pts: Vec<Pos2> = (0..PERIODS)
            .map(|i| Pos2::new(x_at(i), y_at(res.safety_stock[i])))
            .collect();
        dashed(&painter, &pts, Stroke::new(1.4, theme.warn), 5.0, 4.0);
    }
    if opt.lower {
        let pts: Vec<Pos2> = (0..PERIODS)
            .map(|i| Pos2::new(x_at(i), y_at(res.lower_threshold[i])))
            .collect();
        dashed(&painter, &pts, Stroke::new(1.2, theme.bad), 3.0, 4.0);
    }
    if opt.upper {
        let pts: Vec<Pos2> = (0..PERIODS)
            .map(|i| Pos2::new(x_at(i), y_at(res.upper_threshold[i])))
            .collect();
        dashed(&painter, &pts, Stroke::new(1.2, theme.accent2), 6.0, 5.0);
    }

    // The closing line goes on last, so it stays the clearest thing on the
    // plot even where the dashed trigger lines cross it.
    let closing: Vec<Pos2> = (0..PERIODS)
        .map(|i| Pos2::new(x_at(i), y_at(close[i])))
        .collect();
    painter.add(Shape::line(closing, Stroke::new(2.2, theme.text)));

    // Hover readout — the highlighted column is the one the table shows
    // directly underneath it.
    if let Some(pos) = resp.hover_pos() {
        if view.contains(pos) {
            let rel = ((pos.x - rect.left()) / week_w).floor();
            let i = (rel.max(0.0) as usize).min(PERIODS - 1);
            let band = Rect::from_min_max(
                Pos2::new(rect.left() + i as f32 * week_w, top),
                Pos2::new(rect.left() + (i as f32 + 1.0) * week_w, bottom),
            );
            painter.rect_filled(band, 0.0, theme.text.gamma_multiply(0.05));

            let x = x_at(i);
            painter.line_segment(
                [Pos2::new(x, top), Pos2::new(x, bottom)],
                Stroke::new(1.0, theme.text.gamma_multiply(0.35)),
            );
            for (v, col) in [(close[i], theme.text), (res.safety_stock[i], theme.warn)] {
                painter.circle_filled(Pos2::new(x, y_at(v)), 2.8, col);
            }
            if opt.upper {
                painter.circle_filled(
                    Pos2::new(x, y_at(res.upper_threshold[i])),
                    2.8,
                    theme.accent2,
                );
            }

            let label = format!(
                "{}   closing {}   upper {}   safety {}   order {}",
                fmt::period_and_week(first_week, first_year, i),
                fmt::qty(close[i]),
                fmt::qty(res.upper_threshold[i]),
                fmt::qty(res.safety_stock[i]),
                fmt::qty(res.receipts[i]),
            );
            let galley = painter.layout_no_wrap(label, theme::mono(11.0), theme.text);
            let text_pos = Pos2::new(
                (view.right() - 8.0 - galley.size().x).max(view.left() + 6.0),
                top + 8.0,
            );
            let bg = Rect::from_min_size(
                text_pos - Vec2::new(6.0, 4.0),
                galley.size() + Vec2::new(12.0, 8.0),
            );
            painter.rect_filled(bg, 6.0, theme.panel.gamma_multiply(0.94));
            painter.rect_stroke(
                bg,
                6.0,
                Stroke::new(1.0, theme.stroke),
                egui::StrokeKind::Inside,
            );
            painter.galley(text_pos, galley, theme.text);
        }
    }

    Some(Span {
        y0,
        y1,
        step,
        top,
        bottom,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{run, Inputs};

    #[test]
    fn the_graph_draws_the_closing_row_and_never_the_opening_balance() {
        let d = vec![100.0; PERIODS];
        let m = vec![0.0; PERIODS];
        let inp = Inputs {
            opening_stock: 5000.0,
            min_lot: 500.0,
            rounding: 100.0,
            frozen_period: 0,
            order_frequency: 1,
            static_safety: 400.0,
            safety_weeks: 2,
            tolerance_pct: 10.0,
            lower_pct: 50.0,
        };
        let (r, _) = run(&inp, &d, &m);
        let drawn = plotted(&r);
        assert_eq!(drawn.len(), PERIODS);
        for i in 0..PERIODS {
            assert!((drawn[i] - r.end_stock[i]).abs() < 1e-12);
            assert!(
                (drawn[i] - r.stock_balance[i]).abs() > 1e-9,
                "week {} would draw the start-of-week balance ({})",
                i + 1,
                r.stock_balance[i]
            );
        }
    }
}
