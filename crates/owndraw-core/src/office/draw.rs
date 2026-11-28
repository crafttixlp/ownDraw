//! Zeichnen der Objekte im Textdokument (Formen, Textfelder, Zeichnungen, Formeln, Diagramme) auf ein Canvas.
//! Koordinaten: linke obere Ecke im Ursprung, Größe `w` × `h` in pt; der Aufrufer setzt Lage und Maßstab.

use super::fonts::{self, Family, FontKey};
use super::layout::{wrap_simple, INK};
use super::objects::{chart_series, ChartKind, Obj, ObjKind, Shape};
use crate::canvas::{Canvas, Cap, Paint, Style};
use tiny_skia::{Path, PathBuilder};

/// Reihenfarben der Diagramme.
pub const SERIES: [u32; 6] = [0xFF2563EB, 0xFFF59E0B, 0xFF10B981, 0xFFEF4444, 0xFF8B5CF6, 0xFF64748B];

fn text_path(s: &str, font: FontKey, size: f32, x: f32, base: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let mut pen = x;
    for ch in s.chars() {
        fonts::outline(&mut pb, font, ch, pen, base, size);
        pen += fonts::advance(font, ch, size);
    }
    pb.finish()
}

fn text_w(s: &str, font: FontKey, size: f32) -> f32 {
    s.chars().map(|c| fonts::advance(font, c, size)).sum()
}

/// Text mittig in ein Rechteck setzen (umbrochen).
pub fn centered_text(c: &mut Canvas, text: &str, x: f32, y: f32, w: f32, h: f32, size: f32, color: u32, left: bool) {
    if text.trim().is_empty() {
        return;
    }
    let font = FontKey { fam: Family::Sans, bold: false, italic: false };
    let (a, d) = fonts::metrics(font, size);
    let lh = (a + d) * 1.15;
    let lines = wrap_simple(text, font, size, (w - 12.0).max(10.0));
    let total = lines.len() as f32 * lh;
    let mut by = if left { y + 6.0 + a } else { y + (h - total) / 2.0 + a };
    for l in lines {
        let tw = text_w(&l, font, size);
        let tx = if left { x + 6.0 } else { x + (w - tw) / 2.0 };
        if let Some(p) = text_path(&l, font, size, tx, by) {
            c.draw_path(&p, &Paint::fill(color));
        }
        by += lh;
    }
}

fn poly(pts: &[(f32, f32)]) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let (x, y) = pts.first()?;
    pb.move_to(*x, *y);
    for (x, y) in &pts[1..] {
        pb.line_to(*x, *y);
    }
    pb.close();
    pb.finish()
}

/// Umriss einer Form.
pub fn shape_path(s: Shape, w: f32, h: f32) -> Option<Path> {
    match s {
        Shape::Rect | Shape::TextBox => poly(&[(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]),
        Shape::Round => crate::canvas::round_rect_path(0.0, 0.0, w, h, w.min(h) * 0.18),
        Shape::Ellipse => {
            let mut pb = PathBuilder::new();
            pb.push_oval(tiny_skia::Rect::from_xywh(0.0, 0.0, w.max(0.1), h.max(0.1))?);
            pb.finish()
        }
        Shape::Triangle => poly(&[(w / 2.0, 0.0), (w, h), (0.0, h)]),
        Shape::Diamond => poly(&[(w / 2.0, 0.0), (w, h / 2.0), (w / 2.0, h), (0.0, h / 2.0)]),
        Shape::Arrow => {
            let (t, hw) = (h * 0.28, (w * 0.3).min(h));
            poly(&[(0.0, h / 2.0 - t), (w - hw, h / 2.0 - t), (w - hw, 0.0), (w, h / 2.0), (w - hw, h), (w - hw, h / 2.0 + t), (0.0, h / 2.0 + t)])
        }
        Shape::Star => {
            let (cx, cy) = (w / 2.0, h / 2.0);
            let pts: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let a = std::f32::consts::PI * (i as f32 / 5.0) - std::f32::consts::FRAC_PI_2;
                    let r = if i % 2 == 0 { 1.0 } else { 0.42 };
                    (cx + a.cos() * cx * r, cy + a.sin() * cy * r)
                })
                .collect();
            poly(&pts)
        }
        Shape::Line => {
            let mut pb = PathBuilder::new();
            pb.move_to(0.0, h / 2.0);
            pb.line_to(w, h / 2.0);
            pb.finish()
        }
        Shape::Callout => {
            let b = h * 0.75;
            let mut pb = PathBuilder::new();
            let r = (w.min(b) * 0.15).max(1.0);
            pb.move_to(r, 0.0);
            pb.line_to(w - r, 0.0);
            pb.quad_to(w, 0.0, w, r);
            pb.line_to(w, b - r);
            pb.quad_to(w, b, w - r, b);
            pb.line_to(w * 0.4, b);
            pb.line_to(w * 0.18, h);
            pb.line_to(w * 0.25, b);
            pb.line_to(r, b);
            pb.quad_to(0.0, b, 0.0, b - r);
            pb.line_to(0.0, r);
            pb.quad_to(0.0, 0.0, r, 0.0);
            pb.close();
            pb.finish()
        }
    }
}

