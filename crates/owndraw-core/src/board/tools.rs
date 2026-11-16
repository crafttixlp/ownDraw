//! Editor-Werkzeuge im Kern (4.25): Füllen, Pipette, Stempel, Verbinder, Gruppen, Ausrichten/Verteilen,
//! Platz schaffen, Tabellen, Zuschnitt, Kommentar-Stecknadeln, Stiftarten der Auswahl.
//!
//! Jede öffentliche Änderung ist genau ein Rückgängig-Schritt (Verbinder werden im selben Schritt nachgeführt).

use super::*;
use crate::item::{Anchor, Pin, Reply, Table};

/// Ausrichtung mehrerer Objekte ([`Board::align_sel`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    CenterH,
    Right,
    Top,
    CenterV,
    Bottom,
}

/// Stempel ([`Board::stamp`]).
pub const STAMP_CHECK: i32 = 0;
pub const STAMP_CROSS: i32 = 1;
pub const STAMP_STAR: i32 = 2;
pub const STAMP_QUESTION: i32 = 3;
pub const STAMP_EXCLAIM: i32 = 4;

/// Kopien mit neuen uids; Gruppen bekommen neue gids, Verbinder hängen an den mitkopierten Zielen
/// (Andockpunkte an nicht mitkopierte Ziele entfallen).
pub(super) fn dup_all<'a>(src: impl IntoIterator<Item = &'a Item>) -> Vec<Item> {
    let src: Vec<&Item> = src.into_iter().collect();
    let mut out: Vec<Item> = src.iter().map(|it| it.dup()).collect();
    let umap: HashMap<u64, u64> = src.iter().zip(&out).map(|(a, b)| (a.uid, b.uid)).collect();
    let mut gmap: HashMap<u64, u64> = HashMap::new();
    for it in out.iter_mut() {
        if it.gid != 0 {
            it.gid = *gmap.entry(it.gid).or_insert_with(item::new_uid);
        }
        if let Kind::Shape(g) = &mut it.kind {
            for a in g.a.iter_mut() {
                *a = a.and_then(|x| umap.get(&x.uid).map(|u| Anchor { uid: *u, rel: x.rel }));
            }
        }
    }
    out
}

impl Board {
    /// Indizes der Objekte von oben nach unten (Ebenen-Reihenfolge, dann Zeichenreihenfolge); nur sichtbare Ebenen,
    /// mit `editable` nur entsperrte.
    fn top_down(&self, editable: bool) -> Vec<usize> {
        let mut v: Vec<(usize, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| {
                let li = self.layer_idx(it.layer);
                let l = &self.layers[li];
                (l.visible && (!editable || !l.locked)).then_some((li, i))
            })
            .collect();
        v.sort_by(|a, b| b.cmp(a));
        v.into_iter().map(|(_, i)| i).collect()
    }

    /// Ändert ein Objekt (ein Rückgängig-Schritt). `f` liefert false = nichts geändert (kein Schritt).
    fn edit_item(&mut self, id: u64, f: impl FnOnce(&mut Item) -> bool) -> bool {
        let Some(i) = self.idx_of(id) else { return false };
        let mut it = (*self.items[i]).clone();
        if !f(&mut it) {
            return false;
        }
        it.touch();
        self.checkpoint();
        self.items[i] = Rc::new(it);
        self.reroute(false);
        self.frame_ok = false;
        self.refresh();
        self.changed();
        true
    }

    /// Fangradius (Welt) für Pipette, Füllen und Andocken.
    fn pick_r(&self) -> f32 {
        8.0 * self.dp / self.sc
    }

    // ---------- 1. Füllen ----------

    /// Füllt das oberste geschlossene Objekt unter dem Weltpunkt (None = Füllung entfernen). Liefert dessen id.
    pub fn fill_at(&mut self, x: f32, y: f32, color: Option<u32>) -> Option<u64> {
        let id = self.top_down(true).into_iter().map(|i| &self.items[i]).find(|it| it.fillable() && (it.contains(x, y) || it.hit_outline(x, y, 0.0)))?.id;
        self.edit_item(id, |it| {
            if it.fill == color {
                return false;
            }
            it.fill = color;
            true
        });
        Some(id)
    }

    /// Füllfarbe der Auswahl: None ohne füllbares Objekt, `Some(None)` bei gemischten/keinen Füllungen.
    pub fn sel_fill(&self) -> Option<Option<u32>> {
        let mut it = self.sel_items().into_iter().filter(|it| it.fillable()).map(|it| it.fill);
        let first = it.next()?;
        Some(if it.all(|c| c == first) { first } else { None })
    }

    /// Setzt bzw. entfernt die Füllung aller füllbaren ausgewählten Objekte.
    pub fn set_sel_fill(&mut self, color: Option<u32>) -> bool {
        if !self.sel_items().iter().any(|it| it.fillable() && it.fill != color) {
            return false;
        }
        self.checkpoint();
        self.sel_mut(|it| {
            if it.fillable() {
                it.fill = color;
                it.touch();
            }
        });
        self.refresh();
        self.changed();
        true
    }

    // ---------- 2. Pipette ----------

    /// Farbe des obersten Objekts am Weltpunkt: Bild = Pixel des Bildes (falls ladbar), gefüllte Fläche = Füllfarbe,
    /// Umriss/Strich = Linienfarbe, Haftnotiz = Hintergrund, Text/Tabelle = Schriftfarbe. None über leerer Fläche.
    pub fn color_at(&self, x: f32, y: f32) -> Option<u32> {
        let r = self.pick_r() * 0.5;
        for i in self.top_down(false) {
            let it = &self.items[i];
            match &it.kind {
                Kind::Pin(_) => continue,
                Kind::Img(_) if it.hit(x, y, 0.0) => return Some(it.img_pixel(x, y).unwrap_or(it.color)),
                Kind::Note(n) if it.hit(x, y, 0.0) => return Some(if n.bg != 0 { n.bg } else { it.color }),
                Kind::Table(_) if it.hit(x, y, 0.0) => return Some(it.color),
                Kind::Math(_) if it.hit(x, y, 0.0) => return Some(it.color),
                Kind::Stroke(_) | Kind::Shape(_) => {
                    if it.near(x, y, r) && it.hit_outline(x, y, r) {
                        return Some(it.color);
                    }
                    if let Some(f) = it.fill.filter(|_| it.contains(x, y)) {
                        return Some(f);
                    }
                }
                _ => {}
            }
        }
        None
    }

    // ---------- 4. Stempel ----------

