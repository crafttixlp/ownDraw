//! Lineal und Geodreieck: Einblendung über dem Blatt (nur Ansicht, nie gespeichert, Zustand im Editor).
//!
//! Das Lineal liegt in Koordinaten der Zeichenfläche (Punkte relativ zu ihrer linken oberen Ecke) und bleibt beim
//! Verschieben/Zoomen der Ansicht an seiner Stelle auf dem Bildschirm. Bedienung:
//! - ein Finger bzw. die Maus auf dem Lineal verschiebt es, zwei Finger verschieben und drehen,
//!   der Drehgriff dreht mit einem Finger/der Maus; der Winkel rastet nach den Einrast-Einstellungen ein
//!   (`SnapCfg::handles`, Raster und Toleranz der Griffe),
//! - Striche von Stift, Marker und Linien-Formen, die höchstens [`SNAP_PX`] Bildschirmpunkte neben einer Kante
//!   beginnen, werden auf diese Kante projiziert (gerade Linie entlang der Kante, siehe [`Ruler::start_proj`]).
//!
//! Skalen in cm im A4-Maßstab des Boards (`item::MM_PER_UNIT`), passend zur aktuellen Zoomstufe.

use super::{Editor, Shared};
use crate::layout::min_touch;
use crate::style::{self, c};
use egui::{Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Shape, Stroke, Ui, Vec2};
use owndraw_core::item::MM_PER_UNIT;
use owndraw_core::recognizer::SnapCfg;

/// Fangabstand zur Kante (Bildschirmpunkte).
pub const SNAP_PX: f32 = 20.0;
/// Innerhalb des Lineals so nah an der Kante: zeichnen statt verschieben.
const INNER_PX: f32 = 10.0;
/// Breite des Lineals.
const THICK: f32 = 64.0;
/// Radius des Drehgriffs (Trefferfläche etwas größer).
const HANDLE_R: f32 = 12.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Ruler,
    Triangle,
}

/// Was unter einem Punkt liegt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    None,
    /// Kante `i` in Fangweite; `inside` = Punkt liegt auf dem Lineal.
    Edge(usize, bool),
    Body,
    Handle,
}

/// Laufende Projektion eines Strichs auf eine Kante.
#[derive(Clone, Copy, Debug)]
pub struct Proj {
    pub id: u64,
    pub edge: usize,
    pub start: Pos2,
    pub cur: Pos2,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    Move { id: u64, last: Pos2 },
    Rotate { id: u64, off: f32 },
    Two { a: (u64, Pos2), b: (u64, Pos2) },
}

#[derive(Clone, Debug)]
pub struct Ruler {
    pub visible: bool,
    pub kind: Kind,
    /// Bezugspunkt: Mitte des Lineals bzw. Nullpunkt (Mitte der langen Kante) des Geodreiecks.
    pub c: Pos2,
    /// Ungerasteter Winkel (Grad, Bildschirm: positiv im Uhrzeigersinn).
    pub raw: f32,
    /// Angezeigter (eingerasteter) Winkel.
    pub ang: f32,
    /// Länge des Lineals bzw. der langen Kante des Geodreiecks.
    pub len: f32,
    placed: bool,
    drag: Option<Drag>,
    pub proj: Option<Proj>,
}

impl Default for Ruler {
    fn default() -> Self {
        Ruler { visible: false, kind: Kind::Ruler, c: Pos2::new(300.0, 300.0), raw: 0.0, ang: 0.0, len: 600.0, placed: false, drag: None, proj: None }
    }
}

/// Punkt `p` auf die Strecke a–b projiziert (auf die Strecke begrenzt).
pub fn project(a: Pos2, b: Pos2, p: Pos2) -> Pos2 {
    let d = b - a;
    let l2 = d.length_sq();
    if l2 < 1e-9 {
        return a;
    }
    let t = ((p - a).dot(d) / l2).clamp(0.0, 1.0);
    a + d * t
}

fn norm_deg(d: f32) -> f32 {
    let r = d.rem_euclid(360.0);
    if r > 180.0 {
        r - 360.0
    } else {
        r
    }
}

