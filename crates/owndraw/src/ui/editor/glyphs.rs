//! Gezeichnete Symbole der neuen Werkzeuge und Formen (Dreieck … Maßlinie, Stempel, Füllen, Pipette).
//! Mit dem egui-Painter gezeichnet, damit der Kern keine weiteren Symbole braucht.

use super::Tool;
use egui::{Align2, Color32, FontId, Painter, Pos2, Shape, Stroke, Vec2};
use owndraw_core::board;
use owndraw_core::item;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glyph {
    Triangle,
    Diamond,
    Star,
    Hexagon,
    Bubble,
    Arc,
    Dim,
    Check,
    Cross,
    Question,
    Exclaim,
    Fill,
    Picker,
    Stamp,
    Calc,
    Convert,
}

/// Symbol einer Form (`Board::shape_kind` ab 11).
pub fn of_shape(kind: i32) -> Option<Glyph> {
    Some(match kind {
        item::TRIANGLE => Glyph::Triangle,
        item::DIAMOND => Glyph::Diamond,
        item::STAR => Glyph::Star,
        item::HEXAGON => Glyph::Hexagon,
        item::BUBBLE => Glyph::Bubble,
        item::ARC => Glyph::Arc,
        item::DIM => Glyph::Dim,
        _ => return None,
    })
}

/// Symbol eines Stempels (`board::STAMP_*`).
pub fn of_stamp(kind: i32) -> Glyph {
    match kind {
        board::STAMP_CHECK => Glyph::Check,
        board::STAMP_CROSS => Glyph::Cross,
        board::STAMP_STAR => Glyph::Star,
        board::STAMP_QUESTION => Glyph::Question,
        _ => Glyph::Exclaim,
    }
}

/// Symbol eines Werkzeugs der Leiste, falls es ein gezeichnetes hat.
pub fn of_tool(ed: &super::Editor, t: Tool) -> Option<Glyph> {
    match t {
        Tool::Fill => Some(Glyph::Fill),
        Tool::Picker => Some(Glyph::Picker),
        Tool::Stamp => Some(Glyph::Stamp),
        Tool::Calc => Some(Glyph::Calc),
        Tool::Convert => Some(Glyph::Convert),
        Tool::Dim => Some(Glyph::Dim),
        Tool::Shape => of_shape(ed.board.shape_kind),
        _ => None,
    }
}

/// Zeichnet das Werkzeugsymbol (gezeichnete Glyphe, sonst der Maschinenbau-Pfeil) mittig in `rect`.
pub fn paint_tool(p: &Painter, rect: egui::Rect, ed: &super::Editor, t: Tool, size: f32, col: Color32) {
    match of_tool(ed, t) {
        Some(g) => paint(p, rect.center(), size, g, col),
        None => crate::style::paint_arrow_eng(p, egui::Rect::from_center_size(rect.center(), Vec2::splat(size + 4.0)), col),
    }
}

