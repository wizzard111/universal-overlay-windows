//! Lays out and draws the overlay into a [`Canvas`].

use crate::core::SensorHistory;
use crate::frames::{Frame, LATENCY_S};
use crate::render::{Canvas, Text};
use crate::sensors::Snapshot;
use uo_common::config::{Color, Config, GraphMode, GraphScale, Layout, SensorGraph};
use uo_common::sensor::SensorId;

pub struct ViewInput<'a> {
    pub cfg: &'a Config,
    pub snap: &'a Snapshot,
    /// Frame history of the focused process (may be empty).
    pub frames: &'a [Frame],
    pub qpf: f64,
    pub now_qpc: i64,
    /// DPI scale × user scale.
    pub scale: f32,
    pub clock: &'a str,
    /// Status line shown at the bottom (e.g. missing admin rights).
    pub note: Option<&'a str>,
    /// Sampled history of graphed sensors and the current time on its clock.
    pub hist: &'a SensorHistory,
    pub now_s: f32,
}

/// One sensor graph block: title line + plot.
struct SensorBlock<'a> {
    g: &'a SensorGraph,
    title_h: f32,
    plot_h: f32,
}

impl SensorBlock<'_> {
    fn h(&self) -> f32 {
        self.title_h + self.plot_h
    }
}

fn sensor_blocks<'a>(inp: &ViewInput<'a>, ls: f32) -> Vec<SensorBlock<'a>> {
    let k = inp.scale;
    inp.cfg
        .sensor_graphs
        .iter()
        .filter(|g| g.enabled)
        .map(|g| SensorBlock {
            g,
            title_h: if g.show_title { (ls * 1.3).ceil() } else { 0.0 },
            plot_h: (g.height * k).round(),
        })
        .collect()
}

struct Row {
    label: String,
    value: String,
    unit: &'static str,
    color: Color,
    template: &'static str,
}

fn build_rows(inp: &ViewInput) -> Vec<Row> {
    let st = &inp.cfg.style;
    inp.cfg
        .sensors
        .iter()
        .filter(|s| s.enabled)
        .map(|s| {
            let (value, color) = if s.id == SensorId::Clock {
                (inp.clock.to_string(), s.color.unwrap_or(st.value_color))
            } else {
                match inp.snap.get(s.id) {
                    Some(v) => (
                        format!("{:.*}", s.id.decimals(), v),
                        s.color.unwrap_or(st.value_color),
                    ),
                    None => ("—".to_string(), st.unit_color),
                }
            };
            Row {
                label: s.label().to_string(),
                value,
                unit: s.id.unit(),
                color,
                template: s.id.template(),
            }
        })
        .collect()
}

/// Draws everything; the canvas is resized to fit the content.
/// Where the graph goes; produced by [`draw_base`], consumed by [`draw_graph`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub stats_h: f32,
    pub ls: f32,
}

/// Draw everything in one go (base + graph).
pub fn draw(text: &mut Text, cv: &mut Canvas, inp: &ViewInput) {
    if let Some(r) = draw_base(text, cv, inp) {
        draw_graph(text, cv, inp, r);
    }
}

