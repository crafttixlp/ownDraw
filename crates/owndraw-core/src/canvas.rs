//! Zeichenflaeche auf tiny-skia mit einer an android.graphics.Canvas angelehnten API
//! (Transformations-Stapel, Paint, Ebenen mit Deckkraft).

use crate::text;
use std::rc::Rc;
use tiny_skia::{
    Color, FillRule, FilterQuality, LineCap, LineJoin, Mask, Paint as SkPaint, Path, PathBuilder, Pixmap, PixmapPaint, PixmapRef,
    Rect, Stroke, StrokeDash, Transform,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Fill,
    Stroke,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cap {
    Butt,
    Round,
}

/// Farben sind 0xAARRGGBB wie unter Android. Strich-Verbindungen sind immer rund.
#[derive(Clone, Debug)]
pub struct Paint {
    pub color: u32,
    pub style: Style,
    pub stroke_width: f32,
    pub cap: Cap,
    /// Strichmuster (an, aus) in lokalen Einheiten.
    pub dash: Option<(f32, f32)>,
    pub text_size: f32,
}

impl Default for Paint {
    fn default() -> Self {
        Paint { color: 0xFF000000, style: Style::Fill, stroke_width: 1.0, cap: Cap::Butt, dash: None, text_size: 12.0 }
    }
}

impl Paint {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn stroke(color: u32, width: f32, cap: Cap) -> Self {
        Paint { color, style: Style::Stroke, stroke_width: width, cap, dash: None, text_size: 12.0 }
    }
    pub fn fill(color: u32) -> Self {
        Paint { color, ..Self::default() }
    }
}

pub fn sk_color(c: u32) -> Color {
    Color::from_rgba8((c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8)
}

enum Saved {
    Matrix(Transform, Option<Rc<Mask>>),
    Layer(Transform, u8, Option<Rc<Mask>>),
}

pub struct Canvas<'a> {
    base: &'a mut Pixmap,
    /// Offscreen-Ebenen (save_layer_alpha); gezeichnet wird immer auf die oberste.
    layers: Vec<Pixmap>,
    stack: Vec<Saved>,
    m: Transform,
    /// Aktuelle Clip-Maske (Geraete-Pixel), wird mit save()/restore() gesichert.
    clip: Option<Rc<Mask>>,
}

impl<'a> Canvas<'a> {
    pub fn new(base: &'a mut Pixmap) -> Self {
        Canvas { base, layers: Vec::new(), stack: Vec::new(), m: Transform::identity(), clip: None }
    }

    pub fn width(&self) -> u32 {
        self.base.width()
    }
    pub fn height(&self) -> u32 {
        self.base.height()
    }
    pub fn transform(&self) -> Transform {
        self.m
    }

    fn target(&mut self) -> &mut Pixmap {
        match self.layers.last_mut() {
            Some(p) => p,
            None => self.base,
        }
    }

    /// Zielflaeche und aktuelle Clip-Maske.
    fn parts(&mut self) -> (&mut Pixmap, Option<&Mask>) {
        let clip = self.clip.as_deref();
        let t = match self.layers.last_mut() {
            Some(p) => p,
            None => &mut *self.base,
        };
        (t, clip)
    }

    /// Schneidet alles Folgende (bis zum passenden restore()) auf den Pfad zu (aktuelle Transformation, mit Kantenglaettung).
    pub fn clip_path(&mut self, path: &Path) {
        let m = self.m;
        let mask = match self.clip.as_deref() {
            Some(old) => {
                let mut k = old.clone();
                k.intersect_path(path, FillRule::Winding, true, m);
                Some(k)
            }
            None => Mask::new(self.base.width(), self.base.height()).map(|mut k| {
                k.fill_path(path, FillRule::Winding, true, m);
                k
            }),
        };
        if let Some(k) = mask {
            self.clip = Some(Rc::new(k));
        }
    }

    /// Wie clip_path mit einem Rechteck.
    pub fn clip_rect(&mut self, l: f32, t: f32, r: f32, b: f32) {
        if let Some(rc) = Rect::from_ltrb(l.min(r), t.min(b), l.max(r), t.max(b)) {
            self.clip_path(&PathBuilder::from_rect(rc));
        }
    }

    // ---------- Zustand ----------

    pub fn save(&mut self) {
        self.stack.push(Saved::Matrix(self.m, self.clip.clone()));
    }

    /// Alles bis zum passenden restore() wird in eine eigene Ebene gezeichnet und mit alpha (0..255) eingeblendet.
    pub fn save_layer_alpha(&mut self, alpha: u8) {
        match Pixmap::new(self.base.width(), self.base.height()) {
            Some(pm) => {
                self.layers.push(pm);
                self.stack.push(Saved::Layer(self.m, alpha, self.clip.clone()));
            }
            None => self.stack.push(Saved::Matrix(self.m, self.clip.clone())),
        }
    }

    pub fn restore(&mut self) {
        match self.stack.pop() {
            Some(Saved::Matrix(m, clip)) => {
                self.m = m;
                self.clip = clip;
            }
            Some(Saved::Layer(m, alpha, clip)) => {
                self.m = m;
                self.clip = clip;
                if let Some(top) = self.layers.pop() {
                    let pp = PixmapPaint { opacity: alpha as f32 / 255.0, ..PixmapPaint::default() };
                    self.target().draw_pixmap(0, 0, top.as_ref(), &pp, Transform::identity(), None);
                }
            }
            None => {}
        }
    }

    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.m = self.m.pre_translate(dx, dy);
    }
    pub fn scale(&mut self, sx: f32, sy: f32) {
        self.m = self.m.pre_scale(sx, sy);
    }
    pub fn scale_at(&mut self, sx: f32, sy: f32, px: f32, py: f32) {
        self.translate(px, py);
        self.scale(sx, sy);
        self.translate(-px, -py);
    }
    /// Drehung in Grad (im Uhrzeigersinn auf dem Bildschirm) um (px, py).
    pub fn rotate(&mut self, deg: f32, px: f32, py: f32) {
        self.m = self.m.pre_concat(Transform::from_rotate_at(deg, px, py));
    }

    // ---------- Zeichnen ----------

    /// Fuellt die ganze Flaeche (ersetzt nicht, sondern blendet wie drawColor).
    pub fn draw_color(&mut self, c: u32) {
        let (w, h) = (self.base.width() as f32, self.base.height() as f32);
        if let Some(r) = Rect::from_xywh(0.0, 0.0, w, h) {
            let mut sp = SkPaint::default();
            sp.set_color(sk_color(c));
            let (t, clip) = self.parts();
            t.fill_rect(r, &sp, Transform::identity(), clip);
        }
    }

    pub fn draw_path(&mut self, path: &Path, p: &Paint) {
        let mut sp = SkPaint::default();
        sp.set_color(sk_color(p.color));
        sp.anti_alias = true;
        let m = self.m;
        match p.style {
            Style::Fill => {
                let (t, clip) = self.parts();
                t.fill_path(path, &sp, FillRule::Winding, m, clip)
            }
            Style::Stroke => {
                let cap = if p.cap == Cap::Round { LineCap::Round } else { LineCap::Butt };
                let mut st = Stroke { width: p.stroke_width.max(0.0), line_cap: cap, line_join: LineJoin::Round, ..Stroke::default() };
                if let Some((on, off)) = p.dash {
                    if on > 0.0 && off > 0.0 {
                        st.dash = StrokeDash::new(vec![on, off], 0.0);
                    }
                }
                let (t, clip) = self.parts();
                t.stroke_path(path, &sp, &st, m, clip);
            }
        }
    }

    pub fn draw_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, p: &Paint) {
        if x0 == x1 && y0 == y1 {
            if p.cap == Cap::Round {
                let mut q = p.clone();
                q.style = Style::Fill;
                self.draw_circle(x0, y0, p.stroke_width / 2.0, &q);
            }
            return;
        }
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
        if let Some(path) = pb.finish() {
            let mut q = p.clone();
            q.style = Style::Stroke;
            self.draw_path(&path, &q);
        }
    }

    pub fn draw_circle(&mut self, cx: f32, cy: f32, r: f32, p: &Paint) {
        if r.is_nan() || r <= 0.0 {
            return;
        }
        if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
            self.draw_path(&path, p);
        }
    }

    pub fn draw_oval(&mut self, l: f32, t: f32, r: f32, b: f32, p: &Paint) {
        if r - l <= 0.0 || b - t <= 0.0 {
            // entartete Ellipse: als Linie zeichnen, damit sie sichtbar bleibt
            if p.style == Style::Stroke {
                self.draw_line(l, t, r, b, p);
            }
            return;
        }
        if let Some(rect) = Rect::from_ltrb(l, t, r, b) {
            if let Some(path) = PathBuilder::from_oval(rect) {
                self.draw_path(&path, p);
            }
        }
    }

    pub fn draw_rect(&mut self, l: f32, t: f32, r: f32, b: f32, p: &Paint) {
        let (l, r) = if l <= r { (l, r) } else { (r, l) };
        let (t, b) = if t <= b { (t, b) } else { (b, t) };
        let mut pb = PathBuilder::new();
        pb.move_to(l, t);
        pb.line_to(r, t);
        pb.line_to(r, b);
        pb.line_to(l, b);
        pb.close();
        if let Some(path) = pb.finish() {
            self.draw_path(&path, p);
        }
    }

    pub fn draw_round_rect(&mut self, l: f32, t: f32, r: f32, b: f32, rad: f32, p: &Paint) {
        if let Some(path) = round_rect_path(l, t, r, b, rad) {
            self.draw_path(&path, p);
        }
    }

    /// Runde Punkte mit Durchmesser p.stroke_width (wie drawPoints mit Cap.ROUND).
    pub fn draw_points(&mut self, pts: &[f32], p: &Paint) {
        let r = p.stroke_width / 2.0;
        let mut pb = PathBuilder::new();
        for xy in pts.chunks_exact(2) {
            pb.push_circle(xy[0], xy[1], r);
        }
        if let Some(path) = pb.finish() {
            let mut q = p.clone();
            q.style = Style::Fill;
            self.draw_path(&path, &q);
        }
    }

    /// Zeichnet ein Bild in das Zielrechteck (bilinear gefiltert).
    pub fn draw_pixmap(&mut self, pm: PixmapRef, l: f32, t: f32, r: f32, b: f32) {
        if pm.width() == 0 || pm.height() == 0 {
            return;
        }
        let sx = (r - l) / pm.width() as f32;
        let sy = (b - t) / pm.height() as f32;
        if sx == 0.0 || sy == 0.0 || !sx.is_finite() || !sy.is_finite() {
            return;
        }
        let m = self.m.pre_translate(l, t).pre_scale(sx, sy);
        let pp = PixmapPaint { quality: FilterQuality::Bilinear, ..PixmapPaint::default() };
        let (t, clip) = self.parts();
        t.draw_pixmap(0, 0, pm, &pp, m, clip);
    }

    /// Text mit Grundlinie bei y (Schriftgroesse p.text_size, Farbe p.color).
    pub fn draw_text(&mut self, s: &str, x: f32, y: f32, p: &Paint) {
        let mut pb = PathBuilder::new();
        text::append_text(&mut pb, s, x, y, p.text_size);
        if let Some(path) = pb.finish() {
            let mut q = p.clone();
            q.style = Style::Fill;
            self.draw_path(&path, &q);
        }
    }
}

