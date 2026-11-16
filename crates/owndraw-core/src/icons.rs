//! Vektor-Icons auf 24er-Raster (Port von Ic.java).

use crate::canvas::{arc_to, round_rect_path, Canvas, Cap, Paint, Style};
use tiny_skia::{Pixmap, PathBuilder, Rect};

pub const BACK: i32 = 0;
pub const UNDO: i32 = 1;
pub const REDO: i32 = 2;
pub const PEN: i32 = 3;
pub const MARKER: i32 = 4;
pub const ERASER: i32 = 5;
pub const SELECT: i32 = 6;
pub const RECT: i32 = 7;
pub const ELLIPSE: i32 = 8;
pub const LINE: i32 = 9;
pub const ARROW: i32 = 10;
pub const NOTE: i32 = 11;
pub const TEXT: i32 = 12;
pub const MAGIC: i32 = 13;
pub const FINGER: i32 = 14;
pub const GRID: i32 = 15;
pub const DARK: i32 = 16;
pub const EXPORT: i32 = 17;
pub const TRASH: i32 = 18;
pub const COPY: i32 = 19;
pub const PLUS: i32 = 20;
pub const LAYERS: i32 = 21;
pub const EYE: i32 = 22;
pub const EYE_OFF: i32 = 23;
pub const LOCK: i32 = 24;
pub const UNLOCK: i32 = 25;
pub const IMAGE: i32 = 26;
pub const PLAY: i32 = 27;
pub const MORE: i32 = 28;
pub const LASSO: i32 = 29;
pub const CUT: i32 = 30;
pub const PASTE: i32 = 31;
pub const ROTATE: i32 = 32;
pub const FRONT: i32 = 33;
pub const BACKWARD: i32 = 34;
pub const DUPLICATE: i32 = 35;
pub const ERASER_PART: i32 = 36;
/// Maschinenbau-Pfeil: schmaler gefuellter Pfeil (Form-Menue).
pub const MACHINE_ARROW: i32 = 37;
// Lineal, Laserpointer, Platz schaffen (eigener Nummernblock, damit parallel neue Icons nicht kollidieren)
pub const RULER: i32 = 70;
pub const LASER: i32 = 71;
pub const SPACE: i32 = 72;
// Werkzeuge Verbinder, Tabelle, Kommentar, Zuschnitt, Gruppen (eigener Nummernblock ab 60)
pub const CONNECTOR: i32 = 60;
pub const TABLE: i32 = 61;
pub const PIN: i32 = 62;
pub const CROP: i32 = 63;
pub const GROUP: i32 = 64;
pub const UNGROUP: i32 = 65;
pub const ALIGN: i32 = 66;
pub const DISTRIBUTE: i32 = 67;
/// Icons ab 60 (für Tests).
pub const BLOCK60: [i32; 8] = [CONNECTOR, TABLE, PIN, CROP, GROUP, UNGROUP, ALIGN, DISTRIBUTE];

const DASH_SEL: Option<(f32, f32)> = Some((3.0, 3.2));

/// Polygonzug (optional geschlossen), v = x0,y0,x1,y1,...
fn poly(pb: &mut PathBuilder, close: bool, v: &[f32]) {
    if v.len() < 2 {
        return;
    }
    pb.move_to(v[0], v[1]);
    for xy in v[2..].chunks_exact(2) {
        pb.line_to(xy[0], xy[1]);
    }
    if close {
        pb.close();
    }
}

fn rect(pb: &mut PathBuilder, l: f32, t: f32, r: f32, b: f32) {
    if let Some(rc) = Rect::from_ltrb(l, t, r, b) {
        pb.push_rect(rc);
    }
}

fn round_rect(pb: &mut PathBuilder, l: f32, t: f32, r: f32, b: f32, rad: f32) {
    if let Some(p) = round_rect_path(l, t, r, b, rad) {
        pb.push_path(&p);
    }
}

/// Zeichnet den Pfad; bei `fill_stroke` (Android FILL_AND_STROKE) erst gefuellt, dann umrandet.
fn flush(c: &mut Canvas, pb: PathBuilder, p: &Paint, fill_stroke: bool) {
    if let Some(path) = pb.finish() {
        if fill_stroke {
            let mut f = p.clone();
            f.style = Style::Fill;
            f.dash = None;
            c.draw_path(&path, &f);
        }
        let mut s = p.clone();
        s.style = Style::Stroke;
        c.draw_path(&path, &s);
    }
}