/// Draws background, rows and notes — everything except the graph, which
/// changes every frame. Returns where the graph belongs.
pub fn draw_base(text: &mut Text, cv: &mut Canvas, inp: &ViewInput) -> Option<GraphRect> {
    let mut graph_rect = None;
    let st = &inp.cfg.style;
    let g = &inp.cfg.graph;
    let k = inp.scale;
    let vs = st.value_size * k;
    let ls = st.label_size * k;
    let pad = (st.padding * k).round();
    let gap = (st.column_spacing * k).round();
    let ugap = (4.0 * k).round();
    let rgap = (st.row_spacing * k).round();
    let shadow = st.text_shadow;

    let rows = build_rows(inp);
    let (asc, desc) = text.line_metrics(vs);
    let row_h = (asc + desc).ceil();

    // Measure columns.
    let mut label_w: Vec<f32> = Vec::with_capacity(rows.len());
    let mut value_w: Vec<f32> = Vec::with_capacity(rows.len());
    let mut unit_w: Vec<f32> = Vec::with_capacity(rows.len());
    for r in &rows {
        label_w.push(text.measure(&r.label, ls));
        value_w.push(text.measure(&r.value, vs).max(text.measure(r.template, vs)));
        unit_w.push(if r.unit.is_empty() {
            0.0
        } else {
            text.measure(r.unit, ls)
        });
    }

    let graph_on = g.enabled;
    let blocks = sensor_blocks(inp, ls);
    let any_graph = graph_on || !blocks.is_empty();
    let gw = (g.width * k).round();
    let gh = (g.height * k).round();
    let stats_h = if graph_on && g.show_stats {
        (ls * 1.3).ceil()
    } else {
        0.0
    };
    let graph_block_h = if graph_on { stats_h + gh } else { 0.0 };
    let note_h = if inp.note.is_some() {
        (ls * 1.4).ceil()
    } else {
        0.0
    };

    let (w, h);
    match st.layout {
        Layout::Vertical => {
            let lw = label_w.iter().copied().fold(0.0, f32::max);
            let vw = value_w.iter().copied().fold(0.0, f32::max);
            let uw = unit_w.iter().copied().fold(0.0, f32::max);
            let content_w = lw + gap + vw + if uw > 0.0 { ugap + uw } else { 0.0 };
            let rows_h = if rows.is_empty() {
                0.0
            } else {
                rows.len() as f32 * row_h + (rows.len() - 1) as f32 * rgap
            };
            let mut inner_w = if rows.is_empty() { 0.0 } else { content_w };
            if any_graph {
                inner_w = inner_w.max(gw);
            }
            let sgap = rgap * 2.0;
            let sensors_h: f32 = blocks.iter().map(|b| b.h()).sum::<f32>()
                + if blocks.is_empty() {
                    0.0
                } else {
                    sgap * (blocks.len() - 1) as f32 + if graph_on { sgap } else { 0.0 }
                };
            let mut note_w = 0.0;
            if let Some(n) = inp.note {
                note_w = text.measure(n, ls);
                inner_w = inner_w.max(note_w);
            }
            let between = if !rows.is_empty() && any_graph {
                rgap * 2.0
            } else {
                0.0
            };
            w = (inner_w + pad * 2.0).ceil();
            h = (pad * 2.0 + rows_h + between + graph_block_h + sensors_h + note_h).ceil();
            cv.resize_clear(w as i32, h as i32);
            cv.fill_round_rect(0.0, 0.0, w, h, st.corner_radius * k, st.background);

            let mut y = pad;
            for (i, r) in rows.iter().enumerate() {
                let base = y + asc;
                text.draw(cv, pad, base, &r.label, ls, st.label_color, shadow);
                let vx = pad + lw + gap + vw - text.measure(&r.value, vs);
                text.draw(cv, vx, base, &r.value, vs, r.color, shadow);
                if !r.unit.is_empty() {
                    text.draw(
                        cv,
                        pad + lw + gap + vw + ugap,
                        base,
                        r.unit,
                        ls,
                        st.unit_color,
                        shadow,
                    );
                }
                let _ = (label_w[i], unit_w[i]);
                y += row_h + rgap;
            }
            if !rows.is_empty() {
                y -= rgap;
            }
            y += between;
            if graph_on {
                graph_rect = Some(GraphRect {
                    x: pad,
                    y,
                    w: gw.max(inner_w),
                    h: gh,
                    stats_h,
                    ls,
                });
                y += graph_block_h;
            }
            for (i, b) in blocks.iter().enumerate() {
                if graph_on || i > 0 {
                    y += sgap;
                }
                draw_sensor_graph(text, cv, inp, b, pad, y, inner_w, ls);
                y += b.h();
            }
            if let Some(n) = inp.note {
                let _ = note_w;
                text.draw(
                    cv,
                    pad,
                    y + note_h - desc.min(note_h * 0.3),
                    n,
                    ls,
                    st.unit_color,
                    shadow,
                );
            }
        }
        Layout::Horizontal => {
            let mut items_w = 0.0;
            for i in 0..rows.len() {
                let uw = if unit_w[i] > 0.0 {
                    ugap + unit_w[i]
                } else {
                    0.0
                };
                items_w += label_w[i] + ugap * 1.5 + value_w[i] + uw;
                if i + 1 < rows.len() {
                    items_w += gap;
                }
            }
            let graph_gap = if !rows.is_empty() && any_graph {
                gap
            } else {
                0.0
            };
            let blocks_h = blocks.iter().map(|b| b.h()).fold(0.0, f32::max);
            let inner_h = row_h.max(graph_block_h).max(blocks_h);
            let n_plots = blocks.len() + graph_on as usize;
            let plots_w = if n_plots == 0 {
                0.0
            } else {
                n_plots as f32 * gw + (n_plots - 1) as f32 * gap
            };
            let mut inner_w = items_w + graph_gap + plots_w;
            if let Some(n) = inp.note {
                inner_w = inner_w.max(text.measure(n, ls));
            }
            w = (inner_w + pad * 2.0).ceil();
            h = (inner_h + pad * 2.0 + note_h).ceil();
            cv.resize_clear(w as i32, h as i32);
            cv.fill_round_rect(0.0, 0.0, w, h, st.corner_radius * k, st.background);

            let base = pad + (inner_h - row_h) * 0.5 + asc;
            let mut x = pad;
            for (i, r) in rows.iter().enumerate() {
                text.draw(cv, x, base, &r.label, ls, st.label_color, shadow);
                x += label_w[i] + ugap * 1.5;
                let vx = x + value_w[i] - text.measure(&r.value, vs);
                text.draw(cv, vx, base, &r.value, vs, r.color, shadow);
                x += value_w[i];
                if unit_w[i] > 0.0 {
                    text.draw(cv, x + ugap, base, r.unit, ls, st.unit_color, shadow);
                    x += ugap + unit_w[i];
                }
                x += gap;
            }
            let mut gx = pad + items_w + graph_gap;
            if graph_on {
                let gy = pad + (inner_h - graph_block_h) * 0.5;
                graph_rect = Some(GraphRect {
                    x: gx,
                    y: gy,
                    w: gw,
                    h: gh,
                    stats_h,
                    ls,
                });
                gx += gw + gap;
            }
            for b in &blocks {
                draw_sensor_graph(text, cv, inp, b, gx, pad + (inner_h - b.h()) * 0.5, gw, ls);
                gx += gw + gap;
            }
            if let Some(n) = inp.note {
                text.draw(
                    cv,
                    pad,
                    pad + inner_h + note_h - desc.min(note_h * 0.3),
                    n,
                    ls,
                    st.unit_color,
                    shadow,
                );
            }
        }
    }
    graph_rect
}

