//! Werkzeug „Umwandeln“ (Handschrift-Erkennung): Striche per Lasso oder Antippen auswählen, als Bild rendern,
//! Kontext der Seite sammeln und die Striche durch Text bzw. Formel ersetzen (ein Rückgängig-Schritt).

use super::*;
use crate::hw::{self, Ink, Kontext};
use crate::item::Math;

/// Ersatz für die Striche.
#[derive(Clone, Debug, PartialEq)]
pub enum Ersatz {
    /// Freier Text (Notiz ohne Hintergrund).
    Text(String),
    /// Formel-Objekt (LaTeX); Rechenzeilen werden vom Werkzeug „Rechnen“ mitgerechnet.
    Formel(String),
}

/// Höchstens so viele Striche wandert das Antippen ein.
const TAP_MAX: usize = 400;

impl Board {
    /// Handschrift-Objekt? (Freihand-Strich oder Linien-Form, kein Marker, auf sichtbarer, entsperrter Ebene)
    fn is_ink(&self, it: &Item) -> bool {
        if !self.usable(it) || self.hidden == Some(it.id) {
            return false;
        }
        match &it.kind {
            Kind::Stroke(s) => !s.hl && s.n() > 0,
            Kind::Shape(g) => !g.hl && matches!(g.kind, item::LINE | item::ARC),
            _ => false,
        }
    }

    /// Striche, die zu mindestens 60 % im Lasso (Weltkoordinaten) liegen, in Board-Reihenfolge.
    pub fn ink_in_lasso(&self, poly: &[(f32, f32)]) -> Vec<u64> {
        if poly.len() < 3 {
            return Vec::new();
        }
        let mut out = Vec::new();
        for it in self.items.iter().filter(|it| self.is_ink(it)) {
            let pts: Vec<(f32, f32)> = hw::polylines(it).into_iter().flatten().collect();
            if pts.is_empty() {
                continue;
            }
            let inside = pts.iter().filter(|(x, y)| hw::in_poly(poly, *x, *y)).count();
            if inside * 10 >= pts.len() * 6 {
                out.push(it.id);
            }
        }
        out
    }

    /// Antippen: der Strich am Weltpunkt und seine Nachbarn in derselben Zeile (Abstand bis etwa eine
    /// Zeichenhöhe). Leer, wenn dort kein Strich ist.
    pub fn ink_at(&self, wx: f32, wy: f32) -> Vec<u64> {
        let r = 10.0 * self.dp / self.sc;
        let ink: Vec<&Item> = self.items.iter().map(|it| &**it).filter(|it| self.is_ink(it)).collect();
        let Some(seed) = ink.iter().rev().find(|it| it.hit(wx, wy, r)).or_else(|| ink.iter().rev().find(|it| it.near(wx, wy, r * 2.0))) else {
            return Vec::new();
        };
        let bounds: Vec<RectF> = ink.iter().map(|it| it.bounds()).collect();
        let mut taken = vec![false; ink.len()];
        let si = ink.iter().position(|it| it.id == seed.id).unwrap_or(0);
        taken[si] = true;
        let mut g = bounds[si];
        loop {
            // typische Zeichenhöhe der Gruppe (mindestens die Höhe des ersten Strichs bzw. ein paar Punkt)
            let lines: Vec<Vec<(f32, f32)>> = ink.iter().zip(&taken).filter(|(_, t)| **t).flat_map(|(it, _)| hw::polylines(it)).collect();
            let ch = hw::typical_height(&lines).unwrap_or(g.height()).max(6.0 * self.dp / self.sc);
            let band = (g.top - 0.35 * ch, g.bottom + 0.35 * ch);
            let gap = 1.1 * ch;
            let mut grew = false;
            for (i, b) in bounds.iter().enumerate() {
                if taken[i] {
                    continue;
                }
                let cy = b.center_y();
                let vert = (cy >= band.0 && cy <= band.1) || (b.bottom.min(g.bottom) - b.top.max(g.top)) >= 0.5 * b.height().max(1e-3);
                let horiz = b.left <= g.right + gap && b.right >= g.left - gap;
                // Bruchstrich/Unterstreichung unter bzw. über der Gruppe zählt mit, ebenso alles, was hineinragt
                if vert && horiz {
                    taken[i] = true;
                    g.union(b);
                    grew = true;
                }
            }
            if !grew || taken.iter().filter(|t| **t).count() >= TAP_MAX {
                break;
            }
        }
        ink.iter().zip(&taken).filter(|(_, t)| **t).map(|(it, _)| it.id).collect()
    }

