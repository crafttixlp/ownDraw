//! Satz: Absätze in Zeilen umbrechen (Wortumbruch, Ausrichtung inkl. Blocksatz, Tabulatoren, Listen-Marken) und
//! auf Seiten verteilen. Alle Maße in pt, Koordinaten relativ zur linken oberen Ecke der jeweiligen Seite.
//! Dieselbe Berechnung nutzen Editor (Anzeige, Cursor, Treffer), PDF-Export und Vorschaubild.

use super::fonts::{self, Family, FontKey};
use super::model::{Align, CStyle, Kind, Para, PageSpec, Pos, TextDoc, VAlign};

/// Grundfarbe des Textes.
pub const INK: u32 = 0xFF1D1D24;
/// Farbe von Links.
pub const LINK: u32 = 0xFF1A56DB;
/// Tabulator-Raster in pt (1,27 cm).
pub const TAB: f32 = 36.0;
/// Einzug je Stufe in pt.
pub const INDENT: f32 = 24.0;
/// Platz für Listen-Marken.
pub const LIST_GAP: f32 = 20.0;
/// Vorgabe-Zeilenabstand.
pub const LINE: f32 = 1.15;

/// Absatzvorlage aufgelöst.
#[derive(Clone, Copy, Debug)]
pub struct PStyle {
    pub fam: Family,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub before: f32,
    pub after: f32,
    pub color: u32,
}

/// Vorgaben der Absatzvorlagen.
pub fn para_style(doc: &TextDoc, k: Kind) -> PStyle {
    let body = PStyle { fam: doc.family, size: doc.size, bold: false, italic: false, before: 0.0, after: 8.0, color: INK };
    match k {
        Kind::Body => body,
        Kind::Title => PStyle { fam: Family::Sans, size: 26.0, bold: true, after: 14.0, ..body },
        Kind::H1 => PStyle { fam: Family::Sans, size: 18.0, bold: true, before: 18.0, after: 6.0, ..body },
        Kind::H2 => PStyle { fam: Family::Sans, size: 14.0, bold: true, before: 14.0, after: 4.0, ..body },
        Kind::H3 => PStyle { fam: Family::Sans, size: 12.0, bold: true, before: 10.0, after: 3.0, ..body },
        Kind::Quote => PStyle { italic: true, before: 4.0, color: 0xFF4A4A55, ..body },
        Kind::Bullet | Kind::Number => PStyle { after: 3.0, ..body },
        Kind::PageBreak | Kind::Image | Kind::Table | Kind::Object => PStyle { after: 0.0, ..body },
        Kind::Caption => PStyle { fam: Family::Sans, size: (doc.size - 1.5).max(6.0), italic: true, before: 2.0, after: 10.0, color: 0xFF4A4A55, ..body },
    }
}

/// Zeichenformat aufgelöst.
#[derive(Clone, Debug, PartialEq)]
pub struct RStyle {
    pub font: FontKey,
    /// Schriftgröße (bei Hoch-/Tiefstellung verkleinert).
    pub size: f32,
    /// Grundlinien-Versatz (negativ = höher).
    pub dy: f32,
    pub color: u32,
    pub u: bool,
    pub s: bool,
    pub hl: u32,
    pub link: bool,
}

pub fn resolve(ps: &PStyle, st: &CStyle) -> RStyle {
    let size = st.size.unwrap_or(ps.size);
    let (size_eff, dy) = match st.va {
        VAlign::Base => (size, 0.0),
        VAlign::Sup => (size * 0.65, -size * 0.33),
        VAlign::Sub => (size * 0.65, size * 0.14),
    };
    let link = st.link.is_some();
    RStyle {
        font: FontKey { fam: st.fam.unwrap_or(ps.fam), bold: st.b || ps.bold, italic: st.i || ps.italic },
        size: size_eff,
        dy,
        color: if st.color != 0 {
            st.color
        } else if link {
            LINK
        } else {
            ps.color
        },
        u: st.u || link,
        s: st.s,
        hl: st.hl,
        link,
    }
}

