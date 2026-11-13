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
        PageSetup { w, h, bg: PageBg::Lined, pages: vec![Page { id: crate::store::random_hex(8), w, h, bg: PageBg::Lined, extra: Value::Null }] }
    }
}

/// Board-Modus.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum BoardMode {
    #[default]
    Infinite,
    Pages(PageSetup),
}