pub fn round_rect_path(l: f32, t: f32, r: f32, b: f32, rad: f32) -> Option<Path> {
    let (l, r) = if l <= r { (l, r) } else { (r, l) };
    let (t, b) = if t <= b { (t, b) } else { (b, t) };
    let rad = rad.max(0.0).min((r - l) / 2.0).min((b - t) / 2.0);
    let k = rad * 0.447_715_25; // 1 - 0.5522847
    let mut pb = PathBuilder::new();
    pb.move_to(l + rad, t);
    pb.line_to(r - rad, t);
    pb.cubic_to(r - k, t, r, t + k, r, t + rad);
    pb.line_to(r, b - rad);
    pb.cubic_to(r, b - k, r - k, b, r - rad, b);
    pb.line_to(l + rad, b);
    pb.cubic_to(l + k, b, l, b - k, l, b - rad);
    pb.line_to(l, t + rad);
    pb.cubic_to(l, t + k, l + k, t, l + rad, t);
    pb.close();
    pb.finish()
}

/// Kreisbogen wie android.graphics.Path.arcTo(oval, start, sweep): Winkel in Grad, 0 = rechts, im Uhrzeigersinn.
/// Verbindet mit einer Linie zum Bogenanfang, wenn der Pfad schon Punkte hat.
pub fn arc_to(pb: &mut PathBuilder, l: f32, t: f32, r: f32, b: f32, start_deg: f32, sweep_deg: f32) {
    let (cx, cy, rx, ry) = ((l + r) / 2.0, (t + b) / 2.0, (r - l) / 2.0, (b - t) / 2.0);
    let n = ((sweep_deg.abs() / 90.0).ceil() as i32).max(1);
    let step = (sweep_deg / n as f32).to_radians();
    let mut a = start_deg.to_radians();
    let (x0, y0) = (cx + rx * a.cos(), cy + ry * a.sin());
    if pb.is_empty() {
        pb.move_to(x0, y0);
    } else {
        pb.line_to(x0, y0);
    }
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    for _ in 0..n {
        let a1 = a + step;
        let (c0, s0, c1, s1) = (a.cos(), a.sin(), a1.cos(), a1.sin());
        pb.cubic_to(
            cx + rx * (c0 - k * s0),
            cy + ry * (s0 + k * c0),
            cx + rx * (c1 + k * s1),
            cy + ry * (s1 - k * c1),
            cx + rx * c1,
            cy + ry * s1,
        );
        a = a1;
    }
}
