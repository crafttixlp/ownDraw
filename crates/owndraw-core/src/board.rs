//! Unendliche Zeichenflaeche mit Stift-, Gesten- und Auswahl-Logik (Gegenstueck zu BoardView.java).
//!
//! Die Oberflaeche liefert Zeiger-Ereignisse ([`Motion`]) in logischen Pixeln, ruft regelmaessig [`Board::tick`] auf
//! und zeichnet mit [`Board::render`] in ein Pixmap mit Geraete-Pixeln (logisch × `px_scale`).
//! Rueckmeldungen an die Oberflaeche landen in [`Board::events`].

use crate::canvas::{Canvas, Cap, Paint, Style};
use crate::geom::{hypot, RectF};
use crate::icons;
use crate::item::{self, is_linear, Item, Kind, INK};
use crate::layer::Layer;
use crate::pages::{BoardMode, PageBg, PageSetup};
use crate::recognizer;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use tiny_skia::{PathBuilder, Pixmap};

pub const PEN: i32 = 0;
pub const MARKER: i32 = 1;
pub const ERASER: i32 = 2;
pub const SELECT: i32 = 3;
pub const SHAPE: i32 = 4;
pub const NOTE: i32 = 5;
pub const TEXT: i32 = 6;
pub const ACCENT: u32 = 0xFF7C8CFF;
/// Linienabstand des Seitenhintergrunds (liniert/kariert/punktiert) in pt.
pub const PAGE_LINE: f32 = 24.0;

/// Rueckmeldungen an die Oberflaeche (entspricht BoardView.Host).
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Changed,
    AskText {
        wx: f32,
        wy: f32,
        note: bool,
    },
    /// Text/Notiz direkt auf dem Blatt bearbeiten (erneutes Tippen auf eine ausgewaehlte Notiz).
    EditNote(u64),
    /// Kurzer Hinweis (Toast).
    Notice(String),
    /// Haptische Rueckmeldung (Form erkannt).
    Haptic,
}

/// Rechteck in Geraete-Pixeln.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct IRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl IRect {
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }
    pub fn area(&self) -> u64 {
        self.w as u64 * self.h as u64
    }
    pub fn union(self, o: IRect) -> IRect {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return self;
        }
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        let (r, b) = (
            (self.x + self.w).max(o.x + o.w),
            (self.y + self.h).max(o.y + o.h),
        );
        IRect {
            x,
            y,
            w: r - x,
            h: b - y,
        }
    }
    /// Rechteck aus Gleitkomma-Kanten (Geraete-Pixel), nach aussen gerundet und auf w × h beschnitten.
    fn cover(l: f32, t: f32, r: f32, b: f32, w: u32, h: u32) -> IRect {
        if !(l.is_finite() && t.is_finite() && r.is_finite() && b.is_finite()) {
            return IRect { x: 0, y: 0, w, h };
        }
        let x0 = l.floor().clamp(0.0, w as f32) as u32;
        let y0 = t.floor().clamp(0.0, h as f32) as u32;
        let x1 = r.ceil().clamp(0.0, w as f32) as u32;
        let y1 = b.ceil().clamp(0.0, h as f32) as u32;
        if x1 <= x0 || y1 <= y0 {
            return IRect::default();
        }
        IRect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }
}

/// Kleine Menge von Rechtecken (z. B. zwei Streifen beim Verschieben), die nur zusammengelegt werden, wenn
/// dabei wenig unveraenderte Flaeche hinzukommt.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Damage(Vec<IRect>);

impl Damage {
    pub fn rects(&self) -> &[IRect] {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn area(&self) -> u64 {
        self.0.iter().map(|r| r.area()).sum()
    }
    pub fn add(&mut self, r: IRect) {
        if r.is_empty() {
            return;
        }
        let mut r = r;
        loop {
            let hit = self.0.iter().position(|e| {
                let u = e.union(r);
                u.area() * 10 <= (e.area() + r.area()) * 13
            });
            match hit {
                Some(i) => r = self.0.swap_remove(i).union(r),
                None => break,
            }
        }
        self.0.push(r);
        if self.0.len() > 6 {
            let b = self.0.iter().fold(IRect::default(), |a, e| a.union(*e));
            self.0 = vec![b];
        }
    }
    fn shift(&mut self, dx: i32, dy: i32, w: u32, h: u32) {
        let v = std::mem::take(&mut self.0);
        for r in v {
            self.add(shift_rect(r, dx, dy, w, h));
        }
    }
}

/// Ergebnis von [`Board::render`]: was die Oberflaeche zur GPU laden und wie sie die Textur zeigen soll.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// Geaenderte Bereiche des Zielbilds in Ansichts-Koordinaten (leer = unveraendert); nur diese muessen neu
    /// hochgeladen werden (Lage in der Textur: [`torus_parts`] mit `org`).
    pub damage: Vec<IRect>,
    /// Waehrend Zoom-Gesten: die Textur (= Zwischenspeicher) um k skaliert an (dx, dy) zeigen, logische Pixel,
    /// Rest mit [`Board::backdrop_color`] fuellen. Kein Neuzeichnen auf der CPU.
    pub scaled: Option<(f32, f32, f32)>,
    /// Ursprung des Ring-Speichers (Zielbild = Textur), siehe [`torus_parts`].
    pub org: (u32, u32),
}

/// Alles ausser den Objekten und der Verschiebung, was den Zwischenspeicher bestimmt.
#[derive(Clone, Debug, PartialEq)]
struct CacheSig {
    size: (u32, u32, u32),
    sc: u32,
    dark: bool,
    bg: i32,
    layers: Vec<(i32, bool, u32)>,
    pages: Vec<(u32, u32, PageBg)>,
}

/// Zerlegt ein Rechteck der Ansicht in die (hoechstens vier) Stuecke im Ring-Speicher der Groesse w × h mit
/// Ursprung org: Ansichts-Pixel (x, y) liegt bei ((x + org.0) mod w, (y + org.1) mod h). Ergebnis: (Ansicht, Speicher).
/// Beim Verschieben wandert nur der Ursprung; Zwischenspeicher und Textur muessen nicht umkopiert werden.
pub fn torus_parts(r: IRect, org: (u32, u32), w: u32, h: u32) -> Vec<(IRect, IRect)> {
    let split = |v: u32, n: u32, o: u32, size: u32| -> Vec<(u32, u32, u32)> {
        let p = (v + o) % size;
        if p + n <= size {
            vec![(v, p, n)]
        } else {
            let a = size - p;
            vec![(v, p, a), (v + a, 0, n - a)]
        }
    };
    let mut out = Vec::with_capacity(4);
    if r.is_empty() || w == 0 || h == 0 {
        return out;
    }
    for (vy, py, nh) in split(r.y, r.h, org.1, h) {
        for &(vx, px, nw) in &split(r.x, r.w, org.0, w) {
            out.push((
                IRect {
                    x: vx,
                    y: vy,
                    w: nw,
                    h: nh,
                },
                IRect {
                    x: px,
                    y: py,
                    w: nw,
                    h: nh,
                },
            ));
        }
    }
    out
}

/// Kopiert src (Groesse von r) in den Ring-Speicher dst an die Ansichtsstelle r.
fn blit(dst: &mut Pixmap, src: &Pixmap, r: IRect, org: (u32, u32)) {
    let (dw, dh, sw) = (dst.width(), dst.height(), src.width() as usize);
    let d = dst.data_mut();
    let s = src.data();
    for (v, p) in torus_parts(r, org, dw, dh) {
        let n = v.w as usize * 4;
        for row in 0..v.h as usize {
            let so = ((v.y - r.y) as usize + row) * sw * 4 + (v.x - r.x) as usize * 4;
            let o = ((p.y as usize + row) * dw as usize + p.x as usize) * 4;
            d[o..o + n].copy_from_slice(&s[so..so + n]);
        }
    }
}

/// Kopiert den Ansichtsbereich r zwischen zwei Ring-Speichern gleicher Groesse und gleichen Ursprungs.
fn copy_rect(dst: &mut Pixmap, src: &Pixmap, r: IRect, org: (u32, u32)) {
    let (w, h) = (src.width(), src.height());
    let (s, d) = (src.data(), dst.data_mut());
    for (_, p) in torus_parts(r, org, w, h) {
        for row in p.y as usize..(p.y + p.h) as usize {
            let o = (row * w as usize + p.x as usize) * 4;
            d[o..o + p.w as usize * 4].copy_from_slice(&s[o..o + p.w as usize * 4]);
        }
    }
}

/// Schneidet den Ansichtsbereich r aus dem Ring-Speicher src aus.
fn crop(src: &Pixmap, r: IRect, org: (u32, u32)) -> Option<Pixmap> {
    let mut out = Pixmap::new(r.w, r.h)?;
    let (w, h) = (src.width(), src.height());
    let s = src.data();
    let ow = r.w as usize * 4;
    let d = out.data_mut();
    for (v, p) in torus_parts(r, org, w, h) {
        let n = v.w as usize * 4;
        for row in 0..v.h as usize {
            let o = ((p.y as usize + row) * w as usize + p.x as usize) * 4;
            let doff = ((v.y - r.y) as usize + row) * ow + (v.x - r.x) as usize * 4;
            d[doff..doff + n].copy_from_slice(&s[o..o + n]);
        }
    }
    Some(out)
}