/// Zeichnet die Glyphe mit Kantenlänge `size` um `c`.
pub fn paint(p: &Painter, c: Pos2, size: f32, g: Glyph, col: Color32) {
    let s = size * 0.5;
    let st = Stroke::new(1.8, col);
    let pt = |u: f32, v: f32| Pos2::new(c.x + u * s * 2.0, c.y + v * s * 2.0);
    let poly = |v: &[(f32, f32)], close: bool| {
        let pts: Vec<Pos2> = v.iter().map(|&(u, w)| pt(u, w)).collect();
        p.add(if close { Shape::closed_line(pts, st) } else { Shape::line(pts, st) });
    };
    let seg = |a: (f32, f32), b: (f32, f32)| {
        p.line_segment([pt(a.0, a.1), pt(b.0, b.1)], st);
    };
    match g {
        Glyph::Triangle => poly(&[(0.0, -0.42), (0.46, 0.38), (-0.46, 0.38)], true),
        Glyph::Diamond => poly(&[(0.0, -0.48), (0.42, 0.0), (0.0, 0.48), (-0.42, 0.0)], true),
        Glyph::Star => {
            let v: Vec<(f32, f32)> = (0..10)
                .map(|i| {
                    let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                    let r = if i % 2 == 0 { 0.5 } else { 0.21 };
                    (r * a.cos(), r * a.sin())
                })
                .collect();
            poly(&v, true);
        }
        Glyph::Hexagon => {
            let v: Vec<(f32, f32)> = (0..6)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::PI / 3.0;
                    (0.48 * a.cos(), 0.48 * a.sin())
                })
                .collect();
            poly(&v, true);
        }
        Glyph::Bubble => poly(&[(-0.48, -0.38), (0.48, -0.38), (0.48, 0.14), (0.08, 0.14), (-0.2, 0.46), (-0.2, 0.14), (-0.48, 0.14)], true),
        Glyph::Arc => {
            let v: Vec<(f32, f32)> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / 12.0 * 0.85) - 0.0;
                    (0.46 * a.cos(), 0.46 * a.sin() + 0.2)
                })
                .collect();
            poly(&v, false);
        }
        Glyph::Dim => {
            seg((-0.48, 0.0), (0.48, 0.0));
            seg((-0.48, -0.22), (-0.48, 0.22));
            seg((0.48, -0.22), (0.48, 0.22));
            poly(&[(-0.3, -0.12), (-0.48, 0.0), (-0.3, 0.12)], false);
            poly(&[(0.3, -0.12), (0.48, 0.0), (0.3, 0.12)], false);
        }
        Glyph::Check => poly(&[(-0.42, 0.02), (-0.14, 0.32), (0.44, -0.36)], false),
        Glyph::Cross => {
            seg((-0.36, -0.36), (0.36, 0.36));
            seg((0.36, -0.36), (-0.36, 0.36));
        }
        Glyph::Question | Glyph::Exclaim => {
            let t = if g == Glyph::Question { "?" } else { "!" };
            p.text(c, Align2::CENTER_CENTER, t, FontId::proportional(size * 1.05), col);
        }
        Glyph::Fill => {
            // gekippter Eimer mit Tropfen
            poly(&[(-0.5, 0.02), (-0.06, -0.42), (0.34, -0.02), (-0.1, 0.42)], true);
            seg((-0.5, 0.02), (0.34, -0.02));
            p.circle_filled(pt(0.46, 0.26), size * 0.1, col);
        }
        Glyph::Picker => {
            seg((-0.46, 0.46), (0.12, -0.12));
            p.circle_stroke(pt(0.28, -0.28), size * 0.2, st);
        }
        Glyph::Calc => {
            // Taschenrechner: Gehäuse, Anzeige, vier Tasten
            poly(&[(-0.34, -0.48), (0.34, -0.48), (0.34, 0.48), (-0.34, 0.48)], true);
            seg((-0.18, -0.28), (0.18, -0.28));
            for (u, v) in [(-0.14, 0.0), (0.14, 0.0), (-0.14, 0.26), (0.14, 0.26)] {
                p.circle_filled(pt(u, v), size * 0.06, col);
            }
        }
        Glyph::Convert => {
            // Handschrift-Schleife, Pfeil, „T“: Geschriebenes wird zu Text
            let wave: Vec<Pos2> = (0..=12).map(|k| {
                let u = -0.48 + k as f32 * 0.035;
                pt(u, -0.12 + 0.14 * (k as f32 * 1.3).sin())
            }).collect();
            p.add(Shape::line(wave, st));
            seg((-0.1, 0.28), (0.12, 0.28));
            seg((0.05, 0.2), (0.12, 0.28));
            seg((0.05, 0.36), (0.12, 0.28));
            seg((0.2, -0.36), (0.48, -0.36));
            seg((0.34, -0.36), (0.34, 0.1));
        }
        Glyph::Stamp => {
            poly(&[(-0.15, 0.14), (-0.15, -0.08), (-0.32, -0.08), (-0.32, -0.46), (0.32, -0.46), (0.32, -0.08), (0.15, -0.08), (0.15, 0.14)], true);
            seg((-0.46, 0.4), (0.46, 0.4));
            seg((-0.46, 0.14), (0.46, 0.14));
        }
    }
}

/// Stiftarten (`item::PS_*`) mit Namen.
pub const PEN_STYLES: [(u8, &str); 5] = [
    (item::PS_NORMAL, "Normal"),
    (item::PS_FOUNTAIN, "Füller"),
    (item::PS_PENCIL, "Bleistift"),
    (item::PS_DASHED, "Gestrichelt"),
    (item::PS_DOTTED, "Gepunktet"),
];

/// Knopf mit Linienvorschau der Stiftart.
pub fn pen_style_btn(ui: &mut egui::Ui, ps: u8, on: bool, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    crate::style::btn_bg(ui, rect, &resp, on);
    let pal = crate::style::pal();
    let col = crate::style::c(if on { pal.accent } else { pal.text });
    let r = rect.shrink2(Vec2::new(8.0, 0.0));
    let (a, b, y) = (r.left(), r.right(), r.center().y);
    let p = ui.painter();
    let st = Stroke::new(2.2, col);
    match ps {
        item::PS_FOUNTAIN => {
            let n = 14;
            let mut top = Vec::new();
            let mut bot = Vec::new();
            for i in 0..=n {
                let t = i as f32 / n as f32;
                let h = 0.8 + 3.2 * (std::f32::consts::PI * t).sin();
                let x = a + (b - a) * t;
                top.push(Pos2::new(x, y - h * 0.5 + (t - 0.5) * -3.0));
                bot.push(Pos2::new(x, y + h * 0.5 + (t - 0.5) * -3.0));
            }
            bot.reverse();
            top.extend(bot);
            p.add(Shape::convex_polygon(top, col, Stroke::NONE));
        }
        item::PS_PENCIL => {
            let pts: Vec<Pos2> = (0..=16).map(|i| Pos2::new(a + (b - a) * i as f32 / 16.0, y + if i % 2 == 0 { -0.8 } else { 0.8 })).collect();
            p.add(Shape::line(pts, Stroke::new(1.2, col)));
        }
        item::PS_DASHED => {
            p.add(Shape::dashed_line(&[Pos2::new(a, y), Pos2::new(b, y)], st, 5.0, 4.0));
        }
        item::PS_DOTTED => {
            let mut x = a + 1.5;
            while x < b {
                p.circle_filled(Pos2::new(x, y), 1.4, col);
                x += 4.5;
            }
        }
        _ => {
            p.line_segment([Pos2::new(a, y), Pos2::new(b, y)], st);
        }
    }
    resp
}