    /// Setzt einen Stempel (`STAMP_*`) mit Mittelpunkt (x, y) und Kantenlänge `size` (Welt) in der aktuellen Farbe
    /// auf die aktive Ebene. Mehrteilige Stempel werden gruppiert. Liefert die neuen ids (leer bei gesperrter Ebene).
    pub fn stamp(&mut self, kind: i32, x: f32, y: f32, size: f32) -> Vec<u64> {
        if self.layer_blocked() || !(size > 0.0) {
            return Vec::new();
        }
        let (col, w) = (self.color, (size * 0.09).max(1.0));
        let line = |pts: &[(f32, f32)], sharp: bool| -> Item {
            let mut it = Item::stroke(col, w, false, false);
            for &(px, py) in pts {
                it.add_point(x + px * size, y + py * size, 0.5);
            }
            if let Kind::Stroke(s) = &mut it.kind {
                s.sharp = sharp;
            }
            it.touch();
            it
        };
        let mut items: Vec<Item> = match kind {
            STAMP_CHECK => vec![line(&[(-0.4, 0.0), (-0.12, 0.32), (0.42, -0.36)], true)],
            STAMP_CROSS => vec![line(&[(-0.35, -0.35), (0.35, 0.35)], true), line(&[(0.35, -0.35), (-0.35, 0.35)], true)],
            STAMP_STAR => {
                let h = size / 2.0;
                let mut it = Item::shape(item::STAR, item::shape_q(item::STAR, x - h, y - h, x + h, y + h), col, w * 0.5);
                it.fill = Some(col);
                vec![it]
            }
            STAMP_QUESTION => {
                let mut pts: Vec<(f32, f32)> = (0..=12)
                    .map(|i| {
                        let a = std::f32::consts::PI * (1.1 - 1.45 * i as f32 / 12.0);
                        (0.24 * a.cos(), -0.2 + 0.2 * -a.sin())
                    })
                    .collect();
                pts.extend_from_slice(&[(0.0, 0.08), (0.0, 0.18)]);
                vec![line(&pts, false), line(&[(0.0, 0.38)], true)]
            }
            _ => vec![line(&[(0.0, -0.42), (0.0, 0.18)], true), line(&[(0.0, 0.38)], true)],
        };
        let lid = self.active_layer().id;
        let gid = if items.len() > 1 { item::new_uid() } else { 0 };
        self.checkpoint();
        let mut ids = Vec::new();
        for it in items.iter_mut() {
            it.layer = lid;
            it.gid = gid;
            ids.push(it.id);
        }
        self.items.extend(items.into_iter().map(Rc::new));
        self.refresh();
        self.changed();
        ids
    }

    // ---------- 5. Verbinder ----------

    /// Ziel zum Andocken am Weltpunkt: oberstes Objekt (keine Stecknadel, nicht `exclude`) im Fangradius `r`
    /// (None = Standard), dazu der relative Andockpunkt (None = Mitte).
    pub fn dock_target_at(&self, x: f32, y: f32, exclude: Option<u64>, r: Option<f32>) -> Option<(u64, Option<(f32, f32)>)> {
        let r = r.unwrap_or(self.pick_r() * 2.0);
        self.top_down(false)
            .into_iter()
            .map(|i| &*self.items[i])
            .filter(|it| Some(it.id) != exclude && !matches!(it.kind, Kind::Pin(_)) && !it.as_shape().is_some_and(|g| is_linear(g.kind)))
            .find(|it| it.contains(x, y) || it.near(x, y, r))
            .map(|it| (it.id, it.anchor_rel(x, y)))
    }

    /// Dockt die Enden `ends` der (noch nicht eingefügten) Form an Objekte in Fangweite an und rückt sie auf den Rand.
    pub(super) fn auto_dock(&self, it: &mut Item, ends: &[usize]) {
        let Some(g) = it.as_shape() else { return };
        if !is_linear(g.kind) || g.q.len() < 4 {
            return;
        }
        let q = g.q.clone();
        for &e in ends {
            let (x, y) = (q[e * 2], q[e * 2 + 1]);
            let a = self.dock_target_at(x, y, Some(it.id), None).and_then(|(id, rel)| self.items.iter().find(|t| t.id == id).map(|t| Anchor { uid: t.uid, rel }));
            it.set_anchor(e, a);
        }
        // beide Enden am selben Ziel ergeben keinen Verbinder
        if let [Some(a), Some(b)] = it.anchors() {
            if a.uid == b.uid {
                it.set_anchor(1, None);
            }
        }
        let map: HashMap<u64, &Item> = self.items.iter().map(|t| (t.uid, &**t)).collect();
        if let Some(nq) = route(it, &map) {
            it.set_end(0, nq[0], nq[1]);
            it.set_end(1, nq[2], nq[3]);
        }
    }

    /// Hängt das Ende `end` (0 Anfang, 1 Ende) der Linie/des Pfeils `id` an das Objekt `target` (None = lösen),
    /// `rel` = Andockpunkt (None = Mitte). Ein Rückgängig-Schritt.
    pub fn connect(&mut self, id: u64, end: usize, target: Option<u64>, rel: Option<(f32, f32)>) -> bool {
        let a = match target {
            Some(t) if t != id => match self.items.iter().find(|x| x.id == t) {
                Some(x) => Some(Anchor { uid: x.uid, rel }),
                None => return false,
            },
            Some(_) => return false,
            None => None,
        };
        self.edit_item(id, |it| {
            if end > 1 || !it.as_shape().is_some_and(|g| is_linear(g.kind)) || it.anchors()[end] == a {
                return false;
            }
            it.set_anchor(end, a);
            true
        })
    }

    /// Dockt das Ende `end` der Linie `id` an das Objekt unter ihm (Fangradius) an bzw. löst es. Ein Rückgängig-Schritt.
    pub fn dock_end(&mut self, id: u64, end: usize) -> bool {
        let Some(i) = self.idx_of(id) else { return false };
        let mut it = (*self.items[i]).clone();
        let before = it.anchors();
        self.auto_dock(&mut it, &[end.min(1)]);
        if it.anchors() == before {
            return false;
        }
        self.checkpoint();
        self.items[i] = Rc::new(it);
        self.reroute(false);
        self.refresh();
        self.changed();
        true
    }

    /// Führt alle angedockten Verbinder ihren Zielen nach (ohne eigenen Rückgängig-Schritt; Aufrufer hat ihn).
    /// `drop_missing` = Andockpunkte an nicht mehr vorhandene Ziele entfernen (nach dem Löschen).
    pub(super) fn reroute(&mut self, drop_missing: bool) {
        if !self.items.iter().any(|it| it.anchors() != [None, None]) {
            return;
        }
        let mut upd: Vec<(usize, Item)> = Vec::new();
        {
            let map: HashMap<u64, &Item> = self.items.iter().map(|t| (t.uid, &**t)).collect();
            for (i, it) in self.items.iter().enumerate() {
                let an = it.anchors();
                if an == [None, None] {
                    continue;
                }
                let missing = an.map(|a| a.is_some_and(|a| !map.contains_key(&a.uid)));
                let mut n: Option<Item> = None;
                if drop_missing && (missing[0] || missing[1]) {
                    let mut c = (**it).clone();
                    for e in 0..2 {
                        if missing[e] {
                            c.set_anchor(e, None);
                        }
                    }
                    n = Some(c);
                }
                let cur = n.as_ref().unwrap_or(it);
                if let Some(nq) = route(cur, &map) {
                    let q = &cur.as_shape().map(|g| g.q.clone()).unwrap_or_default();
                    if q.len() >= 4 && q[..4].iter().zip(&nq).any(|(a, b)| (a - b).abs() > 1e-3) {
                        let mut c = n.take().unwrap_or_else(|| (**it).clone());
                        c.set_end(0, nq[0], nq[1]);
                        c.set_end(1, nq[2], nq[3]);
                        n = Some(c);
                    }
                }
                if let Some(c) = n {
                    upd.push((i, c));
                }
            }
        }
        if upd.is_empty() {
            return;
        }
        for (i, c) in upd {
            self.items[i] = Rc::new(c);
        }
        self.dirty = true;
        self.invalidate();
    }

    // ---------- 6. Gruppen, Ausrichten, Verteilen ----------