fn intersects(a: IRect, b: IRect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Verschiebt ein Ansichtsrechteck und beschneidet es auf w × h.
fn shift_rect(r: IRect, dx: i32, dy: i32, w: u32, h: u32) -> IRect {
    if r.is_empty() {
        return r;
    }
    IRect::cover(
        (r.x as i32 + dx) as f32,
        (r.y as i32 + dy) as f32,
        (r.x as i32 + r.w as i32 + dx) as f32,
        (r.y as i32 + r.h as i32 + dy) as f32,
        w,
        h,
    )
}

/// Zeichenhuelle eines Objekts in Weltkoordinaten (Huelle plus Schatten/Kantenglaettung).
fn paint_bounds(it: &Item) -> RectF {
    let mut b = it.bounds();
    let pad = match &it.kind {
        Kind::Note(n) => n.size * 0.35,
        _ => it.width * 0.5,
    };
    b.inset(-pad, -pad);
    b
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolType {
    Finger,
    Stylus,
    Eraser,
    Mouse,
}

#[derive(Clone, Copy, Debug)]
pub struct Pointer {
    pub id: i64,
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
    pub tool: ToolType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Down,
    /// Weiterer Finger/Stift; Index in `pointers`.
    PointerDown(usize),
    Move,
    /// Ein Zeiger von mehreren hebt ab; Index in `pointers`.
    PointerUp(usize),
    Up,
    Cancel,
}

/// Ein Zeiger-Ereignis (Gegenstueck zu android.view.MotionEvent). `history` enthaelt optionale
/// Zwischenpunkte (x, y, Druck) des aktiven Zeigers vor der aktuellen Position.
#[derive(Clone, Debug)]
pub struct Motion {
    pub action: Action,
    pub pointers: Vec<Pointer>,
    pub history: Vec<(f32, f32, f32)>,
    /// Stift-Seitentaste gedrueckt (radiert).
    pub stylus_button: bool,
}

impl Motion {
    pub fn single(action: Action, x: f32, y: f32, pressure: f32, tool: ToolType) -> Motion {
        Motion {
            action,
            pointers: vec![Pointer {
                id: 0,
                x,
                y,
                pressure,
                tool,
            }],
            history: Vec::new(),
            stylus_button: false,
        }
    }
    fn find(&self, id: i64) -> Option<usize> {
        self.pointers.iter().position(|p| p.id == id)
    }
}

/// Undo-Schnappschuss: Objekte plus Ebenen-Struktur.
#[derive(Clone)]
pub struct Snap {
    items: Vec<Rc<Item>>,
    layers: Vec<Layer>,
    active: usize,
    /// Seiten-Einstellungen (Seitenmodus: Seiten hinzufuegen/loeschen/verschieben ist rueckgaengig machbar).
    mode: BoardMode,
}

thread_local! {
    /// Zwischenablage (Kopien), ueberlebt Board-Wechsel.
    static CLIP: RefCell<(Vec<Item>, f32, f32, i32)> = const { RefCell::new((Vec::new(), 0.0, 0.0, 0)) };
}

pub struct Board {
    pub items: Vec<Rc<Item>>,
    pub layers: Vec<Layer>,
    /// Index der aktiven Ebene in layers (0 = ganz hinten).
    pub active: usize,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    /// Ausgewaehlte Objekte (Laufzeit-ids).
    pub sel: Vec<u64>,
    pub sc: f32,
    pub tx: f32,
    pub ty: f32,
    pub pen_w: f32,
    pub dp: f32,
    pub tool: i32,
    pub color: u32,
    pub shape_kind: i32,
    /// Gewählter Pfeilstil (Index in `item::ARROW_STYLES`); erkannte Pfeile werden darin umgewandelt.
    pub arrow_style: i32,
    /// Formerkennung: 0 = Stift am Ende kurz halten, 1 = immer, 2 = aus.
    pub recog: i32,
    /// Hintergrund: 0 Punkte, 1 Raster, 2 Linien, 3 leer.
    pub bg: i32,
    pub dark: bool,
    pub finger_draw: bool,
    pub modified: bool,
    /// Auswahl-Modus: 0 = Lasso, 1 = Rechteck.
    pub sel_mode: i32,
    /// Radierer-Modus: 0 = ganze Objekte, 1 = nur ueberstrichene Stellen.
    pub erase_mode: i32,
    pub events: Vec<Event>,
    /// Objekt, das gerade direkt auf dem Blatt bearbeitet wird (wird nicht gezeichnet).
    pub hidden: Option<u64>,
    /// Werkzeug, zu dem nach dem Abwaehlen zurueckgekehrt wird (Text antippen, mittlere Maustaste).
    back_tool: Option<i32>,
    /// Unendlich oder Seiten (WP2).
    pub board_mode: BoardMode,
    /// Neu zeichnen noetig (invalidate).
    pub needs_redraw: bool,

    width: f32,
    height: f32,
    px_scale: f32,

    cache: Option<Pixmap>,
    dirty: bool,
    /// Zwischenspeicher sicher komplett neu zeichnen (Groesse, Laden).
    full_dirty: bool,
    /// Objekte (ohne angehobene) und Zustand, mit denen der Zwischenspeicher gezeichnet wurde (fuer Teil-Updates).
    cache_items: Vec<Rc<Item>>,
    cache_sig: Option<CacheSig>,
    /// Seit dem letzten Bild geaenderter Bereich des Zwischenspeichers.
    cache_dmg: Damage,
    /// Zielbild des letzten render() (Adresse, Groesse) und dort gezeichnete Ueberlagerungen.
    shown: Option<(usize, u32, u32)>,
    last_ov: IRect,
    /// Ursprung des Ring-Speichers von Zwischenspeicher und Zielbild.
    org: (u32, u32),
    /// Ansicht des Zwischenspeichers (Zoom, Verschiebung); kann waehrend Gesten von sc/tx/ty abweichen.
    csc: f32,
    ctx: f32,
    cty: f32,
    /// Mausrad-Zoom laeuft (wird wie eine Finger-Geste mit skaliertem Cache gezeichnet).
    wheel_until: Option<f64>,
    now: f64,

    mode: i32,
    act_id: i64,
    pan_id: i64,
    drag: i32,
    stylus_act: bool,
    zinit: bool,
    erasing: bool,
    erased: bool,
    moved: bool,
    lifted: bool,
    blocked: bool,
    lx: f32,
    ly: f32,
    span: f32,
    lfx: f32,
    lfy: f32,
    dsx: f32,
    dsy: f32,
    sx: f32,
    sy: f32,
    lwx: f32,
    lwy: f32,
    ex: f32,
    ey: f32,
    last_pr: f32,
    hold_x: f32,
    hold_y: f32,
    hold_at: Option<f64>,
    pvx: f32,
    pvy: f32,
    d0: f32,
    cur: Option<Item>,
    snap: Option<Item>,
    prev: Option<Item>,
    marq: Option<RectF>,
    /// Auswahlrahmen: Mittelpunkt, ungedrehte Groesse, Drehung (Bogenmass, im Uhrzeigersinn).
    pub sel_cx: f32,
    pub sel_cy: f32,
    pub sel_w: f32,
    pub sel_h: f32,
    pub sel_ang: f32,
    frame_ok: bool,
    lasso: Vec<f32>,
    pend: Option<u64>,
    prev_sel: Option<Vec<u64>>,
    prev_frame: Option<[f32; 5]>,
    g_a0: f32,
    g_ang0: f32,
    g_scale: f32,
    g_ux: f32,
    g_uy: f32,
    /// Kanten-Griff (0 oben, 1 rechts, 2 unten, 3 links) beim Ziehen mit drag == 7.
    g_edge: usize,
}

impl Default for Board {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl Board {
    /// dp = Bildschirmdichte (Android: density; Desktop: 1).
    pub fn new(dp: f32) -> Board {
        Board {
            items: Vec::new(),
            layers: vec![Layer::new(1, "01-ebene")],
            active: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            sel: Vec::new(),
            sc: 1.0,
            tx: 0.0,
            ty: 0.0,
            pen_w: 3.0,
            dp,
            tool: PEN,
            color: INK,
            shape_kind: 0,
            arrow_style: 0,
            recog: 0,
            bg: 0,
            dark: false,
            finger_draw: false,
            modified: false,
            sel_mode: 0,
            erase_mode: 0,
            events: Vec::new(),
            hidden: None,
            back_tool: None,
            board_mode: BoardMode::Infinite,
            needs_redraw: true,
            width: 0.0,
            height: 0.0,
            px_scale: 1.0,
            cache: None,
            dirty: true,
            full_dirty: true,
            cache_items: Vec::new(),
            cache_sig: None,
            cache_dmg: Damage::default(),
            shown: None,
            last_ov: IRect::default(),
            org: (0, 0),
            csc: 1.0,
            ctx: 0.0,
            cty: 0.0,
            wheel_until: None,
            now: 0.0,
            mode: 0,
            act_id: 0,
            pan_id: 0,
            drag: 0,
            stylus_act: false,
            zinit: false,
            erasing: false,
            erased: false,
            moved: false,
            lifted: false,
            blocked: false,
            lx: 0.0,
            ly: 0.0,
            span: 1.0,
            lfx: 0.0,
            lfy: 0.0,
            dsx: 0.0,
            dsy: 0.0,
            sx: 0.0,
            sy: 0.0,
            lwx: 0.0,
            lwy: 0.0,
            ex: -1.0,
            ey: 0.0,
            last_pr: 0.5,
            hold_x: 0.0,
            hold_y: 0.0,
            hold_at: None,
            pvx: 0.0,
            pvy: 0.0,
            d0: 1.0,
            cur: None,
            snap: None,
            prev: None,
            marq: None,
            sel_cx: 0.0,
            sel_cy: 0.0,
            sel_w: 0.0,
            sel_h: 0.0,
            sel_ang: 0.0,
            frame_ok: false,
            lasso: Vec::new(),
            pend: None,
            prev_sel: None,
            prev_frame: None,
            g_a0: 0.0,
            g_ang0: 0.0,
            g_scale: 1.0,
            g_ux: 1.0,
            g_uy: 0.0,
            g_edge: 0,
        }
    }

    fn changed(&mut self) {
        if self.events.last() != Some(&Event::Changed) {
            self.events.push(Event::Changed);
        }
    }
    fn notice(&mut self, s: &str) {
        self.events.push(Event::Notice(s.to_string()));
    }
    pub fn invalidate(&mut self) {
        self.needs_redraw = true;
    }
    pub fn refresh(&mut self) {
        self.dirty = true;
        self.invalidate();
    }

    /// Groesse der Ansicht in logischen Pixeln und Geraete-Pixel je logischem Pixel.
    pub fn set_size(&mut self, w: f32, h: f32, px_scale: f32) {
        if w != self.width || h != self.height || px_scale != self.px_scale {
            self.width = w;
            self.height = h;
            self.px_scale = px_scale;
            self.cache = None;
            self.full_dirty = true;
            self.refresh();
        }
    }
    pub fn width(&self) -> f32 {
        self.width
    }
    pub fn height(&self) -> f32 {
        self.height
    }
    /// Laeuft gerade ein Strich/eine Geste?
    pub fn busy(&self) -> bool {
        self.mode != 0
    }

    fn idx_of(&self, id: u64) -> Option<usize> {
        self.items.iter().position(|it| it.id == id)
    }
    fn sel_set(&self) -> HashSet<u64> {
        self.sel.iter().copied().collect()
    }
    /// Die ausgewaehlten Objekte in Auswahl-Reihenfolge.
    pub fn sel_items(&self) -> Vec<&Item> {
        let pos: HashMap<u64, usize> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| (it.id, i))
            .collect();
        self.sel
            .iter()
            .filter_map(|id| pos.get(id).map(|i| &*self.items[*i]))
            .collect()
    }
    /// Wendet f auf alle ausgewaehlten Objekte an (Kopie bei Bedarf, Undo-Schnappschuesse bleiben unveraendert).
    fn sel_mut(&mut self, mut f: impl FnMut(&mut Item)) {
        let set = self.sel_set();
        for it in self.items.iter_mut() {
            if set.contains(&it.id) {
                f(Rc::make_mut(it));
            }
        }
    }

    // ---------- Ebenen ----------

    pub fn active_layer(&mut self) -> &Layer {
        if self.active >= self.layers.len() {
            self.active = self.layers.len().saturating_sub(1);
        }
        &self.layers[self.active]
    }

    fn max_layer_id(&self) -> i32 {
        self.layers.iter().map(|l| l.id).max().unwrap_or(0)
    }

    /// Index der Ebene mit dieser id; unbekannte ids landen auf der untersten Ebene.
    pub fn layer_idx(&self, id: i32) -> usize {
        self.layers.iter().position(|l| l.id == id).unwrap_or(0)
    }

    fn usable(&self, it: &Item) -> bool {
        let l = &self.layers[self.layer_idx(it.layer)];
        l.visible && !l.locked
    }

    pub fn add_layer(&mut self, name: Option<&str>) {
        self.checkpoint();
        let l = Layer::new(self.max_layer_id() + 1, name.unwrap_or("Ebene"));
        let at = (self.active + 1).min(self.layers.len());
        self.layers.insert(at, l);
        self.active = at;
        self.sel_clear();
        self.layers_changed();
    }

    pub fn remove_layer(&mut self, idx: usize) {
        if idx >= self.layers.len() || self.layers.len() <= 1 {
            return;
        }
        self.checkpoint();
        let id = self.layers[idx].id;
        let orphan_safe = idx != 0;
        let keep: Vec<bool> = self
            .items
            .iter()
            .map(|it| !(it.layer == id || (!orphan_safe && self.layer_idx(it.layer) == 0)))
            .collect();
        let mut k = keep.iter();
        self.items.retain(|_| *k.next().unwrap_or(&true));
        self.layers.remove(idx);
        if self.active > idx {
            self.active -= 1;
        } else if self.active == idx {
            self.active = idx.saturating_sub(1);
        }
        self.active = self.active.min(self.layers.len() - 1);
        self.sel_clear();
        self.layers_changed();
    }

    pub fn move_layer(&mut self, from: usize, to: usize) {
        let n = self.layers.len();
        if from >= n || to >= n || from == to {
            return;
        }
        self.checkpoint();
        let act = self.layers[self.active].id;
        let l = self.layers.remove(from);
        self.layers.insert(to, l);
        self.active = self.layers.iter().position(|l| l.id == act).unwrap_or(0);
        self.layers_changed();
    }

    pub fn set_active_layer(&mut self, idx: usize) {
        if idx >= self.layers.len() {
            return;
        }
        self.active = idx;
        self.sel_clear();
        self.invalidate();
        self.changed();
    }

    /// Aendert Eigenschaften einer Ebene (Name, Sichtbarkeit, Sperre, Animation ...) ohne Undo-Schritt.
    pub fn edit_layer(&mut self, idx: usize, f: impl FnOnce(&mut Layer)) {
        if let Some(l) = self.layers.get_mut(idx) {
            f(l);
            self.layers_changed();
        }
    }

    pub fn apply_template(&mut self) {
        self.checkpoint();
        let t = Layer::template(self.max_layer_id() + 1);
        let replace = self.items.is_empty() && self.layers.len() == 1;
        let first = if replace {
            self.layers.clear();
            0
        } else {
            self.layers.len()
        };
        self.layers.extend(t);
        self.active = first;
        for i in first..self.layers.len() {
            if self.layers[i].name == "05-figur" {
                self.active = i;
                break;
            }
        }
        self.sel_clear();
        self.layers_changed();
    }

    pub fn layers_changed(&mut self) {
        let unusable: HashSet<u64> = self
            .sel_items()
            .iter()
            .filter(|it| !self.usable(it))
            .map(|it| it.id)
            .collect();
        if !unusable.is_empty() {
            self.sel.retain(|id| !unusable.contains(id));
            self.frame_ok = false;
        }
        self.modified = true;
        self.refresh();
        self.changed();
    }

    fn layer_blocked(&mut self) -> bool {
        let (vis, locked) = {
            let l = self.active_layer();
            (l.visible, l.locked)
        };
        if !vis {
            self.notice("Ebene ist ausgeblendet");
            return true;
        }
        if locked {
            self.notice("Ebene ist gesperrt");
            return true;
        }
        false
    }

    /// Fuegt ein bereits im Bildordner gespeichertes Bild (Dateiname, Pixelgroesse) mittig ein.
    pub fn add_image(&mut self, file: &str, pw: u32, ph: u32) -> bool {
        self.add_image_at(file, pw, ph, None)
    }

    /// Wie `add_image`, aber mit Mittelpunkt `at` (Weltkoordinaten); im Seitenmodus auf der Seite unter `at`.
    pub fn add_image_at(&mut self, file: &str, pw: u32, ph: u32, at: Option<(f32, f32)>) -> bool {
        if self.layer_blocked() || pw == 0 || ph == 0 {
            return false;
        }
        let w = if self.width > 0.0 { self.width } else { 1000.0 };
        let h = if self.height > 0.0 {
            self.height
        } else {
            1000.0
        };
        let (vw, vh) = (w / self.sc, h / self.sc);
        let s = (1.0 / self.sc).min((0.6 * vw / pw as f32).min(0.6 * vh / ph as f32));
        let (iw, ih) = (pw as f32 * s, ph as f32 * s);
        let (cx, cy) = self.paste_center(at);
        let it = Item::new(Kind::Img(item::Img {
            x: cx - iw / 2.0,
            y: cy - ih / 2.0,
            w: iw,
            h: ih,
            ang: 0.0,
            file: file.to_string(),
        }));
        let id = it.id;
        self.add_top(it);
        self.set_tool(SELECT);
        self.sel_clear();
        self.sel.push(id);
        self.frame_ok = false;
        self.refresh();
        self.changed();
        true
    }

    /// Huelle aller sichtbaren Objekte; im Seitenmodus die Huelle aller Seiten.
    pub fn export_bounds(&self) -> Option<RectF> {
        if let BoardMode::Pages(ps) = &self.board_mode {
            return ps.bounds();
        }
        let mut b: Option<RectF> = None;
        for it in &self.items {
            if !self.layers[self.layer_idx(it.layer)].visible {
                continue;
            }
            match b.as_mut() {
                None => b = Some(it.bounds()),
                Some(r) => r.union(&it.bounds()),
            }
        }
        b
    }

    /// Nur die Objekte einer Ebene (ohne Hintergrund) im Ausschnitt b, Massstab k.
    pub fn render_layer(&self, l: &Layer, b: &RectF, k: f32) -> Option<Pixmap> {
        let mut bm = Pixmap::new(
            ((b.width() * k).round() as u32).max(1),
            ((b.height() * k).round() as u32).max(1),
        )?;
        {
            let mut c = Canvas::new(&mut bm);
            c.scale(k, k);
            c.translate(-b.left, -b.top);
            self.clip_pages(&mut c);
            let li = self.layers.iter().position(|x| x == l);
            for it in &self.items {
                let x = self.layer_idx(it.layer);
                if Some(x) == li || (li.is_none() && it.layer == l.id) {
                    it.draw(&mut c, self.dark);
                }
            }
        }
        Some(bm)
    }

    pub fn render_composite(&self, b: &RectF, k: f32, with_bg: bool) -> Option<Pixmap> {
        let mut bm = Pixmap::new(
            ((b.width() * k).round() as u32).max(1),
            ((b.height() * k).round() as u32).max(1),
        )?;
        {
            let mut c = Canvas::new(&mut bm);
            if with_bg {
                c.draw_color(self.base_color());
            }
            c.scale(k, k);
            c.translate(-b.left, -b.top);
            // nur Objekte im Bereich (Seitenvorschau: nicht alle Objekte aller Seiten)
            let mut v = *b;
            v.inset(-4.0 / k, -4.0 / k);
            self.draw_scene(&mut c, Some(&v), false, with_bg, 1.0 / k);
        }
        Some(bm)
    }

    /// Schluessel fuer den Inhalt im Weltrechteck r (aendert sich, wenn sich dort ein Objekt, die Ebenen oder die
    /// Darstellung aendern). Fuer Vorschaubilder, die nur bei Aenderung neu gezeichnet werden.
    pub fn content_key(&self, r: &RectF) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (
            self.dark,
            self.bg,
            r.left.to_bits(),
            r.top.to_bits(),
            r.right.to_bits(),
            r.bottom.to_bits(),
        )
            .hash(&mut h);
        for l in &self.layers {
            (l.id, l.visible, l.opacity.to_bits()).hash(&mut h);
        }
        if let Some(p) = self.pages() {
            for g in &p.pages {
                (g.w.to_bits(), g.h.to_bits(), g.bg as u8).hash(&mut h);
            }
        }
        for it in &self.items {
            let b = paint_bounds(it);
            if RectF::intersects(r, &b) {
                (
                    Rc::as_ptr(it) as usize,
                    it.layer,
                    it.color,
                    b.left.to_bits(),
                    b.top.to_bits(),
                    b.right.to_bits(),
                    b.bottom.to_bits(),
                )
                    .hash(&mut h);
            }
        }
        h.finish()
    }

    /// Zeichnet alle sichtbaren Ebenen hinten nach vorne (Deckkraft per Ebene), vis = Sichtbereich (Welt) oder None.
    fn draw_layered(&self, c: &mut Canvas, vis: Option<&RectF>, skip_lift: bool) {
        let n = self.layers.len();
        let mut bk: Vec<Vec<&Item>> = vec![Vec::new(); n];
        let (mut last_id, mut last_k) = (i32::MIN, 0usize);
        for it in &self.items {
            if (skip_lift && it.lift) || self.hidden == Some(it.id) {
                continue;
            }
            if let Some(v) = vis {
                if !RectF::intersects(v, &paint_bounds(it)) {
                    continue;
                }
            }
            if it.layer != last_id {
                last_id = it.layer;
                last_k = self.layer_idx(last_id);
            }
            bk[last_k].push(it);
        }
        for (k, a) in bk.iter().enumerate() {
            let l = &self.layers[k];
            if !l.visible || a.is_empty() {
                continue;
            }
            let layered = l.opacity < 1.0;
            if layered {
                c.save_layer_alpha((l.opacity.max(0.0) * 255.0).round() as u8);
            }
            for it in a {
                it.draw(c, self.dark);
            }
            if layered {
                c.restore();
            }
        }
    }

    // ---------- Zustand ----------

    pub fn bg_color(&self) -> u32 {
        if self.dark {
            0xFF16171B
        } else {
            0xFFFBFBF8
        }
    }

    pub fn set_tool(&mut self, t: i32) {
        if t != SELECT {
            self.sel_clear();
        }
        self.back_tool = None;
        self.tool = t;
        self.invalidate();
    }

    pub fn set_color(&mut self, c: u32) {
        self.color = c;
        if self.sel.is_empty() || self.sel_items().iter().all(|it| it.is_img()) {
            return;
        }
        self.checkpoint();
        self.sel_mut(|it| {
            if it.is_img() {
                return;
            }
            it.color = c;
            if it.as_note().map(|n| n.bg != 0).unwrap_or(false) {
                it.set_note_bg(note_bg(c));
            }
        });
        self.refresh();
    }

    /// Gemeinsame Farbe der Auswahl (ohne Bilder); None ohne einfärbbare Auswahl oder bei
    /// gemischten Farben (dann `Some(None)`).
    pub fn sel_color(&self) -> Option<Option<u32>> {
        let mut it = self
            .sel_items()
            .into_iter()
            .filter(|it| !it.is_img())
            .map(|it| it.color);
        let first = it.next()?;
        Some(if it.all(|c| c == first) {
            Some(first)
        } else {
            None
        })
    }

    /// Strichstärke der Auswahl (ohne Bilder/Notizen) in Weltkoordinaten; None ohne passende Auswahl.
    pub fn sel_width(&self) -> Option<f32> {
        let w: Vec<f32> = self
            .sel_items()
            .iter()
            .filter(|it| !it.is_img() && it.as_note().is_none())
            .map(|it| it.width)
            .collect();
        if w.is_empty() {
            return None;
        }
        Some(w.iter().sum::<f32>() / w.len() as f32)
    }

    /// Setzt die Strichstärke der Auswahl. `undo_step` = true legt einen Rückgängig-Schritt an
    /// (beim Regler nur zu Beginn des Ziehens, nicht in jedem Frame).
    pub fn set_sel_width(&mut self, w: f32, undo_step: bool) {
        if self.sel_width().is_none() {
            return;
        }
        if undo_step {
            self.checkpoint();
        }
        let w = w.clamp(0.2, 200.0);
        self.sel_mut(|it| {
            if it.is_img() || it.as_note().is_some() {
                return;
            }
            it.width = w;
            it.touch();
        });
        self.frame_ok = false;
        self.ensure_frame();
        self.refresh();
    }

    fn checkpoint(&mut self) {
        let s = self.snapshot();
        self.undo.push(s);
        if self.undo.len() > 80 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.modified = true;
    }

    fn snapshot(&self) -> Snap {
        Snap {
            items: self.items.clone(),
            layers: self.layers.clone(),
            active: self.active,
            mode: self.board_mode.clone(),
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo(&mut self) {
        self.step(true);
    }
    pub fn redo(&mut self) {
        self.step(false);
    }

    fn step(&mut self, back: bool) {
        let from_empty = if back {
            self.undo.is_empty()
        } else {
            self.redo.is_empty()
        };
        if from_empty || self.mode != 0 {
            return;
        }
        let cur = self.snapshot();
        let sn = if back {
            self.redo.push(cur);
            self.undo.pop().unwrap()
        } else {
            self.undo.push(cur);
            self.redo.pop().unwrap()
        };
        self.items = sn.items;
        // Struktur aus dem Schnappschuss, Eigenschaften (Name, Sichtbarkeit ...) bleiben aktuell
        let nl: Vec<Layer> = sn
            .layers
            .iter()
            .map(|o| {
                self.layers
                    .iter()
                    .find(|l| l.id == o.id)
                    .cloned()
                    .unwrap_or_else(|| o.clone())
            })
            .collect();
        self.layers = nl;
        self.active = sn.active.min(self.layers.len() - 1);
        self.board_mode = sn.mode;
        self.sel_clear();
        self.modified = true;
        self.refresh();
        self.changed();
    }

    fn add_top(&mut self, mut it: Item) {
        self.checkpoint();
        it.layer = self.active_layer().id;
        let mut top = {
            let l = &self.layers[self.active];
            l.visible && l.opacity >= 1.0
        };
        for i in self.active + 1..self.layers.len() {
            if top && self.layers[i].visible {
                top = false;
            }
        }
        let ok = top
            && !self.dirty
            && self.cache.is_some()
            && self.csc == self.sc
            && self.cache_sig.is_some();
        let clip = if ok { self.pages_path() } else { None };
        let rc = Rc::new(it);
        if ok {
            let (ps, tx, ty, sc, dark, org) = (
                self.px_scale,
                self.ctx,
                self.cty,
                self.csc,
                self.dark,
                self.org,
            );
            let cache = self.cache.as_mut().unwrap();
            let (cw, ch) = (cache.width(), cache.height());
            let b = paint_bounds(&rc);
            let k = sc * ps;
            let d = IRect::cover(
                (b.left * sc + tx) * ps - 2.0,
                (b.top * sc + ty) * ps - 2.0,
                b.right * k + tx * ps + 2.0,
                b.bottom * k + ty * ps + 2.0,
                cw,
                ch,
            );
            if let Some(mut tmp) = crop(cache, d, org) {
                {
                    let mut c = Canvas::new(&mut tmp);
                    c.translate(-(d.x as f32), -(d.y as f32));
                    c.scale(ps, ps);
                    c.translate(tx, ty);
                    c.scale(sc, sc);
                    if let Some(p) = &clip {
                        c.clip_path(p);
                    }
                    rc.draw(&mut c, dark);
                }
                blit(cache, &tmp, d, org);
            }
            self.cache_dmg.add(d);
            self.cache_items.push(rc.clone());
        } else {
            self.dirty = true;
        }
        self.items.push(rc);
    }

    /// Rechteck (x, y, Schriftgroesse, Breite, Mindesthoehe) eines neuen Texts bzw. einer neuen Haftnotiz
    /// am Weltpunkt bei aktuellem Zoom.
    pub fn new_note_rect(&self, wx: f32, wy: f32, note: bool) -> (f32, f32, f32, f32, f32) {
        let size = 18.0 * self.dp / self.sc;
        if note {
            let w = 210.0 * self.dp / self.sc;
            let mh = w * 0.72;
            (wx - w / 2.0, wy - mh / 2.0, size, w, mh)
        } else {
            (wx, wy - size / 2.0, size, 0.0, 0.0)
        }
    }

    pub fn add_note(&mut self, wx: f32, wy: f32, text: &str, note: bool) {
        let r = self.new_note_rect(wx, wy, note);
        self.add_note_rect(text, r, note);
    }

    /// Fuegt Text/Haftnotiz mit festem Rechteck aus `new_note_rect` hinzu.
    pub fn add_note_rect(&mut self, text: &str, r: (f32, f32, f32, f32, f32), note: bool) {
        let (x, y, size, w, mh) = r;
        let bg = if note { note_bg(self.color) } else { 0 };
        let it = Item::note(text, self.color, size, x, y, w, mh, bg);
        self.add_top(it);
        self.invalidate();
        self.changed();
    }

    /// Die einzige ausgewaehlte Notiz, falls genau eine ausgewaehlt ist.
    pub fn single_note(&self) -> Option<&Item> {
        if self.sel.len() != 1 {
            return None;
        }
        self.sel_items()
            .into_iter()
            .next()
            .filter(|it| it.as_note().is_some())
    }

    pub fn edit_note(&mut self, id: u64, text: &str) {
        let Some(i) = self.idx_of(id) else { return };
        self.checkpoint();
        let mut n = (*self.items[i]).clone();
        n.set_text(text);
        self.items[i] = Rc::new(n);
        self.sel_clear();
        self.sel.push(id);
        self.frame_ok = false;
        self.refresh();
    }

    /// Blendet ein Objekt beim Bearbeiten auf dem Blatt aus (None = wieder zeigen).
    pub fn set_hidden(&mut self, id: Option<u64>) {
        if self.hidden != id {
            self.hidden = id;
            if id.is_some() {
                self.sel_clear();
            }
            self.refresh();
        }
    }

    /// Loescht ein Objekt (z. B. Text, der beim Bearbeiten leer wurde).
    pub fn delete_item(&mut self, id: u64) {
        let Some(i) = self.idx_of(id) else { return };
        self.checkpoint();
        self.items.remove(i);
        self.sel.retain(|s| *s != id);
        self.frame_ok = false;
        self.refresh();
        self.changed();
    }

    /// Waehlt das oberste Objekt unter dem Bildschirmpunkt aus und wechselt dafuer vorruebergehend
    /// zum Auswahlwerkzeug (Rueckkehr beim Abwaehlen). Liefert true, wenn ein Objekt getroffen wurde.
    pub fn select_at_screen(&mut self, x: f32, y: f32, notes_only: bool) -> bool {
        let (wx, wy) = self.screen_to_world(x, y);
        let r = (if notes_only { 6.0 } else { 10.0 }) * self.dp / self.sc;
        // Mittelklick: auch innerhalb ungefuellter Formen/Striche treffen (kleinster umschliessender Rahmen)
        let hit = self.top_at(wx, wy, r).or_else(|| {
            if notes_only {
                None
            } else {
                self.bounds_at(wx, wy)
            }
        });
        let Some(id) = hit else { return false };
        if notes_only
            && !self
                .items
                .iter()
                .any(|it| it.id == id && it.as_note().is_some())
        {
            return false;
        }
        if self.tool != SELECT {
            let back = self.back_tool.or(Some(self.tool));
            self.set_tool(SELECT);
            self.back_tool = back;
        }
        self.select_only(id);
        true
    }

    /// Waehlt genau dieses Objekt aus.
    pub fn select_only(&mut self, id: u64) {
        self.sel_clear();
        self.sel.push(id);
        self.ensure_frame();
        self.invalidate();
    }

    /// Hebt die Auswahl auf (und kehrt ggf. zum vorherigen Werkzeug zurueck).
    pub fn deselect(&mut self) {
        self.sel_clear();
        if let Some(t) = self.back_tool.take() {
            self.tool = t;
        }
        self.invalidate();
    }

    /// Schriftgroesse der ausgewaehlten Texte/Notizen (Welt), None ohne Notiz in der Auswahl.
    pub fn sel_note_size(&self) -> Option<f32> {
        let v: Vec<f32> = self
            .sel_items()
            .iter()
            .filter_map(|it| it.as_note().map(|n| n.size))
            .collect();
        if v.is_empty() {
            return None;
        }
        Some(v.iter().sum::<f32>() / v.len() as f32)
    }

    /// Setzt die Schriftgroesse der ausgewaehlten Texte/Notizen (Haftnotizen wachsen mit).
    pub fn set_sel_note_size(&mut self, size: f32, undo_step: bool) {
        if self.sel_note_size().is_none() {
            return;
        }
        if undo_step {
            self.checkpoint();
        }
        let size = size.clamp(1.0, 2000.0);
        self.sel_mut(|it| {
            if let Kind::Note(n) = &mut it.kind {
                if n.bg != 0 && n.size > 0.0 {
                    let k = size / n.size;
                    n.w *= k;
                    n.mh *= k;
                }
                n.size = size;
                n.layout();
                it.touch();
            }
        });
        self.frame_ok = false;
        self.ensure_frame();
        self.refresh();
        self.changed();
    }

    pub fn delete_sel(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        self.checkpoint();
        let set = self.sel_set();
        self.items.retain(|it| !set.contains(&it.id));
        self.sel_clear();
        self.refresh();
        self.changed();
    }

    pub fn duplicate_sel(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        self.checkpoint();
        let o = 24.0 * self.dp / self.sc;
        let mut ids = Vec::new();
        let copies: Vec<Item> = self.sel_items().iter().map(|it| it.dup()).collect();
        for mut n in copies {
            n.xf(1.0, 0.0, 0.0, o, o);
            ids.push(n.id);
            self.items.push(Rc::new(n));
        }
        self.sel = ids;
        if self.frame_ok {
            self.sel_cx += o;
            self.sel_cy += o;
        }
        self.refresh();
        self.changed();
    }

    pub fn has_clip(&self) -> bool {
        CLIP.with(|c| !c.borrow().0.is_empty())
    }

    fn view_cx(&self) -> f32 {
        ((if self.width > 0.0 { self.width } else { 1000.0 }) / 2.0 - self.tx) / self.sc
    }
    fn view_cy(&self) -> f32 {
        ((if self.height > 0.0 {
            self.height
        } else {
            1000.0
        }) / 2.0
            - self.ty)
            / self.sc
    }

    /// Weltpunkt zu einem Punkt in Board-Bildschirmkoordinaten (Pixel relativ zur Zeichenflaeche).
    pub fn screen_to_world(&self, x: f32, y: f32) -> (f32, f32) {
        ((x - self.tx) / self.sc, (y - self.ty) / self.sc)
    }

    /// Mittelpunkt der Ansicht in Weltkoordinaten.
    pub fn view_center(&self) -> (f32, f32) {
        (self.view_cx(), self.view_cy())
    }

    /// Zielpunkt fuers Einfuegen: `at` oder Ansichtsmitte; im Seitenmodus auf die Seite unter dem Punkt
    /// (liegt er daneben, auf die naechste Seite begrenzt).
    pub fn paste_center(&self, at: Option<(f32, f32)>) -> (f32, f32) {
        let (mut x, mut y) = at.unwrap_or_else(|| self.view_center());
        if let BoardMode::Pages(ps) = &self.board_mode {
            if let Some(r) = ps.nearest_page(x, y).and_then(|i| ps.rect(i)) {
                x = x.clamp(r.left, r.right);
                y = y.clamp(r.top, r.bottom);
            }
        }
        (x, y)
    }

    /// Nur die ausgewaehlten Objekte, transparent gerendert (Zwischenablage-Bild). Aufloesung `scale`-fach
    /// (hoechstens `max_px` Pixel Kantenlaenge). None ohne Auswahl.
    pub fn render_selection(&self, scale: f32, max_px: u32) -> Option<Pixmap> {
        let b = self.sel_bounds()?;
        let k = scale.min(max_px as f32 / b.width().max(b.height()).max(1.0));
        let mut tmp = Board::new(1.0);
        tmp.items = self
            .sel_items()
            .iter()
            .map(|it| Rc::new((*it).clone()))
            .collect();
        tmp.layers = self.layers.clone();
        tmp.dark = self.dark;
        tmp.render_composite(&b, k, false)
    }

    /// Fuegt Objekte als Kopien (neue ids, aktive Ebene) mit Mittelpunkt `at` (Welt) bzw. Ansichtsmitte ein
    /// und waehlt sie aus. Liefert die Anzahl der eingefuegten Objekte.
    pub fn paste_items(&mut self, items: &[Item], at: Option<(f32, f32)>) -> usize {
        if items.is_empty() || self.layer_blocked() {
            return 0;
        }
        let mut b = items[0].bounds();
        for it in &items[1..] {
            b.union(&it.bounds());
        }
        let (cx, cy) = self.paste_center(at);
        let (dx, dy) = (cx - b.center_x(), cy - b.center_y());
        self.checkpoint();
        let lid = self.active_layer().id;
        self.sel_clear();
        for it in items {
            let mut n = it.dup();
            n.layer = lid;
            if let Some(ex) = n.extra.as_mut() {
                // Seitenzuordnung (Xournal++) ergibt sich im Seitenmodus aus der Lage
                ex.remove("page");
            }
            n.xf(1.0, 0.0, 0.0, dx, dy);
            self.sel.push(n.id);
            self.items.push(Rc::new(n));
        }
        self.frame_ok = false;
        self.tool = SELECT;
        self.refresh();
        self.changed();
        items.len()
    }

    pub fn copy_sel(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        let copies: Vec<Item> = self.sel_items().iter().map(|it| it.dup()).collect();
        let n = copies.len();
        let (cx, cy) = (self.view_cx(), self.view_cy());
        CLIP.with(|c| *c.borrow_mut() = (copies, cx, cy, 0));
        self.notice(&if n == 1 {
            "Kopiert".to_string()
        } else {
            format!("{n} Objekte kopiert")
        });
        self.changed();
    }

    pub fn cut_sel(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        self.copy_sel();
        self.delete_sel();
    }

    pub fn paste(&mut self) {
        if !self.has_clip() || self.layer_blocked() {
            return;
        }
        let (cx, cy) = (self.view_cx(), self.view_cy());
        let (sc, dp) = (self.sc, self.dp);
        let (clip, k) = CLIP.with(|c| {
            let mut c = c.borrow_mut();
            if hypot(cx - c.1, cy - c.2) * sc < 12.0 * dp {
                c.3 += 1;
            } else {
                c.3 = 0;
            }
            c.1 = cx;
            c.2 = cy;
            (c.0.clone(), c.3)
        });
        let mut b = clip[0].bounds();
        for it in &clip[1..] {
            b.union(&it.bounds());
        }
        let off = 24.0 * dp / sc * k as f32;
        let (dx, dy) = (cx + off - b.center_x(), cy + off - b.center_y());
        self.checkpoint();
        let lid = self.active_layer().id;
        self.sel_clear();
        for it in &clip {
            let mut n = it.dup();
            n.layer = lid;
            n.xf(1.0, 0.0, 0.0, dx, dy);
            self.sel.push(n.id);
            self.items.push(Rc::new(n));
        }
        self.frame_ok = false;
        self.tool = SELECT;
        self.refresh();
        self.changed();
    }

    pub fn rotate_sel(&mut self, rad: f32) {
        if self.sel.is_empty() {
            return;
        }
        self.ensure_frame();
        self.checkpoint();
        let (cx, cy) = (self.sel_cx, self.sel_cy);
        self.sel_mut(|it| {
            it.lift = false;
            it.rot(rad, cx, cy)
        });
        self.sel_ang += rad;
        self.refresh();
        self.changed();
    }

    pub fn bring_forward(&mut self) {
        self.reorder(true);
    }
    pub fn send_backward(&mut self) {
        self.reorder(false);
    }

    /// Ganz nach vorne / hinten innerhalb der Ebene (die Ebenen selbst bleiben unberuehrt).
    fn reorder(&mut self, front: bool) {
        if self.sel.is_empty() {
            return;
        }
        self.checkpoint();
        let set = self.sel_set();
        let (mv, rest): (Vec<Rc<Item>>, Vec<Rc<Item>>) =
            self.items.drain(..).partition(|it| set.contains(&it.id));
        if front {
            self.items = rest;
            self.items.extend(mv);
        } else {
            self.items = mv;
            self.items.extend(rest);
        }
        self.refresh();
        self.notice(if front {
            "Ganz nach vorne"
        } else {
            "Ganz nach hinten"
        });
        self.changed();
    }

    fn sel_clear(&mut self) {
        self.sel.clear();
        self.frame_ok = false;
    }

    /// Huelle aller Objekte; im Seitenmodus die Huelle aller Seiten.
    pub fn content_bounds(&self) -> Option<RectF> {
        if let BoardMode::Pages(ps) = &self.board_mode {
            return ps.bounds();
        }
        let mut it = self.items.iter();
        let mut b = it.next()?.bounds();
        for x in it {
            b.union(&x.bounds());
        }
        Some(b)
    }

    /// Alles zeigen.
    pub fn fit(&mut self) {
        let (w, hh) = (self.width, self.height);
        if let (BoardMode::Pages(ps), true) = (&self.board_mode, w > 0.0) {
            // Seitenmodus: auf die Seitenbreite einpassen, Oberkante der aktuellen Seite oben
            let rs = ps.rects();
            if let Some(pw) = rs.iter().map(|r| r.width()).reduce(f32::max) {
                let m = 16.0 * self.dp;
                let (cx, cy) = (
                    (w / 2.0 - self.tx) / self.sc,
                    (hh / 2.0 - self.ty) / self.sc,
                );
                let cur = ps.nearest_page(cx, cy).unwrap_or(0);
                self.sc = ((w - 2.0 * m) / pw.max(1.0)).clamp(0.05, 12.0);
                self.tx = w / 2.0 - pw / 2.0 * self.sc;
                self.ty = m - rs[cur].top * self.sc;
                self.refresh();
                self.changed();
                return;
            }
        }
        match self.content_bounds() {
            Some(b) if w > 0.0 => {
                let m = 48.0 * self.dp;
                self.sc = ((w - 2.0 * m) / b.width().max(1.0))
                    .min((hh - 2.0 * m) / b.height().max(1.0))
                    .clamp(0.05, 2.0);
                self.tx = w / 2.0 - b.center_x() * self.sc;
                self.ty = hh / 2.0 - b.center_y() * self.sc;
            }
            _ => {
                self.sc = 1.0;
                self.tx = 0.0;
                self.ty = 0.0;
            }
        }
        self.refresh();
        self.changed();
    }

    /// Zoomt um den Bildschirmpunkt (fx, fy) mit Faktor f (Mausrad/Tasten). Bis zur Ruhe (160 ms) wird der Cache skaliert.
    pub fn zoom_at(&mut self, fx: f32, fy: f32, f: f32) {
        if self.mode != 0 {
            return;
        }
        if self.wheel_until.is_none() {
            self.prep_gesture();
        }
        let ns = (self.sc * f).clamp(0.05, 12.0);
        let k = ns / self.sc;
        self.tx = fx - (fx - self.tx) * k;
        self.ty = fy - (fy - self.ty) * k;
        self.sc = ns;
        self.wheel_until = Some(self.now + 0.16);
        self.invalidate();
        self.changed();
    }

    /// Verschiebt die Ansicht (Leertaste + Ziehen).
    pub fn pan_by(&mut self, dx: f32, dy: f32) {
        if self.wheel_until.is_none() {
            self.prep_gesture();
        }
        self.tx += dx;
        self.ty += dy;
        self.wheel_until = Some(self.now + 0.16);
        self.invalidate();
    }

    /// Zeit (Sekunden, monoton) fortschreiben: Halte-Timer der Formerkennung und Ende von Rad-Gesten.
    pub fn tick(&mut self, now: f64) {
        self.now = now;
        if let Some(t) = self.hold_at {
            if now >= t {
                self.hold_at = None;
                self.hold_fire();
            }
        }
        if let Some(t) = self.wheel_until {
            if now >= t {
                self.wheel_until = None;
                self.dirty = true;
                self.invalidate();
                self.changed();
            }
        }
    }

    /// Wann tick() spaetestens wieder aufgerufen werden sollte.
    pub fn next_deadline(&self) -> Option<f64> {
        match (self.hold_at, self.wheel_until) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn hold_fire(&mut self) {
        if self.mode != 1 || self.snap.is_some() {
            return;
        }
        let Some(cur) = self.cur.as_ref() else { return };
        if let Some(s) = recognizer::run(cur, self.sc) {
            self.snap = Some(self.style_arrow(s));
            self.events.push(Event::Haptic);
            self.invalidate();
        }
    }

    // ---------- Laden / Speichern ----------

    pub fn to_json(&self) -> Value {
        let items: Vec<Value> = self.items.iter().map(|it| it.json()).collect();
        let layers: Vec<Value> = self.layers.iter().map(|l| l.json()).collect();
        json!({"bg": self.bg, "dark": self.dark, "s": item::num(self.sc), "tx": item::num(self.tx), "ty": item::num(self.ty),
               "items": items, "layers": layers, "active": self.active})
    }

    pub fn from_json(&mut self, o: &Value) {
        self.bg = o.get("bg").and_then(item::jint).unwrap_or(0);
        self.dark = item::jbool(o, "dark");
        self.sc = item::jf(o, "s").unwrap_or(1.0);
        self.tx = item::jf(o, "tx").unwrap_or(0.0);
        self.ty = item::jf(o, "ty").unwrap_or(0.0);
        if !(self.sc > 0.04 && self.sc < 13.0) {
            self.sc = 1.0;
        }
        self.layers = o
            .get("layers")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter(|v| v.is_object())
                    .map(Layer::from_json)
                    .collect()
            })
            .unwrap_or_default();
        if self.layers.is_empty() {
            self.layers.push(Layer::new(1, "01-ebene"));
        }
        let act = o.get("active").and_then(item::jint).unwrap_or(0);
        self.active = (act.max(0) as usize).min(self.layers.len() - 1);
        self.undo.clear();
        self.redo.clear();
        self.sel_clear();
        self.items = o
            .get("items")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Item::from_json).map(Rc::new).collect())
            .unwrap_or_default();
        self.full_dirty = true;
        self.refresh();
    }

    fn render_all(&self, c: &mut Canvas, b: &RectF, k: f32, ox: f32, oy: f32) {
        c.draw_color(self.base_color());
        c.save();
        c.translate(ox, oy);
        c.scale(k, k);
        c.translate(-b.left, -b.top);
        self.draw_scene(c, None, false, true, 1.0 / k);
        c.restore();
    }

    /// Vorschaubild 480 × 300.
    pub fn thumb(&self) -> Pixmap {
        let mut bm = Pixmap::new(480, 300).unwrap();
        {
            let mut c = Canvas::new(&mut bm);
            let bounds = match &self.board_mode {
                // Seitenmodus: oberer Teil der ersten Seite
                BoardMode::Pages(ps) => ps.rect(0).map(|r| {
                    let w = r.width() + 40.0;
                    RectF::new(
                        r.left - 20.0,
                        r.top - 20.0,
                        r.left - 20.0 + w,
                        r.top - 20.0 + w * 300.0 / 480.0,
                    )
                }),
                BoardMode::Infinite => self.export_bounds().map(|mut b| {
                    b.inset(-20.0, -20.0);
                    b
                }),
            };
            match bounds {
                None => c.draw_color(self.base_color()),
                Some(b) => {
                    let k = (480.0 / b.width()).min(300.0 / b.height()).min(2.0);
                    self.render_all(
                        &mut c,
                        &b,
                        k,
                        (480.0 - b.width() * k) / 2.0,
                        (300.0 - b.height() * k) / 2.0,
                    );
                }
            }
        }
        bm
    }

    /// Ganzes Board als Bild (lange Kante hoechstens 3072 px); None bei leerem Board.
    pub fn export_image(&self, max_edge: f32, max_k: f32) -> Option<(Pixmap, RectF, f32)> {
        let mut b = self.export_bounds()?;
        b.inset(-40.0, -40.0);
        let k = (max_edge / b.width().max(b.height())).min(max_k);
        let mut bm = Pixmap::new(
            ((b.width() * k) as u32).max(1),
            ((b.height() * k) as u32).max(1),
        )?;
        {
            let mut c = Canvas::new(&mut bm);
            self.render_all(&mut c, &b, k, 0.0, 0.0);
        }
        Some((bm, b, k))
    }

    pub fn export_png(&self) -> Option<Vec<u8>> {
        let (bm, _, _) = self.export_image(3072.0, 2.0)?;
        bm.encode_png().ok()
    }

    /// PDF mit einer Seite (Seitengroesse wie die Android-Version, Inhalt mit doppelter Aufloesung als Bild).
    pub fn export_pdf(&self) -> Option<Vec<u8>> {
        if let BoardMode::Pages(ps) = &self.board_mode {
            return self.export_pdf_pages(ps);
        }
        let mut b = self.export_bounds()?;
        b.inset(-40.0, -40.0);
        let k = (2400.0 / b.width().max(b.height())).min(1.0);
        let (pw, ph) = (
            ((b.width() * k) as u32).max(1),
            ((b.height() * k) as u32).max(1),
        );
        let mut bm = Pixmap::new(pw * 2, ph * 2)?;
        {
            let mut c = Canvas::new(&mut bm);
            self.render_all(&mut c, &b, k * 2.0, 0.0, 0.0);
        }
        Some(crate::export::pdf_from_pixmap(&bm, pw as f32, ph as f32))
    }

    // ---------- Seitenmodus (WP2) ----------

    /// Seiten-Einstellungen im Seitenmodus, sonst None.
    pub fn pages(&self) -> Option<&PageSetup> {
        match &self.board_mode {
            BoardMode::Pages(ps) => Some(ps),
            BoardMode::Infinite => None,
        }
    }

    /// Seitenrechtecke in Weltkoordinaten (leer im unendlichen Modus).
    pub fn page_rects(&self) -> Vec<RectF> {
        self.pages().map(|p| p.rects()).unwrap_or_default()
    }

    /// Seite unter dem Weltpunkt (wx, wy); None im Zwischenraum oder im unendlichen Modus.
    pub fn page_at(&self, wx: f32, wy: f32) -> Option<usize> {
        self.pages()?.page_at(wx, wy)
    }

    /// Seite je Objekt (gleiche Reihenfolge wie `items`): die Seite unter dem Mittelpunkt des Objekts,
    /// liegt er im Zwischenraum/ausserhalb, die naechstgelegene Seite. Im unendlichen Modus ueberall None.
    /// Gedacht fuer den Xournal++-Export (Objekte je Seite, Koordinaten dann relativ zu `page_rects()[i]`).
    pub fn page_of_items(&self) -> Vec<Option<usize>> {
        match self.pages() {
            Some(ps) => self.items.iter().map(|it| ps.page_of(it)).collect(),
            None => vec![None; self.items.len()],
        }
    }

    /// Seitenmodus: liegt der Weltpunkt ausserhalb aller Seiten?
    fn outside_pages(&self, wx: f32, wy: f32) -> bool {
        self.pages()
            .map(|ps| ps.page_at(wx, wy).is_none())
            .unwrap_or(false)
    }

    /// Ersetzt die Seiten durch `new`; `order[neuer Index]` = alter Index (None = neue, leere Seite).
    /// Objekte entfallender Seiten werden geloescht, die uebrigen mit ihrer Seite verschoben. Mit Undo-Schritt.
    fn relayout_pages(&mut self, new: PageSetup, order: &[Option<usize>]) {
        let BoardMode::Pages(old) = &self.board_mode else {
            return;
        };
        let shifts = old.shifts(&new, order);
        let assign: Vec<Option<usize>> = self.items.iter().map(|it| old.page_of(it)).collect();
        self.checkpoint();
        let mut keep = Vec::with_capacity(self.items.len());
        for (it, pg) in self.items.drain(..).zip(assign) {
            match pg.map(|p| shifts[p]) {
                Some(None) => {} // Seite geloescht
                Some(Some(dy)) if dy != 0.0 => {
                    let mut it = it;
                    Rc::make_mut(&mut it).xf(1.0, 0.0, 0.0, 0.0, dy);
                    keep.push(it);
                }
                _ => keep.push(it),
            }
        }
        self.items = keep;
        self.board_mode = BoardMode::Pages(new);
        self.sel_clear();
        self.refresh();
        self.changed();
    }

    /// Fuegt nach Seite `after` (None = vor der ersten) eine leere Seite mit den Vorgaben ein; folgende Seiten
    /// rutschen samt Inhalt nach unten. Liefert den Index der neuen Seite (None im unendlichen Modus).
    pub fn add_page(&mut self, after: Option<usize>) -> Option<usize> {
        let mut new = self.pages()?.clone();
        let n = new.pages.len();
        let at = new.insert_page(after);
        let order: Vec<Option<usize>> = (0..=n)
            .map(|i| {
                if i < at {
                    Some(i)
                } else if i == at {
                    None
                } else {
                    Some(i - 1)
                }
            })
            .collect();
        self.relayout_pages(new, &order);
        Some(at)
    }

    /// Loescht Seite i samt ihrer Objekte; folgende Seiten ruecken nach oben. Die letzte Seite bleibt immer erhalten.
    pub fn remove_page(&mut self, i: usize) -> bool {
        let Some(ps) = self.pages() else { return false };
        if i >= ps.pages.len() || ps.pages.len() <= 1 {
            return false;
        }
        let mut new = ps.clone();
        new.pages.remove(i);
        let order: Vec<Option<usize>> = (0..ps.pages.len()).filter(|&k| k != i).map(Some).collect();
        self.relayout_pages(new, &order);
        true
    }

    /// Verschiebt Seite `from` an Position `to` (Index nach dem Verschieben), Objekte wandern mit.
    pub fn move_page(&mut self, from: usize, to: usize) -> bool {
        let Some(ps) = self.pages() else { return false };
        let n = ps.pages.len();
        if from >= n || to >= n || from == to {
            return false;
        }
        let mut idx: Vec<usize> = (0..n).collect();
        let x = idx.remove(from);
        idx.insert(to, x);
        let mut new = ps.clone();
        new.pages = idx.iter().map(|&k| ps.pages[k].clone()).collect();
        let order: Vec<Option<usize>> = idx.into_iter().map(Some).collect();
        self.relayout_pages(new, &order);
        true
    }

    /// Setzt den Hintergrund von Seite i (None = alle Seiten und Vorgabe fuer neue Seiten). Mit Undo-Schritt.
    pub fn set_page_bg(&mut self, i: Option<usize>, bg: PageBg) -> bool {
        let Some(ps) = self.pages() else { return false };
        if i.is_some_and(|i| i >= ps.pages.len()) {
            return false;
        }
        self.checkpoint();
        if let BoardMode::Pages(ps) = &mut self.board_mode {
            match i {
                Some(i) => ps.pages[i].bg = bg,
                None => {
                    ps.bg = bg;
                    ps.pages.iter_mut().for_each(|p| p.bg = bg);
                }
            }
        }
        self.refresh();
        self.changed();
        true
    }

    /// Ansicht so verschieben, dass Seite i oben (mit kleinem Rand) und waagrecht mittig steht. Zoom bleibt.
    pub fn show_page(&mut self, i: usize) {
        let Some(r) = self.pages().and_then(|p| p.rect(i)) else {
            return;
        };
        self.tx = self.width / 2.0 - r.center_x() * self.sc;
        self.ty = 16.0 * self.dp - r.top * self.sc;
        self.refresh();
        self.changed();
    }

    /// Farbe der Flaeche zwischen den Seiten.
    pub fn desk_color(&self) -> u32 {
        if self.dark {
            0xFF0F1013
        } else {
            0xFFE3E4E8
        }
    }

    /// Papierfarbe einer Seite.
    pub fn page_color(&self) -> u32 {
        if self.dark {
            0xFF202228
        } else {
            0xFFFFFFFF
        }
    }

    /// Grundfarbe der ganzen Flaeche (unendlich: Board-Hintergrund, Seitenmodus: Zwischenraum).
    fn base_color(&self) -> u32 {
        if self.pages().is_some() {
            self.desk_color()
        } else {
            self.bg_color()
        }
    }

    /// Bildschirm-Hintergrund vor dem Inhalt (Koordinaten: logische Pixel).
    fn backdrop(&self, c: &mut Canvas) {
        c.draw_color(self.base_color());
        if self.pages().is_none() {
            self.pattern(c);
        }
    }

    /// Vereinigung aller Seitenrechtecke als Pfad (Weltkoordinaten); None im unendlichen Modus.
    fn pages_path(&self) -> Option<tiny_skia::Path> {
        let ps = self.pages()?;
        let mut pb = PathBuilder::new();
        for r in ps.rects() {
            if let Some(rc) = tiny_skia::Rect::from_ltrb(r.left, r.top, r.right, r.bottom) {
                pb.push_rect(rc);
            }
        }
        // ohne Seiten: leerer Clip (nichts sichtbar)
        Some(pb.finish().unwrap_or_else(|| {
            PathBuilder::from_rect(tiny_skia::Rect::from_ltrb(0.0, 0.0, 0.0001, 0.0001).unwrap())
        }))
    }

    /// Seitenmodus: schneidet alles Folgende auf die Seiten zu (aktuelle Transformation = Welt).
    fn clip_pages(&self, c: &mut Canvas) {
        if let Some(p) = self.pages_path() {
            c.clip_path(&p);
        }
    }

    /// Inhalt in Weltkoordinaten. Seitenmodus: Seitenblaetter (wenn `sheets`) und auf die Seiten geschnittene Objekte.
    /// `px` = Welteinheiten je Pixel (fuer Mindest-Linienbreiten und Schatten).
    fn draw_scene(
        &self,
        c: &mut Canvas,
        vis: Option<&RectF>,
        skip_lift: bool,
        sheets: bool,
        px: f32,
    ) {
        if self.pages().is_none() {
            self.draw_layered(c, vis, skip_lift);
            return;
        }
        if sheets {
            self.draw_sheets(c, vis, px);
        }
        c.save();
        self.clip_pages(c);
        self.draw_layered(c, vis, skip_lift);
        c.restore();
    }

    /// Zeichnet die Seitenblaetter (Schatten, Papier, Linien/Raster/Punkte) in Weltkoordinaten.
    fn draw_sheets(&self, c: &mut Canvas, vis: Option<&RectF>, px: f32) {
        let Some(ps) = self.pages() else { return };
        let sh = 3.0 * px;
        for (p, r) in ps.pages.iter().zip(ps.rects()) {
            if let Some(v) = vis {
                let mut rr = r;
                rr.inset(-sh, -sh);
                if !RectF::intersects(v, &rr) {
                    continue;
                }
            }
            c.draw_rect(
                r.left + sh * 0.5,
                r.top + sh,
                r.right + sh * 0.5,
                r.bottom + sh,
                &Paint::fill(0x30000000),
            );
            c.draw_rect(
                r.left,
                r.top,
                r.right,
                r.bottom,
                &Paint::fill(self.page_color()),
            );
            self.draw_page_bg(c, p.bg, &r, px);
        }
    }

    /// Seitenhintergrund (Linienabstand PAGE_LINE) im Rechteck r.
    fn draw_page_bg(&self, c: &mut Canvas, bg: PageBg, r: &RectF, px: f32) {
        const STEP: f32 = PAGE_LINE;
        let col = if self.dark { 0xFF3B3F4C } else { 0xFFC3CDE0 };
        let lw = 0.6f32.max(px);
        let mut pb = PathBuilder::new();
        match bg {
            PageBg::Plain => return,
            PageBg::Lined => {
                // Rand oben (3 Zeilen), unten eine Zeile frei; roter Rand links
                let mut y = r.top + 3.0 * STEP;
                while y <= r.bottom - STEP + 0.01 {
                    pb.move_to(r.left, y);
                    pb.line_to(r.right, y);
                    y += STEP;
                }
                let mx = r.left + 2.5 * STEP;
                let mcol = if self.dark { 0xFF5A3A40 } else { 0xFFF0B0B0 };
                c.draw_line(mx, r.top, mx, r.bottom, &Paint::stroke(mcol, lw, Cap::Butt));
            }
            PageBg::Grid => {
                let mut y = r.top + STEP;
                while y < r.bottom - 0.01 {
                    pb.move_to(r.left, y);
                    pb.line_to(r.right, y);
                    y += STEP;
                }
                let mut x = r.left + STEP;
                while x < r.right - 0.01 {
                    pb.move_to(x, r.top);
                    pb.line_to(x, r.bottom);
                    x += STEP;
                }
            }
            PageBg::Dotted => {
                let mut dots = Vec::new();
                let mut y = r.top + STEP;
                while y < r.bottom - 0.01 {
                    let mut x = r.left + STEP;
                    while x < r.right - 0.01 {
                        dots.push(x);
                        dots.push(y);
                        x += STEP;
                    }
                    y += STEP;
                }
                c.draw_points(
                    &dots,
                    &Paint::stroke(col, (1.6f32).max(2.0 * px), Cap::Round),
                );
                return;
            }
        }
        if let Some(p) = pb.finish() {
            c.draw_path(&p, &Paint::stroke(col, lw, Cap::Butt));
        }
    }

    /// PDF im Seitenmodus: eine PDF-Seite je Board-Seite in Seitengroesse (pt), Inhalt mit 2× Aufloesung (144 dpi).
    fn export_pdf_pages(&self, ps: &PageSetup) -> Option<Vec<u8>> {
        const K: f32 = 2.0;
        let rects = ps.rects();
        crate::export::pdf_pages(rects.len(), |i| {
            let r = rects.get(i)?;
            let mut bm = Pixmap::new(
                ((r.width() * K).round() as u32).max(1),
                ((r.height() * K).round() as u32).max(1),
            )?;
            {
                let mut c = Canvas::new(&mut bm);
                c.draw_color(self.page_color());
                c.scale(K, K);
                c.translate(-r.left, -r.top);
                self.draw_page_bg(&mut c, ps.pages[i].bg, r, 1.0 / K);
                c.clip_rect(r.left, r.top, r.right, r.bottom);
                self.draw_layered(&mut c, Some(r), false);
            }
            Some((bm, r.width(), r.height()))
        })
    }

    // ---------- Eingabe ----------

    fn is_stylus(t: ToolType) -> bool {
        t == ToolType::Stylus || t == ToolType::Eraser
    }

    pub fn on_touch(&mut self, e: &Motion) {
        if e.pointers.is_empty() {
            return;
        }
        match e.action {
            Action::Down => {
                let st = Self::is_stylus(e.pointers[0].tool);
                if st
                    || self.finger_draw
                    || self.tool == SELECT
                    || self.tool == NOTE
                    || self.tool == TEXT
                {
                    self.begin(e, 0, st);
                } else {
                    self.start_pan(e, 0);
                }
            }
            Action::PointerDown(ai) => {
                if ai >= e.pointers.len() || (self.mode == 1 && self.stylus_act) {
                    return; // Handballen ignorieren
                }
                if Self::is_stylus(e.pointers[ai].tool) {
                    if self.mode == 1 {
                        self.cancel();
                    } else {
                        self.end_gesture();
                    }
                    self.begin(e, ai, true);
                } else {
                    if self.mode == 1 {
                        self.cancel();
                    }
                    if self.mode == 0 || self.mode == 2 || self.mode == 3 {
                        self.prep_gesture();
                        self.mode = 3;
                        self.zinit = true;
                    }
                }
            }
            Action::Move => {
                if self.mode == 1 {
                    let Some(i) = e.find(self.act_id) else { return };
                    for &(x, y, p) in &e.history {
                        self.mv(x, y, p);
                    }
                    let p = e.pointers[i];
                    self.mv(p.x, p.y, p.pressure);
                    self.invalidate();
                } else if self.mode == 2 {
                    let Some(i) = e.find(self.pan_id) else { return };
                    let p = e.pointers[i];
                    self.tx += p.x - self.lx;
                    self.ty += p.y - self.ly;
                    self.lx = p.x;
                    self.ly = p.y;
                    self.invalidate();
                } else if self.mode == 3 && e.pointers.len() >= 2 {
                    let (a, b) = (e.pointers[0], e.pointers[1]);
                    let sp = hypot(b.x - a.x, b.y - a.y).max(1.0);
                    let (fx, fy) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
                    if !self.zinit {
                        let ns = (self.sc * sp / self.span).clamp(0.05, 12.0);
                        let k = ns / self.sc;
                        self.tx = fx - (self.lfx - self.tx) * k;
                        self.ty = fy - (self.lfy - self.ty) * k;
                        self.sc = ns;
                        self.changed();
                    }
                    self.span = sp;
                    self.lfx = fx;
                    self.lfy = fy;
                    self.zinit = false;
                    self.invalidate();
                }
            }
            Action::PointerUp(ai) => {
                if ai >= e.pointers.len() {
                    return;
                }
                let r = if ai == 0 { 1 } else { 0 };
                if self.mode == 1 {
                    if e.pointers[ai].id == self.act_id {
                        self.finish();
                    }
                } else if self.mode == 3 {
                    if e.pointers.len() > 2 {
                        self.zinit = true;
                    } else if r < e.pointers.len() {
                        self.mode = 2;
                        self.pan_id = e.pointers[r].id;
                        self.lx = e.pointers[r].x;
                        self.ly = e.pointers[r].y;
                    }
                } else if self.mode == 2 && e.pointers[ai].id == self.pan_id && r < e.pointers.len()
                {
                    self.pan_id = e.pointers[r].id;
                    self.lx = e.pointers[r].x;
                    self.ly = e.pointers[r].y;
                }
            }
            Action::Up => {
                if self.mode == 1 {
                    self.finish();
                } else {
                    self.end_gesture();
                }
                self.mode = 0;
            }
            Action::Cancel => {
                if self.mode == 1 {
                    self.cancel();
                } else {
                    self.end_gesture();
                }
                self.mode = 0;
            }
        }
    }

    /// Vor Zoom/Verschieben: Zwischenspeicher auf aktuellen Stand bringen (wird dann verschoben bzw. skaliert gezeigt).
    fn prep_gesture(&mut self) {
        if self.width > 0.0 && self.mode != 2 && self.mode != 3 && self.wheel_until.is_none() {
            self.update_cache();
        }
    }

    fn start_pan(&mut self, e: &Motion, i: usize) {
        self.prep_gesture();
        self.mode = 2;
        self.pan_id = e.pointers[i].id;
        self.lx = e.pointers[i].x;
        self.ly = e.pointers[i].y;
    }

    fn end_gesture(&mut self) {
        if self.mode == 2 || self.mode == 3 {
            self.mode = 0;
            self.dirty = true;
            self.invalidate();
            self.changed();
        }
    }

    fn begin(&mut self, e: &Motion, i: usize, st: bool) {
        self.wheel_until = None;
        self.mode = 1;
        let p = e.pointers[i];
        self.act_id = p.id;
        self.stylus_act = st;
        let (x, y) = (p.x, p.y);
        self.dsx = x;
        self.dsy = y;
        self.moved = false;
        self.sx = (x - self.tx) / self.sc;
        self.lwx = self.sx;
        self.sy = (y - self.ty) / self.sc;
        self.lwy = self.sy;
        self.erasing = self.tool == ERASER || p.tool == ToolType::Eraser || (st && e.stylus_button);
        self.erased = false;
        self.cur = None;
        self.snap = None;
        self.prev = None;
        self.marq = None;
        self.drag = 0;
        self.blocked = false;
        if self.tool != SELECT && self.layer_blocked() {
            self.blocked = true;
            self.ex = -1.0;
        } else if !self.erasing && self.tool != SELECT && self.outside_pages(self.sx, self.sy) {
            // Seitenmodus: ausserhalb aller Seiten begonnene Striche/Formen/Texte werden verworfen
            self.blocked = true;
            self.ex = -1.0;
        } else if self.erasing {
            self.ex = x;
            self.ey = y;
            self.erase_step(self.sx, self.sy);
        } else if self.tool == PEN || self.tool == MARKER {
            // Strichstärke in Weltkoordinaten (Punkt), unabhängig von der Zoomstufe
            let w = (if self.tool == MARKER {
                self.pen_w * 4.0 + 8.0
            } else {
                self.pen_w
            }) * self.dp;
            let mut cur = Item::stroke(self.color, w, self.tool == MARKER, st && self.tool == PEN);
            self.last_pr = if st { p.pressure } else { 0.5 };
            cur.add_point(self.sx, self.sy, self.last_pr);
            self.cur = Some(cur);
            self.arm_hold(x, y);
        } else if self.tool == SELECT {
            self.begin_select(self.sx, self.sy);
        }
        self.invalidate();
    }

    fn arm_hold(&mut self, x: f32, y: f32) {
        self.hold_x = x;
        self.hold_y = y;
        self.hold_at = if self.recog == 0 {
            Some(self.now + 0.45)
        } else {
            None
        };
    }

    fn mv(&mut self, x: f32, y: f32, pr: f32) {
        if self.blocked {
            return;
        }
        if !self.moved && hypot(x - self.dsx, y - self.dsy) > 6.0 * self.dp {
            self.moved = true;
        }
        let (wx, wy) = ((x - self.tx) / self.sc, (y - self.ty) / self.sc);
        if self.erasing {
            self.ex = x;
            self.ey = y;
            let dist = hypot(wx - self.lwx, wy - self.lwy) * self.sc;
            let steps = ((dist / (6.0 * self.dp)) as i32).max(1);
            let (lwx, lwy) = (self.lwx, self.lwy);
            for k in 1..=steps {
                let t = k as f32 / steps as f32;
                self.erase_step(lwx + (wx - lwx) * t, lwy + (wy - lwy) * t);
            }
        } else if self.cur.is_some() {
            if self.snap.is_some() {
                return;
            }
            let cur = self.cur.as_ref().unwrap();
            let s = cur.as_stroke().unwrap();
            let n = s.n();
            let (px, py) = (s.p[n * 3 - 3], s.p[n * 3 - 2]);
            if hypot(wx - px, wy - py) * self.sc < 0.7 {
                return;
            }
            self.last_pr = if self.stylus_act {
                self.last_pr * 0.6 + pr * 0.4
            } else {
                0.5
            };
            let lp = self.last_pr;
            self.cur.as_mut().unwrap().add_point(wx, wy, lp);
            if hypot(x - self.hold_x, y - self.hold_y) > 5.0 * self.dp {
                self.arm_hold(x, y);
            }
        } else if self.tool == SHAPE {
            if self.moved {
                self.prev = Some(self.make_shape(self.sx, self.sy, wx, wy));
            }
        } else if self.tool == SELECT {
            self.drag_select(wx, wy);
        }
        self.lwx = wx;
        self.lwy = wy;
    }

    /// Wendet den gewählten Pfeilstil auf eine erkannte Form an.
    fn style_arrow(&self, mut it: Item) -> Item {
        if let Kind::Shape(g) = &mut it.kind {
            g.kind = item::styled_arrow(g.kind, self.arrow_style);
        }
        it
    }

    fn make_shape(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Item {
        let w = self.pen_w * self.dp;
        if self.shape_kind == 0 {
            Item::shape(
                item::POLY,
                vec![x0, y0, x1, y0, x1, y1, x0, y1],
                self.color,
                w,
            )
        } else {
            let k = match self.shape_kind {
                1 => item::ELLIPSE,
                2 => item::LINE,
                4 => item::ARROW_ENG,
                _ => item::styled_arrow(item::ARROW, self.arrow_style),
            };
            Item::shape(k, vec![x0, y0, x1, y1], self.color, w)
        }
    }

    /// Radierer-Radius in Bildschirm-Pixeln fuer den aktuellen Modus. Der Teil-Radierer
    /// richtet sich wie die Strichstärke nach Weltkoordinaten (skaliert also mit dem Zoom),
    /// bleibt aber mindestens 4 Pixel gross.
    fn erase_screen_r(&self) -> f32 {
        if self.erase_mode == 1 {
            ((self.pen_w * 3.0).max(6.0) * self.dp * self.sc).max(4.0 * self.dp)
        } else {
            10.0 * self.dp
        }
    }

    fn erase_step(&mut self, wx: f32, wy: f32) {
        if self.erase_mode == 1 {
            self.erase_part_at(wx, wy);
        } else {
            self.erase_at(wx, wy);
        }
    }

    /// Teil-Radierer: entfernt nur die Stellen unter dem Kreis; Formen werden dabei zu Strichen.
    fn erase_part_at(&mut self, wx: f32, wy: f32) {
        let r = self.erase_screen_r() / self.sc;
        let mut i = self.items.len();
        while i > 0 {
            i -= 1;
            let it = self.items[i].clone();
            let is_stroke = it.as_stroke().is_some();
            if !is_stroke && it.as_shape().is_none() {
                continue;
            }
            if !self.usable(&it) || !it.near(wx, wy, r) {
                continue;
            }
            let mut repl: Vec<Item> = Vec::new();
            if is_stroke {
                let s = it.as_stroke().unwrap();
                let Some(parts) = item::erase_split(&s.p, it.width, wx, wy, r) else {
                    continue;
                };
                repl.extend(parts.into_iter().map(|p| it.part(p)));
            } else {
                let mut ch = false;
                for s in it.to_strokes() {
                    let st = s.as_stroke().unwrap();
                    match item::erase_split(&st.p, s.width, wx, wy, r) {
                        None => repl.push(s),
                        Some(parts) => {
                            ch = true;
                            repl.extend(parts.into_iter().map(|p| s.part(p)));
                        }
                    }
                }
                if !ch {
                    continue;
                }
            }
            if !self.erased {
                self.checkpoint();
                self.erased = true;
            }
            if self.sel.contains(&it.id) {
                self.sel_clear();
            }
            self.items.splice(i..i + 1, repl.into_iter().map(Rc::new));
            self.dirty = true;
        }
    }

    fn erase_at(&mut self, wx: f32, wy: f32) {
        let r = 10.0 * self.dp / self.sc;
        let mut i = self.items.len();
        while i > 0 {
            i -= 1;
            if self.usable(&self.items[i]) && self.items[i].near(wx, wy, r) {
                if !self.erased {
                    self.checkpoint();
                    self.erased = true;
                }
                self.items.remove(i);
                self.dirty = true;
            }
        }
    }

    /// Huelle der ausgewaehlten Objekte.
    pub fn sel_bounds(&self) -> Option<RectF> {
        let s = self.sel_items();
        let mut b = s.first()?.bounds();
        for it in &s[1..] {
            b.union(&it.bounds());
        }
        Some(b)
    }

    // ---------- Auswahl: Rahmen ----------

    fn ensure_frame(&mut self) {
        if self.frame_ok || self.sel.is_empty() {
            return;
        }
        let Some(b) = self.sel_bounds() else { return };
        self.sel_cx = b.center_x();
        self.sel_cy = b.center_y();
        self.sel_w = b.width().max(0.001);
        self.sel_h = b.height().max(0.001);
        self.sel_ang = 0.0;
        self.frame_ok = true;
    }

    /// Lokale Rahmenkoordinate (relativ zur Mitte, ungedreht) nach Welt.
    fn lw(&self, lx: f32, ly: f32) -> (f32, f32) {
        let (cs, sn) = (self.sel_ang.cos(), self.sel_ang.sin());
        (
            self.sel_cx + lx * cs - ly * sn,
            self.sel_cy + lx * sn + ly * cs,
        )
    }

    /// Lokale Rahmenkoordinate nach Bildschirm.
    fn ls(&self, lx: f32, ly: f32) -> (f32, f32) {
        let (x, y) = self.lw(lx, ly);
        (x * self.sc + self.tx, y * self.sc + self.ty)
    }

    /// Objekt mit dem kleinsten Rahmen, der den Punkt enthaelt (sichtbare, entsperrte Ebenen).
    fn bounds_at(&self, wx: f32, wy: f32) -> Option<u64> {
        let mut best: Option<(f32, u64)> = None;
        for it in self.items.iter().rev() {
            let l = &self.layers[self.layer_idx(it.layer)];
            if !l.visible || l.locked {
                continue;
            }
            let b = it.bounds();
            if wx >= b.left && wx <= b.right && wy >= b.top && wy <= b.bottom {
                let a = (b.right - b.left) * (b.bottom - b.top);
                if best.map(|(ba, _)| a < ba).unwrap_or(true) {
                    best = Some((a, it.id));
                }
            }
        }
        best.map(|(_, id)| id)
    }

    fn top_at(&self, wx: f32, wy: f32, r: f32) -> Option<u64> {
        let (mut best_i, mut best_l): (Option<usize>, Option<usize>) = (None, None);
        for i in (0..self.items.len()).rev() {
            let it = &self.items[i];
            let li = self.layer_idx(it.layer);
            if let Some(bl) = best_l {
                if li < bl || (li == bl && best_i.is_some()) {
                    continue;
                }
            }
            let l = &self.layers[li];
            if l.visible && !l.locked && it.near(wx, wy, r) {
                best_i = Some(i);
                best_l = Some(li);
            }
        }
        best_i.map(|i| self.items[i].id)
    }

    /// Endpunkte (x0, y0, x1, y1), wenn genau eine lineare Form (Linie/Pfeil) ausgewaehlt ist.
    fn lin_ends(&self) -> Option<[f32; 4]> {
        if self.sel.len() != 1 {
            return None;
        }
        let it = self.sel_items().into_iter().next()?;
        let g = it.as_shape()?;
        if is_linear(g.kind) && g.q.len() >= 4 {
            Some([g.q[0], g.q[1], g.q[2], g.q[3]])
        } else {
            None
        }
    }

    /// Kanten-Griffe (Mitte jeder Seite, 0 oben, 1 rechts, 2 unten, 3 links) sind nur sichtbar/greifbar,
    /// wenn die Seite am Bildschirm lang genug ist (>= 3 Grifflaengen), damit die Ecken greifbar bleiben.
    fn edge_visible(&self, k: usize) -> bool {
        let (len, sep) = if k % 2 == 0 {
            (self.sel_w, self.sel_h)
        } else {
            (self.sel_h, self.sel_w)
        };
        // zusaetzlich: gegenueberliegende Griffe muessen so weit auseinander liegen, dass die Mitte frei zum Verschieben bleibt
        len * self.sc >= 3.0 * 14.0 * self.dp && sep * self.sc >= 44.0 * self.dp
    }

    fn begin_select(&mut self, wx: f32, wy: f32) {
        self.pend = None;
        self.prev_sel = None;
        self.prev_frame = None;
        self.lasso.clear();
        let dp = self.dp;
        if !self.sel.is_empty() {
            self.ensure_frame();
            let pad = 6.0 * dp / self.sc;
            let (hw, hh, hit) = (self.sel_w / 2.0 + pad, self.sel_h / 2.0 + pad, 22.0 * dp);
            // Endpunkt-Griffe einer einzelnen Linie/eines Pfeils (statt Dreh-/Eck-/Kanten-Griffen)
            if let Some(q) = self.lin_ends() {
                let (sc, tx, ty) = (self.sc, self.tx, self.ty);
                let d0 = hypot(self.dsx - (q[0] * sc + tx), self.dsy - (q[1] * sc + ty));
                let d1 = hypot(self.dsx - (q[2] * sc + tx), self.dsy - (q[3] * sc + ty));
                if d0 < hit || d1 < hit {
                    self.drag = if d0 <= d1 { 5 } else { 6 };
                    return;
                }
                let (dx, dy) = (wx - self.sel_cx, wy - self.sel_cy);
                let (cs, sn) = (self.sel_ang.cos(), self.sel_ang.sin());
                let (lx, ly, m) = (dx * cs + dy * sn, -dx * sn + dy * cs, 8.0 * dp / self.sc);
                if lx.abs() <= hw + m && ly.abs() <= hh + m {
                    self.drag = 1;
                    return;
                }
            } else {
                // Dreh-Griff
                let (rx, ry) = self.ls(0.0, -hh - 32.0 * dp / self.sc);
                if hypot(self.dsx - rx, self.dsy - ry) < hit {
                    let (cx, cy) = self.ls(0.0, 0.0);
                    self.g_a0 = (self.dsy - cy).atan2(self.dsx - cx);
                    self.g_ang0 = self.sel_ang;
                    self.drag = 4;
                    return;
                }
                // Eck-Griffe
                let (sgx, sgy) = ([-1.0f32, 1.0, 1.0, -1.0], [-1.0f32, -1.0, 1.0, 1.0]);
                let mut bk: Option<usize> = None;
                let mut bd = hit;
                for k in 0..4 {
                    let (x, y) = self.ls(sgx[k] * hw, sgy[k] * hh);
                    let d = hypot(self.dsx - x, self.dsy - y);
                    if d < bd {
                        bd = d;
                        bk = Some(k);
                    }
                }
                if let Some(k) = bk {
                    let c1 = self.lw(sgx[k] * self.sel_w / 2.0, sgy[k] * self.sel_h / 2.0);
                    let c2 = self.lw(-sgx[k] * self.sel_w / 2.0, -sgy[k] * self.sel_h / 2.0);
                    self.pvx = c2.0;
                    self.pvy = c2.1;
                    let (ux, uy) = (c1.0 - c2.0, c1.1 - c2.1);
                    let l = hypot(ux, uy).max(0.0001);
                    self.g_ux = ux / l;
                    self.g_uy = uy / l;
                    self.d0 =
                        (l * 0.2).max((wx - self.pvx) * self.g_ux + (wy - self.pvy) * self.g_uy);
                    self.g_scale = 1.0;
                    self.drag = 2;
                    return;
                }
                // Kanten-Griffe: nur Breite bzw. nur Hoehe skalieren, gegenueberliegende Kante bleibt fest
                let (egx, egy) = ([0.0f32, 1.0, 0.0, -1.0], [-1.0f32, 0.0, 1.0, 0.0]);
                let mut ek: Option<usize> = None;
                let mut ed = hit;
                for k in 0..4 {
                    if !self.edge_visible(k) {
                        continue;
                    }
                    let (x, y) = self.ls(egx[k] * hw, egy[k] * hh);
                    let d = hypot(self.dsx - x, self.dsy - y);
                    if d < ed {
                        ed = d;
                        ek = Some(k);
                    }
                }
                if let Some(k) = ek {
                    let c1 = self.lw(egx[k] * self.sel_w / 2.0, egy[k] * self.sel_h / 2.0);
                    let c2 = self.lw(-egx[k] * self.sel_w / 2.0, -egy[k] * self.sel_h / 2.0);
                    self.pvx = c2.0;
                    self.pvy = c2.1;
                    let (ux, uy) = (c1.0 - c2.0, c1.1 - c2.1);
                    let l = hypot(ux, uy).max(0.0001);
                    self.g_ux = ux / l;
                    self.g_uy = uy / l;
                    self.d0 =
                        (l * 0.2).max((wx - self.pvx) * self.g_ux + (wy - self.pvy) * self.g_uy);
                    self.g_scale = 1.0;
                    self.g_edge = k;
                    self.drag = 7;
                    return;
                }
                // innerhalb des Rahmens: verschieben
                let (dx, dy) = (wx - self.sel_cx, wy - self.sel_cy);
                let (cs, sn) = (self.sel_ang.cos(), self.sel_ang.sin());
                let (lx, ly, m) = (dx * cs + dy * sn, -dx * sn + dy * cs, 8.0 * dp / self.sc);
                if lx.abs() <= hw + m && ly.abs() <= hh + m {
                    self.drag = 1;
                    return;
                }
            }
        }
        // ausserhalb: neue Auswahl (die alte bleibt, bis wirklich gezogen wird)
        if !self.sel.is_empty() {
            self.prev_sel = Some(self.sel.clone());
            self.prev_frame = Some([
                self.sel_cx,
                self.sel_cy,
                self.sel_w,
                self.sel_h,
                self.sel_ang,
            ]);
        }
        self.pend = self.top_at(wx, wy, 14.0 * dp / self.sc);
        self.drag = 3;
        self.add_lasso(wx, wy);
        if self.sel_mode == 1 {
            self.marq = Some(RectF::new(wx, wy, wx, wy));
        }
    }

    fn add_lasso(&mut self, wx: f32, wy: f32) {
        if self.sel_mode != 0 {
            return;
        }
        let n = self.lasso.len();
        if n >= 2 && hypot(wx - self.lasso[n - 2], wy - self.lasso[n - 1]) * self.sc < 2.0 * self.dp
        {
            return;
        }
        self.lasso.push(wx);
        self.lasso.push(wy);
    }

    fn lift_sel(&mut self) {
        if self.lifted {
            return;
        }
        self.checkpoint();
        self.sel_mut(|it| it.lift = true);
        self.lifted = true;
        self.dirty = true;
    }

    fn drag_select(&mut self, wx: f32, wy: f32) {
        if self.drag == 3 {
            self.add_lasso(wx, wy);
            if !self.moved {
                return;
            }
            if !self.sel.is_empty() {
                self.sel_clear();
            }
            if let Some(m) = self.marq.as_mut() {
                *m = RectF::new(
                    self.sx.min(wx),
                    self.sy.min(wy),
                    self.sx.max(wx),
                    self.sy.max(wy),
                );
            }
            return;
        }
        if !self.moved || self.drag == 0 {
            return;
        }
        if !self.lifted {
            self.lift_sel();
            self.lwx = self.sx;
            self.lwy = self.sy;
        }
        if self.drag == 1 {
            let (dx, dy) = (wx - self.lwx, wy - self.lwy);
            self.sel_mut(|it| it.xf(1.0, 0.0, 0.0, dx, dy));
            self.sel_cx += dx;
            self.sel_cy += dy;
        } else if self.drag == 2 {
            let d = (wx - self.pvx) * self.g_ux + (wy - self.pvy) * self.g_uy;
            let target = (d / self.d0).clamp(0.05, 20.0);
            let s = target / self.g_scale;
            self.g_scale = target;
            let (pvx, pvy) = (self.pvx, self.pvy);
            self.sel_mut(|it| it.xf(s, pvx, pvy, 0.0, 0.0));
            self.sel_cx = pvx + (self.sel_cx - pvx) * s;
            self.sel_cy = pvy + (self.sel_cy - pvy) * s;
            self.sel_w *= s;
            self.sel_h *= s;
        } else if self.drag == 7 {
            let d = (wx - self.pvx) * self.g_ux + (wy - self.pvy) * self.g_uy;
            let target = (d / self.d0).clamp(0.05, 20.0);
            let s = target / self.g_scale;
            self.g_scale = target;
            let horiz = self.g_edge % 2 == 1;
            let (sx, sy) = if horiz { (s, 1.0) } else { (1.0, s) };
            let (pvx, pvy, ang) = (self.pvx, self.pvy, self.sel_ang);
            self.sel_mut(|it| it.xf_axis(sx, sy, pvx, pvy, ang));
            // Mittelpunkt wandert nur entlang der Griff-Achse
            let along = (self.sel_cx - pvx) * self.g_ux + (self.sel_cy - pvy) * self.g_uy;
            self.sel_cx += self.g_ux * along * (s - 1.0);
            self.sel_cy += self.g_uy * along * (s - 1.0);
            if horiz {
                self.sel_w *= s;
            } else {
                self.sel_h *= s;
            }
        } else if self.drag == 5 || self.drag == 6 {
            if let Some(q) = self.lin_ends() {
                let oth = if self.drag == 5 { 2 } else { 0 };
                let (ox, oy) = (q[oth], q[oth + 1]);
                let (mut nx, mut ny) = (wx, wy);
                let (dx, dy) = (wx - ox, wy - oy);
                let l = hypot(dx, dy);
                if l > 0.0001 {
                    let deg = dy.atan2(dx).to_degrees();
                    let r = (deg / 15.0).round() * 15.0;
                    if (deg - r).abs() <= 4.0 {
                        let a = r.to_radians();
                        nx = ox + l * a.cos();
                        ny = oy + l * a.sin();
                    }
                }
                let i = if self.drag == 5 { 0 } else { 1 };
                self.sel_mut(|it| it.set_end(i, nx, ny));
            }
        } else if self.drag == 4 {
            let a = (wy - self.sel_cy).atan2(wx - self.sel_cx);
            let mut dl = a - self.g_a0;
            let pi = std::f32::consts::PI;
            while dl > pi {
                dl -= 2.0 * pi;
            }
            while dl < -pi {
                dl += 2.0 * pi;
            }
            let mut na = self.g_ang0 + dl;
            let deg = na.to_degrees();
            let r = (deg / 15.0).round() * 15.0;
            if (deg - r).abs() <= 4.0 {
                na = r.to_radians();
            }
            let d = na - self.sel_ang;
            let (cx, cy) = (self.sel_cx, self.sel_cy);
            self.sel_mut(|it| it.rot(d, cx, cy));
            self.sel_ang = na;
        }
    }

    fn in_lasso(&self, x: f32, y: f32) -> bool {
        let l = &self.lasso;
        let n = l.len() / 2;
        let mut inside = false;
        let mut j = n.wrapping_sub(1);
        for i in 0..n {
            let (xi, yi, xj, yj) = (l[i * 2], l[i * 2 + 1], l[j * 2], l[j * 2 + 1]);
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    fn lasso_picks(&self, it: &Item) -> bool {
        let (mut tot, mut ins) = (0, 0);
        let mut test = |x: f32, y: f32| {
            tot += 1;
            if self.in_lasso(x, y) {
                ins += 1;
            }
        };
        match &it.kind {
            Kind::Stroke(st) => {
                let n = st.n();
                let step = (n / 200).max(1);
                let mut i = 0;
                while i < n {
                    test(st.p[i * 3], st.p[i * 3 + 1]);
                    i += step;
                }
            }
            Kind::Shape(g) if g.kind != item::ELLIPSE && g.kind != item::HEART => {
                for xy in g.q.chunks_exact(2) {
                    test(xy[0], xy[1]);
                }
            }
            Kind::Shape(_) => {
                let b = it.bounds();
                let (cx, cy, hx, hy) = (
                    b.center_x(),
                    b.center_y(),
                    b.width() * 0.45,
                    b.height() * 0.45,
                );
                for (x, y) in [
                    (cx, cy),
                    (cx - hx, cy),
                    (cx + hx, cy),
                    (cx, cy - hy),
                    (cx, cy + hy),
                ] {
                    test(x, y);
                }
            }
            _ => {
                let b = it.bounds();
                return self.in_lasso(b.center_x(), b.center_y());
            }
        }
        tot > 0 && ins * 10 >= tot * 6
    }

    fn select_area(&mut self) {
        self.sel_clear();
        let mut out = Vec::new();
        if self.sel_mode == 1 {
            let Some(m) = self.marq else { return };
            for it in &self.items {
                if !self.usable(it) {
                    continue;
                }
                let b = it.bounds();
                if m.contains(&b) || m.contains_pt(b.center_x(), b.center_y()) {
                    out.push(it.id);
                }
            }
        } else if self.lasso.len() >= 6 {
            for it in &self.items {
                if self.usable(it) && self.lasso_picks(it) {
                    out.push(it.id);
                }
            }
        }
        self.sel = out;
        self.frame_ok = false;
    }

    fn drop_lift(&mut self) {
        if !self.lifted {
            return;
        }
        self.sel_mut(|it| it.lift = false);
        self.lifted = false;
        self.dirty = true;
    }

    fn finish(&mut self) {
        self.hold_at = None;
        if self.blocked {
            self.blocked = false;
        } else if self.erasing {
            self.ex = -1.0;
        } else if let Some(cur) = self.cur.take() {
            let mut it = self.snap.take();
            if it.is_none() && self.recog == 1 {
                it = recognizer::run(&cur, self.sc).map(|s| self.style_arrow(s));
            }
            self.add_top(it.unwrap_or(cur));
        } else if self.tool == SHAPE {
            if let Some(p) = self.prev.take() {
                self.add_top(p);
            }
        } else if self.tool == SELECT {
            self.drop_lift();
            if self.drag == 5 || self.drag == 6 {
                self.frame_ok = false;
                self.ensure_frame();
            }
            if self.drag == 3 {
                if self.moved {
                    self.select_area();
                } else {
                    self.sel_clear();
                    if let Some(p) = self.pend {
                        self.sel.push(p);
                        self.frame_ok = false;
                    } else if self.back_tool.is_some() {
                        self.deselect();
                    }
                }
            } else if matches!(self.drag, 1 | 2 | 7) && !self.moved {
                // Erneutes Tippen auf einen ausgewaehlten Text (auch auf einen Griff kleiner Texte): direkt bearbeiten
                if let Some(id) = self.single_note().map(|n| n.id) {
                    self.events.push(Event::EditNote(id));
                }
            }
            self.marq = None;
            self.lasso.clear();
            self.pend = None;
            self.prev_sel = None;
            self.prev_frame = None;
            self.drag = 0;
        } else if (self.tool == NOTE || self.tool == TEXT) && !self.moved {
            // Tippen auf vorhandenen Text: auswaehlen (erweitertes Menue), sonst neuer Text
            let (x, y) = (self.sx * self.sc + self.tx, self.sy * self.sc + self.ty);
            if !self.select_at_screen(x, y, true) {
                self.events.push(Event::AskText {
                    wx: self.sx,
                    wy: self.sy,
                    note: self.tool == NOTE,
                });
            }
        }
        self.cur = None;
        self.snap = None;
        self.prev = None;
        self.mode = 0;
        self.invalidate();
        self.changed();
    }

    fn cancel(&mut self) {
        self.hold_at = None;
        self.drop_lift();
        self.cur = None;
        self.snap = None;
        self.prev = None;
        self.marq = None;
        if self.tool == SELECT && self.drag == 3 && self.sel.is_empty() {
            if let (Some(ps), Some(pf)) = (self.prev_sel.take(), self.prev_frame) {
                self.sel = ps;
                self.sel_cx = pf[0];
                self.sel_cy = pf[1];
                self.sel_w = pf[2];
                self.sel_h = pf[3];
                self.sel_ang = pf[4];
                self.frame_ok = true;
            }
        }
        self.lasso.clear();
        self.pend = None;
        self.prev_sel = None;
        self.prev_frame = None;
        self.drag = 0;
        self.ex = -1.0;
        self.blocked = false;
        self.mode = 0;
        self.invalidate();
        self.changed();
    }

    // ---------- Zeichnen ----------

    fn sig(&self) -> CacheSig {
        let ps = self.px_scale;
        let (dw, dh) = (
            ((self.width * ps).round() as u32).max(1),
            ((self.height * ps).round() as u32).max(1),
        );
        CacheSig {
            size: (dw, dh, ps.to_bits()),
            sc: self.sc.to_bits(),
            dark: self.dark,
            bg: self.bg,
            layers: self
                .layers
                .iter()
                .map(|l| (l.id, l.visible, l.opacity.to_bits()))
                .collect(),
            pages: self
                .pages()
                .map(|p| {
                    p.pages
                        .iter()
                        .map(|g| (g.w.to_bits(), g.h.to_bits(), g.bg))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn cache_size(&self) -> (u32, u32) {
        let ps = self.px_scale;
        (
            ((self.width * ps).round() as u32).max(1),
            ((self.height * ps).round() as u32).max(1),
        )
    }

    /// Zeichnet den Zwischenspeicher komplett neu (Ansicht = sc/tx/ty).
    fn render_cache(&mut self) {
        let (dw, dh) = self.cache_size();
        let ok = matches!(&self.cache, Some(c) if c.width() == dw && c.height() == dh);
        if !ok {
            self.cache = Pixmap::new(dw, dh);
        }
        self.csc = self.sc;
        self.ctx = self.tx;
        self.cty = self.ty;
        self.org = (0, 0);
        let Some(mut cache) = self.cache.take() else {
            return;
        };
        self.draw_cache_area(&mut cache, 0, 0);
        self.cache = Some(cache);
        self.cache_items = self.items.iter().filter(|it| !it.lift).cloned().collect();
        self.cache_sig = Some(self.sig());
        self.cache_dmg = Damage(vec![IRect {
            x: 0,
            y: 0,
            w: dw,
            h: dh,
        }]);
        self.dirty = false;
        self.full_dirty = false;
    }

    /// Zeichnet Hintergrund und ruhende Objekte in pm, dessen linke obere Ecke am Geraete-Pixel (x, y) des
    /// Zwischenspeichers liegt (Ansicht des Zwischenspeichers: csc/ctx/cty).
    fn draw_cache_area(&mut self, pm: &mut Pixmap, x: u32, y: u32) {
        let ps = self.px_scale;
        // pattern()/backdrop() lesen die Ansicht aus sc/tx/ty
        let view = (self.sc, self.tx, self.ty);
        (self.sc, self.tx, self.ty) = (self.csc, self.ctx, self.cty);
        {
            let (w, h) = (pm.width() as f32, pm.height() as f32);
            // grosse Flaechen: aufzeichnen und in Streifen parallel zeichnen
            let par = pm.width() as u64 * pm.height() as u64 >= 250_000;
            let mut c = Canvas::new(pm);
            if par {
                c.start_recording();
            }
            c.translate(-(x as f32), -(y as f32));
            c.scale(ps, ps);
            self.backdrop(&mut c);
            c.translate(self.tx, self.ty);
            c.scale(self.sc, self.sc);
            let (l, t) = (
                (x as f32 / ps - self.tx) / self.sc,
                (y as f32 / ps - self.ty) / self.sc,
            );
            let vis = RectF::new(l, t, l + w / ps / self.sc, t + h / ps / self.sc);
            self.draw_scene(&mut c, Some(&vis), true, true, self.dp / self.sc);
            if par {
                let (rec, keep) = c.take_recording();
                drop(c);
                rec.replay_parallel(pm);
                drop(keep);
            }
        }
        (self.sc, self.tx, self.ty) = view;
    }

    /// Zeichnet den Bereich r des Zwischenspeichers neu.
    fn redraw_cache_rect(&mut self, r: IRect) {
        if r.is_empty() {
            return;
        }
        let Some(mut cache) = self.cache.take() else {
            return;
        };
        if let Some(mut tmp) = Pixmap::new(r.w, r.h) {
            self.draw_cache_area(&mut tmp, r.x, r.y);
            blit(&mut cache, &tmp, r, self.org);
        }
        self.cache = Some(cache);
        self.cache_dmg.add(r);
    }

    /// Weltrechteck → Geraete-Pixel im Zwischenspeicher (mit 2 px Rand fuer die Kantenglaettung).
    fn cache_rect(&self, b: &RectF) -> IRect {
        let (ps, sc) = (self.px_scale, self.csc);
        let (dw, dh) = self.cache_size();
        IRect::cover(
            (b.left * sc + self.ctx) * ps - 2.0,
            (b.top * sc + self.cty) * ps - 2.0,
            (b.right * sc + self.ctx) * ps + 2.0,
            (b.bottom * sc + self.cty) * ps + 2.0,
            dw,
            dh,
        )
    }

    /// Vergleicht die Objekte mit dem Stand des Zwischenspeichers (Zeiger-Identitaet; Aenderungen erzeugen
    /// per Copy-on-Write neue Objekte). Ergebnis: Bereich (Geraete-Pixel), der neu gezeichnet werden muss,
    /// oder None, wenn ein komplettes Neuzeichnen guenstiger ist.
    fn item_diff(&self) -> Option<IRect> {
        let old: HashMap<*const Item, usize> = self
            .cache_items
            .iter()
            .enumerate()
            .map(|(i, it)| (Rc::as_ptr(it), i))
            .collect();
        let mut seen = vec![false; self.cache_items.len()];
        let mut last: isize = -1;
        let mut dmg = IRect::default();
        let limit = {
            let (w, h) = self.cache_size();
            w as u64 * h as u64 / 2
        };
        for it in self.items.iter().filter(|it| !it.lift) {
            match old.get(&Rc::as_ptr(it)) {
                Some(&i) if !seen[i] && i as isize > last => {
                    seen[i] = true;
                    last = i as isize;
                    continue;
                }
                Some(&i) => seen[i] = true,
                None => {}
            }
            dmg = dmg.union(self.cache_rect(&paint_bounds(it)));
            if dmg.area() > limit {
                return None;
            }
        }
        for (i, it) in self.cache_items.iter().enumerate() {
            if !seen[i] {
                dmg = dmg.union(self.cache_rect(&paint_bounds(it)));
                if dmg.area() > limit {
                    return None;
                }
            }
        }
        Some(dmg)
    }

    /// Verschiebt den Inhalt um (dx, dy) Geraete-Pixel: nur der Ursprung des Ring-Speichers wandert, gezeichnet
    /// werden nur die frei gewordenen Streifen.
    fn scroll_cache(&mut self, dx: i32, dy: i32) {
        let Some(cache) = self.cache.as_ref() else {
            return;
        };
        let (w, h) = (cache.width() as i32, cache.height() as i32);
        self.org = (
            (self.org.0 as i32 - dx).rem_euclid(w) as u32,
            (self.org.1 as i32 - dy).rem_euclid(h) as u32,
        );
        let ps = self.px_scale;
        self.ctx += dx as f32 / ps;
        self.cty += dy as f32 / ps;
        let (wu, hu) = (w as u32, h as u32);
        // was im Zielbild schon stand, wandert mit
        self.last_ov = shift_rect(self.last_ov, dx, dy, wu, hu);
        self.cache_dmg.shift(dx, dy, wu, hu);
        if dy > 0 {
            self.redraw_cache_rect(IRect {
                x: 0,
                y: 0,
                w: wu,
                h: dy as u32,
            });
        } else if dy < 0 {
            self.redraw_cache_rect(IRect {
                x: 0,
                y: (h + dy) as u32,
                w: wu,
                h: (-dy) as u32,
            });
        }
        let (y0, y1) = if dy > 0 { (dy, h) } else { (0, h + dy) };
        if dx > 0 {
            self.redraw_cache_rect(IRect {
                x: 0,
                y: y0 as u32,
                w: dx as u32,
                h: (y1 - y0) as u32,
            });
        } else if dx < 0 {
            self.redraw_cache_rect(IRect {
                x: (w + dx) as u32,
                y: y0 as u32,
                w: (-dx) as u32,
                h: (y1 - y0) as u32,
            });
        }
    }

    /// Bringt den Zwischenspeicher auf den aktuellen Stand: verschiebt ihn bei gleichem Zoom, zeichnet geaenderte
    /// Objekte nur in ihrem Bereich neu und nur, wenn noetig, alles.
    fn update_cache(&mut self) {
        let (dw, dh) = self.cache_size();
        let size_ok = matches!(&self.cache, Some(c) if c.width() == dw && c.height() == dh);
        if !size_ok
            || self.full_dirty
            || self.cache_sig.as_ref() != Some(&self.sig())
            || self.csc != self.sc
        {
            self.render_cache();
            return;
        }
        let ps = self.px_scale;
        let (sx, sy) = (
            ((self.tx - self.ctx) * ps).round(),
            ((self.ty - self.cty) * ps).round(),
        );
        if sx != 0.0 || sy != 0.0 {
            if sx.abs() >= dw as f32 * 0.7 || sy.abs() >= dh as f32 * 0.7 {
                self.render_cache();
                return;
            }
            self.scroll_cache(sx as i32, sy as i32);
        }
        let gest = self.mode == 2 || self.mode == 3 || self.wheel_until.is_some();
        if !gest {
            // in Ruhe auf das Pixelraster des Zwischenspeichers einrasten (< 0,5 Geraete-Pixel)
            self.tx = self.ctx;
            self.ty = self.cty;
        }
        if self.dirty {
            match self.item_diff() {
                Some(r) => {
                    self.redraw_cache_rect(r);
                    self.cache_items = self.items.iter().filter(|it| !it.lift).cloned().collect();
                    self.dirty = false;
                }
                None => self.render_cache(),
            }
        }
    }

    fn pattern(&self, c: &mut Canvas) {
        if self.bg == 3 {
            return;
        }
        let dp = self.dp;
        let mut step = 32.0 * dp * self.sc;
        while step < 18.0 * dp {
            step *= 2.0;
        }
        while step > 72.0 * dp {
            step /= 2.0;
        }
        let (w, hh) = (self.width, self.height);
        // eine Reihe vor dem Rand beginnen: auch angeschnittene Punkte/Linien am Rand zeichnen
        // (sonst fehlen sie in Streifen, die beim Verschieben einzeln gezeichnet werden)
        let ox = ((self.tx % step) + step) % step - step;
        let oy = ((self.ty % step) + step) % step - step;
        let col = if self.dark { 0xFF343741 } else { 0xFFD3D6DD };
        if self.bg == 0 {
            let mut dots = Vec::new();
            let mut y = oy;
            while y < hh + step {
                let mut x = ox;
                while x < w + step {
                    dots.push(x);
                    dots.push(y);
                    x += step;
                }
                y += step;
            }
            c.draw_points(&dots, &Paint::stroke(col, 2.2 * dp, Cap::Round));
        } else {
            let mut pb = PathBuilder::new();
            let mut y = oy;
            while y < hh + step {
                pb.move_to(0.0, y);
                pb.line_to(w, y);
                y += step;
            }
            if self.bg == 1 {
                let mut x = ox;
                while x < w + step {
                    pb.move_to(x, 0.0);
                    pb.line_to(x, hh);
                    x += step;
                }
            }
            if let Some(p) = pb.finish() {
                c.draw_path(&p, &Paint::stroke(col, dp, Cap::Butt));
            }
        }
    }

    fn draw_frame(&mut self, c: &mut Canvas) {
        self.ensure_frame();
        let dp = self.dp;
        let pad = 6.0 * dp / self.sc;
        let (hw, hh) = (self.sel_w / 2.0 + pad, self.sel_h / 2.0 + pad);
        let (sgx, sgy) = ([-1.0f32, 1.0, 1.0, -1.0], [-1.0f32, -1.0, 1.0, 1.0]);
        if let Some(q) = self.lin_ends() {
            // Linie/Pfeil: nur die beiden Endpunkt-Griffe
            let (sc, tx, ty) = (self.sc, self.tx, self.ty);
            for (x, y) in [(q[0], q[1]), (q[2], q[3])] {
                let (x, y) = (x * sc + tx, y * sc + ty);
                c.draw_circle(x, y, 7.0 * dp, &Paint::fill(ACCENT));
                c.draw_circle(
                    x,
                    y,
                    7.0 * dp,
                    &Paint::stroke(0xFFFFFFFF, 2.0 * dp, Cap::Butt),
                );
            }
            return;
        }
        let mut corners = [(0.0f32, 0.0f32); 4];
        let mut pb = PathBuilder::new();
        for k in 0..4 {
            let o = self.ls(sgx[k] * hw, sgy[k] * hh);
            corners[k] = o;
            if k == 0 {
                pb.move_to(o.0, o.1);
            } else {
                pb.line_to(o.0, o.1);
            }
        }
        pb.close();
        let mut ui = Paint::stroke(ACCENT, 1.5 * dp, Cap::Butt);
        ui.dash = Some((7.0 * dp, 5.0 * dp));
        if let Some(p) = pb.finish() {
            c.draw_path(&p, &ui);
        }
        ui.dash = None;
        // Dreh-Griff
        let (tx0, ty0) = self.ls(0.0, -hh);
        let (rx, ry) = self.ls(0.0, -hh - 32.0 * dp / self.sc);
        c.draw_line(tx0, ty0, rx, ry, &ui);
        c.draw_circle(rx, ry, 14.0 * dp, &Paint::fill(ACCENT));
        c.draw_circle(
            rx,
            ry,
            14.0 * dp,
            &Paint::stroke(0xFFFFFFFF, 2.0 * dp, Cap::Butt),
        );
        c.save();
        c.translate(rx - 9.0 * dp, ry - 9.0 * dp);
        c.scale(18.0 * dp / 24.0, 18.0 * dp / 24.0);
        icons::draw(c, icons::ROTATE, 0xFFFFFFFF, false);
        c.restore();
        // Eck-Griffe
        for (x, y) in corners {
            c.draw_circle(x, y, 7.0 * dp, &Paint::fill(ACCENT));
            c.draw_circle(
                x,
                y,
                7.0 * dp,
                &Paint::stroke(0xFFFFFFFF, 2.0 * dp, Cap::Butt),
            );
        }
        // Kanten-Griffe (Mitte jeder Seite)
        let (egx, egy) = ([0.0f32, 1.0, 0.0, -1.0], [-1.0f32, 0.0, 1.0, 0.0]);
        for k in 0..4 {
            if self.edge_visible(k) {
                let (x, y) = self.ls(egx[k] * hw, egy[k] * hh);
                c.draw_circle(x, y, 7.0 * dp, &Paint::fill(ACCENT));
                c.draw_circle(
                    x,
                    y,
                    7.0 * dp,
                    &Paint::stroke(0xFFFFFFFF, 2.0 * dp, Cap::Butt),
                );
            }
        }
        if self.mode == 1 && self.drag == 4 && self.moved {
            let deg = (self.sel_ang.to_degrees().round() as i32).rem_euclid(360);
            let t = format!("{deg}\u{00B0}");
            let ts = 14.0 * dp;
            let tw = crate::text::measure(&t, ts);
            let (ph, pw) = (28.0 * dp, tw + 20.0 * dp);
            let (l, tp) = (rx + 20.0 * dp, ry - ph / 2.0);
            c.draw_round_rect(l, tp, l + pw, tp + ph, ph / 2.0, &Paint::fill(0xEE1E1F25));
            let mut tpaint = Paint::fill(0xFFFFFFFF);
            tpaint.text_size = ts;
            c.draw_text(&t, l + 10.0 * dp, tp + ph / 2.0 + 5.0 * dp, &tpaint);
        }
    }

    /// Farbe hinter dem Inhalt (fuer Flaechen ausserhalb der skalierten Textur, siehe [`Frame::scaled`]).
    pub fn backdrop_color(&self) -> u32 {
        self.base_color()
    }

    /// Erzwingt beim naechsten render() ein vollstaendiges Bild (z. B. neue Textur in der Oberflaeche).
    pub fn force_full_frame(&mut self) {
        self.shown = None;
        self.invalidate();
    }

    /// Huelle der Ueberlagerungen (angehobene Auswahl, aktueller Strich, Rahmen, Radierer) in Geraete-Pixeln.
    fn overlay_rect(&mut self, gest: bool, w: u32, h: u32) -> IRect {
        let (ps, sc, tx, ty, dp) = (self.px_scale, self.sc, self.tx, self.ty, self.dp);
        let mut out = IRect::default();
        let world = |b: &RectF| {
            IRect::cover(
                (b.left * sc + tx) * ps - 2.0,
                (b.top * sc + ty) * ps - 2.0,
                (b.right * sc + tx) * ps + 2.0,
                (b.bottom * sc + ty) * ps + 2.0,
                w,
                h,
            )
        };
        let screen = |l: f32, t: f32, r: f32, b: f32| {
            IRect::cover(l * ps - 2.0, t * ps - 2.0, r * ps + 2.0, b * ps + 2.0, w, h)
        };
        if self.lifted && !gest {
            for it in self.sel_items() {
                out = out.union(world(&paint_bounds(it)));
            }
        }
        for it in [self.snap.as_ref().or(self.cur.as_ref()), self.prev.as_ref()]
            .into_iter()
            .flatten()
        {
            out = out.union(world(&paint_bounds(it)));
        }
        if self.tool == SELECT
            && self.mode == 1
            && self.drag == 3
            && self.sel_mode == 0
            && self.lasso.len() > 2
            && self.moved
        {
            let mut r = RectF::new(self.lasso[0], self.lasso[1], self.lasso[0], self.lasso[1]);
            for xy in self.lasso.chunks_exact(2) {
                r.union(&RectF::new(xy[0], xy[1], xy[0], xy[1]));
            }
            r.inset(-3.0 * dp / sc, -3.0 * dp / sc);
            out = out.union(world(&r));
        }
        if let Some(m) = self.marq {
            let mut r = RectF::new(
                m.left.min(m.right),
                m.top.min(m.bottom),
                m.left.max(m.right),
                m.top.max(m.bottom),
            );
            r.inset(-3.0 * dp / sc, -3.0 * dp / sc);
            out = out.union(world(&r));
        }
        if self.tool == SELECT
            && !self.sel.is_empty()
            && !(self.mode == 1 && self.drag == 3 && self.moved)
        {
            self.ensure_frame();
            let pad = 6.0 * dp / sc;
            let (hw, hh) = (self.sel_w / 2.0 + pad, self.sel_h / 2.0 + pad);
            let mut pts = vec![
                self.ls(-hw, -hh),
                self.ls(hw, -hh),
                self.ls(hw, hh),
                self.ls(-hw, hh),
                self.ls(0.0, -hh - 32.0 * dp / sc),
            ];
            if let Some(q) = self.lin_ends() {
                pts.push((q[0] * sc + tx, q[1] * sc + ty));
                pts.push((q[2] * sc + tx, q[3] * sc + ty));
            }
            let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for (x, y) in pts {
                l = l.min(x);
                t = t.min(y);
                r = r.max(x);
                b = b.max(y);
            }
            let m = 18.0 * dp;
            out = out.union(screen(l - m, t - m, r + m, b + m));
            if self.mode == 1 && self.drag == 4 {
                let (rx, ry) = self.ls(0.0, -hh - 32.0 * dp / sc);
                out = out.union(screen(rx, ry - 20.0 * dp, rx + 160.0 * dp, ry + 20.0 * dp));
            }
        }
        if self.ex >= 0.0 {
            let r = self.erase_screen_r() + 3.0 * dp;
            out = out.union(screen(self.ex - r, self.ey - r, self.ex + r, self.ey + r));
        }
        out
    }

    /// Zeichnet die Ansicht in target (Groesse = logische Groesse × px_scale). Liefert den geaenderten Bereich
    /// (Dirty-Rect) bzw. waehrend Zoom-Gesten die Skalierung der Textur.
    pub fn render(&mut self, target: &mut Pixmap) -> Frame {
        self.needs_redraw = false;
        let (tw, th) = (target.width(), target.height());
        let full = IRect {
            x: 0,
            y: 0,
            w: tw,
            h: th,
        };
        if self.width <= 0.0 || self.height <= 0.0 {
            return Frame {
                org: self.org,
                ..Frame::default()
            };
        }
        let gest = self.mode == 2 || self.mode == 3 || self.wheel_until.is_some();
        let key = (target.data().as_ptr() as usize, tw, th);
        let same_target = self.shown == Some(key);
        let size_ok = self.cache_size() == (tw, th);

        // Zoom-Geste: vorhandene Textur auf der GPU skalieren
        if let (true, Some(cache), true) = (
            gest && size_ok && same_target && !self.dirty,
            self.cache.as_ref(),
            self.sc != self.csc && self.cache_sig.is_some(),
        ) {
            let mut fr = Frame::default();
            let mut d = std::mem::take(&mut self.cache_dmg);
            d.add(self.last_ov);
            self.last_ov = IRect::default();
            for r in d.rects() {
                copy_rect(target, cache, *r, self.org);
            }
            fr.damage = d.0;
            let k = self.sc / self.csc;
            fr.scaled = Some((k, self.tx - self.ctx * k, self.ty - self.cty * k));
            fr.org = self.org;
            return fr;
        }

        if !gest || self.sc == self.csc {
            self.update_cache();
        }
        let cache_ok = size_ok && self.cache.is_some() && self.csc == self.sc;
        if !cache_ok {
            // Ausweichweg (z. B. Zoom-Geste ohne gueltigen Zwischenspeicher): alles direkt zeichnen
            {
                let ps = self.px_scale;
                let mut c = Canvas::new(target);
                c.scale(ps, ps);
                self.backdrop(&mut c);
                let vis = RectF::new(
                    -self.tx / self.sc,
                    -self.ty / self.sc,
                    (self.width - self.tx) / self.sc,
                    (self.height - self.ty) / self.sc,
                );
                c.translate(self.tx, self.ty);
                c.scale(self.sc, self.sc);
                self.draw_scene(&mut c, Some(&vis), false, true, self.dp / self.sc);
            }
            self.shown = None;
            return Frame {
                damage: vec![full],
                scaled: None,
                org: (0, 0),
            };
        }

        let org = self.org;
        let ov = self.overlay_rect(gest, tw, th);
        let mut dmg = std::mem::take(&mut self.cache_dmg);
        if same_target {
            dmg.add(self.last_ov);
            dmg.add(ov);
        } else {
            dmg = Damage(vec![full]);
        }
        self.last_ov = ov;
        self.shown = Some(key);
        if dmg.is_empty() {
            return Frame {
                org,
                ..Frame::default()
            };
        }
        // grosser Anteil: alles kopieren, dann nur die Ueberlagerungen
        if dmg.area() * 10 >= full.area() * 6 {
            target
                .data_mut()
                .copy_from_slice(self.cache.as_ref().unwrap().data());
            dmg = Damage(vec![full]);
            if ov.is_empty() {
                return Frame {
                    damage: dmg.0,
                    scaled: None,
                    org,
                };
            }
        }
        // je Bereich: Zwischenspeicher-Ausschnitt + Ueberlagerungen (falls sie hineinreichen)
        let only_ov = dmg.0 == [full];
        let parts: Vec<IRect> = if only_ov { vec![ov] } else { dmg.0.clone() };
        for r in parts {
            let Some(mut tmp) = crop(self.cache.as_ref().unwrap(), r, org) else {
                continue;
            };
            if !ov.is_empty() && intersects(ov, r) {
                let ps = self.px_scale;
                let mut c = Canvas::new(&mut tmp);
                c.translate(-(r.x as f32), -(r.y as f32));
                c.scale(ps, ps);
                self.draw_overlays(&mut c, gest);
            }
            blit(target, &tmp, r, org);
        }
        Frame {
            damage: dmg.0,
            scaled: None,
            org,
        }
    }

    /// Ueberlagerungen ueber dem Zwischenspeicher; c steht auf logischen Bildschirm-Pixeln.
    fn draw_overlays(&mut self, c: &mut Canvas, gest: bool) {
        c.save();
        c.translate(self.tx, self.ty);
        c.scale(self.sc, self.sc);
        c.save();
        self.clip_pages(c);
        if self.lifted && !gest {
            for it in self.sel_items() {
                it.draw(c, self.dark);
            }
        }
        if let Some(s) = &self.snap {
            s.draw(c, self.dark);
        } else if let Some(s) = &self.cur {
            s.draw(c, self.dark);
        }
        if let Some(p) = &self.prev {
            p.draw(c, self.dark);
        }
        c.restore();
        let dp = self.dp;
        if self.tool == SELECT
            && self.mode == 1
            && self.drag == 3
            && self.sel_mode == 0
            && self.lasso.len() > 2
            && self.moved
        {
            let mut pb = PathBuilder::new();
            pb.move_to(self.lasso[0], self.lasso[1]);
            for xy in self.lasso[2..].chunks_exact(2) {
                pb.line_to(xy[0], xy[1]);
            }
            pb.close();
            if let Some(p) = pb.finish() {
                c.draw_path(&p, &Paint::fill(0x227C8CFF));
                let mut st = Paint::stroke(ACCENT, 1.5 * dp / self.sc, Cap::Butt);
                st.dash = Some((7.0 * dp / self.sc, 5.0 * dp / self.sc));
                c.draw_path(&p, &st);
            }
        }
        if let Some(m) = self.marq {
            c.draw_rect(m.left, m.top, m.right, m.bottom, &Paint::fill(0x227C8CFF));
            c.draw_rect(
                m.left,
                m.top,
                m.right,
                m.bottom,
                &Paint::stroke(ACCENT, 1.5 * dp / self.sc, Cap::Butt),
            );
        }
        c.restore();

        if self.tool == SELECT
            && !self.sel.is_empty()
            && !(self.mode == 1 && self.drag == 3 && self.moved)
        {
            self.draw_frame(c);
        }

        if self.ex >= 0.0 {
            let col = if self.dark { 0xAAFFFFFF } else { 0xAA000000 };
            let mut p = Paint::stroke(col, 1.5 * dp, Cap::Butt);
            p.style = Style::Stroke;
            c.draw_circle(self.ex, self.ey, self.erase_screen_r(), &p);
        }
    }
}

/// Hintergrundfarbe einer Haftnotiz zur Stiftfarbe c.
pub fn note_bg(c: u32) -> u32 {
    if c == INK || c == 0xFF8B8D98 {
        return 0xFFFFE27A;
    }
    let (r, g, b) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
    0xFF000000
        | ((r + (255 - r) * 6 / 10) << 16)
        | ((g + (255 - g) * 6 / 10) << 8)
        | (b + (255 - b) * 6 / 10)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(b: &mut Board, pts: &[(f32, f32)], tool: ToolType) {
        b.on_touch(&Motion::single(Action::Down, pts[0].0, pts[0].1, 0.5, tool));
        for p in &pts[1..] {
            b.on_touch(&Motion::single(Action::Move, p.0, p.1, 0.5, tool));
        }
        let l = pts[pts.len() - 1];
        b.on_touch(&Motion::single(Action::Up, l.0, l.1, 0.5, tool));
    }

    fn board() -> Board {
        let mut b = Board::new(1.0);
        b.set_size(800.0, 600.0, 1.0);
        b.recog = 2;
        b
    }

    #[test]
    fn text_antippen_waehlt_aus_und_bearbeitet() {
        let mut b = board();
        b.set_tool(TEXT);
        // Tippen ins Leere: neuer Text wird erfragt
        drag(&mut b, &[(100.0, 100.0)], ToolType::Stylus);
        assert!(b.events.iter().any(|e| matches!(e, Event::AskText { .. })));
        b.events.clear();
        b.add_note_rect(
            "Hallo Welt, ein laengerer Text",
            b.new_note_rect(100.0, 100.0, false),
            false,
        );
        let id = b.items[0].id;
        // Tippen auf den Text: Auswahl (Menü) im Auswahlwerkzeug, kein neuer Text
        drag(&mut b, &[(170.0, 100.0)], ToolType::Stylus);
        assert_eq!(b.sel, vec![id]);
        assert_eq!(b.tool, SELECT);
        assert!(!b.events.iter().any(|e| matches!(e, Event::AskText { .. })));
        // Erneutes Tippen: direkt bearbeiten
        drag(&mut b, &[(170.0, 100.0)], ToolType::Stylus);
        assert!(b.events.contains(&Event::EditNote(id)));
        // Tippen daneben: abwählen und zurück zum Textwerkzeug
        drag(&mut b, &[(500.0, 400.0)], ToolType::Stylus);
        assert!(b.sel.is_empty());
        assert_eq!(b.tool, TEXT);
    }

    #[test]
    fn mittelklick_trifft_innerhalb_einer_form() {
        let mut b = board();
        b.items.push(Rc::new(Item::shape(
            item::POLY,
            vec![100.0, 100.0, 300.0, 100.0, 300.0, 200.0, 100.0, 200.0],
            INK,
            2.0,
        )));
        let id = b.items[0].id;
        // Textwerkzeug trifft nur Texte
        assert!(!b.select_at_screen(200.0, 150.0, true));
        b.set_tool(PEN);
        assert!(b.select_at_screen(200.0, 150.0, false));
        assert_eq!((b.sel.clone(), b.tool), (vec![id], SELECT));
        b.deselect();
        assert_eq!(b.tool, PEN);
        assert!(!b.select_at_screen(500.0, 400.0, false));
    }

    #[test]
    fn textgroesse_und_ausblenden() {
        let mut b = board();
        b.add_note_rect("Hallo", (10.0, 10.0, 20.0, 100.0, 80.0), true);
        let id = b.items[0].id;
        b.select_only(id);
        assert_eq!(b.sel_note_size(), Some(20.0));
        b.set_sel_note_size(40.0, true);
        let n = b.items[0].as_note().unwrap();
        assert_eq!((n.size, n.w, n.mh), (40.0, 200.0, 160.0));
        b.undo();
        assert_eq!(b.items[0].as_note().unwrap().size, 20.0);
        b.set_hidden(Some(id));
        assert!(b.sel.is_empty());
        b.delete_item(id);
        assert!(b.items.is_empty());
    }

    #[test]
    fn draw_undo_redo_and_json() {
        let mut b = board();
        drag(
            &mut b,
            &[(10.0, 10.0), (50.0, 20.0), (90.0, 60.0)],
            ToolType::Stylus,
        );
        assert_eq!(b.items.len(), 1);
        b.undo();
        assert_eq!(b.items.len(), 0);
        b.redo();
        assert_eq!(b.items.len(), 1);
        let j = b.to_json();
        let mut c = board();
        c.from_json(&j);
        assert_eq!(c.to_json(), j);
        let mut pm = Pixmap::new(800, 600).unwrap();
        b.render(&mut pm);
        assert_ne!(pm.pixel(50, 21).unwrap().red(), 0xFB);
    }

    #[test]
    fn strichdicke_unabhaengig_vom_zoom() {
        let mut ws = Vec::new();
        for z in [0.5f32, 1.0, 3.0] {
            let mut b = board();
            b.zoom_at(0.0, 0.0, z);
            b.recog = 2;
            drag(
                &mut b,
                &[(10.0, 100.0), (200.0, 140.0), (390.0, 100.0)],
                ToolType::Stylus,
            );
            b.tool = SHAPE;
            drag(&mut b, &[(10.0, 300.0), (200.0, 400.0)], ToolType::Stylus);
            ws.push(b.items.iter().map(|it| it.width).collect::<Vec<_>>());
        }
        assert!(ws.iter().all(|w| w.len() == 2 && *w == ws[0]), "{ws:?}");
    }

    #[test]
    fn farbe_der_auswahl() {
        let mut b = board();
        b.recog = 2;
        drag(&mut b, &[(10.0, 100.0), (200.0, 140.0)], ToolType::Stylus);
        b.tool = SELECT;
        assert_eq!(b.sel_color(), None);
        b.sel = b.items.iter().map(|it| it.id).collect();
        b.set_color(0xFFE5484D);
        assert_eq!(b.sel_color(), Some(Some(0xFFE5484D)));
        b.undo();
        assert_ne!(b.items[0].color, 0xFFE5484D);
    }

    #[test]
    fn eraser_modes() {
        let mut b = board();
        drag(&mut b, &[(10.0, 100.0), (400.0, 100.0)], ToolType::Stylus);
        b.tool = ERASER;
        b.erase_mode = 1;
        drag(&mut b, &[(200.0, 80.0), (200.0, 120.0)], ToolType::Stylus);
        assert_eq!(b.items.len(), 2);
        b.erase_mode = 0;
        drag(&mut b, &[(20.0, 90.0), (20.0, 110.0)], ToolType::Stylus);
        assert_eq!(b.items.len(), 1);
    }

    #[test]
    fn lasso_move_rotate() {
        let mut b = board();
        drag(&mut b, &[(100.0, 100.0), (150.0, 100.0)], ToolType::Stylus);
        drag(&mut b, &[(500.0, 500.0), (550.0, 500.0)], ToolType::Stylus);
        b.set_tool(SELECT);
        drag(
            &mut b,
            &[
                (80.0, 80.0),
                (200.0, 80.0),
                (200.0, 130.0),
                (80.0, 130.0),
                (80.0, 85.0),
            ],
            ToolType::Stylus,
        );
        assert_eq!(b.sel.len(), 1);
        let before = b.sel_items()[0].bounds();
        drag(
            &mut b,
            &[(125.0, 100.0), (135.0, 110.0), (145.0, 120.0)],
            ToolType::Stylus,
        );
        let after = b.sel_items()[0].bounds();
        assert!((after.left - before.left - 20.0).abs() < 0.01);
        assert_eq!(b.undo.len(), 3);
        b.rotate_sel(std::f32::consts::FRAC_PI_2);
        let r = b.sel_items()[0].bounds();
        assert!(r.height() > r.width());
        b.copy_sel();
        b.paste();
        assert_eq!(b.items.len(), 3);
    }

    #[test]
    fn layers_and_template() {
        let mut b = board();
        b.apply_template();
        assert_eq!(b.layers.len(), 7);
        assert_eq!(b.layers[b.active].name, "05-figur");
        b.add_layer(Some("x"));
        assert_eq!(b.layers.len(), 8);
        b.edit_layer(b.active, |l| l.locked = true);
        drag(&mut b, &[(10.0, 10.0), (50.0, 50.0)], ToolType::Stylus);
        assert!(b.items.is_empty());
        assert!(b
            .events
            .contains(&Event::Notice("Ebene ist gesperrt".into())));
        b.undo();
        assert_eq!(b.layers.len(), 7);
    }

    #[test]
    fn note_bg_colors() {
        assert_eq!(note_bg(INK), 0xFFFFE27A);
        assert_eq!(note_bg(0xFFE5484D), 0xFFF4B5B7);
    }

    // ---------- Auswahl: Kanten- und Endpunkt-Griffe ----------

    fn sel_board(it: Item) -> Board {
        let mut b = board();
        b.items.push(Rc::new(it));
        b.set_tool(SELECT);
        b.sel = vec![b.items[0].id];
        b.frame_ok = false;
        b.ensure_frame();
        b
    }

    #[test]
    fn kanten_griff_skaliert_nur_breite() {
        let mut b = sel_board(Item::shape(
            item::POLY,
            vec![100.0, 100.0, 200.0, 100.0, 200.0, 150.0, 100.0, 150.0],
            INK,
            2.0,
        ));
        // rechter Kanten-Griff bei (206, 125) -> 100 Pixel nach rechts: Breite verdoppelt
        drag(
            &mut b,
            &[(206.0, 125.0), (250.0, 125.0), (306.0, 125.0)],
            ToolType::Stylus,
        );
        let r = b.sel_items()[0].bounds();
        let (w, h) = (r.width(), r.height());
        assert!((w - 200.0 - 2.0).abs() < 8.0, "Breite {w}");
        assert!((h - 50.0 - 2.0).abs() < 4.0, "Hoehe {h}");
        assert!((r.left - 99.0).abs() < 3.0, "linke Kante fest {}", r.left);
        assert!(
            (b.sel_w - w).abs() < 8.0 && (b.sel_h - h).abs() < 8.0,
            "{} {}",
            b.sel_w,
            b.sel_h
        );
    }

    #[test]
    fn endpunkt_griff_und_undo() {
        let mut b = sel_board(Item::shape(
            item::ARROW,
            vec![100.0, 100.0, 200.0, 100.0],
            INK,
            2.0,
        ));
        let n = b.undo.len();
        drag(
            &mut b,
            &[(200.0, 100.0), (250.0, 160.0), (300.0, 200.0)],
            ToolType::Stylus,
        );
        let q = b.sel_items()[0].as_shape().unwrap().q.clone();
        assert_eq!(&q[0..2], &[100.0, 100.0]);
        assert!(q[2] > 250.0 && q[3] > 150.0, "{q:?}");
        assert_eq!(b.undo.len(), n + 1);
        assert!(b.sel_w > 150.0);
        b.undo();
        let q = b.items[0].as_shape().unwrap().q.clone();
        assert_eq!(q, vec![100.0, 100.0, 200.0, 100.0]);
    }

    // ---------- Seitenmodus (WP2) ----------

    use crate::pages::{PageSetup, PAGE_GAP};

    /// Board im Seitenmodus mit n A4-Seiten (595 × 842), Massstab 0,5, Ursprung bei (10, 10).
    fn paged(n: usize, bg: PageBg) -> Board {
        let mut b = board();
        b.board_mode = BoardMode::Pages(PageSetup::new(595.0, 842.0, bg, n));
        b.sc = 0.5;
        b.tx = 10.0;
        b.ty = 10.0;
        b
    }
    /// Welt → Bildschirm fuer paged().
    fn scr(x: f32, y: f32) -> (f32, f32) {
        (10.0 + x * 0.5, 10.0 + y * 0.5)
    }

    #[test]
    fn page_layout_and_lookup() {
        let ps = PageSetup::new(595.0, 842.0, PageBg::Grid, 3);
        let r = ps.rects();
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], RectF::new(0.0, 0.0, 595.0, 842.0));
        assert_eq!(r[1].top, 842.0 + PAGE_GAP);
        assert_eq!(r[2].top, 2.0 * (842.0 + PAGE_GAP));
        assert!(r.iter().all(|x| x.left == 0.0 && x.width() == 595.0));
        assert_eq!(ps.bounds().unwrap().bottom, 3.0 * 842.0 + 2.0 * PAGE_GAP);
        assert_eq!(ps.page_at(10.0, 10.0), Some(0));
        assert_eq!(ps.page_at(300.0, 842.0 + PAGE_GAP + 5.0), Some(1));
        assert_eq!(ps.page_at(300.0, 842.0 + PAGE_GAP / 2.0), None);
        assert_eq!(ps.page_at(-5.0, 10.0), None);
        assert_eq!(ps.nearest_page(300.0, 842.0 + 2.0), Some(0));
        assert_eq!(ps.nearest_page(300.0, 99999.0), Some(2));
        let it = Item::shape(0, vec![10.0, 900.0, 50.0, 940.0], INK, 2.0);
        assert_eq!(ps.page_of(&it), Some(1));
    }

    #[test]
    fn page_ops_move_items_and_undo() {
        let mut b = paged(2, PageBg::Lined);
        // Strich auf Seite 0 und auf Seite 1
        let (a0, a1) = (scr(100.0, 100.0), scr(200.0, 120.0));
        drag(&mut b, &[a0, a1], ToolType::Stylus);
        let y1 = 842.0 + PAGE_GAP + 100.0;
        let (c0, c1) = (scr(100.0, y1), scr(200.0, y1 + 20.0));
        drag(&mut b, &[c0, c1], ToolType::Stylus);
        assert_eq!(b.items.len(), 2);
        assert_eq!(b.page_of_items(), vec![Some(0), Some(1)]);
        let top = |b: &Board, i: usize| b.items[i].bounds().top;
        let (t0, t1) = (top(&b, 0), top(&b, 1));
        let ids: Vec<String> = b
            .pages()
            .unwrap()
            .pages
            .iter()
            .map(|p| p.id.clone())
            .collect();

        // Seite vorne einfuegen: beide Striche rutschen um eine Seite nach unten
        assert_eq!(b.add_page(None), Some(0));
        assert_eq!(b.pages().unwrap().pages.len(), 3);
        assert!((top(&b, 0) - t0 - (842.0 + PAGE_GAP)).abs() < 0.01);
        assert!((top(&b, 1) - t1 - (842.0 + PAGE_GAP)).abs() < 0.01);
        assert_eq!(b.page_of_items(), vec![Some(1), Some(2)]);
        b.undo();
        assert_eq!(b.pages().unwrap().pages.len(), 2);
        assert!((top(&b, 0) - t0).abs() < 0.01);
        b.redo();
        assert_eq!(b.pages().unwrap().pages.len(), 3);
        b.undo();

        // Seiten tauschen
        assert!(b.move_page(1, 0));
        let ids2: Vec<String> = b
            .pages()
            .unwrap()
            .pages
            .iter()
            .map(|p| p.id.clone())
            .collect();
        assert_eq!(ids2, vec![ids[1].clone(), ids[0].clone()]);
        assert_eq!(b.page_of_items(), vec![Some(1), Some(0)]);
        assert!((top(&b, 0) - t0 - (842.0 + PAGE_GAP)).abs() < 0.01);
        assert!((top(&b, 1) - t0).abs() < 0.01);
        b.undo();
        assert_eq!(b.page_of_items(), vec![Some(0), Some(1)]);

        // Seite 0 loeschen: ihr Strich verschwindet, der andere rueckt nach oben
        assert!(b.remove_page(0));
        assert_eq!(b.items.len(), 1);
        assert!((top(&b, 0) - t0).abs() < 0.01);
        assert_eq!(b.pages().unwrap().pages[0].id, ids[1]);
        assert!(!b.remove_page(0), "letzte Seite bleibt");
        b.undo();
        assert_eq!(b.items.len(), 2);
        assert_eq!(b.pages().unwrap().pages.len(), 2);

        // Hintergrund je Seite
        assert!(b.set_page_bg(Some(1), PageBg::Dotted));
        assert_eq!(b.pages().unwrap().pages[1].bg, PageBg::Dotted);
        assert_eq!(b.pages().unwrap().pages[0].bg, PageBg::Lined);
        b.undo();
        assert_eq!(b.pages().unwrap().pages[1].bg, PageBg::Lined);

        // unendlicher Modus: keine Seiten-Operationen
        let mut inf = board();
        assert_eq!(inf.add_page(None), None);
        assert!(!inf.remove_page(0));
    }

    #[test]
    fn strokes_outside_pages_are_dropped() {
        let mut b = paged(2, PageBg::Plain);
        // beginnt im Zwischenraum
        let g = 842.0 + PAGE_GAP / 2.0;
        drag(
            &mut b,
            &[scr(100.0, g), scr(100.0, g + 200.0)],
            ToolType::Stylus,
        );
        assert!(b.items.is_empty());
        // beginnt rechts neben der Seite
        drag(
            &mut b,
            &[scr(700.0, 100.0), scr(300.0, 100.0)],
            ToolType::Stylus,
        );
        assert!(b.items.is_empty());
        // beginnt auf der Seite, endet ausserhalb: bleibt
        drag(
            &mut b,
            &[scr(300.0, 100.0), scr(700.0, 100.0)],
            ToolType::Stylus,
        );
        assert_eq!(b.items.len(), 1);
    }

    #[test]
    fn page_clipping_render() {
        let mut b = paged(2, PageBg::Plain);
        // senkrechter Strich ueber die Seitengrenze hinweg (von Seite 0 bis Seite 1)
        drag(
            &mut b,
            &[scr(300.0, 700.0), scr(300.0, 842.0 + PAGE_GAP + 150.0)],
            ToolType::Stylus,
        );
        assert_eq!(b.items.len(), 1);
        b.pen_w = 6.0;
        let mut pm = Pixmap::new(800, 600).unwrap();
        b.render(&mut pm);
        let px = |x: f32, y: f32| {
            let (sx, sy) = scr(x, y);
            let p = pm.pixel(sx.round() as u32, sy.round() as u32).unwrap();
            0xFF000000 | (p.red() as u32) << 16 | (p.green() as u32) << 8 | p.blue() as u32
        };
        let desk = b.desk_color();
        // im Zwischenraum: Strich geht darueber, sichtbar ist nur der Zwischenraum
        assert_eq!(px(300.0, 842.0 + PAGE_GAP / 2.0), desk);
        // auf der Seite ist der Strich sichtbar
        assert_ne!(px(300.0, 780.0), b.page_color());
        assert_ne!(px(300.0, 780.0), desk);
        // neben dem Strich: Papier
        assert_eq!(px(200.0, 780.0), b.page_color());
        // ausserhalb rechts: Zwischenraum-Farbe
        assert_eq!(px(700.0, 100.0), desk);
        // Export-PNG: alle Seiten untereinander
        let (img, bounds, _) = b.export_image(3072.0, 2.0).unwrap();
        assert!(img.height() > img.width());
        assert!((bounds.height() - (2.0 * 842.0 + PAGE_GAP + 80.0)).abs() < 0.01);
    }

    #[test]
    fn lasso_across_pages_and_eraser() {
        let mut b = paged(2, PageBg::Grid);
        drag(
            &mut b,
            &[scr(100.0, 800.0), scr(200.0, 800.0)],
            ToolType::Stylus,
        );
        let y1 = 842.0 + PAGE_GAP + 30.0;
        drag(&mut b, &[scr(100.0, y1), scr(200.0, y1)], ToolType::Stylus);
        assert_eq!(b.page_of_items(), vec![Some(0), Some(1)]);
        b.set_tool(SELECT);
        b.sel_mode = 0;
        let pts = [
            scr(50.0, 750.0),
            scr(250.0, 750.0),
            scr(250.0, y1 + 50.0),
            scr(50.0, y1 + 50.0),
            scr(50.0, 755.0),
        ];
        drag(&mut b, &pts, ToolType::Stylus);
        assert_eq!(b.sel.len(), 2);
        // Rechteckauswahl ebenso
        b.sel_mode = 1;
        drag(
            &mut b,
            &[scr(30.0, 600.0), scr(150.0, 870.0), scr(260.0, y1 + 60.0)],
            ToolType::Stylus,
        );
        assert_eq!(b.sel.len(), 2);
        // Radierer, der im Zwischenraum beginnt und beide Striche ueberstreicht
        b.set_tool(ERASER);
        drag(
            &mut b,
            &[
                scr(150.0, 780.0),
                scr(150.0, 842.0 + PAGE_GAP / 2.0),
                scr(150.0, y1 + 20.0),
            ],
            ToolType::Stylus,
        );
        assert!(b.items.is_empty());
    }

    #[test]
    fn pdf_has_one_page_per_board_page() {
        let mut b = paged(3, PageBg::Lined);
        drag(
            &mut b,
            &[scr(100.0, 100.0), scr(300.0, 300.0)],
            ToolType::Stylus,
        );
        let pdf = b.export_pdf().unwrap();
        let s = String::from_utf8_lossy(&pdf);
        assert_eq!(s.matches("/Type /Page ").count(), 3);
        assert!(s.contains("/Count 3"));
        assert!(s.contains("/MediaBox [0 0 595.00 842.00]"));
        assert!(s.ends_with("%%EOF\n"));
        // auch ein leeres Seiten-Board ist exportierbar
        let e = paged(1, PageBg::Plain);
        assert!(e.export_bounds().is_some());
        assert_eq!(
            String::from_utf8_lossy(&e.export_pdf().unwrap())
                .matches("/Type /Page ")
                .count(),
            1
        );
    }

    #[test]
    fn fit_pages_width() {
        let mut b = paged(2, PageBg::Plain);
        b.fit();
        assert!((595.0 * b.sc - (800.0 - 32.0)).abs() < 0.01);
        assert!((b.tx + 595.0 / 2.0 * b.sc - 400.0).abs() < 0.01);
        let _ = b.thumb();
    }

    /// Vergleicht das schrittweise aufgebaute Bild (Teil-Updates, Verschieben, Dirty-Rects) mit einem frisch
    /// komplett gezeichneten. Erlaubt minimale Rundungsabweichungen der Kantenglaettung.
    fn assert_frame_eq(b: &mut Board, inc: &mut Pixmap, tag: &str) {
        let f = b.render(inc);
        // Ring-Speicher in normale Lage bringen
        let full = IRect {
            x: 0,
            y: 0,
            w: inc.width(),
            h: inc.height(),
        };
        let lin = crop(inc, full, f.org).unwrap();
        inc.data_mut().copy_from_slice(lin.data());
        let mut fresh = Pixmap::new(inc.width(), inc.height()).unwrap();
        b.full_dirty = true;
        b.dirty = true;
        b.render(&mut fresh);
        let (mut bad, mut worst) = (0usize, 0i32);
        for (a, c) in inc.data().iter().zip(fresh.data()) {
            let d = (*a as i32 - *c as i32).abs();
            worst = worst.max(d);
            if d > 3 {
                bad += 1;
            }
        }
        assert!(
            bad < 40 && worst < 64,
            "{tag}: {bad} Abweichungen, max {worst}"
        );
        // fortan schrittweise auf dem frischen Bild weiter
        inc.data_mut().copy_from_slice(fresh.data());
    }

    fn check_incremental(mut b: Board) {
        let mut pm = Pixmap::new(800, 600).unwrap();
        b.render(&mut pm);
        let stroke = |b: &mut Board, pm: &mut Pixmap, k: f32| {
            let pts: Vec<(f32, f32)> = (0..40)
                .map(|i| {
                    (
                        100.0 + k * 37.0 % 500.0 + i as f32 * 6.0,
                        120.0 + (i as f32 * 0.4 + k).sin() * 50.0 + k * 13.0 % 300.0,
                    )
                })
                .collect();
            b.on_touch(&Motion::single(
                Action::Down,
                pts[0].0,
                pts[0].1,
                0.5,
                ToolType::Stylus,
            ));
            for p in &pts[1..] {
                b.on_touch(&Motion::single(
                    Action::Move,
                    p.0,
                    p.1,
                    0.3 + (p.0 * 0.01).sin().abs() * 0.5,
                    ToolType::Stylus,
                ));
                b.render(pm);
            }
            let l = pts[pts.len() - 1];
            b.on_touch(&Motion::single(Action::Up, l.0, l.1, 0.5, ToolType::Stylus));
        };
        for k in 0..6 {
            stroke(&mut b, &mut pm, k as f32);
        }
        b.tool = MARKER;
        stroke(&mut b, &mut pm, 9.0);
        b.tool = PEN;
        assert_frame_eq(&mut b, &mut pm, "Striche");
        // Radieren (ganze Objekte)
        b.on_touch(&Motion::single(
            Action::Down,
            150.0,
            140.0,
            0.5,
            ToolType::Eraser,
        ));
        for i in 0..30 {
            b.on_touch(&Motion::single(
                Action::Move,
                150.0 + i as f32 * 8.0,
                140.0 + i as f32 * 3.0,
                0.5,
                ToolType::Eraser,
            ));
            b.render(&mut pm);
        }
        b.on_touch(&Motion::single(
            Action::Up,
            390.0,
            230.0,
            0.5,
            ToolType::Eraser,
        ));
        assert_frame_eq(&mut b, &mut pm, "Radieren");
        b.undo();
        b.render(&mut pm);
        b.redo();
        b.render(&mut pm);
        b.undo();
        assert_frame_eq(&mut b, &mut pm, "Rueckgaengig");
        // Verschieben mit dem Finger (krumme Schritte), dann in Ruhe
        b.on_touch(&Motion::single(
            Action::Down,
            400.0,
            300.0,
            0.5,
            ToolType::Finger,
        ));
        for i in 1..25 {
            b.on_touch(&Motion::single(
                Action::Move,
                400.0 - i as f32 * 7.3,
                300.0 + i as f32 * 4.7,
                0.5,
                ToolType::Finger,
            ));
            b.render(&mut pm);
        }
        b.on_touch(&Motion::single(
            Action::Up,
            400.0 - 25.0 * 7.3,
            300.0 + 25.0 * 4.7,
            0.5,
            ToolType::Finger,
        ));
        assert_frame_eq(&mut b, &mut pm, "Verschieben");
        // Mausrad-Verschieben und -Zoom
        for i in 0..8 {
            b.tick(1.0 + i as f64 * 0.02);
            b.pan_by(13.4, -9.1);
            b.render(&mut pm);
        }
        b.tick(5.0);
        assert_frame_eq(&mut b, &mut pm, "Rad-Verschieben");
        for i in 0..8 {
            b.tick(6.0 + i as f64 * 0.02);
            b.zoom_at(400.0, 300.0, 1.07);
            b.render(&mut pm);
        }
        b.tick(9.0);
        assert_frame_eq(&mut b, &mut pm, "Zoom");
        // Ring-Speicher mit Versatz: verschieben, dann zeichnen/radieren/auswaehlen
        for i in 0..5 {
            b.tick(10.0 + i as f64 * 0.02);
            b.pan_by(-31.4, 17.9);
            b.render(&mut pm);
        }
        b.tick(12.0);
        b.render(&mut pm);
        stroke(&mut b, &mut pm, 3.0);
        b.on_touch(&Motion::single(
            Action::Down,
            200.0,
            200.0,
            0.5,
            ToolType::Eraser,
        ));
        for i in 0..20 {
            b.on_touch(&Motion::single(
                Action::Move,
                200.0 + i as f32 * 9.0,
                200.0 + i as f32 * 5.0,
                0.5,
                ToolType::Eraser,
            ));
            b.render(&mut pm);
        }
        b.on_touch(&Motion::single(
            Action::Up,
            380.0,
            300.0,
            0.5,
            ToolType::Eraser,
        ));
        b.render(&mut pm);
        // Auswahl ziehen
        b.set_tool(SELECT);
        drag(
            &mut b,
            &[
                (50.0, 50.0),
                (750.0, 50.0),
                (750.0, 550.0),
                (50.0, 550.0),
                (50.0, 60.0),
            ],
            ToolType::Stylus,
        );
        if let Some(r) = b.sel_bounds() {
            let (cx, cy) = (r.center_x() * b.sc + b.tx, r.center_y() * b.sc + b.ty);
            b.on_touch(&Motion::single(Action::Down, cx, cy, 0.5, ToolType::Stylus));
            for i in 1..15 {
                b.on_touch(&Motion::single(
                    Action::Move,
                    cx + i as f32 * 5.0,
                    cy + i as f32 * 2.0,
                    0.5,
                    ToolType::Stylus,
                ));
                b.render(&mut pm);
            }
            b.on_touch(&Motion::single(
                Action::Up,
                cx + 75.0,
                cy + 30.0,
                0.5,
                ToolType::Stylus,
            ));
        }
        assert_frame_eq(&mut b, &mut pm, "Auswahl ziehen");
        b.delete_sel();
        assert_frame_eq(&mut b, &mut pm, "Loeschen");
    }

    #[test]
    fn incremental_render_matches_full() {
        check_incremental(board());
        let mut b = paged(3, PageBg::Lined);
        b.show_page(0);
        check_incremental(b);
        let mut b = board();
        b.set_size(800.0 / 1.5, 600.0 / 1.5, 1.5);
        check_incremental(b);
    }
}
