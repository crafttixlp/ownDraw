//! Seitenmodus (WP0: Typen; WP2: Layout, Clipping, Seiten-Operationen).
//!
//! Seiten liegen in Weltkoordinaten untereinander (1 pt = 1 Welteinheit), Abstand [`PAGE_GAP`].
//! Seite 0 beginnt bei (0, 0); Seite i bei y = Summe der Höhen davor + i × PAGE_GAP. Alle Seiten sind links bündig (x = 0).

use serde_json::Value;

/// Abstand zwischen zwei Seiten in Welteinheiten (pt).
pub const PAGE_GAP: f32 = 24.0;

/// Seitenhintergrund.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PageBg {
    Plain,
    #[default]
    Lined,
    Grid,
    Dotted,
}

impl PageBg {
    /// Name im Dateiformat ("plain", "lined", "grid", "dotted").
    pub fn name(self) -> &'static str {
        match self {
            PageBg::Plain => "plain",
            PageBg::Lined => "lined",
            PageBg::Grid => "grid",
            PageBg::Dotted => "dotted",
        }
    }
    /// Unbekannte Namen → `Plain`.
    pub fn from_name(s: &str) -> PageBg {
        match s {
            "lined" | "ruled" => PageBg::Lined,
            "grid" | "graph" => PageBg::Grid,
            "dotted" => PageBg::Dotted,
            _ => PageBg::Plain,
        }
    }
}

/// Seitenformat in pt (1/72 Zoll).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageFormat {
    A4,
    A5,
    Letter,
}

impl PageFormat {
    /// (Breite, Höhe) hochkant in pt.
    pub fn size(self) -> (f32, f32) {
        match self {
            PageFormat::A4 => (595.0, 842.0),
            PageFormat::A5 => (420.0, 595.0),
            PageFormat::Letter => (612.0, 792.0),
        }
    }
}

/// Eine Seite. `extra` hält unbekannte/fremde Attribute (z. B. Xournal++-PDF-Hintergrund) unverändert.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub id: String,
    pub w: f32,
    pub h: f32,
    pub bg: PageBg,
    pub extra: Value,
}

/// Seiten-Einstellungen eines Boards im Seitenmodus. Standard: A4 hoch, liniert, eine Seite.
#[derive(Clone, Debug, PartialEq)]
pub struct PageSetup {
    /// Vorgabe für neue Seiten.
    pub w: f32,
    pub h: f32,
    pub bg: PageBg,
    pub pages: Vec<Page>,
}

impl Default for PageSetup {
    fn default() -> Self {
        let (w, h) = PageFormat::A4.size();
        PageSetup {
            w,
            h,
            bg: PageBg::Lined,
            pages: vec![Page {
                id: crate::store::random_hex(8),
                w,
                h,
                bg: PageBg::Lined,
                extra: Value::Null,
            }],
        }
    }
}

/// Board-Modus.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum BoardMode {
    #[default]
    Infinite,
    Pages(PageSetup),
}

// ---------- Layout (WP2) ----------

use crate::geom::RectF;
use crate::item::Item;

impl Page {
    /// Neue Seite mit frischer id.
    pub fn new(w: f32, h: f32, bg: PageBg) -> Page {
        Page {
            id: crate::store::random_hex(8),
            w,
            h,
            bg,
            extra: Value::Null,
        }
    }
}

impl PageSetup {
    /// Einstellungen mit `n` (mind. 1) gleichen Seiten.
    pub fn new(w: f32, h: f32, bg: PageBg, n: usize) -> PageSetup {
        PageSetup {
            w,
            h,
            bg,
            pages: (0..n.max(1)).map(|_| Page::new(w, h, bg)).collect(),
        }
    }

    /// Seitenrechtecke in Weltkoordinaten (untereinander, x = 0, Abstand [`PAGE_GAP`]).
    pub fn rects(&self) -> Vec<RectF> {
        let mut y = 0.0;
        self.pages
            .iter()
            .map(|p| {
                let r = RectF::new(0.0, y, p.w, y + p.h);
                y += p.h + PAGE_GAP;
                r
            })
            .collect()
    }

    /// Rechteck der Seite i.
    pub fn rect(&self, i: usize) -> Option<RectF> {
        self.rects().get(i).copied()
    }

    /// Huelle aller Seiten (None ohne Seiten).
    pub fn bounds(&self) -> Option<RectF> {
        let rs = self.rects();
        let mut it = rs.iter();
        let mut b = *it.next()?;
        for r in it {
            b.union(r);
        }
        Some(b)
    }

    /// Seite, die den Weltpunkt (x, y) enthaelt (Rand eingeschlossen); None im Zwischenraum/ausserhalb.
    pub fn page_at(&self, x: f32, y: f32) -> Option<usize> {
        self.rects()
            .iter()
            .position(|r| x >= r.left && x <= r.right && y >= r.top && y <= r.bottom)
    }

    /// Naechstgelegene Seite zum Weltpunkt (Abstand zum Rechteck; enthaltende Seite gewinnt). None ohne Seiten.
    pub fn nearest_page(&self, x: f32, y: f32) -> Option<usize> {
        let d = |r: &RectF| {
            let dx = (r.left - x).max(0.0).max(x - r.right);
            let dy = (r.top - y).max(0.0).max(y - r.bottom);
            dx * dx + dy * dy
        };
        let rs = self.rects();
        (0..rs.len()).min_by(|&a, &b| d(&rs[a]).total_cmp(&d(&rs[b])))
    }

    /// Seite eines Objekts: die Seite unter seinem Mittelpunkt, sonst die naechstgelegene. None nur ohne Seiten.
    pub fn page_of(&self, it: &Item) -> Option<usize> {
        let b = it.bounds();
        self.nearest_page(b.center_x(), b.center_y())
    }

    /// Fuegt nach Seite `after` (None = am Anfang) eine Seite mit den Vorgaben ein; liefert ihren Index.
    pub fn insert_page(&mut self, after: Option<usize>) -> usize {
        let at = after.map(|a| (a + 1).min(self.pages.len())).unwrap_or(0);
        self.pages.insert(at, Page::new(self.w, self.h, self.bg));
        at
    }

    /// Senkrechte Verschiebung je alter Seite beim Uebergang zu `new`: `order[neuer Index]` = alter Index oder None (neue Seite).
    /// Ergebnis[alter Index] = neue Oberkante − alte Oberkante; None = Seite entfaellt.
    pub fn shifts(&self, new: &PageSetup, order: &[Option<usize>]) -> Vec<Option<f32>> {
        let (old_r, new_r) = (self.rects(), new.rects());
        let mut out = vec![None; self.pages.len()];
        for (ni, oi) in order.iter().enumerate() {
            if let Some(oi) = *oi {
                if let (Some(o), Some(n)) = (old_r.get(oi), new_r.get(ni)) {
                    out[oi] = Some(n.top - o.top);
                }
            }
        }
        out
    }
}