/// Zeichenfolge gleichen Formats in einer Zeile. `words` = sichtbare Wörter mit x (Leerzeichen/Tabs fehlen).
#[derive(Clone, Debug)]
pub struct Seg {
    pub x0: f32,
    pub x1: f32,
    pub st: RStyle,
    pub words: Vec<(f32, String)>,
}

/// Eine gesetzte Zeile.
#[derive(Clone, Debug)]
pub struct Line {
    pub para: usize,
    /// Zeichenbereich im Absatz.
    pub start: usize,
    pub end: usize,
    pub page: usize,
    /// Oberkante und Höhe in pt.
    pub y: f32,
    pub h: f32,
    /// Grundlinie relativ zu `y`.
    pub base: f32,
    /// x jeder Zeichengrenze (Länge = end - start + 1).
    pub xs: Vec<f32>,
    pub segs: Vec<Seg>,
    /// Listen-Marke (nur erste Zeile).
    pub marker: Option<Seg>,
    pub first: bool,
    pub last: bool,
    /// Zeile eines Seitenumbruchs.
    pub pagebreak: bool,
    /// Bild-Zeile: (Datei, x, Breite, Höhe); Oberkante `y`.
    pub img: Option<(String, f32, f32, f32)>,
}

/// Ergebnis des Satzes.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub lines: Vec<Line>,
    pub pages: usize,
    pub page: PageSpec,
    /// Je Absatz: erste und letzte Zeile.
    pub para_lines: Vec<(usize, usize)>,
}

