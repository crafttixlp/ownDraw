//! Mathe-Formeln auf dem Board (Objektart `Kind::Math`, Satz in `crate::math`).
//!
//! Jede öffentliche Änderung ist genau ein Rückgängig-Schritt.

use super::*;
use crate::item::Math;

impl Board {
    /// Schriftgröße neuer Formeln (Welt) bei der aktuellen Zoomstufe.
    pub fn math_size(&self) -> f32 {
        22.0 * self.dp / self.sc
    }

    /// Neue Formel mit linker oberer Ecke (x, y) auf der aktiven Ebene, Farbe = aktuelle Farbe. Liefert die id.
    pub fn add_math(&mut self, x: f32, y: f32, size: f32, src: &str) -> Option<u64> {
        if self.layer_blocked() || !(size > 0.0) || !size.is_finite() {
            return None;
        }
        let mut it = Item::new(Kind::Math(Math::new(x, y, size, src)));
        it.color = self.color;
        let id = it.id;
        self.add_top(it);
        self.invalidate();
        self.changed();
        Some(id)
    }

    /// Ersetzt den Quelltext der Formel `id` (Mittelpunkt bleibt). false, wenn unverändert oder keine Formel.
    pub fn set_math_src(&mut self, id: u64, src: &str) -> bool {
        let Some(i) = self.idx_of(id) else { return false };
        let mut it = (*self.items[i]).clone();
        let Kind::Math(m) = &mut it.kind else { return false };
        if m.src == src {
            return false;
        }
        let (cx, cy) = (m.x + m.w() / 2.0, m.y + m.h() / 2.0);
        let rotated = m.ang != 0.0;
        m.set_src(src);
        if rotated {
            m.x = cx - m.w() / 2.0;
            m.y = cy - m.h() / 2.0;
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

    /// Oberste Formel am Weltpunkt (sichtbare, entsperrte Ebenen).
    pub fn math_at(&self, x: f32, y: f32) -> Option<u64> {
        let r = 4.0 * self.dp / self.sc;
        let mut best: Option<(usize, usize)> = None;
        for (i, it) in self.items.iter().enumerate() {
            if it.as_math().is_none() || !self.usable(it) || !it.near(x, y, r) {
                continue;
            }
            let li = self.layer_idx(it.layer);
            if best.is_none_or(|b| (li, i) > b) {
                best = Some((li, i));
            }
        }
        best.map(|(_, i)| self.items[i].id)
    }

    /// Die einzige ausgewählte Formel, falls genau eine ausgewählt ist.
    pub fn single_math(&self) -> Option<&Item> {
        if self.sel.len() != 1 {
            return None;
        }
        self.sel_items().into_iter().next().filter(|it| it.as_math().is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::json_roundtrip_eq;

    fn board() -> Board {
        let mut b = Board::new(1.0);
        b.set_size(800.0, 600.0, 1.0);
        b
    }

    #[test]
    fn formeln() {
        let mut b = board();
        let id = b.add_math(50.0, 60.0, 30.0, "\\frac{a}{b} + \\sqrt{x^2}").unwrap();
        let get = |b: &Board| b.items.iter().find(|it| it.id == id).unwrap().clone();
        let it = get(&b);
        let m = it.as_math().unwrap();
        assert!(m.errors().is_empty());
        assert!(it.bounds().width() > 30.0 && it.bounds().height() > 30.0);
        assert_eq!(b.math_at(50.0 + m.w() / 2.0, 60.0 + m.h() / 2.0), Some(id));
        assert_eq!(b.math_at(10.0, 10.0), None);
        // JSON-Rundlauf
        let j = it.json();
        assert_eq!(j["t"], "mx");
        assert_eq!(j["tx"], "\\frac{a}{b} + \\sqrt{x^2}");
        assert!(j.get("nw").is_none(), "ältere Versionen sollen den Eintrag auslassen");
        assert!(json_roundtrip_eq(&it));
        // Quelltext ändern = ein Schritt
        let n = b.undo.len();
        assert!(b.set_math_src(id, "\\foo"));
        assert!(!b.set_math_src(id, "\\foo"));
        assert_eq!(b.undo.len(), n + 1);
        assert!(!get(&b).as_math().unwrap().errors().is_empty());
        b.undo();
        assert_eq!(get(&b).as_math().unwrap().src, "\\frac{a}{b} + \\sqrt{x^2}");
        // skalieren, drehen, verschieben
        let w0 = get(&b).as_math().unwrap().w();
        b.sel = vec![id];
        b.sel_mut(|it| it.xf(2.0, 50.0, 60.0, 10.0, 0.0));
        let m = get(&b).as_math().unwrap().clone();
        assert!((m.w() - 2.0 * w0).abs() < 1e-2);
        assert!((m.x - 60.0).abs() < 1e-3);
        b.sel_mut(|it| it.rot(0.5, 0.0, 0.0));
        let it = get(&b);
        assert!((it.as_math().unwrap().ang - 0.5).abs() < 1e-4);
        assert!(json_roundtrip_eq(&it));
        let (cx, cy, ..) = it.frame();
        assert!(it.hit(cx, cy, 0.0));
        b.sel_mut(|it| it.xf_axis(1.5, 1.5, 0.0, 0.0, 0.0));
        // Zeichnen und Export ohne Panik
        let mut pm = Pixmap::new(800, 600).unwrap();
        b.render(&mut pm);
        assert!(b.export_png().is_some());
        assert!(b.export_pdf().is_some());
        // Pipette
        let (cx, cy, ..) = get(&b).frame();
        let _ = b.color_at(cx, cy);
    }

    #[test]
    fn neue_art_alte_versionen() {
        // ältere Versionen fallen auf eine Notiz zurück und scheitern an nw/mh/bg: Eintrag wird ausgelassen
        let v = serde_json::json!({"t":"mx","c":-1,"w":2,"x":1,"y":2,"s":20,"tx":"x^2"});
        let it = Item::from_json(&v).unwrap();
        assert!(it.extra.is_none(), "alle Felder bekannt");
        assert!(Item::from_json(&serde_json::json!({"t":"mx","c":-1,"w":2,"x":1,"y":2,"s":0,"tx":"x"})).is_none());
        assert!(Item::from_json(&serde_json::json!({"t":"mx","c":-1,"w":2,"x":1,"y":2,"s":10})).is_none());
    }
}