/// Winkel nach den Einrast-Einstellungen (Raster/Toleranz der Griffe).
pub fn snap_angle(raw: f32, cfg: &SnapCfg) -> f32 {
    let raw = norm_deg(raw);
    if cfg.handles {
        if let Some(s) = SnapCfg::snap_deg(raw, cfg.handle_step, cfg.handle_tol) {
            return norm_deg(s);
        }
    }
    raw
}

/// Bildschirmlänge (Punkte) bei Zoom `sc` in cm (A4-Maßstab).
pub fn screen_cm(len: f32, sc: f32) -> f32 {
    len / sc.max(1e-6) * MM_PER_UNIT / 10.0
}

/// „12,3 cm“.
pub fn cm_label(cm: f32) -> String {
    format!("{cm:.1} cm").replace('.', ",")
}

impl Ruler {
    /// Lage beim Einblenden: Größe nach der Zeichenfläche, beim ersten Mal (oder wenn es außerhalb liegt) in die Mitte.
    pub fn place(&mut self, w: f32, h: f32) {
        self.len = match self.kind {
            Kind::Ruler => (w * 0.8).clamp(260.0, 1000.0),
            Kind::Triangle => (w.min(h) * 0.55).clamp(220.0, 520.0),
        };
        if !self.placed || self.c.x < 0.0 || self.c.y < 0.0 || self.c.x > w || self.c.y > h {
            self.c = Pos2::new(w / 2.0, h / 2.0);
            if self.kind == Kind::Triangle {
                self.c.y -= self.len * 0.2;
            }
            self.placed = true;
        }
        self.drag = None;
        self.proj = None;
    }

    pub fn u(&self) -> Vec2 {
        let a = self.ang.to_radians();
        Vec2::new(a.cos(), a.sin())
    }

    pub fn v(&self) -> Vec2 {
        let u = self.u();
        Vec2::new(-u.y, u.x)
    }

    /// Umriss (konvex, im Uhrzeigersinn auf dem Bildschirm).
    pub fn outline(&self) -> Vec<Pos2> {
        let (u, v, h) = (self.u(), self.v(), self.len / 2.0);
        match self.kind {
            Kind::Ruler => {
                let t = THICK / 2.0;
                vec![self.c - u * h - v * t, self.c + u * h - v * t, self.c + u * h + v * t, self.c - u * h + v * t]
            }
            Kind::Triangle => vec![self.c - u * h, self.c + u * h, self.c + v * h],
        }
    }

    /// Kanten zum Zeichnen: Lineal die beiden langen (0 = Skala oben), Geodreieck alle drei (0 = lange Kante).
    pub fn edges(&self) -> Vec<(Pos2, Pos2)> {
        let o = self.outline();
        match self.kind {
            Kind::Ruler => vec![(o[0], o[1]), (o[3], o[2])],
            Kind::Triangle => vec![(o[0], o[1]), (o[1], o[2]), (o[2], o[0])],
        }
    }

    pub fn contains(&self, p: Pos2) -> bool {
        let o = self.outline();
        let n = o.len();
        let mut sign = 0.0f32;
        for i in 0..n {
            let (a, b) = (o[i], o[(i + 1) % n]);
            let cr = (b - a).x * (p - a).y - (b - a).y * (p - a).x;
            if cr.abs() < 1e-6 {
                continue;
            }
            if sign == 0.0 {
                sign = cr.signum();
            } else if cr.signum() != sign {
                return false;
            }
        }
        true
    }

    pub fn handle_pos(&self) -> Pos2 {
        match self.kind {
            Kind::Ruler => self.c + self.u() * (self.len / 2.0 - 32.0),
            Kind::Triangle => self.c + self.v() * (self.len / 2.0 - 36.0),
        }
    }

    /// Nächste Kante mit Abstand (Bildschirmpunkte).
    pub fn nearest_edge(&self, p: Pos2) -> Option<(usize, f32)> {
        self.edges().iter().enumerate().map(|(i, (a, b))| (i, project(*a, *b, p).distance(p))).min_by(|x, y| x.1.total_cmp(&y.1))
    }