fn roman(mut n: u32) -> String {
    let t = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
    let mut s = String::new();
    for (v, r) in t {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

fn letters(mut n: u32) -> String {
    let mut s = Vec::new();
    while n > 0 {
        n -= 1;
        s.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    s.iter().rev().collect()
}

/// Text der Listen-Marke.
pub fn marker_text(kind: Kind, level: u8, n: u32) -> String {
    match kind {
        Kind::Bullet => ["•", "◦", "▪"][level as usize % 3].into(),
        Kind::Number => match level % 3 {
            0 => format!("{n}."),
            1 => format!("{})", letters(n)),
            _ => format!("{}.", roman(n)),
        },
        _ => String::new(),
    }
}

/// Linker Einzug eines Absatzes (ab linkem Rand).
pub fn indent_of(p: &Para) -> f32 {
    p.indent as f32 * INDENT
        + match p.kind {
            Kind::Bullet | Kind::Number => LIST_GAP,
            Kind::Quote => 18.0,
            _ => 0.0,
        }
}

fn tab_w(x: f32) -> f32 {
    let next = ((x + 0.01) / TAB).floor() * TAB + TAB;
    (next - x).max(2.0)
}

/// Setzt das ganze Dokument.
pub fn layout(doc: &TextDoc) -> Layout {
    let pg = doc.page;
    let top = pg.mt;
    let bottom = (pg.h - pg.mb).max(top + 24.0);
    let (mut page, mut y) = (0usize, top);
    let mut lines: Vec<Line> = Vec::new();
    let mut para_lines = Vec::with_capacity(doc.paras.len());
    let mut counters = [0u32; 9];
    let mut prev_list = false;
    for (pi, p) in doc.paras.iter().enumerate() {
        let ps = para_style(doc, p.kind);
        let first_idx = lines.len();
        // Nummerierung
        let lvl = p.indent.min(8) as usize;
        let marker = match p.kind {
            Kind::Number | Kind::Bullet => {
                if !prev_list {
                    counters = [0; 9];
                }
                if p.kind == Kind::Number {
                    counters[lvl] += 1;
                    for c in counters.iter_mut().skip(lvl + 1) {
                        *c = 0;
                    }
                }
                Some(marker_text(p.kind, p.indent, counters[lvl]))
            }
            _ => None,
        };
        prev_list = p.kind.is_list();
        let ls = if p.line > 0.0 { p.line } else { LINE };
        if p.kind == Kind::PageBreak {
            let rs = resolve(&ps, &CStyle::default());
            let (a, d) = fonts::metrics(rs.font, rs.size);
            let h = (a + d) * ls;
            if y + h > bottom && y > top + 0.01 {
                page += 1;
                y = top;
            }
            lines.push(Line { para: pi, start: 0, end: 0, page, y, h, base: a, xs: vec![pg.ml], segs: Vec::new(), marker: None, first: true, last: true, pagebreak: true, img: None });
            para_lines.push((first_idx, first_idx));
            page += 1;
            y = top;
            continue;
        }
        if let (Kind::Image, Some(im)) = (p.kind, &p.img) {
            // Bild: höchstens Inhaltsbreite/-höhe, Seitenverhältnis bleibt
            let maxh = (bottom - top).max(24.0);
            let k = (pg.content_w() / im.w.max(1.0)).min(maxh / im.h.max(1.0)).min(1.0);
            let (w, h) = (im.w * k, im.h * k);
            if y > top + 0.01 {
                y += 4.0;
            }
            if y + h > bottom && y > top + 0.01 {
                page += 1;
                y = top;
            }
            let free = (pg.content_w() - w).max(0.0);
            let x = pg.ml + match p.align {
                Align::Center => free / 2.0,
                Align::Right => free,
                _ => 0.0,
            };
            lines.push(Line { para: pi, start: 0, end: 0, page, y, h, base: h, xs: vec![x + w], segs: Vec::new(), marker: None, first: true, last: true, pagebreak: false, img: Some((im.file.clone(), x, w, h)) });
            para_lines.push((first_idx, first_idx));
            y += h + 6.0;
            prev_list = false;
            continue;
        }
        let chars = p.chars();
        let styles: Vec<RStyle> = p.runs.iter().map(|r| resolve(&ps, &r.st)).collect();
        let adv: Vec<f32> = chars.iter().map(|(c, ri)| if *c == '\t' { 0.0 } else { fonts::advance(styles[*ri].font, *c, styles[*ri].size) }).collect();
        let indent = indent_of(p);
        let x0 = pg.ml + indent;
        let maxw = (pg.content_w() - indent).max(24.0);
        let n = chars.len();
        // Umbruch
        let mut breaks: Vec<(usize, usize)> = Vec::new();
        let mut s = 0;
        loop {
            let (mut w, mut i, mut brk) = (0.0f32, s, None);
            while i < n {
                let c = chars[i].0;
                let cw = if c == '\t' { tab_w(w) } else { adv[i] };
                if w + cw > maxw && i > s && c != ' ' {
                    break;
                }
                w += cw;
                i += 1;
                if c == ' ' || c == '-' || c == '\t' || c == '/' {
                    brk = Some(i);
                }
            }
            let e = if i < n { brk.filter(|b| *b > s).unwrap_or(i) } else { n };
            breaks.push((s, e));
            s = e;
            if s >= n {
                break;
            }
        }
        let nl = breaks.len();
        for (li, (s, e)) in breaks.into_iter().enumerate() {
            // Höhe aus den größten Zeichen der Zeile
            let (mut a, mut d) = (0.0f32, 0.0f32);
            let mut probe = |rs: &RStyle| {
                let full = if rs.dy != 0.0 { rs.size / 0.65 } else { rs.size };
                let (fa, fd) = fonts::metrics(rs.font, full);
                a = a.max(fa);
                d = d.max(fd);
            };
            if s == e {
                probe(&resolve(&ps, &p.style_at(s)));
            } else {
                let mut last = usize::MAX;
                for (_, ri) in &chars[s..e] {
                    if *ri != last {
                        probe(&styles[*ri]);
                        last = *ri;
                    }
                }
            }
            let h = (a + d) * ls;
            let base = a + (h - a - d) * 0.3;
            if li == 0 && y > top + 0.01 {
                y += ps.before;
            }
            if y + h > bottom && y > top + 0.01 {
                page += 1;
                y = top;
            }
            // x-Positionen
            let mut rel = Vec::with_capacity(e - s + 1);
            let mut w = 0.0f32;
            rel.push(0.0);
            for i in s..e {
                w += if chars[i].0 == '\t' { tab_w(w) } else { adv[i] };
                rel.push(w);
            }
            let mut trim = e;
            while trim > s && chars[trim - 1].0 == ' ' {
                trim -= 1;
            }
            let used = rel[trim - s];
            let last = li + 1 == nl;
            let free = (maxw - used).max(0.0);
            let (off, extra) = match p.align {
                Align::Left => (0.0, 0.0),
                Align::Center => (free / 2.0, 0.0),
                Align::Right => (free, 0.0),
                Align::Justify => {
                    let spaces = chars[s..trim].iter().filter(|(c, _)| *c == ' ').count();
                    if last || spaces == 0 {
                        (0.0, 0.0)
                    } else {
                        (0.0, free / spaces as f32)
                    }
                }
            };
            let mut xs = Vec::with_capacity(rel.len());
            let mut add = 0.0;
            xs.push(x0 + off);
            for i in s..e {
                if extra > 0.0 && i < trim && chars[i].0 == ' ' {
                    add += extra;
                }
                xs.push(x0 + off + rel[i - s + 1] + add);
            }
            // Abschnitte gleichen Formats und Wörter
            let mut segs: Vec<Seg> = Vec::new();
            let mut i = s;
            while i < e {
                let ri = chars[i].1;
                let mut j = i;
                while j < e && chars[j].1 == ri {
                    j += 1;
                }
                let mut words = Vec::new();
                let mut k = i;
                while k < j {
                    if matches!(chars[k].0, ' ' | '\t') {
                        k += 1;
                        continue;
                    }
                    let st = k;
                    while k < j && !matches!(chars[k].0, ' ' | '\t') {
                        k += 1;
                    }
                    words.push((xs[st - s], chars[st..k].iter().map(|(c, _)| *c).collect()));
                }
                let x1 = if j == e { xs[trim.max(i) - s].max(xs[i - s]) } else { xs[j - s] };
                segs.push(Seg { x0: xs[i - s], x1, st: styles[ri].clone(), words });
                i = j;
            }
            let marker = (li == 0).then(|| marker.clone()).flatten().filter(|m| !m.is_empty()).map(|m| {
                let rs = resolve(&ps, &p.style_at(0));
                let rs = RStyle { u: false, s: false, hl: 0, link: false, dy: 0.0, color: if rs.link { ps.color } else { rs.color }, ..rs };
                let mw: f32 = m.chars().map(|c| fonts::advance(rs.font, c, rs.size)).sum();
                // Marke links im Listen-Einzug; breite Nummern ragen nach links hinaus
                let mx = if mw <= LIST_GAP - 4.0 { x0 - LIST_GAP } else { x0 - mw - 4.0 };
                Seg { x0: mx, x1: mx + mw, st: rs, words: vec![(mx, m)] }
            });
            lines.push(Line { para: pi, start: s, end: e, page, y, h, base, xs, segs, marker, first: li == 0, last, pagebreak: false, img: None });
            y += h;
        }
        y += ps.after;
        para_lines.push((first_idx, lines.len() - 1));
    }
    Layout { lines, pages: page + 1, page: pg, para_lines }
}

impl Layout {
    /// Zeile, in der `pos` steht (am weichen Umbruch die folgende Zeile).
    pub fn line_of(&self, pos: Pos) -> usize {
        let Some(&(a, b)) = self.para_lines.get(pos.p) else { return self.lines.len().saturating_sub(1) };
        let mut li = a;
        for i in a..=b {
            if self.lines[i].start <= pos.o {
                li = i;
            }
        }
        li
    }

    /// Cursor: (Seite, x, Oberkante, Höhe).
    pub fn caret(&self, pos: Pos) -> (usize, f32, f32, f32) {
        let Some(l) = self.lines.get(self.line_of(pos)) else { return (0, self.page.ml, self.page.mt, 14.0) };
        let i = pos.o.clamp(l.start, l.end) - l.start;
        (l.page, l.xs.get(i).copied().unwrap_or(l.xs[0]), l.y, l.h)
    }

    /// Position in Zeile `li` an x.
    pub fn pos_in_line(&self, li: usize, x: f32) -> Pos {
        let l = &self.lines[li];
        let mut best = 0;
        let mut bd = f32::MAX;
        for (i, xv) in l.xs.iter().enumerate() {
            let d = (xv - x).abs();
            if d < bd {
                bd = d;
                best = i;
            }
        }
        let mut o = l.start + best;
        // am weichen Umbruch vor dem Leerzeichen bleiben, sonst springt der Cursor in die nächste Zeile
        if !l.last && o == l.end && l.end > l.start {
            o -= 1;
        }
        Pos::new(l.para, o)
    }

    /// Treffer auf Seite `page` bei (x, y).
    pub fn hit(&self, page: usize, x: f32, y: f32) -> Pos {
        let mut best: Option<(usize, f32)> = None;
        for (i, l) in self.lines.iter().enumerate() {
            if l.page != page {
                continue;
            }
            let d = if y < l.y {
                l.y - y
            } else if y > l.y + l.h {
                y - l.y - l.h
            } else {
                0.0
            };
            if best.is_none_or(|(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        match best {
            Some((i, _)) => self.pos_in_line(i, x),
            None => {
                // leere Seite (nur möglich hinter einem Seitenumbruch): nächste Zeile davor/danach
                let i = self.lines.iter().position(|l| l.page > page).unwrap_or(self.lines.len().saturating_sub(1));
                self.pos_in_line(i, x)
            }
        }
    }

    /// Eine Zeile nach oben (`dir` = -1) oder unten (+1) bei x; None am Anfang/Ende.
    pub fn vertical(&self, pos: Pos, x: f32, dir: i32) -> Option<Pos> {
        let li = self.line_of(pos) as i64 + dir as i64;
        (li >= 0 && (li as usize) < self.lines.len()).then(|| self.pos_in_line(li as usize, x))
    }

    /// Anfang und Ende der Zeile von `pos` (Pos1/Ende).
    pub fn line_bounds(&self, pos: Pos) -> (Pos, Pos) {
        let l = &self.lines[self.line_of(pos)];
        let mut e = l.end;
        if !l.last && e > l.start {
            e -= 1;
        }
        (Pos::new(l.para, l.start), Pos::new(l.para, e))
    }

    /// Markierungsrechtecke (Seite, x0, y, x1, h) für a..b.
    pub fn sel_rects(&self, a: Pos, b: Pos) -> Vec<(usize, f32, f32, f32, f32)> {
        let (a, b) = super::model::ordered(a, b);
        let mut out = Vec::new();
        if a == b {
            return out;
        }
        for p in a.p..=b.p.min(self.para_lines.len().saturating_sub(1)) {
            let (f, t) = self.para_lines[p];
            for l in &self.lines[f..=t] {
                let s = if p == a.p { a.o.max(l.start) } else { l.start };
                let e = if p == b.p { b.o.min(l.end) } else { l.end };
                if s > e || (s == e && !(l.last && p < b.p)) {
                    continue;
                }
                let x0 = l.xs[s - l.start];
                let mut x1 = l.xs[e - l.start];
                // Absatzende mitmarkiert: kleiner Überstand
                if l.last && p < b.p && e == l.end {
                    x1 += 5.0;
                }
                out.push((l.page, x0, l.y, x1, l.h));
            }
        }
        out
    }

    /// Seite, auf der Absatz `p` beginnt.
    pub fn page_of_para(&self, p: usize) -> usize {
        self.para_lines.get(p).and_then(|(f, _)| self.lines.get(*f)).map(|l| l.page).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::office::model::{CStyle, Para, TextDoc};

    fn doc(text: &str) -> TextDoc {
        let mut d = TextDoc::new();
        d.insert_text(Pos::default(), text, &CStyle::default());
        d
    }

    #[test]
    fn umbruch_innerhalb_der_breite() {
        let d = doc(&"Wort ".repeat(200));
        let l = layout(&d);
        assert!(l.lines.len() > 5);
        let right = d.page.w - d.page.mr;
        for ln in &l.lines {
            let mut t = ln.end;
            let ch: Vec<char> = d.paras[0].text().chars().collect();
            while t > ln.start && ch[t - 1] == ' ' {
                t -= 1;
            }
            assert!(ln.xs[t - ln.start] <= right + 0.01);
            assert_eq!(ln.xs.len(), ln.end - ln.start + 1);
        }
        // Zeilen schließen lückenlos aneinander an
        for w in l.lines.windows(2) {
            assert_eq!(w[0].end, w[1].start);
        }
    }

    #[test]
    fn seiten_und_umbruch() {
        let mut d = doc(&"Absatz mit etwas Text.\n".repeat(120));
        let l = layout(&d);
        assert!(l.pages >= 2, "{}", l.pages);
        for ln in &l.lines {
            assert!(ln.y + ln.h <= d.page.h - d.page.mb + 0.01);
        }
        d.paras.insert(1, Para::new(Kind::PageBreak));
        let l2 = layout(&d);
        assert_eq!(l2.lines[l2.para_lines[2].0].page, 1);
    }

    #[test]
    fn blocksatz_fuellt_zeile() {
        let mut d = doc(&"ein zwei drei vier fünf sechs sieben acht neun zehn ".repeat(8));
        d.paras[0].align = Align::Justify;
        let l = layout(&d);
        let ch: Vec<char> = d.paras[0].text().chars().collect();
        let ln = &l.lines[0];
        let mut t = ln.end;
        while ch[t - 1] == ' ' {
            t -= 1;
        }
        assert!((ln.xs[t - ln.start] - (d.page.w - d.page.mr)).abs() < 0.5);
    }

    #[test]
    fn cursor_und_treffer() {
        let d = doc(&"Hallo Welt ".repeat(40));
        let l = layout(&d);
        let p = Pos::new(0, 7);
        let (pg, x, y, h) = l.caret(p);
        assert_eq!(l.hit(pg, x + 0.5, y + h / 2.0), p);
        let down = l.vertical(p, x, 1).unwrap();
        assert!(down.o > p.o);
        assert_eq!(l.vertical(down, l.caret(down).1, -1).unwrap().p, 0);
        assert!(!l.sel_rects(Pos::new(0, 2), Pos::new(0, 200)).is_empty());
    }

    #[test]
    fn listen_marken() {
        assert_eq!(marker_text(Kind::Number, 0, 3), "3.");
        assert_eq!(marker_text(Kind::Number, 1, 28), "ab)");
        assert_eq!(marker_text(Kind::Number, 2, 4), "iv.");
        let mut d = doc("a\nb\nc");
        for p in &mut d.paras {
            p.kind = Kind::Number;
        }
        d.paras[1].indent = 1;
        let l = layout(&d);
        let m: Vec<String> = l.para_lines.iter().map(|(f, _)| l.lines[*f].marker.as_ref().unwrap().words[0].1.clone()).collect();
        assert_eq!(m, ["1.", "a)", "2."]);
    }
}
