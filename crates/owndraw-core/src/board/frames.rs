//! Export-Ausschnitte: Bereich (ganzes Board, eine Seite, Auswahl) als Bild, einzelne Ebene oder Vektor-Aufnahme.

use super::*;
use crate::canvas::VOp;

/// Was exportiert wird.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    /// Ganzes Board (Seitenmodus: alle Seiten).
    Board,
    /// Eine Seite (Index, nur im Seitenmodus).
    Page(usize),
    /// Nur die ausgewählten Objekte.
    Selection,
}

/// Ein Ausgabe-Ausschnitt in Weltkoordinaten (eine PDF-Seite, ein Bild …).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ExportFrame {
    pub rect: RectF,
    /// Seitenindex im Seitenmodus (Hintergrund + Zuschnitt auf die Seite).
    pub page: Option<usize>,
}

impl ExportFrame {
    pub fn width(&self) -> f32 {
        self.rect.width()
    }
    pub fn height(&self) -> f32 {
        self.rect.height()
    }
}

/// Größte Kante und Pixelzahl eines Export-Bildes (Speicher, Browser).
pub const MAX_EDGE: f32 = 12000.0;
pub const MAX_PIXELS: f32 = 80.0e6;

impl Board {
    /// Deckende Hintergrundfarbe für Formate ohne Transparenz (Seitenmodus: Papier, sonst Board).
    pub fn export_bg(&self) -> u32 {
        if self.pages().is_some() {
            self.page_color()
        } else {
            self.bg_color()
        }
    }

    /// Farbe zwischen untereinander gesetzten Seiten.
    pub fn export_desk(&self) -> u32 {
        self.base_color()
    }

    /// Ausschnitte für den Bereich. Leer, wenn es nichts zu exportieren gibt (leeres Board, keine Auswahl).
    pub fn export_frames(&self, scope: Scope) -> Vec<ExportFrame> {
        match scope {
            Scope::Board => match &self.board_mode {
                BoardMode::Pages(ps) => ps.rects().into_iter().enumerate().map(|(i, r)| ExportFrame { rect: r, page: Some(i) }).collect(),
                BoardMode::Infinite => self
                    .export_bounds()
                    .map(|mut b| {
                        b.inset(-40.0, -40.0);
                        vec![ExportFrame { rect: b, page: None }]
                    })
                    .unwrap_or_default(),
            },
            Scope::Page(i) => self.page_rects().get(i).map(|r| vec![ExportFrame { rect: *r, page: Some(i) }]).unwrap_or_default(),
            Scope::Selection => {
                let mut b: Option<RectF> = None;
                for it in self.items.iter().filter(|it| self.sel.contains(&it.id) && !matches!(it.kind, Kind::Pin(_))) {
                    let r = paint_bounds(it);
                    match b.as_mut() {
                        None => b = Some(r),
                        Some(x) => x.union(&r),
                    }
                }
                b.map(|mut b| {
                    b.inset(-20.0, -20.0);
                    vec![ExportFrame { rect: b, page: None }]
                })
                .unwrap_or_default()
            }
        }
    }

    /// Ids der Objekte, auf die der Bereich beschränkt ist (None = alle).
    pub fn scope_ids(&self, scope: Scope) -> Option<HashSet<u64>> {
        match scope {
            Scope::Selection => Some(self.sel.iter().copied().collect()),
            _ => None,
        }
    }

    /// Exportierte Objekte im Ausschnitt in Zeichenreihenfolge (sichtbare Ebenen, ohne Kommentar-Nadeln).
    pub fn export_items(&self, scope: Scope, f: &ExportFrame) -> Vec<Rc<Item>> {
        let only = self.scope_ids(scope);
        let mut v = f.rect;
        v.inset(-4.0, -4.0);
        self.items
            .iter()
            .filter(|it| {
                !matches!(it.kind, Kind::Pin(_))
                    && self.layers[self.layer_idx(it.layer)].visible
                    && only.as_ref().map(|o| o.contains(&it.id)).unwrap_or(true)
                    && RectF::intersects(&v, &paint_bounds(it))
            })
            .cloned()
            .collect()
    }

    /// Maßstab k (Pixel je Welteinheit) für den Ausschnitt, begrenzt auf [`MAX_EDGE`] und [`MAX_PIXELS`].
    pub fn frame_scale(f: &ExportFrame, want: f32) -> f32 {
        let (w, h) = (f.width().max(1.0), f.height().max(1.0));
        let mut k = want.max(0.01);
        k = k.min(MAX_EDGE / w.max(h));
        if w * h * k * k > MAX_PIXELS {
            k = (MAX_PIXELS / (w * h)).sqrt();
        }
        k
    }