    pub fn hit(&self, p: Pos2) -> Hit {
        if !self.visible {
            return Hit::None;
        }
        if p.distance(self.handle_pos()) <= HANDLE_R + 6.0 {
            return Hit::Handle;
        }
        let inside = self.contains(p);
        match self.nearest_edge(p) {
            Some((i, d)) if d <= SNAP_PX && (!inside || d <= INNER_PX) => Hit::Edge(i, inside),
            _ if inside => Hit::Body,
            _ => Hit::None,
        }
    }

    // ------------------------------------------------------------ Projektion

    /// Beginnt ein Strich höchstens [`SNAP_PX`] neben einer Kante, wird er auf sie projiziert: liefert den
    /// projizierten Startpunkt.
    pub fn start_proj(&mut self, id: u64, p: Pos2) -> Option<Pos2> {
        if !self.visible {
            return None;
        }
        let (edge, d) = self.nearest_edge(p)?;
        if d > SNAP_PX {
            return None;
        }
        let (a, b) = self.edges()[edge];
        let q = project(a, b, p);
        self.proj = Some(Proj { id, edge, start: q, cur: q });
        Some(q)
    }

    /// Weiterer Punkt eines projizierten Strichs (None: Zeiger gehört zu keinem).
    pub fn proj_point(&mut self, id: u64, p: Pos2) -> Option<Pos2> {
        let pr = self.proj.filter(|x| x.id == id)?;
        let (a, b) = *self.edges().get(pr.edge)?;
        let q = project(a, b, p);
        self.proj = Some(Proj { cur: q, ..pr });
        Some(q)
    }

    pub fn end_proj(&mut self, id: u64) {
        if self.proj.is_some_and(|x| x.id == id) {
            self.proj = None;
        }
    }

    // ------------------------------------------------------------ Verschieben und Drehen

    pub fn owns(&self, id: u64) -> bool {
        match self.drag {
            Some(Drag::Move { id: i, .. } | Drag::Rotate { id: i, .. }) => i == id,
            Some(Drag::Two { a, b }) => a.0 == id || b.0 == id,
            None => false,
        }
    }

    /// Greift das Lineal (Körper: verschieben, Griff: drehen).
    pub fn grab(&mut self, id: u64, p: Pos2, hit: Hit) {
        self.drag = Some(match hit {
            Hit::Handle => {
                let d = p - self.c;
                let base = if self.kind == Kind::Ruler { 0.0 } else { 90.0 };
                Drag::Rotate { id, off: d.y.atan2(d.x).to_degrees() - base - self.raw }
            }
            _ => Drag::Move { id, last: p },
        });
        self.raw = self.ang;
    }

    /// Zweiter Finger: mit beiden verschieben und drehen. false, wenn das Lineal nicht gegriffen ist.
    pub fn add_finger(&mut self, id: u64, p: Pos2) -> bool {
        match self.drag {
            Some(Drag::Move { id: i, last }) => {
                self.drag = Some(Drag::Two { a: (i, last), b: (id, p) });
                true
            }
            _ => false,
        }
    }

    pub fn moved(&mut self, id: u64, p: Pos2, cfg: &SnapCfg) {
        match self.drag {
            Some(Drag::Move { id: i, last }) if i == id => {
                self.c += p - last;
                self.drag = Some(Drag::Move { id, last: p });
            }
            Some(Drag::Rotate { id: i, off }) if i == id => {
                let d = p - self.c;
                if d.length() > 4.0 {
                    let base = if self.kind == Kind::Ruler { 0.0 } else { 90.0 };
                    self.raw = norm_deg(d.y.atan2(d.x).to_degrees() - base - off);
                    self.ang = snap_angle(self.raw, cfg);
                }
            }
            Some(Drag::Two { a, b }) if a.0 == id || b.0 == id => {
                let (na, nb) = if a.0 == id { ((id, p), b) } else { (a, (id, p)) };
                let (old, new) = (b.1 - a.1, nb.1 - na.1);
                let (m0, m1) = (a.1 + (b.1 - a.1) / 2.0, na.1 + (nb.1 - na.1) / 2.0);
                let da = if old.length() > 2.0 && new.length() > 2.0 { new.y.atan2(new.x) - old.y.atan2(old.x) } else { 0.0 };
                // Bezugspunkt mit den Fingern drehen und verschieben
                let r = self.c - m0;
                let (s, co) = da.sin_cos();
                self.c = m1 + Vec2::new(r.x * co - r.y * s, r.x * s + r.y * co);
                self.raw = norm_deg(self.raw + da.to_degrees());
                self.ang = snap_angle(self.raw, cfg);
                self.drag = Some(Drag::Two { a: na, b: nb });
            }
            _ => {}
        }
    }