/// Zeichnet das Objekt in der Größe `w` × `h`.
pub fn draw_obj(c: &mut Canvas, obj: &Obj, w: f32, h: f32) {
    match &obj.kind {
        ObjKind::Shape { shape, fill, stroke, sw, text, tcol } => {
            if let Some(p) = shape_path(*shape, w, h) {
                if *shape == Shape::Line {
                    c.draw_path(&p, &Paint::stroke(if *stroke != 0 { *stroke } else { INK }, sw.max(0.5), Cap::Round));
                } else {
                    if *fill != 0 {
                        c.draw_path(&p, &Paint::fill(*fill));
                    }
                    if *stroke != 0 && *sw > 0.0 {
                        c.draw_path(&p, &Paint::stroke(*stroke, *sw, Cap::Round));
                    }
                }
            }
            let tb = if *shape == Shape::Callout { h * 0.75 } else { h };
            centered_text(c, text, 0.0, 0.0, w, tb, 11.0, if *tcol != 0 { *tcol } else { INK }, *shape == Shape::TextBox);
        }
        ObjKind::Drawing { strokes } => {
            for s in strokes {
                if s.pts.len() < 2 {
                    continue;
                }
                let mut pb = PathBuilder::new();
                pb.move_to(s.pts[0] * w, s.pts[1] * h);
                if s.pts.len() == 2 {
                    pb.line_to(s.pts[0] * w + 0.01, s.pts[1] * h);
                }
                for q in s.pts[2..].chunks_exact(2) {
                    pb.line_to(q[0] * w, q[1] * h);
                }
                if let Some(p) = pb.finish() {
                    c.draw_path(&p, &Paint::stroke(if s.color != 0 { s.color } else { INK }, s.w, Cap::Round));
                }
            }
        }
        ObjKind::Formula { src } => {
            let f = crate::math::layout(src, 16.0);
            let k = (w / f.w.max(1.0)).min(h / f.h.max(1.0));
            c.save();
            c.translate((w - f.w * k) / 2.0, (h - f.h * k) / 2.0);
            c.scale(k, k);
            f.draw(c, INK);
            c.restore();
        }
        ObjKind::Chart { kind, title, data } => draw_chart(c, *kind, title, data, w, h),
    }
}

/// Natürliche Größe einer Formel in pt (Schriftgröße `size`).
pub fn formula_size(src: &str, size: f32) -> (f32, f32) {
    let f = crate::math::layout(src, size);
    (f.w, f.h)
}

fn nice_max(v: f32) -> f32 {
    if v <= 0.0 {
        return 1.0;
    }
    let e = 10f32.powf(v.log10().floor());
    for m in [1.0, 2.0, 2.5, 5.0, 10.0] {
        if m * e >= v {
            return m * e;
        }
    }
    10.0 * e
}

fn fmt_num(v: f32) -> String {
    if (v - v.round()).abs() < 0.001 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}").replace('.', ",")
    }
}

