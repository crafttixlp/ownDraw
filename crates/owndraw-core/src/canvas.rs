//! Zeichenflaeche auf tiny-skia mit einer an android.graphics.Canvas angelehnten API
//! (Transformations-Stapel, Paint, Ebenen mit Deckkraft).

use crate::text;
use std::rc::Rc;
use tiny_skia::{
    Color, FillRule, FilterQuality, LineCap, LineJoin, Mask, Paint as SkPaint, Path, PathBuilder,
    Pixmap, PixmapPaint, PixmapRef, Rect, Stroke, StrokeDash, Transform,
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
        Paint {
            color: 0xFF000000,
            style: Style::Fill,
            stroke_width: 1.0,
            cap: Cap::Butt,
            dash: None,
            text_size: 12.0,
        }
    }
}

impl Paint {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn stroke(color: u32, width: f32, cap: Cap) -> Self {
        Paint {
            color,
            style: Style::Stroke,
            stroke_width: width,
            cap,
            dash: None,
            text_size: 12.0,
        }
    }
    pub fn fill(color: u32) -> Self {
        Paint {
            color,
            ..Self::default()
        }
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
    /// Aufnahme-Modus: Befehle sammeln statt zeichnen (fuer paralleles Zeichnen in Streifen, siehe [`replay_parallel`]).
    rec: Option<Vec<Cmd>>,
    /// Kopierte Bilder der Aufnahme; Box, weil RawPix auf die (dann feste) Adresse zeigt.
    #[allow(clippy::vec_box)]
    rec_own: Vec<Box<Pixmap>>,
    rec_keep: Vec<Rc<Pixmap>>,
}

/// Aufgezeichneter Zeichenbefehl (Transformation absolut, Geraete-Pixel).
enum Cmd {
    Color(u32),
    Path(Path, Paint, Transform),
    Caps(Vec<[f32; 5]>, u32, Transform),
    Pix(RawPix, [f32; 4], Transform),
    Save,
    Layer(u8),
    Restore,
    Clip(Path, Transform),
}

/// Bild, das waehrend der parallelen Wiedergabe gelesen wird. Der Aufrufer von [`Canvas::draw_image`] haelt es
/// ueber `keep` am Leben, bis die Wiedergabe (scoped threads) beendet ist; niemand veraendert es in der Zeit.
struct RawPix(*const Pixmap);
unsafe impl Send for RawPix {}
unsafe impl Sync for RawPix {}

impl<'a> Canvas<'a> {
    pub fn new(base: &'a mut Pixmap) -> Self {
        Canvas {
            base,
            layers: Vec::new(),
            stack: Vec::new(),
            m: Transform::identity(),
            clip: None,
            rec: None,
            rec_own: Vec::new(),
            rec_keep: Vec::new(),
        }
    }

    /// Ab jetzt nur aufzeichnen (nichts zeichnen).
    pub fn start_recording(&mut self) {
        self.rec = Some(Vec::new());
    }