    /// Finger/Maus losgelassen; mit zwei Fingern bleibt der andere zum Verschieben.
    pub fn released(&mut self, id: u64) {
        self.drag = match self.drag {
            Some(Drag::Two { a, b }) if a.0 == id => Some(Drag::Move { id: b.0, last: b.1 }),
            Some(Drag::Two { a, b }) if b.0 == id => Some(Drag::Move { id: a.0, last: a.1 }),
            d if self.owns(id) => {
                let _ = d;
                None
            }
            d => d,
        };
    }

    // ------------------------------------------------------------ Anzeige

    /// Angezeigter Winkel (mathematisch, gegen den Uhrzeigersinn): Lineal 0–179°, Geodreieck 0–359°.
    pub fn shown_deg(&self) -> f32 {
        let d = (-self.ang).rem_euclid(if self.kind == Kind::Ruler { 180.0 } else { 360.0 });
        if d.round() >= if self.kind == Kind::Ruler { 180.0 } else { 360.0 } {
            0.0
        } else {
            d + 0.0 // -0.0 → 0.0 (sonst Anzeige „-0°“)
        }
    }

    /// Zeichnet das Lineal; `o` = linke obere Ecke der Zeichenfläche, `sc` = Zoom des Boards.
    pub fn paint(&self, pt: &Painter, o: Pos2, sc: f32) {
        let pal = style::pal();
        let at = |p: Pos2| o + p.to_vec2();
        let (fill, ink) = if pal.dark { (Color32::from_rgba_unmultiplied(40, 44, 52, 205), Color32::from_rgb(225, 228, 235)) } else { (Color32::from_rgba_unmultiplied(250, 250, 246, 215), Color32::from_rgb(40, 42, 48)) };
        let accent = c(pal.accent);
        let outline: Vec<Pos2> = self.outline().into_iter().map(at).collect();
        pt.add(Shape::convex_polygon(outline, fill, Stroke::new(1.2, ink.gamma_multiply(0.6))));
        let (u, v) = (self.u(), self.v());
        let edges = self.edges();
        if let Some(pr) = self.proj {
            if let Some((a, b)) = edges.get(pr.edge) {
                pt.line_segment([at(*a), at(*b)], Stroke::new(2.5, accent));
            }
        }
        let mm = sc / MM_PER_UNIT; // Bildschirmpunkte je mm
        let thin = Stroke::new(1.0, ink);
        let font = FontId::proportional(10.0);
        // Skala entlang einer Kante: Nullpunkt `z`, Richtung `dir`, Ticks nach `inn`, bis Länge `max`
        let scale = |z: Pos2, dir: Vec2, inn: Vec2, max: f32, labels: bool| {
            let (step, every) = if mm >= 3.0 {
                (1, 10)
            } else if mm * 10.0 >= 6.0 {
                (10, 10)
            } else if mm * 50.0 >= 6.0 {
                (50, 50)
            } else {
                (100, 100)
            };
            let lab_every = if mm * 10.0 >= 22.0 { 10 } else if mm * 50.0 >= 22.0 { 50 } else { 100 };
            let n = (max / (mm * step as f32)) as i32;
            for k in 0..=n.min(2000) {
                let m = k * step;
                let s = m as f32 * mm;
                let l = if m % every == 0 { if m % 10 == 0 { 14.0 } else { 8.0 } } else if m % 5 == 0 { 9.0 } else { 5.0 };
                let p = z + dir * s;
                pt.line_segment([at(p), at(p + inn * l)], thin);
                // Zahlen unter dem Drehgriff weglassen (sonst verdeckt)
                if labels && m % lab_every == 0 && (p + inn * 24.0).distance(self.handle_pos()) > HANDLE_R + 12.0 {
                    pt.text(at(p + inn * 24.0), Align2::CENTER_CENTER, format!("{}", m / 10), font.clone(), ink);
                }
            }
        };
        let h = self.len / 2.0;
        match self.kind {
            Kind::Ruler => {
                let (a, b) = edges[0];
                scale(a + u * 10.0, u, v, (b - a).length() - 20.0, true);
                let (a2, b2) = edges[1];
                scale(a2 + u * 10.0, u, -v, (b2 - a2).length() - 20.0, false);
            }
            Kind::Triangle => {
                scale(self.c, u, v, h - 10.0, true);
                scale(self.c, -u, v, h - 10.0, true);
                // Winkelskala (Halbkreis um den Nullpunkt)
                let r = h * 0.62;
                for d in (0..=180).step_by(5) {
                    let a = (d as f32).to_radians();
                    let dir = -u * a.cos() + v * a.sin();
                    let l = if d % 10 == 0 { 9.0 } else { 5.0 };
                    pt.line_segment([at(self.c + dir * r), at(self.c + dir * (r - l))], thin);
                    if d % 30 == 0 && d != 0 && d != 180 {
                        pt.text(at(self.c + dir * (r - 18.0)), Align2::CENTER_CENTER, format!("{d}"), font.clone(), ink);
                    }
                }
                pt.line_segment([at(self.c), at(self.c + v * (h * 0.5))], Stroke::new(1.0, ink.gamma_multiply(0.5)));
                pt.circle_stroke(at(self.c), 3.0, thin);
            }
        }
        // Drehgriff
        let hp = at(self.handle_pos());
        pt.circle_filled(hp, HANDLE_R, accent);
        let pts: Vec<Pos2> = (0..=12).map(|i| {
            let a = (40.0 + i as f32 * 22.5f32).to_radians();
            hp + Vec2::new(a.cos(), a.sin()) * 6.0
        }).collect();
        let end = *pts.last().unwrap_or(&hp);
        pt.add(Shape::line(pts, Stroke::new(1.6, Color32::WHITE)));
        pt.circle_filled(end, 1.8, Color32::WHITE);
        // Winkel
        let lp = match self.kind {
            Kind::Ruler => self.c + v * 4.0,
            Kind::Triangle => self.c + v * (h * 0.3),
        };
        pill(pt, at(lp), &format!("{:.0}°", self.shown_deg()), ink, fill);
        // gezeichnete Länge
        if let Some(pr) = self.proj {
            let l = pr.cur.distance(pr.start);
            pill(pt, at(pr.cur) - v * 26.0, &cm_label(screen_cm(l, sc)), Color32::WHITE, accent);
        }
    }
}