    /// Nimmt alle Mitglieder der Gruppen ausgewählter Objekte in die Auswahl auf.
    pub(super) fn expand_groups(&mut self) {
        let gids: HashSet<u64> = self.sel_items().iter().map(|it| it.gid).filter(|g| *g != 0).collect();
        if gids.is_empty() {
            return;
        }
        let have = self.sel_set();
        let more: Vec<u64> = self.items.iter().filter(|it| gids.contains(&it.gid) && !have.contains(&it.id) && self.usable(it)).map(|it| it.id).collect();
        if !more.is_empty() {
            self.sel.extend(more);
            self.frame_ok = false;
            self.ensure_frame();
        }
    }

    /// Gruppiert die Auswahl (ab 2 Objekten). Liefert die neue gid.
    pub fn group_sel(&mut self) -> Option<u64> {
        if self.sel.len() < 2 {
            return None;
        }
        let g = item::new_uid();
        self.checkpoint();
        self.sel_mut(|it| it.gid = g);
        self.refresh();
        self.changed();
        Some(g)
    }

    /// Löst alle Gruppen in der Auswahl auf.
    pub fn ungroup_sel(&mut self) -> bool {
        if !self.sel_items().iter().any(|it| it.gid != 0) {
            return false;
        }
        self.checkpoint();
        self.sel_mut(|it| it.gid = 0);
        self.refresh();
        self.changed();
        true
    }

    /// Ist die Auswahl (ganz oder teilweise) gruppiert?
    pub fn sel_grouped(&self) -> bool {
        self.sel_items().iter().any(|it| it.gid != 0)
    }

    /// Einheiten der Auswahl (Gruppen zählen als eine): ids und gemeinsame Hülle.
    fn sel_units(&self) -> Vec<(Vec<u64>, RectF)> {
        let mut units: Vec<(u64, Vec<u64>, RectF)> = Vec::new();
        for it in self.sel_items() {
            let b = it.bounds();
            match units.iter_mut().find(|u| it.gid != 0 && u.0 == it.gid) {
                Some(u) => {
                    u.1.push(it.id);
                    u.2.union(&b);
                }
                None => units.push((it.gid, vec![it.id], b)),
            }
        }
        units.into_iter().map(|(_, ids, b)| (ids, b)).collect()
    }

    /// Verschiebt Objekte (id → dx, dy) in einem Rückgängig-Schritt.
    fn shift_ids(&mut self, moves: &[(Vec<u64>, f32, f32)]) -> bool {
        let mv: HashMap<u64, (f32, f32)> = moves.iter().flat_map(|(ids, dx, dy)| ids.iter().map(move |id| (*id, (*dx, *dy)))).filter(|(_, d)| d.0.abs() > 1e-4 || d.1.abs() > 1e-4).collect();
        if mv.is_empty() {
            return false;
        }
        self.checkpoint();
        for it in self.items.iter_mut() {
            if let Some((dx, dy)) = mv.get(&it.id) {
                Rc::make_mut(it).xf(1.0, 0.0, 0.0, *dx, *dy);
            }
        }
        self.reroute(false);
        self.frame_ok = false;
        self.ensure_frame();
        self.refresh();
        self.changed();
        true
    }

    /// Richtet die Auswahl (ab 2 Einheiten; Gruppen zählen als eine) an ihrer gemeinsamen Hülle aus.
    pub fn align_sel(&mut self, a: Align) -> bool {
        let units = self.sel_units();
        if units.len() < 2 {
            return false;
        }
        let mut all = units[0].1;
        for u in &units[1..] {
            all.union(&u.1);
        }
        let moves: Vec<(Vec<u64>, f32, f32)> = units
            .into_iter()
            .map(|(ids, b)| {
                let (dx, dy) = match a {
                    Align::Left => (all.left - b.left, 0.0),
                    Align::CenterH => (all.center_x() - b.center_x(), 0.0),
                    Align::Right => (all.right - b.right, 0.0),
                    Align::Top => (0.0, all.top - b.top),
                    Align::CenterV => (0.0, all.center_y() - b.center_y()),
                    Align::Bottom => (0.0, all.bottom - b.bottom),
                };
                (ids, dx, dy)
            })
            .collect();
        self.shift_ids(&moves)
    }

    /// Verteilt die Auswahl (ab 3 Einheiten) waagerecht bzw. senkrecht mit gleichen Abständen; die äußeren bleiben.
    pub fn distribute_sel(&mut self, horizontal: bool) -> bool {
        let mut units = self.sel_units();
        if units.len() < 3 {
            return false;
        }
        let key = |b: &RectF| if horizontal { b.center_x() } else { b.center_y() };
        units.sort_by(|a, b| key(&a.1).total_cmp(&key(&b.1)));
        let (lo, sz): (fn(&RectF) -> f32, fn(&RectF) -> f32) = if horizontal { (|b| b.left, |b| b.width()) } else { (|b| b.top, |b| b.height()) };
        let n = units.len();
        let start = lo(&units[0].1);
        let end = units.iter().map(|u| lo(&u.1) + sz(&u.1)).fold(f32::MIN, f32::max);
        let total: f32 = units.iter().map(|u| sz(&u.1)).sum();
        let gap = (end - start - total) / (n - 1) as f32;
        let mut pos = start;
        let mut moves = Vec::new();
        for (i, (ids, b)) in units.into_iter().enumerate() {
            let d = if i == 0 || i == n - 1 { 0.0 } else { pos - lo(&b) };
            pos += sz(&b) + gap;
            moves.push(if horizontal { (ids, d, 0.0) } else { (ids, 0.0, d) });
        }
        self.shift_ids(&moves)
    }

    // ---------- 7. Platz schaffen ----------

    /// Verschiebt alle Objekte, die ganz unterhalb von y liegen, um dy (negativ = Platz entfernen). Seitenmodus:
    /// nur Objekte der Seite `page` (None = alle). Gesperrte Ebenen bleiben. Ein Rückgängig-Schritt; liefert die Anzahl.
    pub fn make_space(&mut self, page: Option<usize>, y: f32, dy: f32) -> usize {
        if dy == 0.0 {
            return 0;
        }
        let pg: Vec<Option<usize>> = self.page_of_items();
        let ids: Vec<u64> = self
            .items
            .iter()
            .zip(&pg)
            .filter(|(it, p)| {
                let l = &self.layers[self.layer_idx(it.layer)];
                !l.locked && (page.is_none() || **p == page) && it.bounds().top >= y
            })
            .map(|(it, _)| it.id)
            .collect();
        let n = ids.len();
        if n > 0 {
            self.shift_ids(&[(ids, 0.0, dy)]);
        }
        n
    }

    // ---------- 8. Tabellen ----------

    /// Neue Tabelle (rows × cols) mit linker oberer Ecke (x, y) auf der aktiven Ebene; Größe nach Zoom. Liefert die id.
    pub fn add_table(&mut self, x: f32, y: f32, rows: usize, cols: usize) -> Option<u64> {
        if self.layer_blocked() || rows == 0 || cols == 0 || rows * cols > 10_000 {
            return None;
        }
        let size = 16.0 * self.dp / self.sc;
        let t = Table { x, y, cw: vec![size * 7.0; cols], rh: vec![size * 2.0; rows], cells: vec![String::new(); rows * cols], size };
        let mut it = Item::new(Kind::Table(t));
        it.color = self.color;
        it.width = (self.pen_w * self.dp * 0.5).max(1.0 / self.sc);
        let id = it.id;
        self.add_top(it);
        self.invalidate();
        self.changed();
        Some(id)
    }