const NICE_MS: &[f32] = &[
    4.0, 8.33, 16.67, 25.0, 33.33, 50.0, 66.67, 100.0, 150.0, 200.0, 333.3, 500.0, 1000.0,
];

pub fn draw_graph(text: &mut Text, cv: &mut Canvas, inp: &ViewInput, r: GraphRect) {
    let GraphRect {
        x,
        y,
        w,
        h,
        stats_h,
        ls,
    } = r;
    let g = &inp.cfg.graph;
    let st = &inp.cfg.style;
    let k = inp.scale;
    let gy = y + stats_h;
    cv.fill_round_rect(x, gy, w, h, 3.0 * k, g.background);

    // Select the frames that fall into the graph and map them to 0..1 on X.
    let frames = inp.frames;
    let pts: Vec<(f32, f32)> = match g.mode {
        GraphMode::Time => {
            let win = g.time_window_s as f64 * inp.qpf;
            let latency = (LATENCY_S * inp.qpf) as i64;
            let mut end = inp.now_qpc - latency;
            if let Some(l) = frames.last() {
                end = end.max(l.ts);
            }
            let start = end - win as i64;
            frames
                .iter()
                .filter(|f| f.ts > start)
                .map(|f| (1.0 - ((end - f.ts) as f64 / win) as f32, f.ms))
                .collect()
        }
        GraphMode::Frames => {
            let n = g.frame_count as usize;
            let skip = frames.len().saturating_sub(n);
            let tail = &frames[skip..];
            let off = n - tail.len();
            tail.iter()
                .enumerate()
                .map(|(i, f)| ((off + i + 1) as f32 / n as f32, f.ms))
                .collect()
        }
    };

    if stats_h > 0.0 && !pts.is_empty() {
        let avg = pts.iter().map(|p| p.1 as f64).sum::<f64>() / pts.len() as f64;
        let max = pts.iter().map(|p| p.1).fold(0.0, f32::max);
        let s = format!("avg {avg:.2} ms  ·  max {max:.1} ms");
        let (asc, _) = text.line_metrics(ls);
        text.draw(cv, x, y + asc, &s, ls, st.unit_color, st.text_shadow);
    }

    let (lo, hi) = match g.scale {
        GraphScale::Fixed => (g.min_ms, g.max_ms),
        GraphScale::Auto => {
            let max = pts.iter().map(|p| p.1).fold(0.0, f32::max) * 1.1;
            let hi = NICE_MS
                .iter()
                .copied()
                .find(|&v| v >= max)
                .unwrap_or(max.max(1.0));
            (0.0, hi)
        }
    };
    let span = (hi - lo).max(0.001);
    let y_of = |ms: f32| -> f32 { gy + h - ((ms - lo) / span).clamp(0.0, 1.0) * h };

    // Reference lines.
    let small = (ls * 0.85).max(8.0);
    let label_h = text.cap_height(small) + 3.0 * k;
    let fmt_ms = |v: f32| {
        if v >= 100.0 {
            format!("{v:.0}")
        } else {
            format!("{v:.1}")
        }
    };
    let label_color = g.grid_color.with_alpha(0xC0);
    // Top of the scale, so the graph can always be read.
    let mut used: Vec<f32> = Vec::new();
    if g.show_scale {
        let base = gy + label_h - 1.0 * k;
        text.draw(
            cv,
            x + 3.0 * k,
            base,
            &format!("{} ms", fmt_ms(hi)),
            small,
            label_color,
            false,
        );
        used.push(base);
    }
    let mut lines: Vec<f32> = g
        .grid_ms
        .iter()
        .copied()
        .filter(|&v| v > lo && v < hi)
        .collect();
    lines.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    for gm in lines {
        let yy = y_of(gm).round();
        cv.hline_dashed(
            x as i32,
            yy as i32,
            w as i32,
            (3.0 * k).max(2.0) as i32,
            g.grid_color,
        );
        if g.show_scale {
            // Label above the line, or below it when too close to the top.
            let base = if yy - label_h < gy {
                yy + label_h
            } else {
                yy - 2.0 * k
            };
            if used.iter().all(|&u| (u - base).abs() >= label_h) && base <= gy + h {
                text.draw(
                    cv,
                    x + 3.0 * k,
                    base,
                    &fmt_ms(gm),
                    small,
                    label_color,
                    false,
                );
                used.push(base);
            }
        }
    }

    if pts.is_empty() {
        return;
    }

    // Per-pixel-column maximum, so short stutters never disappear.
    let cols = w.max(1.0) as usize;
    let mut col: Vec<f32> = vec![f32::NAN; cols];
    for &(fx, ms) in &pts {
        if fx < 0.0 {
            continue;
        }
        let c = ((fx * cols as f32) as usize).min(cols - 1);
        if col[c].is_nan() || ms > col[c] {
            col[c] = ms;
        }
    }

    let color_of = |ms: f32| -> Color {
        if ms >= g.bad_ms {
            g.bad_color
        } else if ms >= g.warn_ms {
            g.warn_color
        } else {
            g.line_color
        }
    };
    let half = (g.line_width * k * 0.5).max(0.5);
    let bottom = gy + h;
    let mut prev: Option<(usize, f32)> = None;
    for (c, &v) in col.iter().enumerate() {
        if v.is_nan() {
            continue;
        }
        let x0 = x as i32;
        match prev {
            None => {
                let yy = y_of(v);
                if g.fill {
                    let fc = color_of(v);
                    cv.vspan(
                        x0 + c as i32,
                        yy,
                        bottom,
                        fc.with_alpha((fc.a as f32 * g.fill_opacity) as u8),
                    );
                }
                cv.vspan(x0 + c as i32, yy - half, yy + half, color_of(v));
            }
            Some((pc, pv)) => {
                let n = (c - pc) as f32;
                for xi in (pc + 1)..=c {
                    let t0 = (xi - pc - 1) as f32 / n;
                    let t1 = (xi - pc) as f32 / n;
                    let m0 = pv + (v - pv) * t0;
                    let m1 = pv + (v - pv) * t1;
                    let (ya, yb) = (y_of(m0), y_of(m1));
                    let colr = color_of(m1.max(m0));
                    if g.fill {
                        cv.vspan(
                            x0 + xi as i32,
                            yb,
                            bottom,
                            colr.with_alpha((colr.a as f32 * g.fill_opacity) as u8),
                        );
                    }
                    cv.vspan(x0 + xi as i32, ya.min(yb) - half, ya.max(yb) + half, colr);
                }
            }
        }
        prev = Some((c, v));
    }
}

