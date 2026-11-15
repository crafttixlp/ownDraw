//! Board-Objekte: Freihand-Strich, geometrische Form, Notiz/Text und Bild. Koordinaten sind Welt-Koordinaten.

use crate::canvas::{Canvas, Cap, Paint, Style};
use crate::geom::{hypot, RectF};
use crate::text::TextLayout;
use serde_json::{json, Map, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use tiny_skia::{Path, PathBuilder, Pixmap};

pub const INK: u32 = 0xFF111111;

/// Tinte wird im Dunkelmodus hell dargestellt.
pub fn ink(c: u32, dark: bool) -> u32 {
    if dark && c == INK {
        0xFFF1F1F4
    } else {
        c
    }
}

thread_local! {
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static IMG_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// Bilder im Speicher (Name → Bytes), falls es keinen Bildordner gibt (Browser). Vorrang vor dem Ordner.
    static IMG_MEM: RefCell<HashMap<String, Rc<Vec<u8>>>> = RefCell::new(HashMap::new());
    static IMG_CACHE: RefCell<(HashMap<String, Option<Rc<Pixmap>>>, Vec<String>)> = RefCell::new((HashMap::new(), Vec::new()));
}

const MAX_IMG_CACHE: usize = 12;

fn next_id() -> u64 {
    NEXT_ID.with(|c| {
        let v = c.get();
        c.set(v + 1);
        v
    })
}

/// Ordner, aus dem Bild-Objekte ihre Dateien laden.
pub fn set_img_dir(dir: PathBuf) {
    IMG_DIR.with(|d| *d.borrow_mut() = Some(dir));
    IMG_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.0.clear();
        c.1.clear();
    });
}

/// Legt ein Bild im Speicher ab (Browser ohne Dateisystem); `load_image` nimmt es vor dem Bildordner.
pub fn put_mem_image(name: &str, bytes: Vec<u8>) {
    IMG_MEM.with(|m| m.borrow_mut().insert(name.to_string(), Rc::new(bytes)));
    forget_image(name);
}

/// Leert die Bilder im Speicher (beim Öffnen eines anderen Dokuments).
pub fn clear_mem_images() {
    IMG_MEM.with(|m| m.borrow_mut().clear());
}

pub fn img_dir() -> Option<PathBuf> {
    IMG_DIR.with(|d| d.borrow().clone())
}

/// RGBA (nicht vormultipliziert) nach Pixmap.
pub fn pixmap_from_rgba(w: u32, h: u32, mut data: Vec<u8>) -> Option<Pixmap> {
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a != 255 {
            px[0] = ((px[0] as u32 * a + 127) / 255) as u8;
            px[1] = ((px[1] as u32 * a + 127) / 255) as u8;
            px[2] = ((px[2] as u32 * a + 127) / 255) as u8;
        }
    }
    Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h)?)
}

fn load_image(name: &str) -> Option<Rc<Pixmap>> {
    if name.is_empty() {
        return None;
    }
    if let Some(hit) = IMG_CACHE.with(|c| c.borrow().0.get(name).cloned()) {
        return hit;
    }
    let mem = IMG_MEM.with(|m| m.borrow().get(name).cloned());
    let pm = if let Some(b) = mem {
        image::load_from_memory(&b).ok().and_then(|img| {
            let img = img.into_rgba8();
            let (w, h) = (img.width(), img.height());
            pixmap_from_rgba(w, h, img.into_raw()).map(Rc::new)
        })
    } else {
        img_dir().and_then(|d| {
        let img = image::open(d.join(name)).ok()?.into_rgba8();
        let (w, h) = (img.width(), img.height());
        pixmap_from_rgba(w, h, img.into_raw()).map(Rc::new)
        })
    };
    IMG_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.1.len() >= MAX_IMG_CACHE {
            let old = c.1.remove(0);
            c.0.remove(&old);
        }
        c.1.push(name.to_string());
        c.0.insert(name.to_string(), pm.clone());
    });
    pm
}

/// Vergisst ein (z. B. gerade neu geschriebenes) Bild im Zwischenspeicher.
pub fn forget_image(name: &str) {
    IMG_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.0.remove(name);
        c.1.retain(|n| n != name);
    });
}

// ---------------------------------------------------------------- Geometrie-Helfer

/// Punkt (x, y) um (px, py) um rad drehen (positiv = im Uhrzeigersinn auf dem Bildschirm).
pub fn rot_pt(x: f32, y: f32, px: f32, py: f32, rad: f32) -> (f32, f32) {
    let (cs, sn) = (rad.cos(), rad.sin());
    let (dx, dy) = (x - px, y - py);
    (px + dx * cs - dy * sn, py + dx * sn + dy * cs)
}

/// Winkel auf (-pi, pi] normalisieren.
pub fn norm_ang(a: f64) -> f32 {
    let t = 2.0 * std::f64::consts::PI;
    let mut a = a % t;
    if a > std::f64::consts::PI {
        a -= t;
    }
    if a <= -std::f64::consts::PI {
        a += t;
    }
    if a.abs() < 1e-6 {
        a = 0.0;
    }
    a as f32
}

fn rot_arr(a: &mut [f32], stride: usize, rad: f32, px: f32, py: f32) {
    let (cs, sn) = (rad.cos(), rad.sin());
    for c in a.chunks_exact_mut(stride) {
        let (dx, dy) = (c[0] - px, c[1] - py);
        c[0] = px + dx * cs - dy * sn;
        c[1] = py + dx * sn + dy * cs;
    }
}

/// Halbe Breite/Hoehe der achsenparallelen Huelle einer um ang gedrehten Ellipse.
fn ell_half(rx: f32, ry: f32, ang: f32) -> (f32, f32) {
    let (c, s) = ((ang as f64).cos(), (ang as f64).sin());
    let (rx, ry) = (rx as f64, ry as f64);
    ((rx * rx * c * c + ry * ry * s * s).sqrt() as f32, (rx * rx * s * s + ry * ry * c * c).sqrt() as f32)
}

/// Achsenparallele Huelle eines um ang gedrehten Rechtecks.
fn rect_hull(cx: f32, cy: f32, w: f32, h: f32, ang: f32) -> RectF {
    let (hx, hy) = (w.abs() / 2.0, h.abs() / 2.0);
    let (c, s) = (ang.cos().abs(), ang.sin().abs());
    let (ex, ey) = (hx * c + hy * s, hx * s + hy * c);
    RectF::new(cx - ex, cy - ey, cx + ex, cy + ey)
}

/// Abstand des Punktes (px, py) von der Strecke a-b.
pub fn seg(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let l = dx * dx + dy * dy;
    let t = if l == 0.0 { 0.0 } else { (((px - ax) * dx + (py - ay) * dy) / l).clamp(0.0, 1.0) };
    hypot(px - ax - t * dx, py - ay - t * dy)
}