    fn record(&mut self, c: Cmd) -> bool {
        match self.rec.as_mut() {
            Some(v) => {
                v.push(c);
                true
            }
            None => false,
        }
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
    /// Aufloesungsfaktor der aktuellen Transformation (wie tiny-skia beim Strich-Berechnen).
    pub fn res_scale(&self) -> f32 {
        tiny_skia::PathStroker::compute_resolution_scale(&self.m)
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
        if self.record(Cmd::Clip(path.clone(), m)) {
            return;
        }
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
        self.record(Cmd::Save);
        self.stack.push(Saved::Matrix(self.m, self.clip.clone()));
    }

    /// Alles bis zum passenden restore() wird in eine eigene Ebene gezeichnet und mit alpha (0..255) eingeblendet.
    pub fn save_layer_alpha(&mut self, alpha: u8) {
        if self.record(Cmd::Layer(alpha)) {
            self.stack.push(Saved::Matrix(self.m, None));
            return;
        }
        match Pixmap::new(self.base.width(), self.base.height()) {
            Some(pm) => {
                self.layers.push(pm);
                self.stack
                    .push(Saved::Layer(self.m, alpha, self.clip.clone()));
            }
            None => self.stack.push(Saved::Matrix(self.m, self.clip.clone())),
        }
    }

    pub fn restore(&mut self) {
        self.record(Cmd::Restore);
        match self.stack.pop() {
            Some(Saved::Matrix(m, clip)) => {
                self.m = m;
                self.clip = clip;
            }
            Some(Saved::Layer(m, alpha, clip)) => {
                self.m = m;
                self.clip = clip;
                if let Some(top) = self.layers.pop() {
                    let pp = PixmapPaint {
                        opacity: alpha as f32 / 255.0,
                        ..PixmapPaint::default()
                    };
                    self.target()
                        .draw_pixmap(0, 0, top.as_ref(), &pp, Transform::identity(), None);
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
        if self.record(Cmd::Color(c)) {
            return;
        }
        let (w, h) = (self.base.width() as f32, self.base.height() as f32);
        if let Some(r) = Rect::from_xywh(0.0, 0.0, w, h) {
            let mut sp = SkPaint::default();
            sp.set_color(sk_color(c));
            let (t, clip) = self.parts();
            t.fill_rect(r, &sp, Transform::identity(), clip);
        }
    }

    pub fn draw_path(&mut self, path: &Path, p: &Paint) {
        if self.rec.is_some() {
            let m = self.m;
            self.record(Cmd::Path(path.clone(), p.clone(), m));
            return;
        }
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
                let cap = if p.cap == Cap::Round {
                    LineCap::Round
                } else {
                    LineCap::Butt
                };
                let mut st = Stroke {
                    width: p.stroke_width.max(0.0),
                    line_cap: cap,
                    line_join: LineJoin::Round,
                    ..Stroke::default()
                };
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
        if self.fill_capsules(&[[0.0; 5]; 0], p.color) {
            // schneller Weg: jeder Punkt als Kapsel der Laenge 0 (kleine Huelle je Aufruf)
            for xy in pts.chunks_exact(2) {
                self.fill_capsules(&[[xy[0], xy[1], xy[0], xy[1], r]], p.color);
            }
            return;
        }
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

    /// Freihand-Strich als Vereinigung von Kapseln `[x0, y0, x1, y1, r]` (lokale Koordinaten), Farbe color.
    /// Eigener Rasterweg mit Abstandsfeld-Kantenglaettung (Deckung je Pixel = Maximum ueber die Kapseln,
    /// d. h. ueberlappende Segmente decken nicht doppelt – wie ein Strich). Ein Vielfaches schneller als
    /// tiny-skia fuer viele kurze Segmente. Liefert false bei Drehung/Scherung (dann Pfad zeichnen).
    pub fn fill_capsules(&mut self, segs: &[[f32; 5]], color: u32) -> bool {
        let m = self.m;
        if m.kx != 0.0 || m.ky != 0.0 || !(m.sx > 0.0) || (m.sx - m.sy).abs() > m.sx * 1e-4 {
            return false;
        }
        if segs.is_empty() || color >> 24 == 0 {
            return true;
        }
        if self.rec.is_some() {
            self.record(Cmd::Caps(segs.to_vec(), color, m));
            return true;
        }
        let (s, ox, oy) = (m.sx, m.tx, m.ty);
        let (w, h) = (self.base.width() as i32, self.base.height() as i32);
        // Geraete-Huelle
        let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for g in segs {
            let rr = g[4] * s + 1.0;
            l = l.min(g[0].min(g[2]) * s + ox - rr);
            r = r.max(g[0].max(g[2]) * s + ox + rr);
            t = t.min(g[1].min(g[3]) * s + oy - rr);
            b = b.max(g[1].max(g[3]) * s + oy + rr);
        }
        if !(l.is_finite() && t.is_finite() && r.is_finite() && b.is_finite()) {
            return true;
        }
        let (x0, y0) = ((l.floor() as i32).max(0), (t.floor() as i32).max(0));
        let (x1, y1) = ((r.ceil() as i32).min(w), (b.ceil() as i32).min(h));
        if x0 >= x1 || y0 >= y1 {
            return true;
        }
        let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);
        COV.with(|cell| {
            let mut guard = cell.borrow_mut();
            let (cov, rows) = &mut *guard;
            // Invariante: der Puffer ist ausserhalb eines Aufrufs ueberall 0 (nur benutzte Bereiche werden geloescht).
            if cov.len() < bw * bh {
                cov.resize(bw * bh, 0);
            }
            rows.clear();
            rows.resize(bh, (u32::MAX, 0));
            for g in segs {
                let (ax, ay, bx, by) = (g[0] * s + ox, g[1] * s + oy, g[2] * s + ox, g[3] * s + oy);
                let rad = g[4] * s;
                if !(rad > 0.0) {
                    continue;
                }
                let re = rad + 0.5; // Reichweite der Deckung
                let (dx, dy) = (bx - ax, by - ay);
                let ll = dx * dx + dy * dy;
                let inv = if ll > 1e-12 { 1.0 / ll } else { 0.0 };
                let ry0 = ifloor(ay.min(by) - re).max(y0);
                let ry1 = iceil(ay.max(by) + re).min(y1);
                for py in ry0..ry1 {
                    let cy = py as f32 + 0.5;
                    // x-Bereich der Kapsel (konvex) in dieser Zeile: Huelle aus beiden Kreisen und dem Rechteck
                    let (xa, xb) = if ll > 16.0 * re * re {
                        match capsule_row(ax, ay, bx, by, re, cy) {
                            Some(v) => v,
                            None => continue,
                        }
                    } else {
                        (ax.min(bx) - re, ax.max(bx) + re)
                    };
                    let rx0 = ifloor(xa).max(x0);
                    let rx1 = iceil(xb).min(x1);
                    if rx0 >= rx1 {
                        continue;
                    }
                    let row = (py - y0) as usize;
                    let base = row * bw;
                    let ey = cy - ay;
                    let (full2, out2) = ((re - 1.0).max(0.0) * (re - 1.0).max(0.0), re * re);
                    for (px, o) in (rx0..rx1)
                        .zip(cov[base + (rx0 - x0) as usize..base + (rx1 - x0) as usize].iter_mut())
                    {
                        if *o == 255 {
                            continue;
                        }
                        let ex = px as f32 + 0.5 - ax;
                        let u = ((ex * dx + ey * dy) * inv).clamp(0.0, 1.0);
                        let (qx, qy) = (ex - u * dx, ey - u * dy);
                        let d2 = qx * qx + qy * qy;
                        let v = if d2 <= full2 {
                            255u8
                        } else if d2 >= out2 {
                            continue;
                        } else {
                            ((re - d2.sqrt()).min(1.0) * 255.0 + 0.5) as u8
                        };
                        if v > *o {
                            *o = v;
                        }
                    }
                    let rr = &mut rows[row];
                    rr.0 = rr.0.min((rx0 - x0) as u32);
                    rr.1 = rr.1.max((rx1 - x0) as u32);
                }
            }
            // Zusammensetzen (vormultipliziert, source-over), mit Clip-Maske
            let sa = color >> 24;
            let pm = |c: u32| div255(c * sa);
            let (sr, sg, sb) = (
                pm((color >> 16) & 255),
                pm((color >> 8) & 255),
                pm(color & 255),
            );
            let (tgt, clip) = self.parts();
            let tw = tgt.width() as usize;
            let mask = clip.map(|k| k.data());
            let data = tgt.data_mut();
            for (row, &(ra, rb)) in rows.iter().enumerate() {
                if ra >= rb {
                    continue;
                }
                let py = y0 as usize + row;
                for cx in ra as usize..rb as usize {
                    let mut k = cov[row * bw + cx] as u32;
                    if k == 0 {
                        continue;
                    }
                    let px = x0 as usize + cx;
                    let idx = py * tw + px;
                    if let Some(mk) = mask {
                        k = div255(k * mk[idx] as u32);
                        if k == 0 {
                            continue;
                        }
                    }
                    let ea = div255(sa * k);
                    let inv = 255 - ea;
                    let d = &mut data[idx * 4..idx * 4 + 4];
                    d[0] = (div255(sr * k) + div255(d[0] as u32 * inv)) as u8;
                    d[1] = (div255(sg * k) + div255(d[1] as u32 * inv)) as u8;
                    d[2] = (div255(sb * k) + div255(d[2] as u32 * inv)) as u8;
                    d[3] = (ea + div255(d[3] as u32 * inv)) as u8;
                }
                cov[row * bw + ra as usize..row * bw + rb as usize].fill(0);
            }
        });
        true
    }

    /// Zeichnet ein Bild in das Zielrechteck (bilinear gefiltert).
    /// Wie draw_pixmap; im Aufnahme-Modus wird das Bild nicht kopiert, sondern festgehalten (siehe take_recording).
    pub fn draw_image(&mut self, pm: &Rc<Pixmap>, l: f32, t: f32, r: f32, b: f32) {
        if self.rec.is_some() {
            self.rec_keep.push(pm.clone());
            let m = self.m;
            self.record(Cmd::Pix(RawPix(Rc::as_ptr(pm)), [l, t, r, b], m));
            return;
        }
        self.draw_pixmap(pm.as_ref().as_ref(), l, t, r, b);
    }

    pub fn draw_pixmap(&mut self, pm: PixmapRef, l: f32, t: f32, r: f32, b: f32) {
        if self.rec.is_some() {
            // selten (nur ueber draw_image ohne Rc): Kopie aufzeichnen
            let own = Box::new(pm.to_owned());
            let m = self.m;
            self.record(Cmd::Pix(RawPix(&*own as *const Pixmap), [l, t, r, b], m));
            self.rec_own.push(own);
            return;
        }
        if pm.width() == 0 || pm.height() == 0 {
            return;
        }
        let sx = (r - l) / pm.width() as f32;
        let sy = (b - t) / pm.height() as f32;
        if sx == 0.0 || sy == 0.0 || !sx.is_finite() || !sy.is_finite() {
            return;
        }
        let m = self.m.pre_translate(l, t).pre_scale(sx, sy);
        let pp = PixmapPaint {
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
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

impl<'a> Canvas<'a> {
    /// Beendet die Aufnahme. Die zurueckgegebenen Bilder muessen bis nach der Wiedergabe gehalten werden.
    pub fn take_recording(&mut self) -> (Recording, Vec<Rc<Pixmap>>) {
        (
            Recording(
                self.rec.take().unwrap_or_default(),
                std::mem::take(&mut self.rec_own),
            ),
            std::mem::take(&mut self.rec_keep),
        )
    }
}

/// Aufgezeichnete Befehle (siehe [`Canvas::start_recording`]).
#[allow(clippy::vec_box)]
pub struct Recording(Vec<Cmd>, #[allow(dead_code)] Vec<Box<Pixmap>>);

impl Recording {
    /// Spielt die Befehle in pm ab; pm liegt mit seiner Zeile 0 auf Zeile y0 des aufgezeichneten Ziels.
    fn replay(&self, pm: &mut Pixmap, y0: u32) {
        let off = Transform::from_translate(0.0, -(y0 as f32));
        let mut c = Canvas::new(pm);
        for cmd in &self.0 {
            match cmd {
                Cmd::Color(col) => c.draw_color(*col),
                Cmd::Path(p, paint, m) => {
                    c.m = off.pre_concat(*m);
                    c.draw_path(p, paint);
                }
                Cmd::Caps(segs, col, m) => {
                    c.m = off.pre_concat(*m);
                    c.fill_capsules(segs, *col);
                }
                Cmd::Pix(raw, [l, t, r, b], m) => {
                    c.m = off.pre_concat(*m);
                    // Sicherheit: siehe RawPix
                    let px: &Pixmap = unsafe { &*raw.0 };
                    c.draw_pixmap(px.as_ref(), *l, *t, *r, *b);
                }
                Cmd::Save => c.save(),
                Cmd::Layer(a) => c.save_layer_alpha(*a),
                Cmd::Restore => c.restore(),
                Cmd::Clip(p, m) => {
                    c.m = off.pre_concat(*m);
                    c.clip_path(p);
                }
            }
        }
    }

    /// Zeichnet die Aufnahme in pm, aufgeteilt in waagrechte Streifen auf mehrere Threads.
    pub fn replay_parallel(&self, pm: &mut Pixmap) {
        let (w, h) = (pm.width(), pm.height());
        let n = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(8) as u32;
        if n <= 1 || h < 64 {
            self.replay(pm, 0);
            return;
        }
        // mehr Streifen als Threads (Lastausgleich), Streifen reihum verteilt
        let bands = (n * 3).min(h / 16).max(1);
        let bh = h.div_ceil(bands);
        let row = w as usize * 4;
        let mut parts: Vec<(u32, &mut [u8])> = pm
            .data_mut()
            .chunks_mut(bh as usize * row)
            .enumerate()
            .map(|(i, d)| (i as u32 * bh, d))
            .collect();
        let mut groups: Vec<Vec<(u32, &mut [u8])>> = (0..n).map(|_| Vec::new()).collect();
        for (i, p) in parts.drain(..).enumerate() {
            groups[i % n as usize].push(p);
        }
        std::thread::scope(|sc| {
            for g in groups {
                sc.spawn(move || {
                    for (y0, d) in g {
                        let hh = (d.len() / row) as u32;
                        let Some(mut band) = Pixmap::new(w, hh) else {
                            continue;
                        };
                        band.data_mut().copy_from_slice(d);
                        self.replay(&mut band, y0);
                        d.copy_from_slice(band.data());
                    }
                });
            }
        });
    }
}

thread_local! {
    /// Deckungspuffer und Zeilenbereiche fuer fill_capsules (wiederverwendet, keine Allokation je Strich).
    static COV: std::cell::RefCell<(Vec<u8>, Vec<(u32, u32)>)> = const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
}

#[inline]
fn ifloor(x: f32) -> i32 {
    let i = x as i32;
    i - ((x < i as f32) as i32)
}
#[inline]
fn iceil(x: f32) -> i32 {
    let i = x as i32;
    i + ((x > i as f32) as i32)
}

#[inline]
fn div255(x: u32) -> u32 {
    (x + 128 + ((x + 128) >> 8)) >> 8
}

/// x-Bereich, in dem die Zeile y die Kapsel (Segment a–b, Radius re) schneidet.
#[inline]
fn capsule_row(ax: f32, ay: f32, bx: f32, by: f32, re: f32, y: f32) -> Option<(f32, f32)> {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for (cx, cy) in [(ax, ay), (bx, by)] {
        let dy = y - cy;
        let q = re * re - dy * dy;
        if q >= 0.0 {
            let k = q.sqrt();
            lo = lo.min(cx - k);
            hi = hi.max(cx + k);
        }
    }
    // Rechteck um das Segment: |n·(p-a)| <= re und 0 <= u·(p-a) <= L, mit p = (x, y)
    let (dx, dy) = (bx - ax, by - ay);
    let len = (dx * dx + dy * dy).sqrt();
    if len > 1e-6 {
        let (ux, uy) = (dx / len, dy / len);
        let (nx, ny) = (-uy, ux);
        let (mut a, mut b) = (f32::MIN, f32::MAX);
        let ey = y - ay;
        // lo_c <= kx * (x - ax) + ky * ey <= hi_c
        for (kx, ky, lc, hc) in [(nx, ny, -re, re), (ux, uy, 0.0, len)] {
            let c = ky * ey;
            if kx.abs() < 1e-9 {
                if c < lc || c > hc {
                    a = f32::MAX;
                    b = f32::MIN;
                }
                continue;
            }
            let (p, q) = ((lc - c) / kx, (hc - c) / kx);
            a = a.max(p.min(q));
            b = b.min(p.max(q));
        }
        if a <= b {
            lo = lo.min(ax + a);
            hi = hi.max(ax + b);
        }
    }
    (lo <= hi).then_some((lo, hi))
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
pub fn arc_to(
    pb: &mut PathBuilder,
    l: f32,
    t: f32,
    r: f32,
    b: f32,
    start_deg: f32,
    sweep_deg: f32,
) {
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
