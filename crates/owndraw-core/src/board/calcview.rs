//! Werkzeug „Rechnen“ auf dem Board: Formel-Objekte einer Seite ausrechnen und Ergebnisse zeichnen.
//!
//! Ergebnisse werden nicht gespeichert, sondern bei jeder Änderung der Formeln neu berechnet (Zwischenspeicher
//! über eine Prüfsumme). Sie erscheinen in eigener Farbe hinter dem „=“, Fehler rot, Rechenweg klein darunter.

use super::*;
use crate::calc::{self, Out};
use crate::item::{Math, MATH_BASE};
use crate::math::{layout, Formula};
use std::hash::{Hash, Hasher};

/// Farbe berechneter Werte (hell, dunkel).
const CALC: (u32, u32) = (0xFF1F6FEB, 0xFF79B0FF);

/// Gezeichnetes Ergebnis einer Formel-Zeile.
pub struct CalcView {
    /// Linke Seite neu gesetzt (bei Wunsch-Einheit/Platzhalter), sonst None = Formel wie geschrieben.
    left: Option<Formula>,
    /// Zwischenschritte in der Zeile (Rechenweg), je mit „=“ dahinter.
    chain: Vec<Formula>,
    value: Option<Formula>,
    unit: Option<Formula>,
    err: bool,
    /// Rechenweg und Meldungen darunter (Formel, Fehlerfarbe).
    below: Vec<(Formula, bool)>,
}

pub type CalcViews = Rc<HashMap<u64, CalcView>>;

fn text_tex(s: &str) -> String {
    let esc: String = s.chars().map(|c| if "{}\\%$&#_".contains(c) { format!("\\{c}") } else { c.to_string() }).collect();
    format!("\\text{{{esc}}}")
}