/// Teil-Radierer (reine Logik). p = Tripel x, y, Druck; Radierer-Kreis (cx, cy, r).
/// None, wenn der Radierer den Strich nicht beruehrt; sonst die Tripel-Listen der uebrig bleibenden
/// Teilstuecke (jeweils >= 2 Punkte, leere Liste = alles geloescht). Ein Einzelpunkt wird nur entfernt, nie geteilt.
pub fn erase_split(p: &[f32], width: f32, cx: f32, cy: f32, r: f32) -> Option<Vec<Vec<f32>>> {
    let n = p.len() / 3;
    if n == 0 {
        return None;
    }
    let lim = r + width / 2.0;
    if n == 1 {
        return if hypot(p[0] - cx, p[1] - cy) < lim { Some(Vec::new()) } else { None };
    }
    let max_seg = (r / 2.0).max(1e-4);
    let mut q: Vec<f32> = Vec::with_capacity(n * 3);
    q.extend_from_slice(&p[0..3]);
    for i in 1..n {
        let (ax, ay, ap) = (p[i * 3 - 3], p[i * 3 - 2], p[i * 3 - 1]);
        let (bx, by, bp) = (p[i * 3], p[i * 3 + 1], p[i * 3 + 2]);
        let (dx, dy) = (bx - ax, by - ay);
        let len = hypot(dx, dy);
        if len > max_seg && seg(cx, cy, ax, ay, bx, by) < lim + max_seg {
            let k = ((len / max_seg).ceil() as i32).min(4096);
            for j in 1..k {
                let t = j as f32 / k as f32;
                q.extend_from_slice(&[ax + dx * t, ay + dy * t, ap + (bp - ap) * t]);
            }
        }
        q.extend_from_slice(&[bx, by, bp]);
    }
    let m = q.len() / 3;
    let gone: Vec<bool> = (0..m).map(|i| hypot(q[i * 3] - cx, q[i * 3 + 1] - cy) < lim).collect();
    if !gone.iter().any(|g| *g) {
        return None;
    }
    let mut out = Vec::new();
    let mut s: Option<usize> = None;
    for i in 0..=m {
        let keep = i < m && !gone[i];
        if keep {
            if s.is_none() {
                s = Some(i);
            }
        } else if let Some(st) = s.take() {
            if i - st >= 2 {
                out.push(q[st * 3..i * 3].to_vec());
            }
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- Datentypen

/// Freihand-Strich: Tripel aus x, y, Druck.
#[derive(Clone, Debug, Default)]
pub struct Stroke {
    pub p: Vec<f32>,
    /// Marker (halbtransparent).
    pub hl: bool,
    /// Druckabhaengige Breite.
    pub var: bool,
    /// Eckige Polylinie ohne Glaettung (aus Formen entstanden, z. B. durch den Teil-Radierer).
    pub sharp: bool,
}

impl Stroke {
    pub fn n(&self) -> usize {
        self.p.len() / 3
    }
}

pub const LINE: i32 = 0;
pub const ARROW: i32 = 1;
pub const ELLIPSE: i32 = 2;
pub const POLY: i32 = 3;
pub const ARROW2: i32 = 4;
pub const HEART: i32 = 5;
/// Maschinenbau-Pfeil (gefülltes Dreieck, 15° gesamt), WP3.
pub const ARROW_ENG: i32 = 6;
/// Pfeil mit gefüllter Spitze.
pub const ARROW_FILLED: i32 = 7;
/// Doppelpfeil mit gefüllten Spitzen.
pub const ARROW2_FILLED: i32 = 8;
/// Gestrichelter Pfeil (offene Spitze).
pub const ARROW_DASHED: i32 = 9;
/// Gestrichelter Doppelpfeil (offene Spitzen).
pub const ARROW2_DASHED: i32 = 10;

/// Aufbau eines einfachen Pfeils: (Spitzen an beiden Enden?, gefüllt?, gestrichelt?).
/// None für alles, was kein solcher Pfeil ist (auch ARROW_ENG, der eigene Geometrie hat).
pub fn arrow_spec(kind: i32) -> Option<(bool, bool, bool)> {
    match kind {
        ARROW => Some((false, false, false)),
        ARROW2 => Some((true, false, false)),
        ARROW_FILLED => Some((false, true, false)),
        ARROW2_FILLED => Some((true, true, false)),
        ARROW_DASHED => Some((false, false, true)),
        ARROW2_DASHED => Some((true, false, true)),
        _ => None,
    }
}

/// Wählbare Pfeilstile (Index = `Board::arrow_style`): Name und Zieltyp für (einfacher, doppelter) erkannter Pfeil.
/// Index 0 = wie erkannt.
pub const ARROW_STYLES: [(&str, i32, i32); 7] = [
    ("Wie gezeichnet", ARROW, ARROW2),
    ("Offene Spitze", ARROW, ARROW),
    ("Gefüllte Spitze", ARROW_FILLED, ARROW_FILLED),
    ("Maschinenbau 15°", ARROW_ENG, ARROW_ENG),
    ("Doppelpfeil", ARROW2, ARROW2),
    ("Doppelpfeil gefüllt", ARROW2_FILLED, ARROW2_FILLED),
    ("Gestrichelt", ARROW_DASHED, ARROW2_DASHED),
];

/// Wandelt einen erkannten Pfeil (ARROW/ARROW2) in den gewählten Stil um; andere Typen bleiben.
pub fn styled_arrow(kind: i32, style: i32) -> i32 {
    if kind != ARROW && kind != ARROW2 {
        return kind;
    }
    let Some(&(_, single, double)) = ARROW_STYLES.get(style.max(0) as usize) else { return kind };
    if style == 0 {
        return kind;
    }
    // Stile mit nur einer Spitzenart (Gefüllt/Offen/Maschinenbau) behalten einen gezeichneten Doppelpfeil als Doppelpfeil.
    if kind == ARROW2 {
        return match single {
            ARROW => ARROW2,
            ARROW_FILLED | ARROW_ENG => ARROW2_FILLED,
            _ => double,
        };
    }
    single
}

/// Spitzenlänge des Maschinenbau-Pfeils: max(10 × Linienbreite, 12).
pub fn arrow_eng_len(width: f32) -> f32 {
    (10.0 * width).max(12.0)
}

/// Dreieckspunkte des Maschinenbau-Pfeils von (x0, y0) nach (x1, y1), gefüllt mit 15° Öffnungswinkel gesamt
/// (7,5° je Seite zur Achse). Rückgabe: Spitze (x1, y1), Basis links, Basis rechts als [x, y; 3].
/// Die Basis liegt `arrow_eng_len(width)` hinter der Spitze; die Linie endet dort (Spitze bleibt scharf).
pub fn arrow_eng_head(x0: f32, y0: f32, x1: f32, y1: f32, width: f32) -> [f32; 6] {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let l = hypot(dx, dy);
    if l <= 0.0 {
        return [x1, y1, x1, y1, x1, y1];
    }
    let (ux, uy) = (dx / l, dy / l);
    let len = arrow_eng_len(width);
    let half = len * 7.5f32.to_radians().tan();
    let (bx, by) = (x1 - ux * len, y1 - uy * len);
    let (nx, ny) = (-uy * half, ux * half);
    [x1, y1, bx + nx, by + ny, bx - nx, by - ny]
}

/// Strich-/Lückenlänge gestrichelter Pfeile.
pub fn arrow_dash(width: f32) -> (f32, f32) {
    ((width * 4.0).max(6.0), (width * 3.0).max(4.0))
}

/// Mitte der Basis eines Maschinenbau-Pfeil-Dreiecks (Endpunkt der Linie).
fn arrow_eng_base(h: &[f32; 6]) -> (f32, f32) {
    ((h[2] + h[4]) / 2.0, (h[3] + h[5]) / 2.0)
}

/// Geometrische Form: Linie, Pfeil, Doppelpfeil, Ellipse, Herz oder geschlossenes Polygon.
/// q: LINE/ARROW/ARROW2/ARROW_ENG = {x0,y0,x1,y1}; ELLIPSE/HEART = ungedrehte Box {x0,y0,x1,y1} + ang; POLY = Eckpunkte.
#[derive(Clone, Debug, Default)]
pub struct Shape {
    pub kind: i32,
    pub q: Vec<f32>,
    pub hl: bool,
    /// Drehung (Bogenmass) der Ellipse bzw. des Herzens um den Mittelpunkt.
    pub ang: f32,
}

/// Herz im Einheitsquadrat (y nach unten): rechte Haelfte als Kubik-Bezier Kerbe -> Spitze; die linke ist gespiegelt.
pub const HEART_R: [f32; 8] = [0.5, 0.238, 0.5, -0.172, 1.625, -0.132, 0.5, 1.0];

/// Abgetastete Herzkontur (2n Punkte, geschlossen, ab der Kerbe) der Box q, gedreht um ang um die Boxmitte.
pub fn heart_pts(q: &[f32], ang: f32, n: usize) -> Vec<f32> {
    let (x0, y0) = (q[0].min(q[2]), q[1].min(q[3]));
    let (w, h) = ((q[2] - q[0]).abs(), (q[3] - q[1]).abs());
    let (cx, cy, cs, sn) = (x0 + w / 2.0, y0 + h / 2.0, ang.cos(), ang.sin());
    let hr = HEART_R;
    let mut o = vec![0.0f32; n * 4];
    for i in 0..2 * n {
        let t = if i < n { i as f32 / n as f32 } else { 1.0 - (i - n) as f32 / n as f32 };
        let u = 1.0 - t;
        let mut bx = u * u * u * hr[0] + 3.0 * u * u * t * hr[2] + 3.0 * u * t * t * hr[4] + t * t * t * hr[6];
        let by = u * u * u * hr[1] + 3.0 * u * u * t * hr[3] + 3.0 * u * t * t * hr[5] + t * t * t * hr[7];
        if i >= n {
            bx = 1.0 - bx;
        }
        let (dx, dy) = (x0 + bx * w - cx, y0 + by * h - cy);
        o[i * 2] = cx + dx * cs - dy * sn;
        o[i * 2 + 1] = cy + dx * sn + dy * cs;
    }
    o
}

/// Haftnotiz (bg != 0) oder freier Text (bg == 0).
#[derive(Clone, Debug, Default)]
pub struct Note {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub mh: f32,
    pub size: f32,
    pub bg: u32,
    /// Drehung (Bogenmass) um den eigenen Mittelpunkt.
    pub ang: f32,
    pub text: String,
    lay: Option<Rc<TextLayout>>,
}

const NOTE_BASE: f32 = 40.0;

impl Note {
    /// Innenabstand (Haftnotiz) in Weltkoordinaten.
    pub fn pad(&self) -> f32 {
        if self.bg != 0 {
            self.size * 0.7
        } else {
            0.0
        }
    }

    /// Berechnet Zeilenumbruch, Hoehe (und bei freiem Text die Breite) neu.
    pub fn layout(&mut self) {
        let k = self.size / NOTE_BASE;
        if self.bg == 0 {
            let mw = self.text.split('\n').map(|l| crate::text::measure(l, NOTE_BASE)).fold(0.0f32, f32::max);
            self.w = (mw + 2.0).min(NOTE_BASE * 26.0) * k;
        }
        let lw = ((self.w - 2.0 * self.pad()) / k).ceil().max(1.0);
        let lay = TextLayout::new(&self.text, NOTE_BASE, lw, 1.15);
        let need = lay.height() * k + 2.0 * self.pad();
        self.h = if self.bg != 0 { self.mh.max(need) } else { need };
        self.lay = Some(Rc::new(lay));
    }
}

/// Bild-Objekt: Datei im Bildordner, Rechteck in Weltkoordinaten.
#[derive(Clone, Debug, Default)]
pub struct Img {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Drehung (Bogenmass) um den eigenen Mittelpunkt.
    pub ang: f32,
    pub file: String,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Stroke(Stroke),
    Shape(Shape),
    Note(Note),
    Img(Img),
}

/// Ein Board-Objekt. `id` ist nur zur Laufzeit eindeutig (Auswahl), nicht Teil des Dateiformats.
#[derive(Clone, Debug)]
pub struct Item {
    pub id: u64,
    pub color: u32,
    pub width: f32,
    /// Layer.id der Ebene, auf der das Objekt liegt.
    pub layer: i32,
    pub kind: Kind,
    /// Wird gerade verschoben/gedreht (nicht im Board-Cache, sondern obenauf gezeichnet).
    pub lift: bool,
    /// Unbekannte JSON-Felder (z. B. Xournal++-Attribute "xa", "xraw", Seitenzuordnung "page"), werden unverändert zurückgeschrieben.
    pub extra: Option<Box<Map<String, Value>>>,
    b: Cell<Option<RectF>>,
    path: RefCell<Option<Path>>,
}

impl Item {
    pub fn new(kind: Kind) -> Item {
        Item { id: next_id(), color: INK, width: 4.0, layer: 1, kind, lift: false, extra: None, b: Cell::new(None), path: RefCell::new(None) }
    }

    pub fn stroke(color: u32, width: f32, hl: bool, var: bool) -> Item {
        let mut it = Item::new(Kind::Stroke(Stroke { p: Vec::with_capacity(96), hl, var, sharp: false }));
        it.color = color;
        it.width = width;
        it
    }

    pub fn shape(kind: i32, q: Vec<f32>, color: u32, width: f32) -> Item {
        let mut it = Item::new(Kind::Shape(Shape { kind, q, hl: false, ang: 0.0 }));
        it.color = color;
        it.width = width;
        it
    }

    /// Kopie mit neuer Laufzeit-id (eigenstaendiges Objekt).
    pub fn dup(&self) -> Item {
        let mut c = self.clone();
        c.id = next_id();
        c.lift = false;
        c
    }

    /// Nach direkter Aenderung von Geometrie-Feldern aufrufen (verwirft Zwischenspeicher).
    pub fn touch(&mut self) {
        self.b.set(None);
        *self.path.borrow_mut() = None;
    }

    pub fn as_stroke(&self) -> Option<&Stroke> {
        match &self.kind {
            Kind::Stroke(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_shape(&self) -> Option<&Shape> {
        match &self.kind {
            Kind::Shape(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_note(&self) -> Option<&Note> {
        match &self.kind {
            Kind::Note(s) => Some(s),
            _ => None,
        }
    }
    pub fn is_img(&self) -> bool {
        matches!(self.kind, Kind::Img(_))
    }

    /// Haengt einen Punkt an einen Strich an.
    pub fn add_point(&mut self, x: f32, y: f32, pr: f32) {
        if let Kind::Stroke(s) = &mut self.kind {
            s.p.extend_from_slice(&[x, y, pr]);
        }
        self.touch();
    }

    pub fn weight(&self) -> usize {
        match &self.kind {
            Kind::Stroke(s) => s.n(),
            Kind::Img(_) => 50,
            _ => 1,
        }
    }

    pub fn bounds(&self) -> RectF {
        if let Some(b) = self.b.get() {
            return b;
        }
        let w = self.width;
        let b = match &self.kind {
            Kind::Stroke(s) => {
                let mut r = RectF::new(f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for c in s.p.chunks_exact(3) {
                    r.left = r.left.min(c[0]);
                    r.right = r.right.max(c[0]);
                    r.top = r.top.min(c[1]);
                    r.bottom = r.bottom.max(c[1]);
                }
                if s.p.len() < 3 {
                    r = RectF::new(0.0, 0.0, 0.0, 0.0);
                }
                RectF::new(r.left - w, r.top - w, r.right + w, r.bottom + w)
            }
            Kind::Shape(g) => {
                let q = &g.q;
                if g.kind == HEART {
                    let hp = heart_pts(q, g.ang, 24);
                    let mut r = RectF::new(hp[0], hp[1], hp[0], hp[1]);
                    for c in hp.chunks_exact(2) {
                        r.union(&RectF::new(c[0], c[1], c[0], c[1]));
                    }
                    RectF::new(r.left - w, r.top - w, r.right + w, r.bottom + w)
                } else if g.kind == ELLIPSE && g.ang != 0.0 {
                    let (cx, cy) = ((q[0] + q[2]) / 2.0, (q[1] + q[3]) / 2.0);
                    let (hx, hy) = ell_half((q[2] - q[0]).abs() / 2.0, (q[3] - q[1]).abs() / 2.0, g.ang);
                    RectF::new(cx - hx - w, cy - hy - w, cx + hx + w, cy + hy + w)
                } else if g.kind == ARROW_ENG {
                    // Dreieckspunkte gehören zur Huelle (die Basis liegt seitlich ueber der Linie).
                    let h = arrow_eng_head(q[0], q[1], q[2], q[3], w);
                    let mut r = RectF::new(q[0], q[1], q[0], q[1]);
                    for c in q[..4].chunks_exact(2).chain(h.chunks_exact(2)) {
                        r.union(&RectF::new(c[0], c[1], c[0], c[1]));
                    }
                    RectF::new(r.left - w, r.top - w, r.right + w, r.bottom + w)
                } else {
                    let mut r = RectF::new(q[0], q[1], q[0], q[1]);
                    for c in q.chunks_exact(2) {
                        r.union(&RectF::new(c[0], c[1], c[0], c[1]));
                    }
                    let w = if arrow_spec(g.kind).is_some() { w * 3.0 } else { w };
                    RectF::new(r.left - w, r.top - w, r.right + w, r.bottom + w)
                }
            }
            Kind::Note(n) => {
                if n.ang != 0.0 {
                    rect_hull(n.x + n.w / 2.0, n.y + n.h / 2.0, n.w, n.h, n.ang)
                } else {
                    RectF::new(n.x, n.y, n.x + n.w, n.y + n.h)
                }
            }
            Kind::Img(m) => {
                if m.ang != 0.0 {
                    rect_hull(m.x + m.w / 2.0, m.y + m.h / 2.0, m.w, m.h, m.ang)
                } else {
                    RectF::new(m.x.min(m.x + m.w), m.y.min(m.y + m.h), m.x.max(m.x + m.w), m.y.max(m.y + m.h))
                }
            }
        };
        self.b.set(Some(b));
        b
    }

    fn line_color(&self, hl: bool, dark: bool) -> u32 {
        let col = ink(self.color, dark);
        if hl {
            (col & 0xFFFFFF) | 0x66000000
        } else {
            col
        }
    }

    pub fn draw(&self, c: &mut Canvas, dark: bool) {
        match &self.kind {
            Kind::Stroke(s) => self.draw_stroke(s, c, dark),
            Kind::Shape(g) => self.draw_shape(g, c, dark),
            Kind::Note(n) => {
                let Some(lay) = n.lay.clone() else { return };
                if n.ang != 0.0 {
                    c.save();
                    c.rotate(n.ang.to_degrees(), n.x + n.w / 2.0, n.y + n.h / 2.0);
                }
                if n.bg != 0 {
                    let r = n.size * 0.4;
                    c.draw_round_rect(
                        n.x + n.size * 0.1,
                        n.y + n.size * 0.2,
                        n.x + n.w + n.size * 0.1,
                        n.y + n.h + n.size * 0.2,
                        r,
                        &Paint::fill(0x26000000),
                    );
                    c.draw_round_rect(n.x, n.y, n.x + n.w, n.y + n.h, r, &Paint::fill(n.bg));
                }
                let k = n.size / NOTE_BASE;
                c.save();
                c.translate(n.x + n.pad(), n.y + n.pad());
                c.scale(k, k);
                lay.draw(c, if n.bg != 0 { 0xFF1B1B1F } else { ink(self.color, dark) });
                c.restore();
                if n.ang != 0.0 {
                    c.restore();
                }
            }
            Kind::Img(m) => {
                if m.ang != 0.0 {
                    c.save();
                    c.rotate(m.ang.to_degrees(), m.x + m.w / 2.0, m.y + m.h / 2.0);
                }
                match load_image(&m.file) {
                    Some(pm) => {
                        c.draw_image(&pm, m.x, m.y, m.x + m.w, m.y + m.h);
                    }
                    None => {
                        c.draw_rect(m.x, m.y, m.x + m.w, m.y + m.h, &Paint::fill(0xFFC9CBD2));
                        let p = Paint::stroke(0xFF7A7D88, (m.w.abs().min(m.h.abs()) / 60.0).max(1.0), Cap::Butt);
                        c.draw_rect(m.x, m.y, m.x + m.w, m.y + m.h, &p);
                        c.draw_line(m.x, m.y, m.x + m.w, m.y + m.h, &p);
                        c.draw_line(m.x + m.w, m.y, m.x, m.y + m.h, &p);
                    }
                }
                if m.ang != 0.0 {
                    c.restore();
                }
            }
        }
    }

    fn draw_stroke(&self, s: &Stroke, c: &mut Canvas, dark: bool) {
        let n = s.n();
        if n == 0 {
            return;
        }
        let p = &s.p;
        let mut pt = Paint::stroke(self.line_color(s.hl, dark), self.width, Cap::Round);
        if n == 1 {
            pt.style = Style::Fill;
            let k = if s.var { 0.35 + 1.3 * p[2] } else { 1.0 };
            c.draw_circle(p[0], p[1], self.width * k / 2.0, &pt);
            return;
        }
        // Schneller Weg: Kapseln mit Abstandsfeld-Kantenglaettung (Canvas::fill_capsules)
        let segs = self.capsule_segs(s, c.res_scale());
        if c.fill_capsules(&segs, pt.color) {
            return;
        }
        for (w, path) in self.stroke_runs(s) {
            match path {
                Some(path) if w < 0.0 => {
                    pt.style = Style::Fill;
                    c.draw_path(&path, &pt);
                }
                Some(path) => {
                    pt.style = Style::Stroke;
                    pt.stroke_width = w;
                    c.draw_path(&path, &pt);
                }
                None => {
                    pt.style = Style::Fill;
                    c.draw_circle(p[0], p[1], w / 2.0, &pt);
                }
            }
        }
    }

    /// Mittellinien des Strichs mit Breite. Druckstriche werden in Abschnitte nahezu gleicher Breite
    /// zusammengefasst (statt einer Linie je Segment). `None` = entarteter Abschnitt (als Punkt zeichnen).
    fn stroke_runs(&self, s: &Stroke) -> Vec<(f32, Option<Path>)> {
        let (n, p) = (s.n(), &s.p);
        if s.hl || !s.var {
            let mut cache = self.path.borrow_mut();
            if cache.is_none() {
                let mut pb = PathBuilder::new();
                pb.move_to(p[0], p[1]);
                if s.sharp {
                    for i in 1..n {
                        pb.line_to(p[i * 3], p[i * 3 + 1]);
                    }
                } else {
                    for i in 1..n {
                        let (ax, ay) = (p[i * 3 - 3], p[i * 3 - 2]);
                        pb.quad_to(ax, ay, (ax + p[i * 3]) / 2.0, (ay + p[i * 3 + 1]) / 2.0);
                    }
                    pb.line_to(p[n * 3 - 3], p[n * 3 - 2]);
                }
                *cache = pb.finish();
            }
            let path = cache.as_ref().filter(|path| path.bounds().width() > 0.0 || path.bounds().height() > 0.0).cloned();
            return vec![(self.width, path)];
        }
        let wid = |i: usize| self.width * (0.35 + 1.3 * (p[i * 3 - 1] + p[i * 3 + 2]) * 0.5);
        let mut out = Vec::new();
        let mut i = 1;
        while i < n {
            let w0 = wid(i);
            let tol = (w0 * 0.04).max(0.02);
            let (mut sum, mut cnt) = (0.0, 0);
            let mut pb = PathBuilder::new();
            pb.move_to(p[i * 3 - 3], p[i * 3 - 2]);
            let mut moved = false;
            while i < n {
                let w = wid(i);
                if cnt > 0 && (w - w0).abs() > tol {
                    break;
                }
                let (x, y) = (p[i * 3], p[i * 3 + 1]);
                moved |= x != p[i * 3 - 3] || y != p[i * 3 - 2];
                pb.line_to(x, y);
                sum += w;
                cnt += 1;
                i += 1;
            }
            let w = sum / cnt as f32;
            if moved {
                out.push((w, pb.finish()));
            } else {
                // nur Punkte an derselben Stelle: Kreis am Abschnittsende
                let k = (i - 1) * 3;
                let mut pb = PathBuilder::new();
                pb.push_circle(p[k], p[k + 1], w / 2.0);
                out.push((-1.0, pb.finish()));
            }
        }
        out
    }

    /// Segmente (x0, y0, x1, y1, Radius) des Strichs in Weltkoordinaten. Druckstriche: ein Segment je
    /// Punktpaar mit mittlerem Druck; sonst die geglaettete Mittellinie (Quadratik-Kurven, abgeflacht fuer
    /// den Aufloesungsfaktor res) mit halber Breite.
    fn capsule_segs(&self, s: &Stroke, res: f32) -> Vec<[f32; 5]> {
        let (n, p) = (s.n(), &s.p);
        let mut v = Vec::with_capacity(n * 2);
        if s.var && !s.hl {
            for i in 1..n {
                let r = self.width * (0.35 + 1.3 * (p[i * 3 - 1] + p[i * 3 + 2]) * 0.5) * 0.5;
                v.push([p[i * 3 - 3], p[i * 3 - 2], p[i * 3], p[i * 3 + 1], r]);
            }
            return v;
        }
        let r = self.width * 0.5;
        if s.sharp {
            for i in 1..n {
                v.push([p[i * 3 - 3], p[i * 3 - 2], p[i * 3], p[i * 3 + 1], r]);
            }
            return v;
        }
        // wie der Pfad: move_to(p0), quad_to(p[i-1], Mitte(p[i-1], p[i])), line_to(p[n-1])
        let (mut x, mut y) = (p[0], p[1]);
        for i in 1..n {
            let (cx, cy) = (p[i * 3 - 3], p[i * 3 - 2]);
            let (ex, ey) = ((cx + p[i * 3]) / 2.0, (cy + p[i * 3 + 1]) / 2.0);
            let dd = hypot(x - 2.0 * cx + ex, y - 2.0 * cy + ey) * res;
            let k = ((dd / 1.2).sqrt().ceil() as usize).clamp(1, 32);
            for j in 1..=k {
                let t = j as f32 / k as f32;
                let u = 1.0 - t;
                let (nx, ny) = (u * u * x + 2.0 * u * t * cx + t * t * ex, u * u * y + 2.0 * u * t * cy + t * t * ey);
                v.push([x, y, nx, ny, r]);
                x = nx;
                y = ny;
            }
        }
        v.push([x, y, p[n * 3 - 3], p[n * 3 - 2], r]);
        v
    }

    /// Die zwei Widerhaken einer Pfeilspitze an (tx, ty), Schaft kommt von (fx, fy).
    fn barbs(&self, tx: f32, ty: f32, fx: f32, fy: f32, max_rel: f32) -> Option<[f32; 4]> {
        let (dx, dy) = (tx - fx, ty - fy);
        let l = hypot(dx, dy);
        if l <= 0.0 {
            return None;
        }
        let hd = (l * max_rel).min(self.width * 5.0);
        let (ux, uy, cs, sn) = (dx / l, dy / l, 0.88f32, 0.47f32);
        Some([tx - hd * (ux * cs - uy * sn), ty - hd * (uy * cs + ux * sn), tx - hd * (ux * cs + uy * sn), ty - hd * (uy * cs - ux * sn)])
    }

    /// Widerhaken für Ende (Index 0) und – bei Doppelpfeilen – Anfang (Index 1).
    fn arrow_heads(&self, q: &[f32], both: bool) -> [Option<[f32; 4]>; 2] {
        let rel = if both { 0.4 } else { 0.5 };
        let end = self.barbs(q[2], q[3], q[0], q[1], rel);
        let start = if both { self.barbs(q[0], q[1], q[2], q[3], rel) } else { None };
        [end, start]
    }

    /// Gefüllte Pfeilspitzen als Dreiecke [Spitze, Basis 1, Basis 2] (für Export, der nur Striche kennt).
    pub fn filled_heads(&self) -> Vec<[f32; 6]> {
        let Kind::Shape(g) = &self.kind else { return Vec::new() };
        let q = &g.q;
        if q.len() < 4 {
            return Vec::new();
        }
        if g.kind == ARROW_ENG {
            return vec![arrow_eng_head(q[0], q[1], q[2], q[3], self.width)];
        }
        match arrow_spec(g.kind) {
            Some((both, true, _)) => {
                let tips = [(q[2], q[3]), (q[0], q[1])];
                self.arrow_heads(q, both).iter().zip(tips).filter_map(|(h, t)| h.map(|h| [t.0, t.1, h[0], h[1], h[2], h[3]])).collect()
            }
            _ => Vec::new(),
        }
    }

    fn draw_shape(&self, g: &Shape, c: &mut Canvas, dark: bool) {
        let q = &g.q;
        if q.len() < 4 {
            return;
        }
        let p = Paint::stroke(self.line_color(g.hl, dark), self.width, Cap::Round);
        match g.kind {
            LINE => c.draw_line(q[0], q[1], q[2], q[3], &p),
            ARROW | ARROW2 | ARROW_FILLED | ARROW2_FILLED | ARROW_DASHED | ARROW2_DASHED => {
                let (both, filled, dashed) = arrow_spec(g.kind).unwrap_or_default();
                let heads = self.arrow_heads(q, both);
                // Schaft: bei gefüllten Spitzen bis zur Basis, damit die Spitze scharf bleibt
                let (mut a, mut b) = ((q[0], q[1]), (q[2], q[3]));
                if filled {
                    if let Some(h) = heads[0] {
                        b = ((h[0] + h[2]) / 2.0, (h[1] + h[3]) / 2.0);
                    }
                    if let Some(h) = heads[1] {
                        a = ((h[0] + h[2]) / 2.0, (h[1] + h[3]) / 2.0);
                    }
                }
                if dashed {
                    let mut pd = p.clone();
                    pd.cap = Cap::Butt;
                    pd.dash = Some(arrow_dash(self.width));
                    c.draw_line(a.0, a.1, b.0, b.1, &pd);
                } else {
                    c.draw_line(a.0, a.1, b.0, b.1, &p);
                }
                let tips = [(q[2], q[3]), (q[0], q[1])];
                for (h, t) in heads.iter().zip(tips) {
                    let Some(h) = h else { continue };
                    if filled {
                        let mut pb = PathBuilder::new();
                        pb.move_to(t.0, t.1);
                        pb.line_to(h[0], h[1]);
                        pb.line_to(h[2], h[3]);
                        pb.close();
                        if let Some(path) = pb.finish() {
                            c.draw_path(&path, &Paint::fill(p.color));
                        }
                    } else {
                        c.draw_line(t.0, t.1, h[0], h[1], &p);
                        c.draw_line(t.0, t.1, h[2], h[3], &p);
                    }
                }
            }
            ARROW_ENG => {
                // Schaft bis zur Basis, Spitze gefuellt (sie ragt nicht ueber die Basis hinaus)
                let h = arrow_eng_head(q[0], q[1], q[2], q[3], self.width);
                let (bx, by) = arrow_eng_base(&h);
                c.draw_line(q[0], q[1], bx, by, &p);
                let mut pb = PathBuilder::new();
                pb.move_to(h[0], h[1]);
                pb.line_to(h[2], h[3]);
                pb.line_to(h[4], h[5]);
                pb.close();
                if let Some(path) = pb.finish() {
                    c.draw_path(&path, &Paint::fill(p.color));
                }
            }
            ELLIPSE => {
                if g.ang != 0.0 {
                    c.save();
                    c.rotate(g.ang.to_degrees(), (q[0] + q[2]) / 2.0, (q[1] + q[3]) / 2.0);
                }
                c.draw_oval(q[0].min(q[2]), q[1].min(q[3]), q[0].max(q[2]), q[1].max(q[3]), &p);
                if g.ang != 0.0 {
                    c.restore();
                }
            }
            HEART => {
                let (x0, y0) = (q[0].min(q[2]), q[1].min(q[3]));
                let (w, h) = ((q[2] - q[0]).abs(), (q[3] - q[1]).abs());
                let mut cache = self.path.borrow_mut();
                if cache.is_none() {
                    let hr = HEART_R;
                    let mut pb = PathBuilder::new();
                    pb.move_to(x0 + hr[0] * w, y0 + hr[1] * h);
                    pb.cubic_to(x0 + hr[2] * w, y0 + hr[3] * h, x0 + hr[4] * w, y0 + hr[5] * h, x0 + hr[6] * w, y0 + hr[7] * h);
                    pb.cubic_to(
                        x0 + (1.0 - hr[4]) * w,
                        y0 + hr[5] * h,
                        x0 + (1.0 - hr[2]) * w,
                        y0 + hr[3] * h,
                        x0 + hr[0] * w,
                        y0 + hr[1] * h,
                    );
                    pb.close();
                    *cache = pb.finish();
                }
                if let Some(path) = cache.as_ref() {
                    if g.ang != 0.0 {
                        c.save();
                        c.rotate(g.ang.to_degrees(), x0 + w / 2.0, y0 + h / 2.0);
                    }
                    c.draw_path(path, &p);
                    if g.ang != 0.0 {
                        c.restore();
                    }
                }
            }
            _ => {
                let mut cache = self.path.borrow_mut();
                if cache.is_none() {
                    let mut pb = PathBuilder::new();
                    pb.move_to(q[0], q[1]);
                    for xy in q[2..].chunks_exact(2) {
                        pb.line_to(xy[0], xy[1]);
                    }
                    pb.close();
                    *cache = pb.finish();
                }
                if let Some(path) = cache.as_ref() {
                    c.draw_path(path, &p);
                }
            }
        }
    }

    /// Trifft der Kreis (x, y, r) das Objekt?
    pub fn hit(&self, x: f32, y: f32, r: f32) -> bool {
        let lim = r + self.width / 2.0;
        match &self.kind {
            Kind::Stroke(s) => {
                let (p, n) = (&s.p, s.n());
                if n == 0 {
                    return false;
                }
                if n == 1 {
                    return hypot(x - p[0], y - p[1]) <= lim;
                }
                (1..n).any(|i| seg(x, y, p[i * 3 - 3], p[i * 3 - 2], p[i * 3], p[i * 3 + 1]) <= lim)
            }
            Kind::Shape(g) => {
                let q = &g.q;
                match g.kind {
                    LINE | ARROW | ARROW2 | ARROW_FILLED | ARROW2_FILLED | ARROW_DASHED | ARROW2_DASHED => seg(x, y, q[0], q[1], q[2], q[3]) <= lim,
                    ARROW_ENG => {
                        let h = arrow_eng_head(q[0], q[1], q[2], q[3], self.width);
                        seg(x, y, q[0], q[1], q[2], q[3]) <= lim || poly_hit(&h, x, y, lim)
                    }
                    HEART => poly_hit(&heart_pts(q, g.ang, 32), x, y, lim),
                    ELLIPSE => {
                        let (cx, cy) = ((q[0] + q[2]) / 2.0, (q[1] + q[3]) / 2.0);
                        let (rx, ry) = ((q[2] - q[0]).abs() / 2.0, (q[3] - q[1]).abs() / 2.0);
                        let (x, y) = if g.ang != 0.0 { rot_pt(x, y, cx, cy, -g.ang) } else { (x, y) };
                        if rx < 0.001 || ry < 0.001 {
                            return seg(x, y, q[0], q[1], q[2], q[3]) <= lim;
                        }
                        let (u, v) = (((x - cx) / rx) as f64, ((y - cy) / ry) as f64);
                        (((u * u + v * v).sqrt() - 1.0).abs() * rx.min(ry) as f64) as f32 <= lim
                    }
                    _ => poly_hit(q, x, y, lim),
                }
            }
            Kind::Note(n) => {
                let (px, py) = if n.ang != 0.0 { rot_pt(x, y, n.x + n.w / 2.0, n.y + n.h / 2.0, -n.ang) } else { (x, y) };
                px >= n.x - r && px <= n.x + n.w + r && py >= n.y - r && py <= n.y + n.h + r
            }
            Kind::Img(m) => {
                let (px, py) = if m.ang != 0.0 { rot_pt(x, y, m.x + m.w / 2.0, m.y + m.h / 2.0, -m.ang) } else { (x, y) };
                px >= m.x.min(m.x + m.w) - r && px <= m.x.max(m.x + m.w) + r && py >= m.y.min(m.y + m.h) - r && py <= m.y.max(m.y + m.h) + r
            }
        }
    }

    /// Wie hit(), aber mit schnellem Vorab-Test ueber die Huelle.
    pub fn near(&self, x: f32, y: f32, r: f32) -> bool {
        let b = self.bounds();
        x >= b.left - r && x <= b.right + r && y >= b.top - r && y <= b.bottom + r && self.hit(x, y, r)
    }

    /// Skaliert um s um den Punkt (px, py) und verschiebt danach um (dx, dy).
    pub fn xf(&mut self, s: f32, px: f32, py: f32, dx: f32, dy: f32) {
        match &mut self.kind {
            Kind::Stroke(st) => {
                for c in st.p.chunks_exact_mut(3) {
                    c[0] = px + (c[0] - px) * s + dx;
                    c[1] = py + (c[1] - py) * s + dy;
                }
                self.width *= s;
            }
            Kind::Shape(g) => {
                for c in g.q.chunks_exact_mut(2) {
                    c[0] = px + (c[0] - px) * s + dx;
                    c[1] = py + (c[1] - py) * s + dy;
                }
                self.width *= s;
            }
            Kind::Note(n) => {
                n.x = px + (n.x - px) * s + dx;
                n.y = py + (n.y - py) * s + dy;
                if s != 1.0 {
                    n.w *= s;
                    n.mh *= s;
                    n.size *= s;
                    n.layout();
                }
            }
            Kind::Img(m) => {
                m.x = px + (m.x - px) * s + dx;
                m.y = py + (m.y - py) * s + dy;
                m.w *= s;
                m.h *= s;
            }
        }
        self.touch();
    }

    /// Dreht das Objekt um rad (im Uhrzeigersinn auf dem Bildschirm) um den Punkt (px, py).
    pub fn rot(&mut self, rad: f32, px: f32, py: f32) {
        match &mut self.kind {
            Kind::Stroke(st) => rot_arr(&mut st.p, 3, rad, px, py),
            Kind::Shape(g) => {
                if g.kind == ELLIPSE || g.kind == HEART {
                    let q = &mut g.q;
                    let (hx, hy) = ((q[2] - q[0]) / 2.0, (q[3] - q[1]) / 2.0);
                    let (cx, cy) = rot_pt((q[0] + q[2]) / 2.0, (q[1] + q[3]) / 2.0, px, py, rad);
                    q[0] = cx - hx;
                    q[2] = cx + hx;
                    q[1] = cy - hy;
                    q[3] = cy + hy;
                    g.ang = norm_ang(g.ang as f64 + rad as f64);
                } else {
                    rot_arr(&mut g.q, 2, rad, px, py);
                }
            }
            Kind::Note(n) => {
                let (cx, cy) = rot_pt(n.x + n.w / 2.0, n.y + n.h / 2.0, px, py, rad);
                n.x = cx - n.w / 2.0;
                n.y = cy - n.h / 2.0;
                n.ang = norm_ang(n.ang as f64 + rad as f64);
            }
            Kind::Img(m) => {
                let (cx, cy) = rot_pt(m.x + m.w / 2.0, m.y + m.h / 2.0, px, py, rad);
                m.x = cx - m.w / 2.0;
                m.y = cy - m.h / 2.0;
                m.ang = norm_ang(m.ang as f64 + rad as f64);
            }
        }
        self.touch();
    }

    /// Neuer Strich mit denselben Eigenschaften, aber den Punkten a (Tripel).
    pub fn part(&self, a: Vec<f32>) -> Item {
        let mut it = self.dup();
        if let Kind::Stroke(s) = &mut it.kind {
            s.p = a;
        }
        it.touch();
        it
    }

    fn mk(&self, hl: bool, v: &[f32]) -> Item {
        let mut p = Vec::with_capacity(v.len() / 2 * 3);
        for xy in v.chunks_exact(2) {
            p.extend_from_slice(&[xy[0], xy[1], 0.5]);
        }
        let mut it = Item::new(Kind::Stroke(Stroke { p, hl, var: false, sharp: true }));
        it.color = self.color;
        it.width = self.width;
        it.layer = self.layer;
        it
    }

    /// Wandelt eine Form in gleich aussehende Freihand-Striche um (fuer den Teil-Radierer).
    pub fn to_strokes(&self) -> Vec<Item> {
        let Kind::Shape(g) = &self.kind else { return Vec::new() };
        let q = &g.q;
        let hl = g.hl;
        let mut out = Vec::new();
        match g.kind {
            LINE => out.push(self.mk(hl, &[q[0], q[1], q[2], q[3]])),
            ARROW | ARROW2 => {
                // unverändert zur Vorversion (Teil-Radierer)
                let both = g.kind == ARROW2;
                out.push(self.mk(hl, &[q[0], q[1], q[2], q[3]]));
                let heads = self.arrow_heads(q, both);
                if both {
                    if let [Some(h1), Some(h2)] = heads {
                        out.push(self.mk(hl, &[h1[0], h1[1], q[2], q[3], h1[2], h1[3]]));
                        out.push(self.mk(hl, &[h2[0], h2[1], q[0], q[1], h2[2], h2[3]]));
                    }
                } else if let Some(h) = heads[0] {
                    out.push(self.mk(hl, &[q[2], q[3], h[0], h[1]]));
                    out.push(self.mk(hl, &[q[2], q[3], h[2], h[3]]));
                }
            }
            ARROW_FILLED | ARROW2_FILLED | ARROW_DASHED | ARROW2_DASHED => {
                let (both, filled, dashed) = arrow_spec(g.kind).unwrap_or_default();
                let heads = self.arrow_heads(q, both);
                let (mut a, mut b) = ((q[0], q[1]), (q[2], q[3]));
                if filled {
                    if let Some(h) = heads[0] {
                        b = ((h[0] + h[2]) / 2.0, (h[1] + h[3]) / 2.0);
                    }
                    if let Some(h) = heads[1] {
                        a = ((h[0] + h[2]) / 2.0, (h[1] + h[3]) / 2.0);
                    }
                }
                if dashed {
                    let (on, off) = arrow_dash(self.width);
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    let l = hypot(dx, dy);
                    let mut t = 0.0;
                    while l > 0.0 && t < l {
                        let e = (t + on).min(l);
                        out.push(self.mk(hl, &[a.0 + dx * t / l, a.1 + dy * t / l, a.0 + dx * e / l, a.1 + dy * e / l]));
                        t += on + off;
                    }
                } else {
                    out.push(self.mk(hl, &[a.0, a.1, b.0, b.1]));
                }
                let tips = [(q[2], q[3]), (q[0], q[1])];
                // Spitzen zuletzt (xopp-Export füllt sie über filled_heads)
                for (h, t) in heads.iter().zip(tips) {
                    let Some(h) = h else { continue };
                    if filled {
                        out.push(self.mk(hl, &[t.0, t.1, h[0], h[1], h[2], h[3], t.0, t.1]));
                    } else {
                        out.push(self.mk(hl, &[h[0], h[1], t.0, t.1, h[2], h[3]]));
                    }
                }
            }
            ARROW_ENG => {
                let h = arrow_eng_head(q[0], q[1], q[2], q[3], self.width);
                let (bx, by) = arrow_eng_base(&h);
                out.push(self.mk(hl, &[q[0], q[1], bx, by]));
                // Spitze als geschlossener Strich (Strich kann nicht gefuellt werden)
                out.push(self.mk(hl, &[h[0], h[1], h[2], h[3], h[4], h[5], h[0], h[1]]));
            }
            ELLIPSE => {
                let (cx, cy) = ((q[0] + q[2]) / 2.0, (q[1] + q[3]) / 2.0);
                let (rx, ry) = ((q[2] - q[0]).abs() / 2.0, (q[3] - q[1]).abs() / 2.0);
                let (ca, sa) = (g.ang.cos(), g.ang.sin());
                let n = 72;
                let mut v = Vec::with_capacity((n + 1) * 2);
                for i in 0..=n {
                    let t = 2.0 * std::f64::consts::PI * (i % n) as f64 / n as f64;
                    let (ux, uy) = (rx * t.cos() as f32, ry * t.sin() as f32);
                    v.extend_from_slice(&[cx + ux * ca - uy * sa, cy + ux * sa + uy * ca]);
                }
                out.push(self.mk(hl, &v));
            }
            _ => {
                let hp = if g.kind == HEART { heart_pts(q, g.ang, 36) } else { q.clone() };
                let m = hp.len() / 2;
                let mut v = Vec::with_capacity((m + 1) * 2);
                for i in 0..=m {
                    v.extend_from_slice(&[hp[(i % m) * 2], hp[(i % m) * 2 + 1]]);
                }
                out.push(self.mk(hl, &v));
            }
        }
        out
    }

    // ---------------------------------------------------------------- JSON (Dateiformat wie die Java-Version)

    pub fn json(&self) -> Value {
        let mut o = Map::new();
        let c = self.color as i32;
        match &self.kind {
            Kind::Stroke(s) => {
                o.insert("t".into(), json!("s"));
                o.insert("c".into(), json!(c));
                o.insert("w".into(), num(self.width));
                o.insert("h".into(), json!(s.hl));
                o.insert("v".into(), json!(s.var));
                o.insert("sh".into(), json!(s.sharp));
                o.insert("p".into(), arr(&s.p));
            }
            Kind::Shape(g) => {
                o.insert("t".into(), json!("g"));
                o.insert("c".into(), json!(c));
                o.insert("w".into(), num(self.width));
                o.insert("k".into(), json!(g.kind));
                o.insert("h".into(), json!(g.hl));
                o.insert("q".into(), arr(&g.q));
                if g.ang != 0.0 {
                    o.insert("an".into(), num(g.ang));
                }
            }
            Kind::Note(n) => {
                o.insert("t".into(), json!("n"));
                o.insert("c".into(), json!(c));
                o.insert("w".into(), num(self.width));
                o.insert("x".into(), num(n.x));
                o.insert("y".into(), num(n.y));
                o.insert("nw".into(), num(n.w));
                o.insert("mh".into(), num(n.mh));
                o.insert("s".into(), num(n.size));
                o.insert("bg".into(), json!(n.bg as i32));
                o.insert("tx".into(), json!(n.text));
                if n.ang != 0.0 {
                    o.insert("an".into(), num(n.ang));
                }
            }
            Kind::Img(m) => {
                o.insert("t".into(), json!("i"));
                o.insert("c".into(), json!(c));
                o.insert("w".into(), num(self.width));
                o.insert("x".into(), num(m.x));
                o.insert("y".into(), num(m.y));
                o.insert("iw".into(), num(m.w));
                o.insert("ih".into(), num(m.h));
                o.insert("f".into(), json!(m.file));
                if m.ang != 0.0 {
                    o.insert("an".into(), num(m.ang));
                }
            }
        }
        o.insert("ly".into(), json!(self.layer));
        if let Some(ex) = &self.extra {
            for (k, v) in ex.iter() {
                o.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        Value::Object(o)
    }

    /// Liest ein Objekt; None bei defekten oder leeren Eintraegen.
    pub fn from_json(o: &Value) -> Option<Item> {
        let t = o.get("t")?.as_str()?;
        let kind = match t {
            "s" => {
                let mut p = farr(o.get("p")?)?;
                p.truncate(p.len() / 3 * 3);
                if p.is_empty() {
                    return None;
                }
                Kind::Stroke(Stroke { p, hl: jbool(o, "h"), var: jbool(o, "v"), sharp: jbool(o, "sh") })
            }
            "g" => {
                let kind = jint(o.get("k")?)?;
                let mut q = farr(o.get("q")?)?;
                q.truncate(q.len() / 2 * 2);
                if q.len() < 4 {
                    return None;
                }
                Kind::Shape(Shape { kind, q, hl: jbool(o, "h"), ang: jf(o, "an").unwrap_or(0.0) })
            }
            "i" => Kind::Img(Img {
                x: jf(o, "x")?,
                y: jf(o, "y")?,
                w: jf(o, "iw")?,
                h: jf(o, "ih")?,
                file: o.get("f")?.as_str()?.to_string(),
                ang: jf(o, "an").unwrap_or(0.0),
            }),
            _ => {
                let mut n = Note {
                    x: jf(o, "x")?,
                    y: jf(o, "y")?,
                    w: jf(o, "nw")?,
                    h: 0.0,
                    mh: jf(o, "mh")?,
                    size: jf(o, "s")?,
                    bg: jint(o.get("bg")?)? as u32,
                    text: o.get("tx")?.as_str()?.to_string(),
                    ang: jf(o, "an").unwrap_or(0.0),
                    lay: None,
                };
                if !(n.size > 0.0) || !n.size.is_finite() {
                    return None;
                }
                n.layout();
                Kind::Note(n)
            }
        };
        let mut it = Item::new(kind);
        it.color = jint(o.get("c")?)? as u32;
        it.width = jf(o, "w")?;
        it.layer = o.get("ly").and_then(jint).unwrap_or(1);
        if let Some(m) = o.as_object() {
            let ex: Map<String, Value> = m.iter().filter(|(k, _)| !KNOWN_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
            if !ex.is_empty() {
                it.extra = Some(Box::new(ex));
            }
        }
        Some(it)
    }

    /// Neue Notiz (note = Haftnotiz mit Hintergrund bg) bzw. freier Text.
    pub fn note(text: &str, color: u32, size: f32, x: f32, y: f32, w: f32, mh: f32, bg: u32) -> Item {
        let mut n = Note { x, y, w, h: 0.0, mh, size, bg, ang: 0.0, text: text.to_string(), lay: None };
        n.layout();
        let mut it = Item::new(Kind::Note(n));
        it.color = color;
        it
    }

    /// Ersetzt den Text einer Notiz und berechnet das Layout neu.
    pub fn set_text(&mut self, text: &str) {
        if let Kind::Note(n) = &mut self.kind {
            n.text = text.to_string();
            n.layout();
        }
        self.touch();
    }

    /// Setzt die Hintergrundfarbe einer Haftnotiz.
    pub fn set_note_bg(&mut self, bg: u32) {
        if let Kind::Note(n) = &mut self.kind {
            n.bg = bg;
        }
    }
}

fn poly_hit(q: &[f32], x: f32, y: f32, lim: f32) -> bool {
    let m = q.len() / 2;
    (0..m).any(|i| {
        let j = (i + 1) % m;
        seg(x, y, q[i * 2], q[i * 2 + 1], q[j * 2], q[j * 2 + 1]) <= lim
    })
}

// ---------------------------------------------------------------- JSON-Helfer

/// Vom Kern selbst geschriebene Item-Felder; alle anderen landen in `Item::extra`.
const KNOWN_KEYS: [&str; 21] = ["an", "bg", "c", "f", "h", "ih", "iw", "k", "ly", "mh", "nw", "p", "q", "s", "sh", "t", "tx", "v", "w", "x", "y"];

/// f32 als JSON-Zahl (kuerzeste Darstellung; NaN/Unendlich werden 0).
pub fn num(v: f32) -> Value {
    if v.is_finite() {
        json!(v)
    } else {
        json!(0)
    }
}

/// Koordinaten auf zwei Nachkommastellen gerundet.
fn arr(a: &[f32]) -> Value {
    Value::Array(
        a.iter()
            .map(|v| {
                let r = ((*v as f64) * 100.0 + 0.5).floor() / 100.0;
                if r.is_finite() {
                    json!(r)
                } else {
                    json!(0)
                }
            })
            .collect(),
    )
}

/// Lineare Formtypen (Linie, Pfeile): zwei Endpunkte q[0..2] und q[2..4].
pub fn is_linear(kind: i32) -> bool {
    matches!(kind, LINE | ARROW | ARROW2 | ARROW_ENG) || (7..=10).contains(&kind)
}

impl Item {
    /// Skaliert achsenweise um den Pivot (px, py): sx entlang der um `ang` (Bogenmass) gedrehten x-Achse,
    /// sy entlang der dazu senkrechten y-Achse. Kein Spiegeln (Faktoren > 0 erwartet). Linienbreite bleibt.
    pub fn xf_axis(&mut self, sx: f32, sy: f32, px: f32, py: f32, ang: f32) {
        let (cs, sn) = (ang.cos(), ang.sin());
        let pt = |x: f32, y: f32| -> (f32, f32) {
            let (dx, dy) = (x - px, y - py);
            let (u, v) = ((dx * cs + dy * sn) * sx, (-dx * sn + dy * cs) * sy);
            (px + u * cs - v * sn, py + u * sn + v * cs)
        };
        // Faktoren entlang der Achsen eines Objekts, dessen Box um `own` gedreht ist (Projektion; bei 0°/90° exakt).
        let box_f = |own: f32| -> (f32, f32) {
            let d = own - ang;
            let (c, s) = (d.cos(), d.sin());
            (((sx * c).powi(2) + (sy * s).powi(2)).sqrt(), ((sx * s).powi(2) + (sy * c).powi(2)).sqrt())
        };
        match &mut self.kind {
            Kind::Stroke(st) => {
                for c in st.p.chunks_exact_mut(3) {
                    let (x, y) = pt(c[0], c[1]);
                    c[0] = x;
                    c[1] = y;
                }
            }
            Kind::Shape(g) => {
                if g.kind == ELLIPSE || g.kind == HEART {
                    let q = &mut g.q;
                    let (hx, hy) = ((q[2] - q[0]) / 2.0, (q[3] - q[1]) / 2.0);
                    let (cx, cy) = pt((q[0] + q[2]) / 2.0, (q[1] + q[3]) / 2.0);
                    let (fx, fy) = box_f(g.ang);
                    let (hx, hy) = (hx * fx, hy * fy);
                    q[0] = cx - hx;
                    q[2] = cx + hx;
                    q[1] = cy - hy;
                    q[3] = cy + hy;
                } else {
                    for c in g.q.chunks_exact_mut(2) {
                        let (x, y) = pt(c[0], c[1]);
                        c[0] = x;
                        c[1] = y;
                    }
                }
            }
            Kind::Note(n) => {
                let (cx, cy) = pt(n.x + n.w / 2.0, n.y + n.h / 2.0);
                let (fx, fy) = box_f(n.ang);
                n.w *= fx;
                n.mh *= fy;
                if n.bg == 0 {
                    // freier Text: Breite folgt der Schrift
                    n.size *= (fx * fy).sqrt();
                }
                n.layout();
                n.x = cx - n.w / 2.0;
                n.y = cy - n.h / 2.0;
            }
            Kind::Img(m) => {
                let (cx, cy) = pt(m.x + m.w / 2.0, m.y + m.h / 2.0);
                let (fx, fy) = box_f(m.ang);
                m.w *= fx;
                m.h *= fy;
                m.x = cx - m.w / 2.0;
                m.y = cy - m.h / 2.0;
            }
        }
        self.touch();
    }

    /// Setzt bei linearen Formen den Endpunkt `i` (0 = Anfang, 1 = Ende) auf (x, y).
    pub fn set_end(&mut self, i: usize, x: f32, y: f32) {
        if let Kind::Shape(g) = &mut self.kind {
            if is_linear(g.kind) && g.q.len() >= 4 && i < 2 {
                g.q[i * 2] = x;
                g.q[i * 2 + 1] = y;
            }
        }
        self.touch();
    }
}

fn farr(v: &Value) -> Option<Vec<f32>> {
    v.as_array()?.iter().map(|x| x.as_f64().map(|f| f as f32)).collect()
}

/// Ganzzahl; akzeptiert vorzeichenbehaftete und vorzeichenlose 32-Bit-Werte (Farben).
pub fn jint(v: &Value) -> Option<i32> {
    if let Some(i) = v.as_i64() {
        return Some(i as i32);
    }
    v.as_f64().map(|f| f as i64 as i32)
}

pub fn jf(o: &Value, k: &str) -> Option<f32> {
    o.get(k)?.as_f64().map(|f| f as f32)
}

pub fn jbool(o: &Value, k: &str) -> bool {
    o.get(k).and_then(Value::as_bool).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_java_format() {
        // So schreibt die Java-Version (org.json): vorzeichenbehaftete Farben, optionale Felder
        let src = r#"[
          {"t":"s","c":-15658735,"w":3.0,"h":false,"v":true,"sh":false,"p":[1,2,0.5,3.25,4,0.6],"ly":2},
          {"t":"g","c":-1750963,"w":4,"k":2,"h":false,"q":[0,0,10,20],"ly":1,"an":0.5},
          {"t":"n","c":-15658735,"w":4,"x":1,"y":2,"nw":210,"mh":151.2,"s":18,"bg":-7558,"tx":"Hallo\nWelt","ly":1},
          {"t":"i","c":-15658735,"w":4,"x":0,"y":0,"iw":100,"ih":50,"f":"abc.png","ly":1}
        ]"#;
        let v: Value = serde_json::from_str(src).unwrap();
        let items: Vec<Item> = v.as_array().unwrap().iter().filter_map(Item::from_json).collect();
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].color, INK);
        assert_eq!(items[0].layer, 2);
        assert_eq!(items[0].as_stroke().unwrap().n(), 2);
        assert_eq!(items[1].as_shape().unwrap().ang, 0.5);
        let n = items[2].as_note().unwrap();
        assert_eq!(n.bg, 0xFFFFE27A);
        assert!(n.h >= n.mh);
        for it in &items {
            let again = Item::from_json(&it.json()).unwrap();
            assert_eq!(again.json(), it.json());
        }
        assert_eq!(items[0].json()["c"], json!(-15658735));
    }

    #[test]
    fn erase_split_cuts_middle() {
        let p = [0.0, 0.0, 0.5, 100.0, 0.0, 0.5];
        let parts = erase_split(&p, 2.0, 50.0, 0.0, 5.0).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(parts[0][parts[0].len() - 3] < 50.0 && parts[1][0] > 50.0);
        assert!(erase_split(&p, 2.0, 50.0, 40.0, 5.0).is_none());
        assert_eq!(erase_split(&[1.0, 1.0, 0.5], 2.0, 1.0, 1.0, 5.0).unwrap().len(), 0);
    }

    #[test]
    fn transform_and_hit() {
        let mut it = Item::shape(ELLIPSE, vec![0.0, 0.0, 100.0, 50.0], INK, 2.0);
        assert!(it.near(100.0, 25.0, 2.0));
        assert!(!it.near(50.0, 25.0, 2.0));
        it.rot(std::f32::consts::FRAC_PI_2, 50.0, 25.0);
        assert!(it.near(50.0, 75.0, 2.0));
        it.xf(2.0, 50.0, 25.0, 10.0, 0.0);
        assert_eq!(it.width, 4.0);
        assert_eq!(it.to_strokes().len(), 1);
    }

    #[test]
    fn xf_axis_ellipse_und_poly() {
        let mut e = Item::shape(ELLIPSE, vec![0.0, 0.0, 100.0, 50.0], INK, 2.0);
        e.xf_axis(2.0, 1.0, 0.0, 0.0, 0.0);
        assert_eq!(e.as_shape().unwrap().q, vec![0.0, 0.0, 200.0, 50.0]);
        assert_eq!(e.width, 2.0);
        // um 90 Grad gedrehter Rahmen: sx wirkt auf die Hoehe
        let mut r = Item::shape(ELLIPSE, vec![0.0, 0.0, 100.0, 50.0], INK, 2.0);
        r.xf_axis(1.0, 2.0, 0.0, 0.0, 0.0);
        assert_eq!(r.as_shape().unwrap().q, vec![0.0, 0.0, 100.0, 100.0]);
        let mut p = Item::shape(POLY, vec![0.0, 0.0, 10.0, 0.0, 10.0, 10.0, 0.0, 10.0], INK, 2.0);
        p.xf_axis(3.0, 1.0, 0.0, 0.0, 0.0);
        assert!(p.bounds().width() > 29.0);
        assert!(is_linear(ARROW) && is_linear(8) && !is_linear(ELLIPSE));
    }

    #[test]
    fn arrow_eng_angle_and_length() {
        for w in [0.5f32, 1.0, 2.0, 4.0] {
            let h = arrow_eng_head(0.0, 0.0, 100.0, 40.0, w);
            let (tip, l, r) = ((h[0] as f64, h[1] as f64), (h[2] as f64, h[3] as f64), (h[4] as f64, h[5] as f64));
            let (vl, vr) = ((l.0 - tip.0, l.1 - tip.1), (r.0 - tip.0, r.1 - tip.1));
            let dot = vl.0 * vr.0 + vl.1 * vr.1;
            let cos = dot / ((vl.0.hypot(vl.1)) * (vr.0.hypot(vr.1)));
            let deg = cos.acos().to_degrees();
            assert!((deg - 15.0).abs() < 0.1, "w={w}: Winkel {deg}");
            // Spitzenlaenge: Abstand Spitze -> Basismitte
            let base = ((l.0 + r.0) / 2.0, (l.1 + r.1) / 2.0);
            let len = (tip.0 - base.0).hypot(tip.1 - base.1);
            assert!((len - arrow_eng_len(w) as f64).abs() < 1e-3, "w={w}: Laenge {len}");
        }
    }

    #[test]
    fn arrow_eng_length_rule() {
        assert_eq!(arrow_eng_len(0.5), 12.0);
        assert_eq!(arrow_eng_len(1.0), 12.0);
        assert_eq!(arrow_eng_len(1.2), 12.0);
        assert_eq!(arrow_eng_len(2.0), 20.0);
        assert_eq!(arrow_eng_len(3.5), 35.0);
        // degenerierte Linie: keine NaN-Punkte
        let h = arrow_eng_head(5.0, 5.0, 5.0, 5.0, 2.0);
        assert!(h.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn arrow_eng_json_roundtrip() {
        let it = Item::shape(ARROW_ENG, vec![1.0, 2.0, 30.5, 40.25], 0xFF0000FF, 2.0);
        let j = it.json();
        assert_eq!(j["k"], json!(ARROW_ENG));
        assert_eq!(j["t"], json!("g"));
        let again = Item::from_json(&j).unwrap();
        assert_eq!(again.as_shape().unwrap().kind, ARROW_ENG);
        assert_eq!(again.json(), j);
        // Datei-Eintrag wie von der Java-Version/anderen Clients geschrieben
        let v: Value = serde_json::from_str(r#"{"t":"g","c":-16776961,"w":2,"k":6,"h":false,"q":[0,0,50,0],"ly":1}"#).unwrap();
        let it = Item::from_json(&v).unwrap();
        assert_eq!(it.as_shape().unwrap().kind, ARROW_ENG);
        assert_eq!(it.as_shape().unwrap().q, vec![0.0, 0.0, 50.0, 0.0]);
    }

    #[test]
    fn arrow_eng_render_and_hit() {
        // Pfeil horizontal nach rechts, Spitze bei x = 50, Linienbreite 2 -> Spitzenlaenge 20
        let it = Item::shape(ARROW_ENG, vec![5.0, 30.0, 50.0, 30.0], 0xFFFF0000, 2.0);
        let mut pm = Pixmap::new(60, 60).unwrap();
        {
            let mut c = Canvas::new(&mut pm);
            it.draw(&mut c, false);
        }
        let alpha = |x: u32, y: u32| pm.pixel(x, y).unwrap().alpha();
        // Mitte der Spitze (innerhalb des Dreiecks)
        assert!(alpha(41, 30) > 200, "Spitze nicht gefaerbt");
        // Schaft
        assert!(alpha(15, 30) > 200, "Schaft nicht gefaerbt");
        // knapp neben der Spitze, ausserhalb des Dreiecks
        assert_eq!(alpha(45, 27), 0, "Pixel neben der Spitze gefaerbt");
        assert_eq!(alpha(51, 30), 0, "Pixel hinter der Spitze gefaerbt");
        // Trefferpruefung: nahe Spitze und Schaft
        assert!(it.hit(45.0, 30.0, 1.0));
        assert!(!it.hit(45.0, 20.0, 1.0));
    }

    #[test]
    fn arrow_eng_strokes_and_bounds() {
        let it = Item::shape(ARROW_ENG, vec![0.0, 0.0, 100.0, 0.0], INK, 2.0);
        let strokes = it.to_strokes();
        assert_eq!(strokes.len(), 2);
        // Schaft endet an der Basis, nicht in der Spitze
        let shaft = strokes[0].as_stroke().unwrap();
        assert!(shaft.p[3] < 100.0 - 15.0);
        let b = it.bounds();
        assert!(b.right >= 100.0 + 2.0 && b.left <= 0.0 && b.top < -1.0 && b.bottom > 1.0);
    }
}

#[cfg(test)]
mod extra_tests {
    use super::*;
    #[test]
    fn unknown_fields_survive() {
        let v = json!({"t":"s","c":-1,"w":2,"p":[0,0,0.5,1,1,0.5],"xa":{"style":"dash"},"page":1});
        let it = Item::from_json(&v).unwrap();
        let o = it.json();
        assert_eq!(o["xa"]["style"], "dash");
        assert_eq!(o["page"], 1);
    }

    #[test]
    fn pfeilstile_umwandeln() {
        assert_eq!(styled_arrow(ARROW, 0), ARROW);
        assert_eq!(styled_arrow(ARROW2, 0), ARROW2);
        assert_eq!(styled_arrow(ARROW, 2), ARROW_FILLED);
        assert_eq!(styled_arrow(ARROW2, 2), ARROW2_FILLED);
        assert_eq!(styled_arrow(ARROW, 3), ARROW_ENG);
        assert_eq!(styled_arrow(ARROW, 4), ARROW2);
        assert_eq!(styled_arrow(ARROW2, 6), ARROW2_DASHED);
        assert_eq!(styled_arrow(LINE, 2), LINE);
        assert_eq!(styled_arrow(ARROW, 99), ARROW);
    }

    #[test]
    fn neue_pfeile_json_und_striche() {
        for k in [ARROW_FILLED, ARROW2_FILLED, ARROW_DASHED, ARROW2_DASHED] {
            let it = Item::shape(k, vec![0.0, 0.0, 100.0, 0.0], INK, 2.0);
            let again = Item::from_json(&it.json()).unwrap();
            assert_eq!(again.as_shape().unwrap().kind, k);
            assert!(!it.to_strokes().is_empty());
            assert!(it.hit(50.0, 0.0, 2.0));
        }
        let f = Item::shape(ARROW2_FILLED, vec![0.0, 0.0, 100.0, 0.0], INK, 2.0);
        assert_eq!(f.filled_heads().len(), 2);
        assert!(f.to_strokes().len() == 3);
        let d = Item::shape(ARROW_DASHED, vec![0.0, 0.0, 100.0, 0.0], INK, 2.0);
        assert!(d.to_strokes().len() > 4, "gestrichelter Schaft in Stücken");
        assert!(d.filled_heads().is_empty());
    }

}