    /// Hintergrund und Inhalt des Ausschnitts in Weltkoordinaten (aktuelle Transformation = Welt).
    /// `only` = nur diese Objekte; `px` = Welteinheiten je Ausgabe-Pixel.
    fn draw_export_frame(&self, c: &mut Canvas, f: &ExportFrame, with_bg: bool, only: Option<HashSet<u64>>, px: f32) {
        let r = f.rect;
        if with_bg {
            match (f.page, self.pages()) {
                (Some(i), Some(ps)) => {
                    c.draw_rect(r.left, r.top, r.right, r.bottom, &Paint::fill(self.page_color()));
                    if let Some(p) = ps.pages.get(i) {
                        self.draw_page_bg(c, p.bg, &r, px);
                    }
                }
                _ => {
                    c.draw_rect(r.left, r.top, r.right, r.bottom, &Paint::fill(self.base_color()));
                    // Seitenmodus, Ausschnitt über mehrere Seiten: Seitenblätter mitzeichnen
                    self.draw_sheets(c, Some(&r), px);
                }
            }
        }
        *self.export_only.borrow_mut() = only;
        c.save();
        c.clip_rect(r.left, r.top, r.right, r.bottom);
        self.clip_pages(c);
        let mut v = r;
        v.inset(-4.0, -4.0);
        self.draw_layered(c, Some(&v), false, false);
        c.restore();
        *self.export_only.borrow_mut() = None;
    }

    /// Ausschnitt als Bild mit Maßstab k (siehe [`Board::frame_scale`]). `with_bg` = sonst durchsichtig.
    pub fn render_frame(&self, scope: Scope, f: &ExportFrame, k: f32, with_bg: bool) -> Option<Pixmap> {
        self.render_frame_ids(f, k, with_bg, self.scope_ids(scope))
    }

    /// Wie [`Board::render_frame`], aber nur mit den Objekten `only` (None = alle), z. B. für den Zeichenverlauf.
    pub fn render_frame_ids(&self, f: &ExportFrame, k: f32, with_bg: bool, only: Option<HashSet<u64>>) -> Option<Pixmap> {
        let mut bm = Pixmap::new(((f.width() * k).round() as u32).max(1), ((f.height() * k).round() as u32).max(1))?;
        {
            let mut c = Canvas::new(&mut bm);
            c.scale(k, k);
            c.translate(-f.rect.left, -f.rect.top);
            self.draw_export_frame(&mut c, f, with_bg, only, 1.0 / k);
        }
        Some(bm)
    }

    /// Nur eine Ebene (Index in `layers`, auch ausgeblendete) ohne Hintergrund und ohne Ebenen-Deckkraft.
    pub fn render_frame_layer(&self, scope: Scope, f: &ExportFrame, layer: usize, k: f32) -> Option<Pixmap> {
        let l = self.layers.get(layer)?;
        let only = self.scope_ids(scope);
        let mut bm = Pixmap::new(((f.width() * k).round() as u32).max(1), ((f.height() * k).round() as u32).max(1))?;
        {
            let mut c = Canvas::new(&mut bm);
            c.scale(k, k);
            c.translate(-f.rect.left, -f.rect.top);
            c.clip_rect(f.rect.left, f.rect.top, f.rect.right, f.rect.bottom);
            self.clip_pages(&mut c);
            for it in &self.items {
                if it.layer == l.id && !matches!(it.kind, Kind::Pin(_)) && only.as_ref().map(|o| o.contains(&it.id)).unwrap_or(true) {
                    it.draw(&mut c, self.dark);
                }
            }
        }
        Some(bm)
    }

    /// Ausschnitt als Vektor-Befehle; Ausgaberaum = Welteinheiten mit Ursprung links oben im Ausschnitt.
    pub fn record_frame(&self, scope: Scope, f: &ExportFrame, with_bg: bool) -> Vec<VOp> {
        let Some(mut pm) = Pixmap::new(1, 1) else { return Vec::new() };
        let mut c = Canvas::new(&mut pm);
        c.start_recording();
        c.translate(-f.rect.left, -f.rect.top);
        self.draw_export_frame(&mut c, f, with_bg, self.scope_ids(scope), 1.0);
        let (rec, keep) = c.take_recording();
        let ops = rec.vector_ops(f.width(), f.height());
        drop(keep);
        ops
    }
}
