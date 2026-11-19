//! Laserpointer: kein Board-Objekt, nicht gespeichert, nicht im Rückgängig. Eine Spur aus Punkten mit Zeitstempel,
//! die nach [`FADE`] Sekunden verblasst. Die Oberfläche legt sie mit [`Laser::draw`] über das Board; andere Teilnehmer
//! bekommen neue Punkte als Collab-Nachricht `laser` ([`crate::collab::LaserMsg`]).

use crate::canvas::{Canvas, Cap, Paint};

/// Lebensdauer eines Punktes (s).
pub const FADE: f64 = 1.5;
/// Standardfarbe (Rot).
pub const LASER_COLOR: u32 = 0xFFFF3B30;
/// Farben zur Auswahl.
pub const COLORS: [(u32, &str); 8] = [
    (0xFFFF3B30, "Rot"),
    (0xFFFF9500, "Orange"),
    (0xFFFFCC00, "Gelb"),
    (0xFF34C759, "Grün"),
    (0xFF00B8D9, "Türkis"),
    (0xFF007AFF, "Blau"),
    (0xFFAF52DE, "Violett"),
    (0xFFFF2D92, "Pink"),
];
/// Durchmesser des Laserpunkts in Bildschirmpunkten (Standard, kleinster, größter).
pub const DOT_SIZE: f32 = 32.0;
pub const DOT_MIN: f32 = 12.0;
pub const DOT_MAX: f32 = 96.0;
/// Ausblenden des Punkts nach dem Absetzen (s).
pub const DOT_FADE: f64 = 0.35;

/// Spur eines Laserpointers (Weltkoordinaten, Zeit in s wie bei `Board::tick`).
#[derive(Clone, Debug, Default)]
pub struct Laser {
    pub color: u32,
    /// (x, y, Zeit); `None` trennt Spuren (abgesetzt).
    pts: Vec<Option<(f32, f32, f64)>>,
    /// Noch nicht gesendete Punkte (für die Zusammenarbeit).
    unsent: Vec<(f32, f32)>,
}

impl Laser {
    pub fn new(color: u32) -> Laser {
        Laser { color, pts: Vec::new(), unsent: Vec::new() }
    }

    /// Neuer Punkt der aktuellen Spur.
    pub fn add(&mut self, x: f32, y: f32, now: f64) {
        self.pts.push(Some((x, y, now)));
        self.unsent.push((x, y));
    }

    /// Stift abgesetzt: die nächste Spur wird nicht mit dieser verbunden.
    pub fn lift(&mut self) {
        if matches!(self.pts.last(), Some(Some(_))) {
            self.pts.push(None);
        }
    }

    /// Punkte eines anderen Teilnehmers übernehmen (eine zusammenhängende Spur, Zeit = jetzt).
    pub fn add_remote(&mut self, pts: &[(f32, f32)], now: f64) {
        self.lift();
        self.pts.extend(pts.iter().map(|&(x, y)| Some((x, y, now))));
        self.lift();
    }

    /// Entfernt verblasste Punkte. Liefert true, solange noch etwas sichtbar ist.
    pub fn prune(&mut self, now: f64) -> bool {
        let keep = self.pts.iter().position(|p| p.is_some_and(|p| now - p.2 < FADE)).unwrap_or(self.pts.len());
        self.pts.drain(..keep);
        while matches!(self.pts.first(), Some(None)) {
            self.pts.remove(0);
        }
        self.pts.iter().any(Option::is_some)
    }

    /// Punkte (x, y, Zeit) der Spuren; `None` trennt Spuren (zum Zeichnen in der Oberfläche).
    pub fn points(&self) -> &[Option<(f32, f32, f64)>] {
        &self.pts
    }

    pub fn is_empty(&self) -> bool {
        !self.pts.iter().any(Option::is_some)
    }

    /// Seit dem letzten Aufruf hinzugekommene eigene Punkte (zum Senden).
    pub fn take_unsent(&mut self) -> Vec<(f32, f32)> {
        std::mem::take(&mut self.unsent)
    }

    /// Zeichnet die Spur; `c` steht auf Weltkoordinaten, `px` = Welteinheiten je Bildschirmpixel (Strichbreite
    /// bleibt am Bildschirm gleich). Ältere Abschnitte werden dünner und durchsichtiger.
    pub fn draw(&self, c: &mut Canvas, now: f64, px: f32) {
        let rgb = self.color & 0xFFFFFF;
        let mut prev: Option<(f32, f32, f64)> = None;
        for p in &self.pts {
            let Some(b) = *p else {
                prev = None;
                continue;
            };
            let k = (1.0 - (now - b.2) / FADE).clamp(0.0, 1.0) as f32;
            if k > 0.0 {
                let a = prev.unwrap_or(b);
                let glow = Paint::stroke(rgb | (((k * 70.0) as u32) << 24), 12.0 * px * (0.5 + 0.5 * k), Cap::Round);
                let core = Paint::stroke(rgb | (((k * 255.0) as u32) << 24), 4.0 * px * (0.4 + 0.6 * k), Cap::Round);
                c.draw_line(a.0, a.1, b.0, b.1, &glow);
                c.draw_line(a.0, a.1, b.0, b.1, &core);
            }
            prev = Some(b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verblasst_nach_fade() {
        let mut l = Laser::new(LASER_COLOR);
        l.add(0.0, 0.0, 0.0);
        l.add(10.0, 0.0, 0.5);
        assert_eq!(l.take_unsent().len(), 2);
        assert!(l.take_unsent().is_empty());
        assert!(l.prune(1.0));
        assert!(l.prune(1.9), "zweiter Punkt lebt noch");
        assert!(!l.prune(2.1));
        assert!(l.is_empty());
    }

    #[test]
    fn zeichnet_ohne_absturz() {
        let mut pm = tiny_skia::Pixmap::new(40, 40).unwrap();
        let mut l = Laser::new(LASER_COLOR);
        l.add(5.0, 20.0, 0.0);
        l.add(35.0, 20.0, 0.1);
        l.lift();
        l.add_remote(&[(5.0, 5.0)], 0.1);
        {
            let mut c = Canvas::new(&mut pm);
            l.draw(&mut c, 0.2, 1.0);
        }
        assert!(pm.pixel(20, 20).unwrap().alpha() > 100);
    }
}