    /// Hülle der Objekte (Weltkoordinaten).
    pub fn bounds_of(&self, ids: &[u64]) -> Option<RectF> {
        let mut r: Option<RectF> = None;
        for it in self.items.iter().filter(|it| ids.contains(&it.id)) {
            let b = it.bounds();
            match r.as_mut() {
                Some(r) => r.union(&b),
                None => r = Some(b),
            }
        }
        r
    }

    /// Graustufenbild der Striche für die Erkennung.
    pub fn render_ink(&self, ids: &[u64]) -> Option<Ink> {
        let its: Vec<&Item> = self.items.iter().filter(|it| ids.contains(&it.id)).map(|it| &**it).collect();
        hw::render(&its)
    }

    /// Wissen der Seite rund um die Striche: Variablen/Einheiten der Formeln (Rechenzeilen), Wörter der Texte.
    /// Im Seitenmodus nur die Seite der Striche.
    pub fn hw_context(&self, ids: &[u64]) -> Kontext {
        let mut k = Kontext::default();
        let page = self.bounds_of(ids).and_then(|b| self.page_at(b.center_x(), b.center_y()));
        let pages = if self.pages().is_some() { Some(self.page_of_items()) } else { None };
        for (i, it) in self.items.iter().enumerate() {
            if ids.contains(&it.id) {
                continue;
            }
            if let (Some(p), Some(pg)) = (page, pages.as_ref()) {
                if pg.get(i).copied().flatten() != Some(p) {
                    continue;
                }
            }
            match &it.kind {
                Kind::Math(m) => k.add_calc_line(&crate::calc::to_plain(&m.src)),
                Kind::Note(n) => {
                    for l in n.text.lines() {
                        if crate::calc::is_calc_line(l) {
                            k.add_calc_line(l);
                        }
                    }
                    k.add_text(&n.text)
                }
                _ => {}
            }
        }
        k
    }