fn pill(pt: &Painter, p: Pos2, s: &str, fg: Color32, bg: Color32) {
    let g = pt.layout_no_wrap(s.to_string(), FontId::proportional(12.0), fg);
    let r = Rect::from_center_size(p, g.size() + Vec2::new(12.0, 4.0));
    pt.rect_filled(r, CornerRadius::same(8), bg);
    pt.galley(r.min + Vec2::new(6.0, 2.0), g, fg);
}

/// Inhalt des Popovers „Lineal“.
pub(super) fn options(ui: &mut Ui, ed: &mut Editor, sh: &mut Shared) {
    let h = min_touch(sh.env.touch).max(34.0);
    style::caption(ui, "Art");
    ui.horizontal_wrapped(|ui| {
        for (k, l) in [(Kind::Ruler, "Lineal"), (Kind::Triangle, "Geodreieck")] {
            if style::chip(ui, l, ed.ext.ruler.kind == k, h).clicked() && ed.ext.ruler.kind != k {
                ed.ext.ruler.kind = k;
                ed.ext.ruler.place(ed.canvas.width(), ed.canvas.height());
            }
        }
    });
    let hint = "Verschieben: auf dem Lineal ziehen. Drehen: mit zwei Fingern oder am runden Griff. Striche an der Kante werden gerade.";
    ui.label(egui::RichText::new(hint).size(12.0).color(c(style::pal().dim)));
    ui.add_space(4.0);
    if style::plain_btn(ui, "Ausblenden", h).clicked() {
        ed.ext.ruler.visible = false;
        ed.st.opts_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(kind: Kind, ang: f32) -> Ruler {
        let mut r = Ruler { visible: true, kind, ..Ruler::default() };
        r.place(800.0, 600.0);
        r.c = Pos2::new(400.0, 300.0);
        r.ang = ang;
        r.raw = ang;
        r
    }

    fn near(a: Pos2, b: Pos2) -> bool {
        a.distance(b) < 1e-3
    }

    #[test]
    fn projektion_auf_strecke() {
        let (a, b) = (Pos2::new(0.0, 0.0), Pos2::new(100.0, 0.0));
        assert!(near(project(a, b, Pos2::new(30.0, 12.0)), Pos2::new(30.0, 0.0)));
        assert!(near(project(a, b, Pos2::new(-30.0, 5.0)), a), "auf die Strecke begrenzt");
        assert!(near(project(a, b, Pos2::new(130.0, -5.0)), b));
        assert!(near(project(a, a, Pos2::new(5.0, 5.0)), a), "entartete Strecke");
        // schräg
        let q = project(Pos2::new(0.0, 0.0), Pos2::new(100.0, 100.0), Pos2::new(100.0, 0.0));
        assert!(near(q, Pos2::new(50.0, 50.0)));
    }

    #[test]
    fn strich_an_der_kante_wird_gerade() {
        let mut r = at(Kind::Ruler, 0.0);
        let top = 300.0 - THICK / 2.0;
        // knapp über der oberen Kante beginnen
        let s = r.start_proj(7, Pos2::new(380.0, top - 12.0)).expect("in Fangweite");
        assert!(near(s, Pos2::new(380.0, top)));
        // wackelige Bewegung bleibt auf der Kante
        for (x, y) in [(400.0, top - 30.0), (450.0, top + 5.0), (520.0, top - 60.0)] {
            let q = r.proj_point(7, Pos2::new(x, y)).unwrap();
            assert!((q.y - top).abs() < 1e-3 && near(q, Pos2::new(x, top)));
        }
        assert!(r.proj_point(8, Pos2::new(0.0, 0.0)).is_none(), "anderer Zeiger");
        r.end_proj(7);
        assert!(r.proj.is_none());
        // zu weit weg: keine Projektion
        assert!(r.start_proj(1, Pos2::new(380.0, top - SNAP_PX - 1.0)).is_none());
        // untere Kante
        let s = r.start_proj(2, Pos2::new(300.0, 300.0 + THICK / 2.0 + 5.0)).unwrap();
        assert_eq!(r.proj.unwrap().edge, 1);
        assert!(near(s, Pos2::new(300.0, 300.0 + THICK / 2.0)));
    }

    #[test]
    fn gedrehtes_lineal() {
        let mut r = at(Kind::Ruler, 30.0);
        let (u, v) = (r.u(), r.v());
        let on_edge = r.c - v * (THICK / 2.0) + u * 50.0;
        let s = r.start_proj(1, on_edge - v * 10.0 + Vec2::new(0.0, 0.0)).unwrap();
        assert!(near(s, on_edge));
        let q = r.proj_point(1, on_edge + u * 80.0 - v * 15.0).unwrap();
        assert!(near(q, on_edge + u * 80.0));
        // Richtung der gezeichneten Linie = Winkel des Lineals
        let d = q - s;
        assert!((d.y.atan2(d.x).to_degrees() - 30.0).abs() < 0.01);
    }

    #[test]
    fn geodreieck_alle_kanten() {
        let mut r = at(Kind::Triangle, 0.0);
        let h = r.len / 2.0;
        let e = r.edges();
        assert_eq!(e.len(), 3);
        // lange Kante waagerecht durch den Nullpunkt, Spitze mit 90° darunter
        assert!(near(e[0].0, Pos2::new(400.0 - h, 300.0)) && near(e[0].1, Pos2::new(400.0 + h, 300.0)));
        let apex = Pos2::new(400.0, 300.0 + h);
        assert!(near(e[1].1, apex));
        let (l1, l2) = (e[1].0 - e[1].1, e[2].1 - e[2].0);
        assert!(l1.dot(l2).abs() < 1e-2, "rechter Winkel an der Spitze");
        // 45°-Kante: Strich daneben wird auf sie projiziert
        let mid = e[1].0 + (e[1].1 - e[1].0) * 0.5;
        let out = mid + Vec2::new(1.0, 1.0).normalized() * 10.0;
        let s = r.start_proj(3, out).unwrap();
        assert_eq!(r.proj.unwrap().edge, 1);
        assert!(near(s, mid));
        let q = r.proj_point(3, out + Vec2::new(-40.0, 40.0)).unwrap();
        let d = q - s;
        assert!((d.y.atan2(d.x).to_degrees() - 135.0).abs() < 0.01);
    }

    #[test]
    fn treffer_koerper_kante_griff() {
        let r = at(Kind::Ruler, 0.0);
        assert_eq!(r.hit(Pos2::new(400.0, 300.0)), Hit::Body);
        assert_eq!(r.hit(Pos2::new(400.0, 300.0 - THICK / 2.0 + 4.0)), Hit::Edge(0, true));
        assert_eq!(r.hit(Pos2::new(400.0, 300.0 - THICK / 2.0 - 15.0)), Hit::Edge(0, false));
        assert_eq!(r.hit(Pos2::new(400.0, 100.0)), Hit::None);
        assert_eq!(r.hit(r.handle_pos()), Hit::Handle);
        let hidden = Ruler { visible: false, ..r.clone() };
        assert_eq!(hidden.hit(Pos2::new(400.0, 300.0)), Hit::None);
        let t = at(Kind::Triangle, 0.0);
        assert_eq!(t.hit(Pos2::new(400.0, 300.0 + t.len * 0.25)), Hit::Body);
        assert_eq!(t.hit(Pos2::new(400.0, 280.0)), Hit::Edge(0, false));
    }

    #[test]
    fn verschieben_und_drehen() {
        let cfg = SnapCfg::default();
        let mut r = at(Kind::Ruler, 0.0);
        r.grab(1, Pos2::new(400.0, 300.0), Hit::Body);
        r.moved(1, Pos2::new(410.0, 320.0), &cfg);
        assert!(near(r.c, Pos2::new(410.0, 320.0)));
        // zweiter Finger: Drehung um 90° (rastet ein)
        assert!(r.add_finger(2, Pos2::new(510.0, 320.0)));
        r.moved(2, Pos2::new(410.0, 420.0), &cfg);
        assert!((r.ang - 90.0).abs() < 1e-3, "{}", r.ang);
        r.released(2);
        assert!(r.owns(1) && !r.owns(2));
        r.released(1);
        assert!(!r.owns(1));
        // Drehgriff: 47° rastet bei Raster 15°/Toleranz 4° auf 45° ein, 52° bleibt frei
        let mut r = at(Kind::Ruler, 0.0);
        let hp = r.handle_pos();
        r.grab(5, hp, Hit::Handle);
        let d = hp.distance(r.c);
        let rc = r.c;
        let to = |deg: f32| rc + Vec2::new(deg.to_radians().cos(), deg.to_radians().sin()) * d;
        let p47 = to(47.0);
        r.moved(5, p47, &cfg);
        assert!((r.ang - 45.0).abs() < 1e-3, "{}", r.ang);
        let p52 = to(52.0);
        r.moved(5, p52, &cfg);
        assert!((r.ang - 52.0).abs() < 1e-2, "{}", r.ang);
        let off = SnapCfg { handles: false, ..cfg };
        r.moved(5, p47, &off);
        assert!((r.ang - 47.0).abs() < 1e-2, "Einrasten aus");
    }

    #[test]
    fn winkel_und_laenge() {
        let mut r = at(Kind::Ruler, -30.0);
        assert!((r.shown_deg() - 30.0).abs() < 1e-3);
        r.ang = 30.0;
        assert!((r.shown_deg() - 150.0).abs() < 1e-3);
        r.ang = 0.0;
        assert_eq!(r.shown_deg(), 0.0);
        assert_eq!(snap_angle(181.0, &SnapCfg::default()), 180.0);
        // 1 cm = 10 / MM_PER_UNIT Welteinheiten; bei Zoom 2 doppelt so viele Bildschirmpunkte
        let one_cm = 10.0 / MM_PER_UNIT;
        assert!((screen_cm(one_cm * 2.0, 2.0) - 1.0).abs() < 1e-4);
        assert_eq!(cm_label(12.34), "12,3 cm");
    }
}