    /// Zelle (Zeile, Spalte) der Tabelle `id` am Weltpunkt.
    pub fn table_cell_at(&self, id: u64, x: f32, y: f32) -> Option<(usize, usize)> {
        match &self.items.iter().find(|it| it.id == id)?.kind {
            Kind::Table(t) => t.cell_at(x, y),
            _ => None,
        }
    }

    fn edit_table(&mut self, id: u64, f: impl FnOnce(&mut Table) -> bool) -> bool {
        self.edit_item(id, |it| match &mut it.kind {
            Kind::Table(t) => f(t),
            _ => false,
        })
    }

    /// Setzt den Text einer Zelle; die Zeile wächst bei Bedarf mit.
    pub fn set_cell_text(&mut self, id: u64, r: usize, c: usize, text: &str) -> bool {
        self.edit_table(id, |t| {
            if r >= t.rows() || c >= t.cols() || t.cell(r, c) == text {
                return false;
            }
            let k = r * t.cols() + c;
            t.cells[k] = text.to_string();
            t.fit_row(r);
            true
        })
    }

    /// Fügt vor Zeile `at` (≥ Zeilenzahl = am Ende) eine leere Zeile ein (Höhe wie die Nachbarzeile).
    pub fn add_row(&mut self, id: u64, at: usize) -> bool {
        self.edit_table(id, |t| {
            let at = at.min(t.rows());
            let h = t.rh[at.min(t.rows() - 1)];
            let cols = t.cols();
            t.rh.insert(at, h);
            t.cells.splice(at * cols..at * cols, std::iter::repeat_n(String::new(), cols));
            true
        })
    }

    /// Fügt vor Spalte `at` (≥ Spaltenzahl = am Ende) eine leere Spalte ein.
    pub fn add_col(&mut self, id: u64, at: usize) -> bool {
        self.edit_table(id, |t| {
            let (rows, cols) = (t.rows(), t.cols());
            let at = at.min(cols);
            let w = t.cw[at.min(cols - 1)];
            t.cw.insert(at, w);
            for r in (0..rows).rev() {
                t.cells.insert(r * cols + at, String::new());
            }
            true
        })
    }

    /// Entfernt Zeile `at` (die letzte Zeile bleibt).
    pub fn remove_row(&mut self, id: u64, at: usize) -> bool {
        self.edit_table(id, |t| {
            if t.rows() <= 1 || at >= t.rows() {
                return false;
            }
            let cols = t.cols();
            t.rh.remove(at);
            t.cells.drain(at * cols..(at + 1) * cols);
            true
        })
    }

    /// Entfernt Spalte `at` (die letzte Spalte bleibt).
    pub fn remove_col(&mut self, id: u64, at: usize) -> bool {
        self.edit_table(id, |t| {
            let (rows, cols) = (t.rows(), t.cols());
            if cols <= 1 || at >= cols {
                return false;
            }
            t.cw.remove(at);
            for r in (0..rows).rev() {
                t.cells.remove(r * cols + at);
            }
            true
        })
    }

    // ---------- 9. Stiftart der Auswahl ----------

    /// Setzt die Stiftart (`item::PS_*`) aller ausgewählten Striche.
    pub fn set_sel_pen_style(&mut self, ps: u8) -> bool {
        let ps = ps.min(item::PS_DOTTED);
        if !self.sel_items().iter().any(|it| it.as_stroke().is_some_and(|s| s.ps != ps && !s.hl)) {
            return false;
        }
        self.checkpoint();
        self.sel_mut(|it| {
            if let Kind::Stroke(s) = &mut it.kind {
                if !s.hl {
                    s.ps = ps;
                }
            }
            it.touch();
        });
        self.refresh();
        self.changed();
        true
    }

    // ---------- 10. Zuschnitt ----------

    /// Schneidet das Bild `id` zu: [links, oben, rechts, unten] relativ zum ganzen Bild (0..1), None = ganzes Bild.
    pub fn set_crop(&mut self, id: u64, cr: Option<[f32; 4]>) -> bool {
        self.edit_item(id, |it| match &mut it.kind {
            Kind::Img(m) => {
                let cr = cr.and_then(item::valid_crop);
                if m.cr == cr {
                    return false;
                }
                m.set_crop(cr);
                true
            }
            _ => false,
        })
    }

    // ---------- 11. Kommentare ----------

    /// Neue Kommentar-Stecknadel mit Spitze bei (x, y) auf der aktiven Ebene. `time` = ms seit 1970. Liefert die id.
    pub fn add_pin(&mut self, x: f32, y: f32, author: &str, time: i64, text: &str) -> Option<u64> {
        if self.layer_blocked() {
            return None;
        }
        let mut it = Item::new(Kind::Pin(Pin { x, y, author: author.to_string(), time, text: text.to_string(), replies: Vec::new() }));
        it.color = self.color;
        let id = it.id;
        self.add_top(it);
        self.invalidate();
        self.changed();
        Some(id)
    }

    /// Antwort an die Stecknadel `id` anhängen.
    pub fn reply_pin(&mut self, id: u64, author: &str, time: i64, text: &str) -> bool {
        self.edit_item(id, |it| match &mut it.kind {
            Kind::Pin(p) => {
                p.replies.push(Reply { author: author.to_string(), time, text: text.to_string() });
                true
            }
            _ => false,
        })
    }

    /// Ändert den Text der Stecknadel `id`.
    pub fn set_pin_text(&mut self, id: u64, text: &str) -> bool {
        self.edit_item(id, |it| match &mut it.kind {
            Kind::Pin(p) if p.text != text => {
                p.text = text.to_string();
                true
            }
            _ => false,
        })
    }

    /// Stecknadel am Weltpunkt (oberste), für das Öffnen des Kommentars.
    pub fn pin_at(&self, x: f32, y: f32) -> Option<u64> {
        let r = self.pick_r();
        self.top_down(false).into_iter().map(|i| &self.items[i]).find(|it| matches!(it.kind, Kind::Pin(_)) && it.hit(x, y, r)).map(|it| it.id)
    }
}