    /// Ersetzt die Striche durch Text bzw. Formel an gleicher Stelle und in passender Größe (ein Rückgängig-Schritt).
    /// Farbe = häufigste Strichfarbe, Ebene = Ebene der Striche. Liefert die id des neuen Objekts.
    pub fn replace_ink(&mut self, ids: &[u64], neu: &Ersatz) -> Option<u64> {
        let src = match neu {
            Ersatz::Text(s) | Ersatz::Formel(s) => s.trim(),
        };
        if src.is_empty() {
            return None;
        }
        let olds: Vec<Rc<Item>> = self.items.iter().filter(|it| ids.contains(&it.id) && self.usable(it)).cloned().collect();
        if olds.is_empty() {
            return None;
        }
        let mut b = olds[0].bounds();
        for it in &olds[1..] {
            b.union(&it.bounds());
        }
        // häufigste Farbe, Ebene des ersten Strichs
        let mut cols: Vec<(u32, usize)> = Vec::new();
        for it in &olds {
            match cols.iter_mut().find(|c| c.0 == it.color) {
                Some(c) => c.1 += 1,
                None => cols.push((it.color, 1)),
            }
        }
        let color = cols.iter().max_by_key(|c| c.1).map(|c| c.0).unwrap_or(self.color);
        let layer = olds[0].layer;
        let (bw, bh) = (b.width().max(1.0), b.height().max(1.0));
        let mut it = match neu {
            Ersatz::Formel(src) => {
                let m0 = Math::new(0.0, 0.0, item::MATH_BASE, src);
                let (w0, h0) = (m0.w().max(1.0), m0.h().max(1.0));
                let size = (item::MATH_BASE * (bh / h0).min(1.25 * bw / w0)).max(2.0 * self.dp / self.sc);
                let m = Math::new(0.0, 0.0, size, src);
                let (x, y) = (b.left, b.center_y() - m.h() / 2.0);
                Item::new(Kind::Math(Math::new(x, y, size, src)))
            }
            Ersatz::Text(src) => {
                let lines = src.lines().count().max(1) as f32;
                let size = (bh / (1.25 * lines)).max(4.0 * self.dp / self.sc);
                let mut n = Item::note(src, color, size, b.left, 0.0, 0.0, 0.0, 0);
                let h = n.as_note().map(|n| n.h).unwrap_or(size);
                if let Kind::Note(nn) = &mut n.kind {
                    nn.y = b.center_y() - h / 2.0;
                }
                n.touch();
                n
            }
        };
        it.color = color;
        it.layer = layer;
        let id = it.id;
        self.checkpoint();
        self.items.retain(|x| !olds.iter().any(|o| o.id == x.id));
        self.items.push(Rc::new(it));
        self.sel_clear();
        self.reroute(true);
        self.dirty = true;
        self.refresh();
        self.changed();
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Board {
        let mut b = Board::new(1.0);
        b.set_size(800.0, 600.0, 1.0);
        b
    }

    /// Strich über die Punkte (mit Zwischenpunkten).
    fn line(b: &mut Board, pts: &[(f32, f32)]) -> u64 {
        let mut it = Item::stroke(0xFF223344, 2.0, false, false);
        for w in pts.windows(2) {
            for k in 0..5 {
                let t = k as f32 / 5.0;
                it.add_point(w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t, 0.5);
            }
        }
        let l = pts[pts.len() - 1];
        it.add_point(l.0, l.1, 0.5);
        let id = it.id;
        b.add_top(it);
        id
    }

    /// „a = 1“ in einer Zeile bei y 100–120 und eine zweite Zeile bei y 200.
    fn sample(b: &mut Board) -> (Vec<u64>, Vec<u64>) {
        let a = line(b, &[(100.0, 110.0), (110.0, 100.0), (118.0, 110.0), (110.0, 120.0), (100.0, 110.0)]);
        let e1 = line(b, &[(130.0, 107.0), (145.0, 107.0)]);
        let e2 = line(b, &[(130.0, 114.0), (145.0, 114.0)]);
        let one = line(b, &[(160.0, 100.0), (160.0, 120.0)]);
        let far = line(b, &[(400.0, 100.0), (400.0, 120.0)]);
        let z2 = line(b, &[(100.0, 200.0), (120.0, 220.0)]);
        (vec![a, e1, e2, one], vec![far, z2])
    }

    #[test]
    fn antippen_nimmt_die_zeile() {
        let mut b = board();
        let (zeile, rest) = sample(&mut b);
        let mut got = b.ink_at(160.0, 110.0);
        got.sort();
        let mut want = zeile.clone();
        want.sort();
        assert_eq!(got, want, "Strich + Nachbarn der Zeile, nicht der ferne Strich und nicht die nächste Zeile");
        assert!(!got.contains(&rest[0]) && !got.contains(&rest[1]));
        assert!(b.ink_at(600.0, 500.0).is_empty(), "nichts getroffen");
        // Marker und Text zählen nicht
        let mut m = Item::stroke(0x80FFFF00, 10.0, true, false);
        m.add_point(300.0, 300.0, 0.5);
        m.add_point(320.0, 300.0, 0.5);
        b.add_top(m);
        assert!(b.ink_at(310.0, 300.0).is_empty());
    }

    #[test]
    fn lasso_waehlt_umkreiste_striche() {
        let mut b = board();
        let (zeile, rest) = sample(&mut b);
        let poly = [(90.0, 90.0), (170.0, 90.0), (170.0, 130.0), (90.0, 130.0)];
        assert_eq!(b.ink_in_lasso(&poly), zeile);
        // Strich, der nur zur Hälfte drin ist, zählt nicht
        let poly = [(90.0, 90.0), (170.0, 90.0), (170.0, 210.0), (90.0, 210.0)];
        assert!(!b.ink_in_lasso(&poly).contains(&rest[1]));
        assert!(b.ink_in_lasso(&poly[..2]).is_empty());
        // gesperrte Ebene: nichts
        b.edit_layer(0, |l| l.locked = true);
        assert!(b.ink_in_lasso(&[(0.0, 0.0), (800.0, 0.0), (800.0, 600.0), (0.0, 600.0)]).is_empty());
    }

    #[test]
    fn rendern_und_kontext() {
        let mut b = board();
        let (zeile, _) = sample(&mut b);
        let ink = b.render_ink(&zeile).unwrap();
        assert!(ink.w > ink.h, "Zeile ist breiter als hoch");
        assert!(ink.gray.iter().any(|g| *g < 50) && ink.gray.iter().any(|g| *g == 255));
        b.add_math(100.0, 300.0, 20.0, &crate::calc::to_latex("F: 2 kN"));
        b.add_note(100.0, 400.0, "Kraft am Hebel", false);
        let k = b.hw_context(&zeile);
        assert!(k.vars.contains(&"F".to_string()), "{k:?}");
        assert!(k.einheiten.contains(&"kN".to_string()), "{k:?}");
        assert!(k.woerter.contains(&"hebel".to_string()));
    }

    #[test]
    fn ersetzen_ist_ein_schritt() {
        let mut b = board();
        let (zeile, rest) = sample(&mut b);
        let n_items = b.items.len();
        let n_undo = b.undo.len();
        let bb = b.bounds_of(&zeile).unwrap();
        let id = b.replace_ink(&zeile, &Ersatz::Formel("a=1".into())).unwrap();
        assert_eq!(b.undo.len(), n_undo + 1, "ein Rückgängig-Schritt");
        assert_eq!(b.items.len(), n_items - zeile.len() + 1);
        let it = b.items.iter().find(|it| it.id == id).unwrap();
        let m = it.as_math().expect("Formel-Objekt (Kind::Math)");
        assert_eq!(m.src, "a=1");
        assert_eq!(it.color, 0xFF223344, "Farbe der Striche");
        // gleiche Stelle, ähnliche Höhe
        let nb = it.bounds();
        assert!((nb.left - bb.left).abs() < 1.0 && (nb.center_y() - bb.center_y()).abs() < 2.0, "{nb:?} {bb:?}");
        assert!(nb.height() <= bb.height() * 1.05 && nb.height() >= bb.height() * 0.4, "{nb:?} {bb:?}");
        assert!(rest.iter().all(|r| b.items.iter().any(|it| it.id == *r)), "andere Striche bleiben");
        // Rückgängig stellt die Striche wieder her
        b.undo();
        assert_eq!(b.items.len(), n_items);
        assert!(zeile.iter().all(|z| b.items.iter().any(|it| it.id == *z)));
        // Text
        let id = b.replace_ink(&zeile, &Ersatz::Text("Hallo".into())).unwrap();
        let it = b.items.iter().find(|it| it.id == id).unwrap();
        assert_eq!(it.as_note().unwrap().text, "Hallo");
        assert_eq!(it.as_note().unwrap().bg, 0, "freier Text, keine Haftnotiz");
        let nb = it.bounds();
        assert!((nb.center_y() - bb.center_y()).abs() < 3.0 && nb.height() <= bb.height() * 1.2, "{nb:?} {bb:?}");
        // leerer Text oder unbekannte Striche: nichts
        assert!(b.replace_ink(&rest, &Ersatz::Text("  ".into())).is_none());
        assert!(b.replace_ink(&[987654], &Ersatz::Text("x".into())).is_none());
        // Rechnen sieht die neue Formel
        b.undo();
        let id = b.replace_ink(&zeile, &Ersatz::Formel(crate::calc::to_latex("a: 4"))).unwrap();
        b.add_math(100.0, 160.0, 20.0, &crate::calc::to_latex("a ="));
        let res = b.calc_results();
        assert!(res.iter().any(|(_, o)| o.text() == "4"), "{res:?}");
        assert!(b.items.iter().any(|it| it.id == id));
    }
}