/// Zeichnet das Icon id in das 24x24-Feld des aktuellen Koordinatensystems.
pub fn draw(c: &mut Canvas, id: i32, tint: u32, filled: bool) {
    let mut p = Paint { color: tint, style: Style::Stroke, stroke_width: 1.9, cap: Cap::Round, dash: None, text_size: 12.0 };
    let mut pb = PathBuilder::new();
    match id {
        BACK => poly(&mut pb, false, &[15.0, 5.0, 8.0, 12.0, 15.0, 19.0]),
        REDO => {
            c.save();
            c.scale_at(-1.0, 1.0, 12.0, 12.0);
            draw(c, UNDO, tint, filled);
            c.restore();
            return;
        }
        UNDO => {
            poly(&mut pb, false, &[9.0, 6.0, 5.0, 10.0, 9.0, 14.0]);
            pb.move_to(5.0, 10.0);
            pb.line_to(14.5, 10.0);
            arc_to(&mut pb, 10.0, 10.0, 19.0, 19.0, 270.0, 180.0);
            pb.line_to(9.0, 19.0);
        }
        PEN => {
            poly(&mut pb, true, &[5.0, 19.0, 6.0, 15.0, 16.0, 5.0, 19.0, 8.0, 9.0, 18.0]);
            poly(&mut pb, false, &[14.0, 7.0, 17.0, 10.0]);
        }
        MARKER => {
            poly(&mut pb, true, &[8.0, 14.0, 15.5, 4.0, 20.0, 8.5, 12.0, 18.0, 8.0, 18.0]);
            poly(&mut pb, false, &[4.0, 21.0, 14.0, 21.0]);
        }
        ERASER => {
            poly(&mut pb, true, &[4.0, 14.0, 12.0, 5.0, 20.0, 12.0, 13.0, 19.0, 8.0, 19.0]);
            poly(&mut pb, false, &[8.0, 9.5, 16.0, 16.0]);
        }
        ERASER_PART => {
            poly(&mut pb, true, &[9.2, 10.4, 15.0, 4.0, 20.8, 9.0, 15.7, 14.0, 12.1, 14.0]);
            poly(&mut pb, false, &[12.1, 7.2, 17.9, 11.9]);
            flush(c, pb, &p, false);
            // gestrichelte Linie
            let mut pb2 = PathBuilder::new();
            p.dash = Some((2.6, 3.2));
            poly(&mut pb2, false, &[3.5, 19.0, 17.0, 19.0]);
            flush(c, pb2, &p, false);
            p.dash = None;
            pb = PathBuilder::new();
            pb.push_circle(20.3, 19.0, 0.6);
        }
        SELECT => {
            p.dash = DASH_SEL;
            rect(&mut pb, 5.0, 5.0, 19.0, 19.0);
        }
        RECT => rect(&mut pb, 4.5, 6.0, 19.5, 18.0),
        ELLIPSE => {
            if let Some(r) = Rect::from_ltrb(4.0, 6.0, 20.0, 18.0) {
                pb.push_oval(r);
            }
        }
        LINE => poly(&mut pb, false, &[5.0, 19.0, 19.0, 5.0]),
        ARROW => {
            poly(&mut pb, false, &[5.0, 19.0, 19.0, 5.0]);
            poly(&mut pb, false, &[11.0, 5.0, 19.0, 5.0, 19.0, 13.0]);
        }
        MACHINE_ARROW => {
            // Schaft mit schmaler, gefuellter Spitze (Achse 45 Grad nach oben rechts)
            poly(&mut pb, false, &[4.0, 20.0, 14.3, 9.7]);
            poly(&mut pb, true, &[20.0, 4.0, 16.2, 11.5, 12.5, 7.8]);
        }
        NOTE => {
            poly(&mut pb, true, &[5.0, 5.0, 19.0, 5.0, 19.0, 14.0, 14.0, 19.0, 5.0, 19.0]);
            poly(&mut pb, false, &[19.0, 14.0, 14.0, 14.0, 14.0, 19.0]);
        }
        TEXT => {
            poly(&mut pb, false, &[6.0, 8.0, 6.0, 6.0, 18.0, 6.0, 18.0, 8.0]);
            poly(&mut pb, false, &[12.0, 6.0, 12.0, 19.0]);
            poly(&mut pb, false, &[9.5, 19.0, 14.5, 19.0]);
        }
        MAGIC => poly(&mut pb, true, &[12.0, 3.0, 14.0, 10.0, 21.0, 12.0, 14.0, 14.0, 12.0, 21.0, 10.0, 14.0, 3.0, 12.0, 10.0, 10.0]),
        FINGER => {
            pb.push_circle(12.0, 12.0, 3.0);
            pb.push_circle(12.0, 12.0, 8.0);
        }
        GRID => {
            rect(&mut pb, 4.5, 4.5, 19.5, 19.5);
            poly(&mut pb, false, &[9.5, 4.5, 9.5, 19.5]);
            poly(&mut pb, false, &[14.5, 4.5, 14.5, 19.5]);
            poly(&mut pb, false, &[4.5, 9.5, 19.5, 9.5]);
            poly(&mut pb, false, &[4.5, 14.5, 19.5, 14.5]);
        }
        DARK => {
            pb.push_circle(12.0, 12.0, 8.0);
            flush(c, pb, &p, false);
            // gefuelltes Tortenstueck (drawArc mit useCenter)
            let mut pie = PathBuilder::new();
            pie.move_to(12.0, 12.0);
            arc_to(&mut pie, 4.0, 4.0, 20.0, 20.0, 270.0, 180.0);
            pie.close();
            if let Some(path) = pie.finish() {
                p.style = Style::Fill;
                c.draw_path(&path, &p);
            }
            return;
        }
        EXPORT => {
            poly(&mut pb, false, &[12.0, 15.0, 12.0, 4.0]);
            poly(&mut pb, false, &[8.0, 8.0, 12.0, 4.0, 16.0, 8.0]);
            poly(&mut pb, false, &[5.0, 14.0, 5.0, 20.0, 19.0, 20.0, 19.0, 14.0]);
        }
        TRASH => {
            poly(&mut pb, false, &[5.0, 7.0, 19.0, 7.0]);
            poly(&mut pb, false, &[9.0, 7.0, 9.0, 4.5, 15.0, 4.5, 15.0, 7.0]);
            poly(&mut pb, false, &[7.0, 7.0, 8.0, 20.0, 16.0, 20.0, 17.0, 7.0]);
        }
        COPY => {
            rect(&mut pb, 8.5, 8.5, 20.0, 20.0);
            poly(&mut pb, false, &[15.5, 8.5, 15.5, 4.0, 4.0, 4.0, 4.0, 15.5, 8.5, 15.5]);
        }
        LAYERS => {
            poly(&mut pb, true, &[12.0, 4.0, 20.0, 8.5, 12.0, 13.0, 4.0, 8.5]);
            poly(&mut pb, false, &[4.0, 12.5, 12.0, 17.0, 20.0, 12.5]);
            poly(&mut pb, false, &[4.0, 16.5, 12.0, 21.0, 20.0, 16.5]);
        }
        EYE | EYE_OFF => {
            pb.move_to(2.5, 12.0);
            pb.quad_to(12.0, 3.0, 21.5, 12.0);
            pb.quad_to(12.0, 21.0, 2.5, 12.0);
            pb.push_circle(12.0, 12.0, 3.0);
            if id == EYE_OFF {
                poly(&mut pb, false, &[5.0, 19.5, 19.0, 4.5]);
            }
        }
        LOCK | UNLOCK => {
            round_rect(&mut pb, 5.5, 11.0, 18.5, 20.0, 2.5);
            pb.move_to(8.5, 11.0);
            pb.line_to(8.5, 8.0);
            arc_to(&mut pb, 8.5, 4.5, 15.5, 11.5, 180.0, 180.0);
            if id == LOCK {
                pb.line_to(15.5, 11.0);
            }
        }
        IMAGE => {
            rect(&mut pb, 4.5, 5.0, 19.5, 19.0);
            pb.push_circle(9.0, 10.0, 1.5);
            poly(&mut pb, false, &[5.0, 18.0, 10.0, 13.0, 13.0, 16.0, 15.5, 13.5, 19.0, 17.0]);
        }
        PLAY => poly(&mut pb, true, &[8.0, 5.5, 18.0, 12.0, 8.0, 18.5]),
        MORE => {
            pb.push_circle(6.0, 12.0, 1.2);
            pb.push_circle(12.0, 12.0, 1.2);
            pb.push_circle(18.0, 12.0, 1.2);
        }
        LASSO => {
            pb.move_to(5.0, 10.0);
            pb.cubic_to(5.0, 6.0, 10.0, 4.0, 14.5, 5.0);
            pb.cubic_to(19.5, 6.0, 21.0, 10.5, 18.0, 14.0);
            pb.cubic_to(15.0, 17.5, 8.5, 17.0, 6.0, 14.0);
            pb.cubic_to(4.5, 12.5, 4.5, 11.0, 5.0, 10.0);
            pb.close();
            p.dash = DASH_SEL;
            flush(c, pb, &p, false);
            p.dash = None;
            pb = PathBuilder::new();
            pb.move_to(10.0, 16.8);
            pb.quad_to(9.0, 20.5, 13.0, 20.5);
            pb.quad_to(15.5, 20.5, 15.5, 18.5);
        }
        CUT => {
            pb.push_circle(7.5, 17.0, 2.5);
            pb.push_circle(16.5, 17.0, 2.5);
            poly(&mut pb, false, &[9.1, 15.0, 17.0, 4.0]);
            poly(&mut pb, false, &[14.9, 15.0, 7.0, 4.0]);
        }
        PASTE => {
            round_rect(&mut pb, 5.0, 5.5, 19.0, 20.5, 2.0);
            poly(&mut pb, true, &[9.0, 5.5, 9.0, 3.5, 15.0, 3.5, 15.0, 5.5]);
            poly(&mut pb, false, &[8.5, 11.0, 15.5, 11.0]);
            poly(&mut pb, false, &[8.5, 15.0, 13.0, 15.0]);
        }
        ROTATE => {
            // Bogen mit forceMoveTo: eigener Teilpfad am Bogenanfang
            let a = 135.0f32.to_radians();
            pb.move_to(12.0 + 7.0 * a.cos(), 12.0 + 7.0 * a.sin());
            arc_to(&mut pb, 5.0, 5.0, 19.0, 19.0, 135.0, 270.0);
            poly(&mut pb, false, &[21.4, 17.0, 16.95, 17.0, 16.95, 12.5]);
        }
        FRONT => {
            rect(&mut pb, 9.5, 9.5, 20.0, 20.0);
            flush(c, pb, &p, true);
            pb = PathBuilder::new();
            poly(&mut pb, false, &[14.5, 9.5, 14.5, 4.0, 4.0, 4.0, 4.0, 14.5, 9.5, 14.5]);
        }
        BACKWARD => {
            poly(&mut pb, true, &[4.0, 4.0, 14.5, 4.0, 14.5, 9.5, 9.5, 9.5, 9.5, 14.5, 4.0, 14.5]);
            flush(c, pb, &p, true);
            pb = PathBuilder::new();
            rect(&mut pb, 9.5, 9.5, 20.0, 20.0);
        }
        DUPLICATE => {
            rect(&mut pb, 9.5, 9.5, 20.0, 20.0);
            poly(&mut pb, false, &[14.75, 12.5, 14.75, 17.0]);
            poly(&mut pb, false, &[12.5, 14.75, 17.0, 14.75]);
            poly(&mut pb, false, &[14.5, 9.5, 14.5, 4.0, 4.0, 4.0, 4.0, 14.5, 9.5, 14.5]);
        }
        RULER => {
            poly(&mut pb, true, &[2.5, 15.5, 15.5, 2.5, 21.5, 8.5, 8.5, 21.5]);
            for (i, t) in [0.2f32, 0.35, 0.5, 0.65, 0.8].iter().enumerate() {
                let (x, y) = (2.5 + 13.0 * t, 15.5 - 13.0 * t);
                let l = if i % 2 == 0 { 3.2 } else { 1.9 };
                poly(&mut pb, false, &[x, y, x + l, y + l]);
            }
        }
        LASER => {
            poly(&mut pb, false, &[4.0, 20.0, 13.0, 11.0]);
            pb.push_circle(16.5, 7.5, 2.3);
            for v in [[16.5, 3.0, 16.5, 1.5], [21.0, 7.5, 22.5, 7.5], [19.7, 4.3, 20.8, 3.2], [13.3, 4.3, 12.2, 3.2], [19.7, 10.7, 20.8, 11.8]] {
                poly(&mut pb, false, &v);
            }
        }
        SPACE => {
            poly(&mut pb, false, &[3.0, 12.0, 21.0, 12.0]);
            poly(&mut pb, false, &[12.0, 9.5, 12.0, 3.0]);
            poly(&mut pb, false, &[8.5, 6.5, 12.0, 3.0, 15.5, 6.5]);
            poly(&mut pb, false, &[12.0, 14.5, 12.0, 21.0]);
            poly(&mut pb, false, &[8.5, 17.5, 12.0, 21.0, 15.5, 17.5]);
        }
        CONNECTOR => {
            rect(&mut pb, 3.5, 3.5, 9.5, 9.5);
            rect(&mut pb, 14.5, 14.5, 20.5, 20.5);
            poly(&mut pb, false, &[9.5, 6.5, 13.0, 6.5, 13.0, 17.5, 14.5, 17.5]);
        }
        TABLE => {
            rect(&mut pb, 4.0, 5.0, 20.0, 19.0);
            poly(&mut pb, false, &[4.0, 9.7, 20.0, 9.7]);
            poly(&mut pb, false, &[4.0, 14.3, 20.0, 14.3]);
            poly(&mut pb, false, &[9.3, 5.0, 9.3, 19.0]);
            poly(&mut pb, false, &[14.7, 5.0, 14.7, 19.0]);
        }
        PIN => {
            pb.push_circle(12.0, 8.5, 4.5);
            poly(&mut pb, false, &[12.0, 13.0, 12.0, 21.0]);
        }
        CROP => {
            poly(&mut pb, false, &[7.0, 3.0, 7.0, 17.0, 21.0, 17.0]);
            poly(&mut pb, false, &[3.0, 7.0, 17.0, 7.0, 17.0, 21.0]);
        }
        GROUP | UNGROUP => {
            if id == GROUP {
                p.dash = DASH_SEL;
                rect(&mut pb, 3.5, 3.5, 20.5, 20.5);
                flush(c, pb, &p, false);
                p.dash = None;
                pb = PathBuilder::new();
                rect(&mut pb, 7.0, 7.0, 13.0, 13.0);
                rect(&mut pb, 11.0, 11.0, 17.0, 17.0);
            } else {
                rect(&mut pb, 4.0, 4.0, 11.0, 11.0);
                rect(&mut pb, 13.0, 13.0, 20.0, 20.0);
            }
        }
        ALIGN => {
            poly(&mut pb, false, &[5.0, 4.0, 5.0, 20.0]);
            rect(&mut pb, 8.0, 6.5, 19.0, 10.5);
            rect(&mut pb, 8.0, 13.5, 15.0, 17.5);
        }
        DISTRIBUTE => {
            poly(&mut pb, false, &[3.5, 4.0, 3.5, 20.0]);
            poly(&mut pb, false, &[20.5, 4.0, 20.5, 20.0]);
            rect(&mut pb, 9.0, 7.0, 15.0, 17.0);
        }
        _ => {
            poly(&mut pb, false, &[12.0, 5.0, 12.0, 19.0]);
            poly(&mut pb, false, &[5.0, 12.0, 19.0, 12.0]);
        }
    }
    flush(c, pb, &p, filled);
}