fn draw_chart(c: &mut Canvas, kind: ChartKind, title: &str, data: &[Vec<String>], w: f32, h: f32) {
    let font = FontKey { fam: Family::Sans, bold: false, italic: false };
    let bold = FontKey { bold: true, ..font };
    let small = 8.0;
    let (labels, series) = chart_series(data);
    c.draw_rect(0.0, 0.0, w, h, &Paint::fill(0xFFFFFFFF));
    c.draw_rect(0.0, 0.0, w, h, &Paint { style: Style::Stroke, color: 0xFFD8DBE2, stroke_width: 0.75, ..Paint::default() });
    let mut top = 8.0;
    if !title.trim().is_empty() {
        let tw = text_w(title, bold, 11.0);
        if let Some(p) = text_path(title, bold, 11.0, (w - tw) / 2.0, 18.0) {
            c.draw_path(&p, &Paint::fill(INK));
        }
        top = 26.0;
    }
    // Legende unten
    let leg_h = if series.len() > 1 || kind == ChartKind::Pie { 16.0 } else { 0.0 };
    let names: Vec<String> = if kind == ChartKind::Pie { labels.clone() } else { series.iter().map(|s| s.0.clone()).collect() };
    if leg_h > 0.0 {
        let total: f32 = names.iter().map(|n| text_w(n, font, small) + 22.0).sum();
        let mut x = ((w - total) / 2.0).max(4.0);
        for (i, n) in names.iter().enumerate() {
            c.draw_rect(x, h - 12.0, x + 8.0, h - 4.0, &Paint::fill(SERIES[i % SERIES.len()]));
            if let Some(p) = text_path(n, font, small, x + 11.0, h - 4.5) {
                c.draw_path(&p, &Paint::fill(0xFF4A4A55));
            }
            x += text_w(n, font, small) + 22.0;
        }
    }
    let bottom = h - 8.0 - leg_h - small - 4.0;
    if kind == ChartKind::Pie {
        let vals: Vec<f32> = series.first().map(|s| s.1.iter().map(|v| v.max(0.0)).collect()).unwrap_or_default();
        let sum: f32 = vals.iter().sum();
        if sum <= 0.0 {
            return;
        }
        let r = ((bottom + small + 4.0 - top) / 2.0).min(w / 2.0 - 8.0).max(4.0);
        let (cx, cy) = (w / 2.0, top + r);
        let mut a0 = -90.0f32;
        for (i, v) in vals.iter().enumerate() {
            let sweep = v / sum * 360.0;
            let mut pb = PathBuilder::new();
            pb.move_to(cx, cy);
            crate::canvas::arc_to(&mut pb, cx - r, cy - r, cx + r, cy + r, a0, sweep.max(0.01));
            pb.close();
            if let Some(p) = pb.finish() {
                c.draw_path(&p, &Paint::fill(SERIES[i % SERIES.len()]));
                c.draw_path(&p, &Paint::stroke(0xFFFFFFFF, 1.0, Cap::Round));
            }
            a0 += sweep;
        }
        return;
    }
    let maxv = nice_max(series.iter().flat_map(|s| s.1.iter().copied()).fold(0.0, f32::max));
    let left = 8.0 + text_w(&fmt_num(maxv), font, small) + 4.0;
    let right = w - 8.0;
    let ph = (bottom - top).max(4.0);
    // Raster und Achse
    for i in 0..=4 {
        let v = maxv * i as f32 / 4.0;
        let y = bottom - ph * i as f32 / 4.0;
        c.draw_line(left, y, right, y, &Paint::stroke(if i == 0 { 0xFF9CA3AF } else { 0xFFE5E7EB }, 0.6, Cap::Butt));
        let s = fmt_num(v);
        if let Some(p) = text_path(&s, font, small, left - 4.0 - text_w(&s, font, small), y + 3.0) {
            c.draw_path(&p, &Paint::fill(0xFF6B7280));
        }
    }
    let n = labels.len().max(1);
    let slot = (right - left) / n as f32;
    for (i, l) in labels.iter().enumerate() {
        let tw = text_w(l, font, small);
        if let Some(p) = text_path(l, font, small, left + slot * (i as f32 + 0.5) - tw / 2.0, bottom + small + 2.0) {
            c.draw_path(&p, &Paint::fill(0xFF4A4A55));
        }
    }
    match kind {
        ChartKind::Bar => {
            let ns = series.len().max(1) as f32;
            let bw = slot * 0.7 / ns;
            for (si, (_, vals)) in series.iter().enumerate() {
                for (i, v) in vals.iter().enumerate() {
                    let x = left + slot * i as f32 + slot * 0.15 + bw * si as f32;
                    let bh = ph * (v.max(0.0) / maxv);
                    c.draw_rect(x, bottom - bh, x + bw * 0.92, bottom, &Paint::fill(SERIES[si % SERIES.len()]));
                }
            }
        }
        ChartKind::Line => {
            for (si, (_, vals)) in series.iter().enumerate() {
                let mut pb = PathBuilder::new();
                for (i, v) in vals.iter().enumerate() {
                    let (x, y) = (left + slot * (i as f32 + 0.5), bottom - ph * (v.max(0.0) / maxv));
                    if i == 0 {
                        pb.move_to(x, y);
                    } else {
                        pb.line_to(x, y);
                    }
                    c.draw_circle(x, y, 2.2, &Paint::fill(SERIES[si % SERIES.len()]));
                }
                if let Some(p) = pb.finish() {
                    c.draw_path(&p, &Paint::stroke(SERIES[si % SERIES.len()], 1.8, Cap::Round));
                }
            }
        }
        ChartKind::Pie => {}
    }
}
