//! Unendliche Zeichenflaeche mit Stift-, Gesten- und Auswahl-Logik (Gegenstueck zu BoardView.java).
//!
//! Die Oberflaeche liefert Zeiger-Ereignisse ([`Motion`]) in logischen Pixeln, ruft regelmaessig [`Board::tick`] auf
//! und zeichnet mit [`Board::render`] in ein Pixmap mit Geraete-Pixeln (logisch × `px_scale`).
//! Rueckmeldungen an die Oberflaeche landen in [`Board::events`].

use crate::canvas::{Canvas, Cap, Paint, Style};
use crate::geom::{hypot, RectF};
use crate::icons;
use crate::item::{self, Item, Kind, INK};
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
    AskText { wx: f32, wy: f32, note: bool },
    /// Kurzer Hinweis (Toast).
    Notice(String),
    /// Haptische Rueckmeldung (Form erkannt).
    Haptic,
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
        Motion { action, pointers: vec![Pointer { id: 0, x, y, pressure, tool }], history: Vec::new(), stylus_button: false }
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
    /// Unendlich oder Seiten (WP2).
    pub board_mode: BoardMode,
    /// Neu zeichnen noetig (invalidate).
    pub needs_redraw: bool,

    width: f32,
    height: f32,
    px_scale: f32,

    cache: Option<Pixmap>,
    dirty: bool,
    heavy: bool,
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
            recog: 0,
            bg: 0,
            dark: false,
            finger_draw: false,
            modified: false,
            sel_mode: 0,
            erase_mode: 0,
            events: Vec::new(),
            board_mode: BoardMode::Infinite,
            needs_redraw: true,
            width: 0.0,
            height: 0.0,
            px_scale: 1.0,
            cache: None,
            dirty: true,
            heavy: false,
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
        let pos: HashMap<u64, usize> = self.items.iter().enumerate().map(|(i, it)| (it.id, i)).collect();
        self.sel.iter().filter_map(|id| pos.get(id).map(|i| &*self.items[*i])).collect()
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
        let keep: Vec<bool> = self.items.iter().map(|it| !(it.layer == id || (!orphan_safe && self.layer_idx(it.layer) == 0))).collect();
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
        let unusable: HashSet<u64> = self.sel_items().iter().filter(|it| !self.usable(it)).map(|it| it.id).collect();
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
        let h = if self.height > 0.0 { self.height } else { 1000.0 };
        let (vw, vh) = (w / self.sc, h / self.sc);
        let s = (1.0 / self.sc).min((0.6 * vw / pw as f32).min(0.6 * vh / ph as f32));
        let (iw, ih) = (pw as f32 * s, ph as f32 * s);
        let (cx, cy) = self.paste_center(at);
        let it = Item::new(Kind::Img(item::Img { x: cx - iw / 2.0, y: cy - ih / 2.0, w: iw, h: ih, ang: 0.0, file: file.to_string() }));
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
        let mut bm = Pixmap::new(((b.width() * k).round() as u32).max(1), ((b.height() * k).round() as u32).max(1))?;
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
        let mut bm = Pixmap::new(((b.width() * k).round() as u32).max(1), ((b.height() * k).round() as u32).max(1))?;
        {
            let mut c = Canvas::new(&mut bm);
            if with_bg {
                c.draw_color(self.base_color());
            }
            c.scale(k, k);
            c.translate(-b.left, -b.top);
            self.draw_scene(&mut c, None, false, with_bg, 1.0 / k);
        }
        Some(bm)
    }

    /// Zeichnet alle sichtbaren Ebenen hinten nach vorne (Deckkraft per Ebene), vis = Sichtbereich (Welt) oder None.
    fn draw_layered(&self, c: &mut Canvas, vis: Option<&RectF>, skip_lift: bool) {
        let n = self.layers.len();
        let mut bk: Vec<Vec<&Item>> = vec![Vec::new(); n];
        let (mut last_id, mut last_k) = (i32::MIN, 0usize);
        for it in &self.items {
            if skip_lift && it.lift {
                continue;
            }
            if let Some(v) = vis {
                if !RectF::intersects(v, &it.bounds()) {
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
        Snap { items: self.items.clone(), layers: self.layers.clone(), active: self.active, mode: self.board_mode.clone() }
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
        let from_empty = if back { self.undo.is_empty() } else { self.redo.is_empty() };
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
        let nl: Vec<Layer> =
            sn.layers.iter().map(|o| self.layers.iter().find(|l| l.id == o.id).cloned().unwrap_or_else(|| o.clone())).collect();
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
        let clip = if top && !self.dirty { self.pages_path() } else { None };
        if let (true, false, Some(cache)) = (top, self.dirty, self.cache.as_mut()) {
            let (ps, tx, ty, sc, dark) = (self.px_scale, self.tx, self.ty, self.sc, self.dark);
            let mut c = Canvas::new(cache);
            c.scale(ps, ps);
            c.translate(tx, ty);
            c.scale(sc, sc);
            if let Some(p) = &clip {
                c.clip_path(p);
            }
            it.draw(&mut c, dark);
        } else {
            self.dirty = true;
        }
        self.items.push(Rc::new(it));
    }

    pub fn add_note(&mut self, wx: f32, wy: f32, text: &str, note: bool) {
        let size = 18.0 * self.dp / self.sc;
        let it = if note {
            let w = 210.0 * self.dp / self.sc;
            let mh = w * 0.72;
            Item::note(text, self.color, size, wx - w / 2.0, wy - mh / 2.0, w, mh, note_bg(self.color))
        } else {
            Item::note(text, self.color, size, wx, wy - size / 2.0, 0.0, 0.0, 0)
        };
        self.add_top(it);
        self.invalidate();
        self.changed();
    }

    /// Die einzige ausgewaehlte Notiz, falls genau eine ausgewaehlt ist.
    pub fn single_note(&self) -> Option<&Item> {
        if self.sel.len() != 1 {
            return None;
        }
        self.sel_items().into_iter().next().filter(|it| it.as_note().is_some())
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
        ((if self.height > 0.0 { self.height } else { 1000.0 }) / 2.0 - self.ty) / self.sc
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
        tmp.items = self.sel_items().iter().map(|it| Rc::new((*it).clone())).collect();
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
        self.notice(&if n == 1 { "Kopiert".to_string() } else { format!("{n} Objekte kopiert") });
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
        let (mv, rest): (Vec<Rc<Item>>, Vec<Rc<Item>>) = self.items.drain(..).partition(|it| set.contains(&it.id));
        if front {
            self.items = rest;
            self.items.extend(mv);
        } else {
            self.items = mv;
            self.items.extend(rest);
        }
        self.refresh();
        self.notice(if front { "Ganz nach vorne" } else { "Ganz nach hinten" });
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
                let (cx, cy) = ((w / 2.0 - self.tx) / self.sc, (hh / 2.0 - self.ty) / self.sc);
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
                self.sc = ((w - 2.0 * m) / b.width().max(1.0)).min((hh - 2.0 * m) / b.height().max(1.0)).clamp(0.05, 2.0);
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
            if self.cache.is_none() || self.dirty {
                self.render_cache();
            }
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
            if self.cache.is_none() || self.dirty {
                self.render_cache();
            }
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
            self.snap = Some(s);
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
        self.layers = o.get("layers").and_then(Value::as_array).map(|a| a.iter().filter(|v| v.is_object()).map(Layer::from_json).collect()).unwrap_or_default();
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
                    RectF::new(r.left - 20.0, r.top - 20.0, r.left - 20.0 + w, r.top - 20.0 + w * 300.0 / 480.0)
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
                    self.render_all(&mut c, &b, k, (480.0 - b.width() * k) / 2.0, (300.0 - b.height() * k) / 2.0);
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
        let mut bm = Pixmap::new(((b.width() * k) as u32).max(1), ((b.height() * k) as u32).max(1))?;
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
        let (pw, ph) = (((b.width() * k) as u32).max(1), ((b.height() * k) as u32).max(1));
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
        self.pages().map(|ps| ps.page_at(wx, wy).is_none()).unwrap_or(false)
    }

    /// Ersetzt die Seiten durch `new`; `order[neuer Index]` = alter Index (None = neue, leere Seite).
    /// Objekte entfallender Seiten werden geloescht, die uebrigen mit ihrer Seite verschoben. Mit Undo-Schritt.
    fn relayout_pages(&mut self, new: PageSetup, order: &[Option<usize>]) {
        let BoardMode::Pages(old) = &self.board_mode else { return };
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
        let order: Vec<Option<usize>> = (0..=n).map(|i| if i < at { Some(i) } else if i == at { None } else { Some(i - 1) }).collect();
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
        let Some(r) = self.pages().and_then(|p| p.rect(i)) else { return };
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
        Some(pb.finish().unwrap_or_else(|| PathBuilder::from_rect(tiny_skia::Rect::from_ltrb(0.0, 0.0, 0.0001, 0.0001).unwrap())))
    }

    /// Seitenmodus: schneidet alles Folgende auf die Seiten zu (aktuelle Transformation = Welt).
    fn clip_pages(&self, c: &mut Canvas) {
        if let Some(p) = self.pages_path() {
            c.clip_path(&p);
        }
    }

    /// Inhalt in Weltkoordinaten. Seitenmodus: Seitenblaetter (wenn `sheets`) und auf die Seiten geschnittene Objekte.
    /// `px` = Welteinheiten je Pixel (fuer Mindest-Linienbreiten und Schatten).
    fn draw_scene(&self, c: &mut Canvas, vis: Option<&RectF>, skip_lift: bool, sheets: bool, px: f32) {
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
            c.draw_rect(r.left + sh * 0.5, r.top + sh, r.right + sh * 0.5, r.bottom + sh, &Paint::fill(0x30000000));
            c.draw_rect(r.left, r.top, r.right, r.bottom, &Paint::fill(self.page_color()));
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
                c.draw_points(&dots, &Paint::stroke(col, (1.6f32).max(2.0 * px), Cap::Round));
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
            let mut bm = Pixmap::new(((r.width() * K).round() as u32).max(1), ((r.height() * K).round() as u32).max(1))?;
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
                if st || self.finger_draw || self.tool == SELECT || self.tool == NOTE || self.tool == TEXT {
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
                } else if self.mode == 2 && e.pointers[ai].id == self.pan_id && r < e.pointers.len() {
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

    fn prep_gesture(&mut self) {
        if self.heavy && (self.dirty || self.cache.is_none()) && self.width > 0.0 {
            self.render_cache();
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
            let w = (if self.tool == MARKER { self.pen_w * 4.0 + 8.0 } else { self.pen_w }) * self.dp / self.sc;
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
        self.hold_at = if self.recog == 0 { Some(self.now + 0.45) } else { None };
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
            self.last_pr = if self.stylus_act { self.last_pr * 0.6 + pr * 0.4 } else { 0.5 };
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

    fn make_shape(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Item {
        let w = self.pen_w * self.dp / self.sc;
        if self.shape_kind == 0 {
            Item::shape(item::POLY, vec![x0, y0, x1, y0, x1, y1, x0, y1], self.color, w)
        } else {
            let k = match self.shape_kind {
                1 => item::ELLIPSE,
                2 => item::LINE,
                4 => item::ARROW_ENG,
                _ => item::ARROW,
            };
            Item::shape(k, vec![x0, y0, x1, y1], self.color, w)
        }
    }

    /// Radierer-Radius in Bildschirm-Pixeln fuer den aktuellen Modus.
    fn erase_screen_r(&self) -> f32 {
        if self.erase_mode == 1 {
            (self.pen_w * 3.0).max(6.0) * self.dp
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
                let Some(parts) = item::erase_split(&s.p, it.width, wx, wy, r) else { continue };
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
        (self.sel_cx + lx * cs - ly * sn, self.sel_cy + lx * sn + ly * cs)
    }

    /// Lokale Rahmenkoordinate nach Bildschirm.
    fn ls(&self, lx: f32, ly: f32) -> (f32, f32) {
        let (x, y) = self.lw(lx, ly);
        (x * self.sc + self.tx, y * self.sc + self.ty)
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
                self.d0 = (l * 0.2).max((wx - self.pvx) * self.g_ux + (wy - self.pvy) * self.g_uy);
                self.g_scale = 1.0;
                self.drag = 2;
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
        // ausserhalb: neue Auswahl (die alte bleibt, bis wirklich gezogen wird)
        if !self.sel.is_empty() {
            self.prev_sel = Some(self.sel.clone());
            self.prev_frame = Some([self.sel_cx, self.sel_cy, self.sel_w, self.sel_h, self.sel_ang]);
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
        if n >= 2 && hypot(wx - self.lasso[n - 2], wy - self.lasso[n - 1]) * self.sc < 2.0 * self.dp {
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
                *m = RectF::new(self.sx.min(wx), self.sy.min(wy), self.sx.max(wx), self.sy.max(wy));
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
                let (cx, cy, hx, hy) = (b.center_x(), b.center_y(), b.width() * 0.45, b.height() * 0.45);
                for (x, y) in [(cx, cy), (cx - hx, cy), (cx + hx, cy), (cx, cy - hy), (cx, cy + hy)] {
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
                it = recognizer::run(&cur, self.sc);
            }
            self.add_top(it.unwrap_or(cur));
        } else if self.tool == SHAPE {
            if let Some(p) = self.prev.take() {
                self.add_top(p);
            }
        } else if self.tool == SELECT {
            self.drop_lift();
            if self.drag == 3 {
                if self.moved {
                    self.select_area();
                } else {
                    self.sel_clear();
                    if let Some(p) = self.pend {
                        self.sel.push(p);
                        self.frame_ok = false;
                    }
                }
            }
            self.marq = None;
            self.lasso.clear();
            self.pend = None;
            self.prev_sel = None;
            self.prev_frame = None;
            self.drag = 0;
        } else if (self.tool == NOTE || self.tool == TEXT) && !self.moved {
            self.events.push(Event::AskText { wx: self.sx, wy: self.sy, note: self.tool == NOTE });
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

    fn render_cache(&mut self) {
        let ps = self.px_scale;
        let (dw, dh) = (((self.width * ps).round() as u32).max(1), ((self.height * ps).round() as u32).max(1));
        let ok = matches!(&self.cache, Some(c) if c.width() == dw && c.height() == dh);
        if !ok {
            self.cache = Pixmap::new(dw, dh);
        }
        let Some(mut cache) = self.cache.take() else { return };
        {
            let mut c = Canvas::new(&mut cache);
            c.scale(ps, ps);
            self.backdrop(&mut c);
            c.translate(self.tx, self.ty);
            c.scale(self.sc, self.sc);
            let vis = RectF::new(-self.tx / self.sc, -self.ty / self.sc, (self.width - self.tx) / self.sc, (self.height - self.ty) / self.sc);
            self.draw_scene(&mut c, Some(&vis), true, true, self.dp / self.sc);
        }
        let wt: usize = self.items.iter().map(|it| it.weight()).sum();
        self.heavy = wt > 30000;
        self.csc = self.sc;
        self.ctx = self.tx;
        self.cty = self.ty;
        self.dirty = false;
        self.cache = Some(cache);
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
        let ox = ((self.tx % step) + step) % step;
        let oy = ((self.ty % step) + step) % step;
        let col = if self.dark { 0xFF343741 } else { 0xFFD3D6DD };
        if self.bg == 0 {
            let mut dots = Vec::new();
            let mut y = oy;
            while y < hh {
                let mut x = ox;
                while x < w {
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
            while y < hh {
                pb.move_to(0.0, y);
                pb.line_to(w, y);
                y += step;
            }
            if self.bg == 1 {
                let mut x = ox;
                while x < w {
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
        c.draw_circle(rx, ry, 14.0 * dp, &Paint::stroke(0xFFFFFFFF, 2.0 * dp, Cap::Butt));
        c.save();
        c.translate(rx - 9.0 * dp, ry - 9.0 * dp);
        c.scale(18.0 * dp / 24.0, 18.0 * dp / 24.0);
        icons::draw(c, icons::ROTATE, 0xFFFFFFFF, false);
        c.restore();
        // Eck-Griffe
        for (x, y) in corners {
            c.draw_circle(x, y, 7.0 * dp, &Paint::fill(ACCENT));
            c.draw_circle(x, y, 7.0 * dp, &Paint::stroke(0xFFFFFFFF, 2.0 * dp, Cap::Butt));
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

    /// Zeichnet die Ansicht in target (Groesse = logische Groesse × px_scale).
    pub fn render(&mut self, target: &mut Pixmap) {
        self.needs_redraw = false;
        if self.width <= 0.0 || self.height <= 0.0 {
            return;
        }
        let gest = self.mode == 2 || self.mode == 3 || self.wheel_until.is_some();
        let ps = self.px_scale;
        let scaled_cache = if gest && (self.heavy || self.wheel_until.is_some()) && !self.dirty { self.cache.as_ref() } else { None };
        if let Some(cache) = scaled_cache {
            let mut c = Canvas::new(target);
            c.scale(ps, ps);
            self.backdrop(&mut c);
            let k = self.sc / self.csc;
            let (l, t) = (self.tx - self.ctx * k, self.ty - self.cty * k);
            c.draw_pixmap(cache.as_ref(), l, t, l + self.width * k, t + self.height * k);
        } else if gest {
            let mut c = Canvas::new(target);
            c.scale(ps, ps);
            self.backdrop(&mut c);
            let vis = RectF::new(-self.tx / self.sc, -self.ty / self.sc, (self.width - self.tx) / self.sc, (self.height - self.ty) / self.sc);
            c.translate(self.tx, self.ty);
            c.scale(self.sc, self.sc);
            self.draw_scene(&mut c, Some(&vis), false, true, self.dp / self.sc);
        } else {
            if self.dirty || self.cache.is_none() {
                self.render_cache();
            }
            match self.cache.as_ref() {
                Some(cache) if cache.width() == target.width() && cache.height() == target.height() => {
                    target.data_mut().copy_from_slice(cache.data());
                }
                Some(cache) => {
                    let mut c = Canvas::new(target);
                    c.draw_color(self.base_color());
                    c.draw_pixmap(cache.as_ref(), 0.0, 0.0, cache.width() as f32, cache.height() as f32);
                }
                None => {}
            }
        }

        let mut c = Canvas::new(target);
        c.scale(ps, ps);
        c.save();
        c.translate(self.tx, self.ty);
        c.scale(self.sc, self.sc);
        c.save();
        self.clip_pages(&mut c);
        if self.lifted && !gest {
            for it in self.sel_items() {
                it.draw(&mut c, self.dark);
            }
        }
        if let Some(s) = &self.snap {
            s.draw(&mut c, self.dark);
        } else if let Some(s) = &self.cur {
            s.draw(&mut c, self.dark);
        }
        if let Some(p) = &self.prev {
            p.draw(&mut c, self.dark);
        }
        c.restore();
        let dp = self.dp;
        if self.tool == SELECT && self.mode == 1 && self.drag == 3 && self.sel_mode == 0 && self.lasso.len() > 2 && self.moved {
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
            c.draw_rect(m.left, m.top, m.right, m.bottom, &Paint::stroke(ACCENT, 1.5 * dp / self.sc, Cap::Butt));
        }
        c.restore();

        if self.tool == SELECT && !self.sel.is_empty() && !(self.mode == 1 && self.drag == 3 && self.moved) {
            self.draw_frame(&mut c);
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
    0xFF000000 | ((r + (255 - r) * 6 / 10) << 16) | ((g + (255 - g) * 6 / 10) << 8) | (b + (255 - b) * 6 / 10)
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
    fn draw_undo_redo_and_json() {
        let mut b = board();
        drag(&mut b, &[(10.0, 10.0), (50.0, 20.0), (90.0, 60.0)], ToolType::Stylus);
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
        drag(&mut b, &[(80.0, 80.0), (200.0, 80.0), (200.0, 130.0), (80.0, 130.0), (80.0, 85.0)], ToolType::Mouse);
        assert_eq!(b.sel.len(), 1);
        let before = b.sel_items()[0].bounds();
        drag(&mut b, &[(125.0, 100.0), (135.0, 110.0), (145.0, 120.0)], ToolType::Mouse);
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
        assert!(b.events.contains(&Event::Notice("Ebene ist gesperrt".into())));
        b.undo();
        assert_eq!(b.layers.len(), 7);
    }

    #[test]
    fn note_bg_colors() {
        assert_eq!(note_bg(INK), 0xFFFFE27A);
        assert_eq!(note_bg(0xFFE5484D), 0xFFF4B5B7);
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
        let ids: Vec<String> = b.pages().unwrap().pages.iter().map(|p| p.id.clone()).collect();

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
        let ids2: Vec<String> = b.pages().unwrap().pages.iter().map(|p| p.id.clone()).collect();
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
        drag(&mut b, &[scr(100.0, g), scr(100.0, g + 200.0)], ToolType::Stylus);
        assert!(b.items.is_empty());
        // beginnt rechts neben der Seite
        drag(&mut b, &[scr(700.0, 100.0), scr(300.0, 100.0)], ToolType::Stylus);
        assert!(b.items.is_empty());
        // beginnt auf der Seite, endet ausserhalb: bleibt
        drag(&mut b, &[scr(300.0, 100.0), scr(700.0, 100.0)], ToolType::Stylus);
        assert_eq!(b.items.len(), 1);
    }

    #[test]
    fn page_clipping_render() {
        let mut b = paged(2, PageBg::Plain);
        // senkrechter Strich ueber die Seitengrenze hinweg (von Seite 0 bis Seite 1)
        drag(&mut b, &[scr(300.0, 700.0), scr(300.0, 842.0 + PAGE_GAP + 150.0)], ToolType::Stylus);
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
        drag(&mut b, &[scr(100.0, 800.0), scr(200.0, 800.0)], ToolType::Stylus);
        let y1 = 842.0 + PAGE_GAP + 30.0;
        drag(&mut b, &[scr(100.0, y1), scr(200.0, y1)], ToolType::Stylus);
        assert_eq!(b.page_of_items(), vec![Some(0), Some(1)]);
        b.set_tool(SELECT);
        b.sel_mode = 0;
        let pts = [scr(50.0, 750.0), scr(250.0, 750.0), scr(250.0, y1 + 50.0), scr(50.0, y1 + 50.0), scr(50.0, 755.0)];
        drag(&mut b, &pts, ToolType::Mouse);
        assert_eq!(b.sel.len(), 2);
        // Rechteckauswahl ebenso
        b.sel_mode = 1;
        drag(&mut b, &[scr(30.0, 600.0), scr(150.0, 870.0), scr(260.0, y1 + 60.0)], ToolType::Mouse);
        assert_eq!(b.sel.len(), 2);
        // Radierer, der im Zwischenraum beginnt und beide Striche ueberstreicht
        b.set_tool(ERASER);
        drag(&mut b, &[scr(150.0, 780.0), scr(150.0, 842.0 + PAGE_GAP / 2.0), scr(150.0, y1 + 20.0)], ToolType::Stylus);
        assert!(b.items.is_empty());
    }

    #[test]
    fn pdf_has_one_page_per_board_page() {
        let mut b = paged(3, PageBg::Lined);
        drag(&mut b, &[scr(100.0, 100.0), scr(300.0, 300.0)], ToolType::Stylus);
        let pdf = b.export_pdf().unwrap();
        let s = String::from_utf8_lossy(&pdf);
        assert_eq!(s.matches("/Type /Page ").count(), 3);
        assert!(s.contains("/Count 3"));
        assert!(s.contains("/MediaBox [0 0 595.00 842.00]"));
        assert!(s.ends_with("%%EOF\n"));
        // auch ein leeres Seiten-Board ist exportierbar
        let e = paged(1, PageBg::Plain);
        assert!(e.export_bounds().is_some());
        assert_eq!(String::from_utf8_lossy(&e.export_pdf().unwrap()).matches("/Type /Page ").count(), 1);
    }

    #[test]
    fn fit_pages_width() {
        let mut b = paged(2, PageBg::Plain);
        b.fit();
        assert!((595.0 * b.sc - (800.0 - 32.0)).abs() < 0.01);
        assert!((b.tx + 595.0 / 2.0 * b.sc - 400.0).abs() < 0.01);
        let _ = b.thumb();
    }
}