/// Title ("GPU TEMP  63 °C") and a history plot of one sensor. Drawn into
/// the base layer, i.e. once per sensor tick, not on every graph frame.
#[allow(clippy::too_many_arguments)]
fn draw_sensor_graph(
    text: &mut Text,
    cv: &mut Canvas,
    inp: &ViewInput,
    b: &SensorBlock,
    x: f32,
    y: f32,
    w: f32,
    ls: f32,
) {
    let sg = b.g;
    let st = &inp.cfg.style;
    let k = inp.scale;
    let id = sg.sensor;
    if b.title_h > 0.0 {
        let label = inp
            .cfg
            .sensors
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.label().to_string())
            .unwrap_or_else(|| id.default_label().into());
        let (asc, _) = text.line_metrics(ls);
        let base = y + asc;
        let lw = text.draw(cv, x, base, &label, ls, st.label_color, st.text_shadow);
        let value = match inp.snap.get(id) {
            Some(v) => format!("{:.*} {}", id.decimals(), v, id.unit()),
            None => "—".into(),
        };
        text.draw(
            cv,
            x + lw + (6.0 * k).round(),
            base,
            value.trim_end(),
            ls,
            sg.color,
            st.text_shadow,
        );
    }
    let gy = y + b.title_h;
    let h = b.plot_h;
    cv.fill_round_rect(x, gy, w, h, 3.0 * k, inp.cfg.graph.background);

    let Some(hist) = inp.hist.get(&id) else {
        return;
    };
    let from = inp.now_s - sg.window_s;
    let pts: Vec<(f32, f32)> = hist
        .iter()
        .filter(|p| p.0 >= from - 1.0)
        .map(|&(t, v)| ((t - from) / sg.window_s * w, v))
        .collect();
    if pts.is_empty() {
        return;
    }
    let (lo, hi) = if sg.auto_scale {
        let (mn, mx) = pts
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
        let pad = ((mx - mn) * 0.15).max(1.0);
        let lo = if mn >= 0.0 {
            (mn - pad).max(0.0)
        } else {
            mn - pad
        };
        (lo, mx + pad)
    } else {
        (sg.min, sg.max)
    };
    let span = (hi - lo).max(1e-3);
    let y_of = |v: f32| gy + h - ((v - lo) / span).clamp(0.0, 1.0) * h;
    let half = (sg.line_width * k * 0.5).max(0.5);
    let fill = sg.color.with_alpha((sg.color.a as f32 * 0.22) as u8);
    let bottom = gy + h;
    let x0 = x.round() as i32;
    let cols = w.max(1.0) as i32;
    let mut prev: Option<(f32, f32)> = None;
    // Next column to draw: every pixel column is painted exactly once, so
    // segment joints do not get a double-alpha fill.
    let mut next_col = 0i32;
    for &(px, v) in &pts {
        if let Some((ppx, pv)) = prev {
            let c0 = next_col.max(ppx.max(0.0).floor() as i32);
            let c1 = (px.min(w).ceil() as i32).min(cols);
            next_col = next_col.max(c1);
            let dx = (px - ppx).max(1e-3);
            for c in c0..c1 {
                let t0 = ((c as f32 - ppx) / dx).clamp(0.0, 1.0);
                let t1 = ((c as f32 + 1.0 - ppx) / dx).clamp(0.0, 1.0);
                let (ya, yb) = (y_of(pv + (v - pv) * t0), y_of(pv + (v - pv) * t1));
                if sg.fill {
                    cv.vspan(x0 + c, ya.max(yb), bottom, fill);
                }
                cv.vspan(x0 + c, ya.min(yb) - half, ya.max(yb) + half, sg.color);
            }
        }
        prev = Some((px, v));
    }
}