/// Rendert ein Icon in ein quadratisches Pixmap mit Kantenlaenge px (transparenter Hintergrund).
pub fn render(id: i32, px: u32, tint: u32, filled: bool) -> Option<tiny_skia::Pixmap> {
    let mut pm = Pixmap::new(px, px)?;
    {
        let mut c = Canvas::new(&mut pm);
        let s = px as f32 / 24.0;
        c.scale(s, s);
        draw(&mut c, id, tint, filled);
    }
    Some(pm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::{Color, PixmapPaint, Transform};

    #[test]
    fn alle_icons_sichtbar() {
        for id in (0..=37).chain(BLOCK60) {
            for filled in [false, true] {
                let pm = render(id, 48, 0xFFFFFFFF, filled).expect("pixmap");
                let n = pm.pixels().iter().filter(|p| p.alpha() > 0).count();
                assert!(n >= 20, "icon {id} filled={filled}: nur {n} Pixel");
            }
        }
    }

    #[test]
    fn lineal_laser_platz_sichtbar() {
        for id in [RULER, LASER, SPACE] {
            let pm = render(id, 48, 0xFFFFFFFF, false).expect("pixmap");
            assert!(pm.pixels().iter().filter(|p| p.alpha() > 0).count() >= 20, "icon {id}");
        }
    }

    #[test]
    #[ignore]
    fn sheet() {
        let (cols, cell) = (6u32, 96u32);
        let rows = (38 + cols - 1) / cols;
        let mut sheet = Pixmap::new(cols * cell, rows * cell).expect("sheet");
        sheet.fill(Color::from_rgba8(0x1C, 0x1D, 0x22, 0xFF));
        for id in 0..=37 {
            let pm = render(id, cell, 0xFFFFFFFF, false).expect("pixmap");
            let (x, y) = ((id as u32 % cols) * cell, (id as u32 / cols) * cell);
            sheet.draw_pixmap(x as i32, y as i32, pm.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        }
        sheet.save_png("/home/claude/agents/icons/icons-sheet.png").expect("png");
    }
}