impl Board {
    /// Prüfsumme über alles, was Ergebnisse ändert (Formeln, Lage, Seiten, Einstellungen). 0 = Rechnen aus.
    pub(super) fn calc_sig(&self) -> u64 {
        if !self.calc.on {
            return 0;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let o = &self.calc;
        (o.deg, o.digits, o.comma, o.steps).hash(&mut h);
        self.calc_pre.hash(&mut h);
        let mut any = false;
        for it in &self.items {
            if let Kind::Math(m) = &it.kind {
                any = true;
                (it.id, &m.src, m.x.to_bits(), m.y.to_bits(), self.layer_shown(it.layer)).hash(&mut h);
            }
        }
        if !any {
            return 0;
        }
        self.pages().map(|p| p.pages.len()).hash(&mut h);
        h.finish() | 1
    }

    fn layer_shown(&self, id: i32) -> bool {
        self.layers.iter().find(|l| l.id == id).is_none_or(|l| l.visible)
    }

    /// Ergebnisse je Formel-id (zwischengespeichert).
    pub fn calc_views(&self) -> CalcViews {
        let sig = self.calc_sig();
        if let Some((s, v)) = self.calc_cache.borrow().as_ref() {
            if *s == sig {
                return v.clone();
            }
        }
        let v = Rc::new(if sig == 0 { HashMap::new() } else { self.compute_calc() });
        *self.calc_cache.borrow_mut() = Some((sig, v.clone()));
        v
    }

    /// Ergebnisse als Text je Formel-id (Tests, Barrierefreiheit).
    pub fn calc_results(&self) -> Vec<(u64, Out)> {
        self.calc_lines().into_iter().filter_map(|(id, o)| o.map(|o| (id, o))).collect()
    }

    /// Bekannte und gefragte Größen der Seite am Weltpunkt (für Formel-Vorschläge im Rechnen-Werkzeug).
    pub fn calc_names_at(&self, wx: f32, wy: f32) -> (Vec<String>, Vec<String>) {
        let want = self.pages().and_then(|p| p.nearest_page(wx, wy));
        let mut g: Vec<(f32, f32, &str)> = Vec::new();
        for it in &self.items {
            let Kind::Math(m) = &it.kind else { continue };
            if !self.layer_shown(it.layer) || self.pages().and_then(|p| p.page_of(it)) != want {
                continue;
            }
            g.push((m.y + m.h() / 2.0, m.x, &m.src));
        }
        g.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)));
        let srcs: Vec<&str> = g.iter().map(|x| x.2).collect();
        calc::page_names(&srcs, &self.calc, &self.calc_pre)
    }

    fn calc_lines(&self) -> Vec<(u64, Option<Out>)> {
        // Formeln je Seite in Lesereihenfolge (oben nach unten, bei gleicher Höhe links nach rechts)
        let mut groups: HashMap<Option<usize>, Vec<(f32, f32, u64, &str)>> = HashMap::new();
        for it in &self.items {
            let Kind::Math(m) = &it.kind else { continue };
            if !self.layer_shown(it.layer) {
                continue;
            }
            let page = self.pages().and_then(|p| p.page_of(it));
            let cy = m.y + m.h() / 2.0;
            groups.entry(page).or_default().push((cy, m.x, it.id, &m.src));
        }
        let mut out = Vec::new();
        for (_, mut g) in groups {
            // Zeilen: gleiche Höhe = Mitte innerhalb halber Zeilenhöhe
            g.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)));
            let srcs: Vec<&str> = g.iter().map(|x| x.3).collect();
            let res = calc::page_with(&srcs, &self.calc, &self.calc_pre);
            out.extend(g.iter().map(|x| x.2).zip(res));
        }
        out
    }

    fn compute_calc(&self) -> HashMap<u64, CalcView> {
        let mut map = HashMap::new();
        for (id, o) in self.calc_lines() {
            let Some(o) = o else { continue };
            let v = match o {
                Out::Result { left, value, unit, steps, err } => {
                    let it = self.items.iter().find(|it| it.id == id);
                    let src_ok = it.and_then(|it| it.as_math()).is_some_and(|m| m.src.trim_end().ends_with('=') && unit.is_empty());
                    let chain = steps.iter().filter(|s| !s.starts_with("\\text{")).map(|s| layout(&format!("{s} ="), MATH_BASE)).collect();
                    let mut below: Vec<(Formula, bool)> = steps.iter().filter(|s| s.starts_with("\\text{")).map(|s| (layout(s, MATH_BASE * 0.55), false)).collect();
                    if let Some(e) = &err {
                        below.push((layout(&text_tex(e), MATH_BASE * 0.6), true));
                    }
                    CalcView {
                        left: (!src_ok && !left.is_empty()).then(|| layout(&left, MATH_BASE)),
                        chain,
                        value: Some(layout(&value, MATH_BASE)),
                        unit: (!unit.is_empty()).then(|| layout(&unit, MATH_BASE)),
                        err: err.is_some(),
                        below,
                    }
                }
                Out::Error(e) => CalcView { left: None, chain: Vec::new(), value: None, unit: None, err: true, below: vec![(layout(&text_tex(&e), MATH_BASE * 0.6), true)] },
            };
            map.insert(id, v);
        }
        map
    }

    /// Fläche, die eine Formel samt Ergebnis einnimmt (Export), ohne Drehung.
    pub(super) fn calc_extent(&self, m: &Math, v: &CalcView) -> RectF {
        let k = m.size / MATH_BASE;
        let main = v.left.as_ref().unwrap_or(m.formula());
        let gap = MATH_BASE * 0.12;
        let mut w = main.w;
        let mut h = main.h;
        for f in v.chain.iter().chain([v.value.as_ref(), v.unit.as_ref()].into_iter().flatten()) {
            w += f.w + gap * 0.3;
            h = h.max(main.asc - f.asc + f.h);
        }
        for (f, _) in &v.below {
            w = w.max(MATH_BASE * 0.2 + f.w);
            h += f.h;
        }
        RectF::new(m.x, m.y, m.x + w * k, m.y + h * k)
    }

    /// Zeichnet die Formel `m` samt Ergebnis (statt `Item::draw`).
    pub(super) fn draw_calc(&self, it: &Item, m: &Math, v: &CalcView, c: &mut Canvas) {
        let ink = item::ink(it.color, self.dark);
        let calc_col = if self.dark { CALC.1 } else { CALC.0 };
        let k = m.size / MATH_BASE;
        c.save();
        if m.ang != 0.0 {
            c.rotate(m.ang.to_degrees(), m.x + m.w() / 2.0, m.y + m.h() / 2.0);
        }
        c.translate(m.x, m.y);
        c.scale(k, k);
        let main = v.left.as_ref().unwrap_or(m.formula());
        main.draw(c, ink);
        let gap = MATH_BASE * 0.12;
        let mut x = main.w;
        let mut bottom = main.h;
        for f in &v.chain {
            let y = main.asc - f.asc;
            c.save();
            c.translate(x + gap * 0.3, y);
            f.draw(c, calc_col);
            c.restore();
            x += f.w + gap * 0.3;
            bottom = bottom.max(y + f.h);
        }
        for (f, col, tint) in [(v.value.as_ref(), if v.err { calc::ERR } else { calc_col }, true), (v.unit.as_ref(), ink, false)] {
            let Some(f) = f else { continue };
            let y = main.asc - f.asc;
            if tint {
                let a = (if self.dark { 0x30 } else { 0x1C }) << 24;
                c.draw_rect(x + gap * 0.3, y + f.h * 0.08, x + gap * 0.3 + f.w, y + f.h * 0.92, &Paint::fill((col & 0x00FF_FFFF) | a));
            }
            c.save();
            c.translate(x + gap * 0.3, y);
            f.draw(c, col);
            c.restore();
            x += f.w + gap * 0.3;
            bottom = bottom.max(y + f.h);
        }
        let mut y = bottom;
        for (f, e) in &v.below {
            c.save();
            c.translate(MATH_BASE * 0.2, y);
            f.draw(c, if *e { calc::ERR } else { calc_col });
            c.restore();
            y += f.h;
        }
        c.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rechnen_auf_dem_blatt() {
        let mut b = Board::new(1.0);
        b.set_size(800.0, 600.0, 1.0);
        // absichtlich nicht in Lesereihenfolge angelegt
        let q = b.add_math(20.0, 200.0, 24.0, &calc::to_latex("M = kN·m")).unwrap();
        b.add_math(20.0, 20.0, 24.0, &calc::to_latex("a: 1 N")).unwrap();
        b.add_math(20.0, 80.0, 24.0, &calc::to_latex("b = 2 m")).unwrap();
        b.add_math(20.0, 140.0, 24.0, &calc::to_latex("M = a·b")).unwrap();
        let e = b.add_math(20.0, 260.0, 24.0, &calc::to_latex("M =")).unwrap();
        let r: HashMap<u64, String> = b.calc_results().into_iter().map(|(id, o)| (id, o.text())).collect();
        assert_eq!(r[&q], "0{,}002 \\mathrm{kN}\\cdot \\mathrm{m}");
        assert_eq!(r[&e], "2\\,\\mathrm{N}\\cdot \\mathrm{m}");
        assert_eq!(b.calc_views().len(), 2);
        // Wert ändern → neu gerechnet
        let s0 = b.calc_sig();
        let a = b.items.iter().find(|it| it.as_math().is_some_and(|m| m.src.starts_with("a:"))).unwrap().id;
        b.set_math_src(a, &calc::to_latex("a: 3 N"));
        assert_ne!(b.calc_sig(), s0);
        assert!(b.calc_results().iter().any(|(id, o)| *id == e && o.text().starts_with("6")));
        // Zeichnen und Export ohne Panik
        b.calc.steps = true;
        let mut pm = Pixmap::new(800, 600).unwrap();
        b.render(&mut pm);
        assert!(b.export_png().is_some());
        if let Ok(p) = std::env::var("OWNDRAW_CALC_PNG") {
            std::fs::write(p, b.export_png().unwrap()).unwrap();
        }
        // ausgeschaltet: keine Ergebnisse
        b.calc.on = false;
        assert!(b.calc_views().is_empty());
    }
}