/// Neue Endpunkte [x0, y0, x1, y1] eines angedockten Verbinders (None ohne Andockpunkte/Ziele).
fn route(it: &Item, map: &HashMap<u64, &Item>) -> Option<[f32; 4]> {
    let g = it.as_shape()?;
    if !is_linear(g.kind) || g.q.len() < 4 {
        return None;
    }
    let an = g.a;
    let tg = [an[0].and_then(|a| map.get(&a.uid).copied()), an[1].and_then(|a| map.get(&a.uid).copied())];
    if tg[0].is_none() && tg[1].is_none() {
        return None;
    }
    // Bezugspunkte: fester Andockpunkt, Mitte des Ziels oder der freie Endpunkt
    let refp = |e: usize| -> (f32, f32) {
        match (an[e], tg[e]) {
            (Some(Anchor { rel: Some(r), .. }), Some(t)) => t.rel_point(r),
            (Some(_), Some(t)) => {
                let (cx, cy, ..) = t.frame();
                (cx, cy)
            }
            _ => (g.q[e * 2], g.q[e * 2 + 1]),
        }
    };
    let r = [refp(0), refp(1)];
    let mut out = [g.q[0], g.q[1], g.q[2], g.q[3]];
    for e in 0..2 {
        let (Some(a), Some(t)) = (an[e], tg[e]) else { continue };
        let p = match a.rel {
            Some(_) => r[e],
            None => {
                let o = r[1 - e];
                t.edge_toward(o.0, o.1)
            }
        };
        out[e * 2] = p.0;
        out[e * 2 + 1] = p.1;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{json_roundtrip_eq, ARROW, DIAMOND, DIM, ELLIPSE, HEXAGON, POLY, STAR, TRIANGLE};

    fn board() -> Board {
        let mut b = Board::new(1.0);
        b.set_size(800.0, 600.0, 1.0);
        b.recog = 2;
        b
    }

    fn drag(b: &mut Board, pts: &[(f32, f32)], tool: ToolType) {
        b.on_touch(&Motion::single(Action::Down, pts[0].0, pts[0].1, 0.5, tool));
        for p in &pts[1..] {
            b.on_touch(&Motion::single(Action::Move, p.0, p.1, 0.5, tool));
        }
        let l = pts[pts.len() - 1];
        b.on_touch(&Motion::single(Action::Up, l.0, l.1, 0.5, tool));
    }

    fn add(b: &mut Board, it: Item) -> u64 {
        let id = it.id;
        b.items.push(Rc::new(it));
        id
    }

    fn get(b: &Board, id: u64) -> &Item {
        b.items.iter().find(|it| it.id == id).unwrap()
    }

    fn json_ok(b: &Board) {
        for it in &b.items {
            assert!(json_roundtrip_eq(it), "JSON-Rundlauf: {:?}", it.json());
        }
    }

    #[test]
    fn fuellen_und_pipette() {
        let mut b = board();
        let r = add(&mut b, Item::shape(POLY, vec![0.0, 0.0, 100.0, 0.0, 100.0, 100.0, 0.0, 100.0], 0xFF0000FF, 2.0));
        let e = add(&mut b, Item::shape(ELLIPSE, vec![200.0, 0.0, 300.0, 100.0], INK, 2.0));
        let line = add(&mut b, Item::shape(ARROW, vec![0.0, 200.0, 100.0, 200.0], INK, 2.0));
        assert_eq!(b.fill_at(50.0, 50.0, Some(0xFFFF0000)), Some(r));
        assert_eq!(get(&b, r).fill, Some(0xFFFF0000));
        assert_eq!(b.fill_at(250.0, 50.0, Some(0xFF00FF00)), Some(e));
        assert_eq!(b.fill_at(50.0, 200.0, Some(0xFF00FF00)), None, "Pfeil ist nicht geschlossen");
        assert!(get(&b, line).fill.is_none());
        // Pipette: innen = Füllfarbe, Rand = Linienfarbe, daneben nichts
        assert_eq!(b.color_at(50.0, 50.0), Some(0xFFFF0000));
        assert_eq!(b.color_at(0.0, 50.0), Some(0xFF0000FF));
        assert_eq!(b.color_at(500.0, 500.0), None);
        // ein Rückgängig-Schritt je Füllung
        b.undo();
        assert!(get(&b, e).fill.is_none());
        assert_eq!(get(&b, r).fill, Some(0xFFFF0000));
        // Auswahl füllen / entfernen
        b.sel = vec![r, e, line];
        assert_eq!(b.sel_fill(), Some(None));
        assert!(b.set_sel_fill(Some(0xFF123456)));
        assert_eq!(b.sel_fill(), Some(Some(0xFF123456)));
        assert!(get(&b, line).fill.is_none());
        json_ok(&b);
        // gefüllte Fläche trifft innen
        assert!(get(&b, r).hit(50.0, 50.0, 0.0) && !get(&b, r).hit_outline(50.0, 50.0, 0.0));
        // gezeichnet: Füllung innen sichtbar
        let mut pm = Pixmap::new(120, 120).unwrap();
        {
            let mut c = Canvas::new(&mut pm);
            get(&b, r).draw(&mut c, false);
        }
        let px = pm.pixel(50, 50).unwrap();
        assert_eq!((px.red(), px.green(), px.blue(), px.alpha()), (0x12, 0x34, 0x56, 255));
    }

    #[test]
    fn geschlossener_strich_fuellbar() {
        let mut b = board();
        let mut s = Item::stroke(INK, 2.0, false, false);
        for i in 0..=36 {
            let a = i as f32 / 36.0 * std::f32::consts::TAU;
            s.add_point(100.0 + 50.0 * a.cos(), 100.0 + 50.0 * a.sin(), 0.5);
        }
        let mut open = Item::stroke(INK, 2.0, false, false);
        for i in 0..=18 {
            let a = i as f32 / 36.0 * std::f32::consts::TAU;
            open.add_point(300.0 + 50.0 * a.cos(), 100.0 + 50.0 * a.sin(), 0.5);
        }
        assert!(s.fillable() && !open.fillable());
        let id = add(&mut b, s);
        add(&mut b, open);
        assert_eq!(b.fill_at(100.0, 100.0, Some(0xFF00FF00)), Some(id));
        assert_eq!(b.fill_at(300.0, 120.0, Some(0xFF00FF00)), None);
        assert_eq!(b.color_at(100.0, 100.0), Some(0xFF00FF00));
        json_ok(&b);
    }

    #[test]
    fn neue_formen_aus_zieh_rechteck() {
        let mut b = board();
        b.tool = SHAPE;
        for (k, n) in [(TRIANGLE, 3), (DIAMOND, 4), (STAR, 10), (HEXAGON, 6), (item::BUBBLE, 7), (item::ARC, 33)] {
            b.shape_kind = k;
            b.shape_fill = Some(0xFFFFEE00);
            drag(&mut b, &[(100.0, 100.0), (150.0, 150.0), (300.0, 260.0)], ToolType::Stylus);
            let it = b.items.last().unwrap().clone();
            let g = it.as_shape().unwrap();
            assert_eq!(g.kind, k);
            assert_eq!(g.q.len(), n * 2, "Kind {k}");
            let bb = it.bounds();
            assert!(bb.left <= 101.0 && bb.right >= 299.0 && bb.top <= 101.0 && bb.bottom >= 259.0, "Kind {k}: {bb:?}");
            assert_eq!(it.fill.is_some(), k != item::ARC, "Bogen ist offen");
            assert!(json_roundtrip_eq(&it));
            assert!(!it.to_strokes().is_empty());
            // drehen wie Rechteck
            let mut r = (*it).clone();
            r.rot(0.5, 200.0, 180.0);
            assert!(r.as_shape().unwrap().q.iter().all(|v| v.is_finite()));
        }
        // Bogen: offen, Mitte des Rechtecks wird nicht getroffen, der Scheitel schon
        let arc = b.items.last().unwrap();
        assert!(arc.hit(200.0, 100.0, 3.0) && !arc.hit(200.0, 255.0, 3.0));
        assert_eq!(b.items.len(), 6);
        b.undo();
        assert_eq!(b.items.len(), 5);
    }

    #[test]
    fn masslinie_in_mm() {
        let mut b = board();
        b.tool = SHAPE;
        b.shape_kind = DIM;
        // 595 Einheiten = Breite A4 = 210 mm
        drag(&mut b, &[(100.0, 100.0), (200.0, 100.0), (695.0, 100.0)], ToolType::Stylus);
        let it = b.items.last().unwrap();
        let g = it.as_shape().unwrap();
        assert_eq!(g.kind, DIM);
        assert!(is_linear(DIM));
        assert_eq!(item::dim_text(595.0), "209,9 mm");
        assert_eq!(item::dim_text(72.0), "25,4 mm");
        assert_eq!(it.filled_heads().len(), 2);
        let st = it.to_strokes();
        assert_eq!(st.len(), 5, "Linie, 2 Begrenzungen, 2 Spitzen");
        // Text oberhalb der Linie in der Hülle
        assert!(it.bounds().top < 100.0 - 10.0);
        let mut pm = Pixmap::new(800, 200).unwrap();
        {
            let mut c = Canvas::new(&mut pm);
            it.draw(&mut c, false);
        }
        let ink_above = (380..420).any(|x| (80..98).any(|y| pm.pixel(x, y).unwrap().alpha() > 0));
        assert!(ink_above, "Maßtext fehlt");
        assert!(json_roundtrip_eq(it));
    }

    #[test]
    fn stempel() {
        let mut b = board();
        b.color = 0xFF00AA00;
        for k in [STAMP_CHECK, STAMP_CROSS, STAMP_STAR, STAMP_QUESTION, STAMP_EXCLAIM] {
            let n0 = b.items.len();
            let ids = b.stamp(k, 100.0, 100.0, 40.0);
            assert!(!ids.is_empty());
            assert_eq!(b.items.len(), n0 + ids.len());
            let gids: HashSet<u64> = ids.iter().map(|id| get(&b, *id).gid).collect();
            assert_eq!(gids.len(), 1);
            assert_eq!(ids.len() > 1, !gids.contains(&0), "mehrteilig = gruppiert");
            for id in &ids {
                let it = get(&b, *id);
                assert_eq!(it.color, 0xFF00AA00);
                let bb = it.bounds();
                assert!(bb.left > 70.0 && bb.right < 130.0 && bb.top > 70.0 && bb.bottom < 130.0, "Stempel {k}: {bb:?}");
            }
        }
        json_ok(&b);
        b.undo();
        assert_eq!(b.items.len(), 6, "ein Schritt je Stempel");
    }

    #[test]
    fn verbinder_folgen_dem_ziel() {
        let mut b = board();
        let a = add(&mut b, Item::shape(POLY, vec![0.0, 0.0, 100.0, 0.0, 100.0, 100.0, 0.0, 100.0], INK, 2.0));
        let z = add(&mut b, Item::shape(ELLIPSE, vec![300.0, 0.0, 400.0, 100.0], INK, 2.0));
        // Pfeil mit dem Formwerkzeug von A nach Z ziehen: dockt an beide an, Enden auf den Rändern
        b.dock = true;
        b.tool = SHAPE;
        b.shape_kind = 3;
        drag(&mut b, &[(50.0, 50.0), (100.0, 50.0), (350.0, 50.0)], ToolType::Stylus);
        let c = b.items.last().unwrap().id;
        let an = get(&b, c).anchors();
        assert_eq!(an[0].map(|x| x.uid), Some(get(&b, a).uid));
        assert_eq!(an[1].map(|x| x.uid), Some(get(&b, z).uid));
        let q = get(&b, c).as_shape().unwrap().q.clone();
        assert!((q[0] - 100.0).abs() < 0.5 && (q[2] - 300.0).abs() < 0.5, "auf den Rändern: {q:?}");
        assert!(json_roundtrip_eq(get(&b, c)));
        // Ziel verschieben (Auswahlwerkzeug, ziehen): Verbinder folgt im selben Rückgängig-Schritt
        b.tool = SELECT;
        b.select_only(z);
        let undo0 = b.undo.len();
        drag(&mut b, &[(350.0, 50.0), (360.0, 80.0), (350.0, 250.0)], ToolType::Stylus);
        assert_eq!(b.undo.len(), undo0 + 1);
        let q = get(&b, c).as_shape().unwrap().q.clone();
        assert!(q[3] > 100.0, "Ende folgt: {q:?}");
        let (cx, cy, ..) = get(&b, z).frame();
        assert!(((q[2] - cx).powi(2) + (q[3] - cy).powi(2)).sqrt() < 51.0, "auf dem Ellipsenrand");
        b.undo();
        let q = get(&b, c).as_shape().unwrap().q.clone();
        assert!((q[2] - 300.0).abs() < 0.5 && (q[3] - 50.0).abs() < 0.5);
        // Drehen und Skalieren über die API
        b.select_only(a);
        b.rotate_sel(0.3);
        assert!(get(&b, c).as_shape().unwrap().q[0] != 100.0);
        // Ziel löschen: Verbinder bleibt frei
        b.select_only(z);
        b.delete_sel();
        assert!(b.items.iter().any(|it| it.id == c));
        assert_eq!(get(&b, c).anchors()[1], None);
        assert!(get(&b, c).anchors()[0].is_some());
        assert!(get(&b, c).json().get("a1").is_none());
        // Andocken per API an eine Kantenposition
        assert!(b.connect(c, 1, Some(a), Some((1.0, 0.25))));
        let q = get(&b, c).as_shape().unwrap().q.clone();
        let (px, py) = get(&b, a).rel_point((1.0, 0.25));
        assert!((q[2] - px).abs() < 0.01 && (q[3] - py).abs() < 0.01);
        assert!(b.connect(c, 1, None, None));
        assert_eq!(get(&b, c).anchors()[1], None);
    }

    #[test]
    fn endpunkt_ziehen_dockt_an() {
        let mut b = board();
        let z = add(&mut b, Item::shape(POLY, vec![300.0, 0.0, 400.0, 0.0, 400.0, 100.0, 300.0, 100.0], INK, 2.0));
        let l = add(&mut b, Item::shape(item::LINE, vec![0.0, 300.0, 100.0, 300.0], INK, 2.0));
        b.dock = true;
        b.tool = SELECT;
        b.select_only(l);
        drag(&mut b, &[(100.0, 300.0), (150.0, 250.0), (390.0, 10.0)], ToolType::Stylus);
        assert_eq!(get(&b, l).anchors()[1].map(|a| a.uid), Some(get(&b, z).uid));
        assert!(!b.dock_end(l, 1), "unverändert");
    }

    #[test]
    fn gruppen_auswahl_und_kopie() {
        let mut b = board();
        let a = add(&mut b, Item::shape(POLY, vec![0.0, 0.0, 50.0, 0.0, 50.0, 50.0, 0.0, 50.0], INK, 2.0));
        let c = add(&mut b, Item::shape(POLY, vec![100.0, 0.0, 150.0, 0.0, 150.0, 50.0, 100.0, 50.0], INK, 2.0));
        let d = add(&mut b, Item::shape(POLY, vec![300.0, 0.0, 350.0, 0.0, 350.0, 50.0, 300.0, 50.0], INK, 2.0));
        b.sel = vec![a];
        assert!(b.group_sel().is_none(), "ein Objekt");
        b.sel = vec![a, c];
        let g = b.group_sel().unwrap();
        assert_eq!(get(&b, a).gid, g);
        assert!(json_roundtrip_eq(get(&b, a)));
        // Tippen auf ein Mitglied wählt die Gruppe
        b.tool = SELECT;
        b.sel.clear();
        drag(&mut b, &[(0.0, 25.0)], ToolType::Stylus);
        let s: HashSet<u64> = b.sel.iter().copied().collect();
        assert_eq!(s, [a, c].into_iter().collect());
        // Kopie bekommt eine neue Gruppe
        b.duplicate_sel();
        let ng: HashSet<u64> = b.sel_items().iter().map(|it| it.gid).collect();
        assert_eq!(ng.len(), 1);
        assert!(!ng.contains(&g) && !ng.contains(&0));
        b.undo();
        b.sel = vec![a, c];
        assert!(b.ungroup_sel());
        assert_eq!(get(&b, c).gid, 0);
        let _ = d;
    }

    #[test]
    fn ausrichten_und_verteilen() {
        let mut b = board();
        let sq = |x: f32, y: f32, s: f32| Item::shape(POLY, vec![x, y, x + s, y, x + s, y + s, x, y + s], INK, 0.0);
        let a = add(&mut b, sq(0.0, 0.0, 10.0));
        let c = add(&mut b, sq(30.0, 50.0, 20.0));
        let d = add(&mut b, sq(100.0, 20.0, 10.0));
        b.sel = vec![a];
        assert!(!b.align_sel(Align::Left));
        b.sel = vec![a, c, d];
        assert!(b.align_sel(Align::Top));
        for id in [a, c, d] {
            assert!((get(&b, id).bounds().top - 0.0).abs() < 1e-3);
        }
        assert!(b.align_sel(Align::Right));
        for id in [a, c, d] {
            assert!((get(&b, id).bounds().right - 110.0).abs() < 1e-3);
        }
        b.undo();
        b.undo();
        assert!((get(&b, c).bounds().top - 50.0).abs() < 1e-3);
        // verteilen: gleiche Abstände, äußere bleiben
        b.sel = vec![a, c, d];
        assert!(b.distribute_sel(true));
        let (ba, bc, bd) = (get(&b, a).bounds(), get(&b, c).bounds(), get(&b, d).bounds());
        assert!((bc.left - ba.right - (bd.left - bc.right)).abs() < 1e-3);
        assert_eq!(ba.left, 0.0);
        assert_eq!(bd.left, 100.0);
        b.sel = vec![a, c];
        assert!(!b.distribute_sel(false), "erst ab 3");
        // Gruppen zählen als eine Einheit
        b.sel = vec![a, c];
        b.group_sel();
        b.sel = vec![a, c, d];
        assert!(b.align_sel(Align::Bottom));
        assert!((get(&b, a).bounds().bottom - get(&b, c).bounds().bottom).abs() > 1.0, "Gruppe bleibt in sich unverändert");
    }

    #[test]
    fn platz_schaffen() {
        let mut b = board();
        let top = add(&mut b, Item::shape(item::LINE, vec![0.0, 10.0, 100.0, 10.0], INK, 2.0));
        let low = add(&mut b, Item::shape(item::LINE, vec![0.0, 200.0, 100.0, 200.0], INK, 2.0));
        let mut locked = Item::shape(item::LINE, vec![0.0, 300.0, 100.0, 300.0], INK, 2.0);
        b.add_layer(None);
        let lid = b.layers[b.active].id;
        locked.layer = lid;
        let lk = add(&mut b, locked);
        b.edit_layer(b.active, |l| l.locked = true);
        let n0 = b.undo.len();
        assert_eq!(b.make_space(None, 100.0, 50.0), 1);
        assert_eq!(b.undo.len(), n0 + 1);
        assert_eq!(get(&b, top).as_shape().unwrap().q[1], 10.0);
        assert_eq!(get(&b, low).as_shape().unwrap().q[1], 250.0);
        assert_eq!(get(&b, lk).as_shape().unwrap().q[1], 300.0);
        b.undo();
        assert_eq!(get(&b, low).as_shape().unwrap().q[1], 200.0);
    }

    #[test]
    fn platz_schaffen_nur_auf_der_seite() {
        let mut b = board();
        b.board_mode = crate::pages::BoardMode::Pages(crate::pages::PageSetup::default());
        b.add_page(Some(0));
        let rs = b.page_rects();
        let p0 = add(&mut b, Item::shape(item::LINE, vec![10.0, rs[0].top + 400.0, 50.0, rs[0].top + 400.0], INK, 2.0));
        let p1 = add(&mut b, Item::shape(item::LINE, vec![10.0, rs[1].top + 400.0, 50.0, rs[1].top + 400.0], INK, 2.0));
        assert_eq!(b.make_space(Some(1), rs[1].top + 100.0, 30.0), 1);
        assert_eq!(get(&b, p0).as_shape().unwrap().q[1], rs[0].top + 400.0);
        assert_eq!(get(&b, p1).as_shape().unwrap().q[1], rs[1].top + 430.0);
    }

    #[test]
    fn tabellen() {
        let mut b = board();
        let id = b.add_table(10.0, 20.0, 2, 3).unwrap();
        let t = |b: &Board| match &get(b, id).kind {
            Kind::Table(t) => t.clone(),
            _ => unreachable!(),
        };
        assert_eq!((t(&b).rows(), t(&b).cols()), (2, 3));
        let (cw, rh) = (t(&b).cw[0], t(&b).rh[0]);
        assert_eq!(b.table_cell_at(id, 10.0 + cw * 1.5, 20.0 + rh * 0.5), Some((0, 1)));
        assert_eq!(b.table_cell_at(id, 0.0, 0.0), None);
        assert!(b.set_cell_text(id, 1, 2, "Hallo"));
        assert!(!b.set_cell_text(id, 5, 0, "x"));
        assert_eq!(t(&b).cell(1, 2), "Hallo");
        // langer Text: Zeile wächst
        assert!(b.set_cell_text(id, 0, 0, "ein sehr langer Text der umbrechen muss und noch mehr Wörter"));
        assert!(t(&b).rh[0] > rh);
        assert!(b.add_row(id, 1));
        assert_eq!(t(&b).rows(), 3);
        assert_eq!(t(&b).cell(2, 2), "Hallo");
        assert!(b.add_col(id, 0));
        assert_eq!(t(&b).cols(), 4);
        assert_eq!(t(&b).cell(2, 3), "Hallo");
        assert!(b.remove_col(id, 0));
        assert!(b.remove_row(id, 1));
        assert_eq!(t(&b).cell(1, 2), "Hallo");
        assert!(b.remove_row(id, 0) && !b.remove_row(id, 0), "eine Zeile bleibt");
        json_ok(&b);
        let n = b.undo.len();
        b.undo();
        assert_eq!(b.undo.len(), n - 1);
        assert_eq!(t(&b).rows(), 2);
        // Zeichnen und Export
        let mut pm = Pixmap::new(600, 200).unwrap();
        {
            let mut c = Canvas::new(&mut pm);
            get(&b, id).draw(&mut c, false);
        }
        assert!(pm.pixel(10, 21).unwrap().alpha() > 0);
        assert!(b.export_png().is_some());
        // skalieren
        let w0 = t(&b).w();
        b.sel = vec![id];
        b.sel_mut(|it| it.xf(2.0, 10.0, 20.0, 0.0, 0.0));
        assert!((t(&b).w() - 2.0 * w0).abs() < 1e-3);
    }

    #[test]
    fn zuschnitt() {
        let mut b = board();
        let it = Item::new(Kind::Img(item::Img { x: 0.0, y: 0.0, w: 200.0, h: 100.0, ang: 0.0, file: "x.png".into(), cr: None }));
        let id = add(&mut b, it);
        assert!(b.set_crop(id, Some([0.25, 0.0, 0.75, 0.5])));
        let m = match &get(&b, id).kind {
            Kind::Img(m) => m.clone(),
            _ => unreachable!(),
        };
        assert_eq!((m.x, m.y, m.w, m.h), (50.0, 0.0, 100.0, 50.0));
        assert_eq!(m.full_rect(), Some((0.0, 0.0, 200.0, 100.0)));
        assert!(json_roundtrip_eq(get(&b, id)));
        assert_eq!(get(&b, id).bounds().width(), 100.0);
        // zurück auf ganzes Bild
        assert!(b.set_crop(id, None));
        let m = match &get(&b, id).kind {
            Kind::Img(m) => m.clone(),
            _ => unreachable!(),
        };
        assert_eq!((m.x, m.y, m.w, m.h, m.cr), (0.0, 0.0, 200.0, 100.0, None));
        assert!(get(&b, id).json().get("cr").is_none());
        b.undo();
        assert!(matches!(&get(&b, id).kind, Kind::Img(m) if m.cr.is_some()));
        // ungültig
        assert!(item::valid_crop([0.5, 0.0, 0.5, 1.0]).is_none());
    }

    #[test]
    fn zuschnitt_zeichnet_nur_ausschnitt() {
        // Bild links rot, rechts blau; Zuschnitt auf die rechte Hälfte
        let mut rgba = Vec::new();
        for _y in 0..4 {
            for x in 0..4 {
                rgba.extend_from_slice(if x < 2 { &[255, 0, 0, 255] } else { &[0, 0, 255, 255] });
            }
        }
        let mut png = Vec::new();
        image::RgbaImage::from_raw(4, 4, rgba).unwrap().write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        item::put_mem_image("crop-test.png", png);
        let mut it = Item::new(Kind::Img(item::Img { x: 0.0, y: 0.0, w: 40.0, h: 40.0, ang: 0.0, file: "crop-test.png".into(), cr: None }));
        if let Kind::Img(m) = &mut it.kind {
            m.set_crop(Some([0.5, 0.0, 1.0, 1.0]));
        }
        let mut pm = Pixmap::new(60, 60).unwrap();
        {
            let mut c = Canvas::new(&mut pm);
            it.draw(&mut c, false);
        }
        let p = pm.pixel(30, 20).unwrap();
        assert!(p.blue() > 200 && p.red() < 50, "rechte Hälfte (blau) sichtbar");
        assert_eq!(pm.pixel(10, 20).unwrap().alpha(), 0, "links nichts");
        assert_eq!(pm.pixel(45, 20).unwrap().alpha(), 0, "Box = Ausschnitt");
        // Pipette liest das Bild
        let mut b = board();
        add(&mut b, it);
        let c = b.color_at(30.0, 20.0).unwrap();
        assert_eq!(c, 0xFF0000FF);
    }

    #[test]
    fn kommentare() {
        let mut b = board();
        let id = b.add_pin(100.0, 100.0, "Ana", 1_700_000_000_000, "Bitte prüfen").unwrap();
        assert!(b.reply_pin(id, "Ben", 1_700_000_100_000, "Erledigt"));
        assert!(b.set_pin_text(id, "Bitte prüfen!"));
        assert_eq!(b.pin_at(100.0, 100.0 - item::PIN_R * 2.2), Some(id));
        let it = get(&b, id);
        let j = it.json();
        assert_eq!(j["t"], "pin");
        assert_eq!(j["re"][0]["au"], "Ben");
        assert!(json_roundtrip_eq(it));
        match &Item::from_json(&j).unwrap().kind {
            Kind::Pin(p) => {
                assert_eq!(p.replies.len(), 1);
                assert_eq!(p.time, 1_700_000_000_000);
            }
            _ => panic!(),
        }
        // im Export ausgeblendet: PNG mit nur der Nadel ist einfarbig
        b.add_pin(300.0, 300.0, "Ana", 0, "").unwrap();
        let (pm, rb, k) = b.export_image(400.0, 1.0).unwrap();
        let (x, y) = (((100.0 - rb.left) * k) as u32, ((100.0 - item::PIN_R * 2.2 - rb.top) * k) as u32);
        let bg = pm.pixel(0, 0).unwrap();
        assert_eq!(pm.pixel(x, y).unwrap(), bg, "Nadel nicht im Export");
        // am Bildschirm sichtbar
        let mut t = Pixmap::new(800, 600).unwrap();
        b.render(&mut t);
        let (sx, sy) = ((100.0 * b.sc + b.tx) as u32, ((100.0 - item::PIN_R * 2.2) * b.sc + b.ty) as u32);
        assert_ne!(t.pixel(sx, sy).unwrap(), t.pixel(5, 5).unwrap(), "Nadel am Bildschirm");
        let n = b.items.len();
        b.delete_item(id);
        assert_eq!(b.items.len(), n - 1);
        b.undo();
        assert_eq!(b.items.len(), n);
    }

    #[test]
    fn stiftarten() {
        let mut b = board();
        for ps in [item::PS_NORMAL, item::PS_FOUNTAIN, item::PS_PENCIL, item::PS_DASHED, item::PS_DOTTED] {
            b.tool = PEN;
            b.pen_style = ps;
            let y = 50.0 + ps as f32 * 60.0;
            drag(&mut b, &[(50.0, y), (100.0, y + 20.0), (200.0, y + 5.0), (300.0, y + 30.0)], ToolType::Stylus);
            let it = b.items.last().unwrap();
            assert_eq!(it.as_stroke().unwrap().ps, ps);
            assert!(json_roundtrip_eq(it));
            assert_eq!(it.json().get("ps").is_some(), ps != 0);
            let mut pm = Pixmap::new(400, 400).unwrap();
            let mut c = Canvas::new(&mut pm);
            it.draw(&mut c, false);
            drop(c);
            let painted = pm.pixels().iter().filter(|p| p.alpha() > 0).count();
            assert!(painted > 100, "Stiftart {ps} zeichnet ({painted})");
        }
        // Bleistift ist deterministisch
        let it = b.items[2].clone();
        let draw = || {
            let mut pm = Pixmap::new(400, 400).unwrap();
            let mut c = Canvas::new(&mut pm);
            it.draw(&mut c, false);
            drop(c);
            pm
        };
        assert_eq!(draw().data(), draw().data());
        // Auswahl umstellen: ein Schritt
        b.sel = b.items.iter().map(|it| it.id).collect();
        let n = b.undo.len();
        assert!(b.set_sel_pen_style(item::PS_DASHED));
        assert_eq!(b.undo.len(), n + 1);
        assert!(b.items.iter().all(|it| it.as_stroke().unwrap().ps == item::PS_DASHED));
    }

    #[test]
    fn neue_typen_alte_versionen() {
        // ältere Rust-Versionen fielen bei unbekanntem "t" auf eine Notiz zurück und verwarfen den Eintrag (kein Absturz).
        // Unbekannte Felder an bekannten Typen bleiben über extra erhalten; neue Felder stehen in KNOWN_KEYS.
        let v = serde_json::json!({"t":"g","c":-1,"w":2,"k":11,"h":false,"q":[0,0,10,0,5,8],"fc":-65536,"gid":"00000000000000aa","zz":1});
        let it = Item::from_json(&v).unwrap();
        assert_eq!(it.fill, Some(0xFFFF0000));
        assert_eq!(it.gid, 0xaa);
        assert!(it.extra.as_ref().is_some_and(|e| e.len() == 1 && e.contains_key("zz")));
        assert!(Item::from_json(&serde_json::json!({"t":"zukunft","x":1})).is_none());
    }
}
