//! Satz: Absätze in Zeilen umbrechen (Wortumbruch, Ausrichtung inkl. Blocksatz, Tabulatoren, Listen-Marken) und
//! auf Seiten verteilen. Alle Maße in pt, Koordinaten relativ zur linken oberen Ecke der jeweiligen Seite.
//! Dieselbe Berechnung nutzen Editor (Anzeige, Cursor, Treffer), PDF-Export und Vorschaubild.

use super::fields::{field_text, FieldCx, Numbers};
use super::fonts::{self, Family, FontKey};
use super::model::{Align, CStyle, Kind, Para, PageSpec, Pos, TextDoc, VAlign};
use super::objects::{Field, Wrap, OBJ};

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

/// Designs: (Name, Farbe der Überschriften, Schrift der Überschriften).
pub const THEMES: [(&str, u32, Family); 6] = [
    ("Standard", INK, Family::Sans),
    ("Blau", 0xFF1D4ED8, Family::Sans),
    ("Grün", 0xFF15803D, Family::Sans),
    ("Klassisch", 0xFF111111, Family::Serif),
    ("Violett", 0xFF6D28D9, Family::Sans),
    ("Warm", 0xFFB45309, Family::Serif),
];

/// Vorgaben der Absatzvorlagen.
pub fn para_style(doc: &TextDoc, k: Kind) -> PStyle {
    let body = PStyle { fam: doc.family, size: doc.size, bold: false, italic: false, before: 0.0, after: 8.0, color: INK };
    let (_, hc, hf) = THEMES[(doc.ex.theme as usize).min(THEMES.len() - 1)];
    let head = PStyle { fam: hf, color: hc, bold: true, ..body };
    match k {
        Kind::Body => body,
        Kind::Title => PStyle { size: 26.0, after: 14.0, ..head },
        Kind::H1 => PStyle { size: 18.0, before: 18.0, after: 6.0, ..head },
        Kind::H2 => PStyle { size: 14.0, before: 14.0, after: 4.0, ..head },
        Kind::H3 => PStyle { size: 12.0, before: 10.0, after: 3.0, ..head },
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

/// Farbe nachverfolgter Änderungen je Änderungs-ID.
pub fn rev_color(id: u32) -> u32 {
    [0xFFC2410C, 0xFF7C3AED, 0xFF0E7490, 0xFFBE185D, 0xFF4D7C0F][(id as usize) % 5]
}
/// Hintergrund kommentierter Stellen.
pub const CMT_BG: u32 = 0x66FCD34D;

pub fn resolve(ps: &PStyle, st: &CStyle) -> RStyle {
    let size = st.size.unwrap_or(ps.size);
    let (size_eff, dy) = match st.va {
        VAlign::Base => (size, 0.0),
        VAlign::Sup => (size * 0.65, -size * 0.33),
        VAlign::Sub => (size * 0.65, size * 0.14),
    };
    let link = st.link.is_some();
    let rev = st.ins.max(st.del);
    RStyle {
        font: FontKey { fam: st.fam.unwrap_or(ps.fam), bold: st.b || ps.bold, italic: st.i || ps.italic },
        size: size_eff,
        dy,
        color: if rev > 0 {
            rev_color(rev)
        } else if st.color != 0 {
            st.color
        } else if link {
            LINK
        } else {
            ps.color
        },
        u: st.u || link || st.ins > 0,
        s: st.s || st.del > 0,
        hl: if st.hl == 0 && st.cmt > 0 { CMT_BG } else { st.hl },
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
    /// Objekt-Zeile: (x, Breite, Höhe); Objekt in `doc.paras[para].obj`.
    pub obj: Option<(f32, f32, f32)>,
    /// Tabellenzeile.
    pub row: Option<RowBox>,
    /// Zeile endet mit Silbentrennung (Bindestrich zeichnen).
    pub hyph: bool,
}

impl Line {
    fn block(para: usize, page: usize, y: f32, h: f32, x: f32) -> Line {
        Line { para, start: 0, end: 0, page, y, h, base: h, xs: vec![x], segs: Vec::new(), marker: None, first: true, last: true, pagebreak: false, img: None, obj: None, row: None, hyph: false }
    }
    /// Linke und rechte Kante eines Blocks (Bild, Objekt, Tabelle).
    pub fn block_x(&self) -> Option<(f32, f32)> {
        if let Some((_, x, w, _)) = &self.img {
            return Some((*x, x + w));
        }
        if let Some((x, w, _)) = self.obj {
            return Some((x, x + w));
        }
        self.row.as_ref().map(|r| (r.cells.first().map(|c| c.x).unwrap_or(0.0), r.cells.last().map(|c| c.x + c.w).unwrap_or(0.0)))
    }
}

/// Gesetzte Tabellenzeile.
#[derive(Clone, Debug)]
pub struct RowBox {
    /// Zeile in der Tabelle.
    pub r: usize,
    /// Wiederholte Kopfzeile auf Folgeseite.
    pub repeat: bool,
    pub cells: Vec<CellBox>,
}

/// Gesetzte Zelle: Textzeilen (x, Grundlinie relativ zur Zeilen-Oberkante, Text).
#[derive(Clone, Debug)]
pub struct CellBox {
    pub x: f32,
    pub w: f32,
    pub bg: u32,
    pub font: FontKey,
    pub size: f32,
    pub color: u32,
    pub lines: Vec<(f32, f32, String)>,
}

/// Gesetzte Fußnote unten auf der Seite.
#[derive(Clone, Debug)]
pub struct NoteBox {
    pub page: usize,
    pub num: u32,
    /// Oberkante.
    pub y: f32,
    pub h: f32,
    pub x: f32,
    /// Zeilen (Grundlinie absolut, Text).
    pub lines: Vec<(f32, String)>,
    /// Position des Fußnotenzeichens im Text.
    pub pos: Pos,
}

/// Schriftgröße von Fußnoten und Tabellen-Zeilenabstand.
pub const NOTE_SIZE: f32 = 9.0;
const CELL_PAD: f32 = 4.0;
const COL_GAP: f32 = 18.0;

/// Ergebnis des Satzes.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub lines: Vec<Line>,
    pub pages: usize,
    pub page: PageSpec,
    /// Je Absatz: erste und letzte Zeile.
    pub para_lines: Vec<(usize, usize)>,
    pub notes: Vec<NoteBox>,
    /// Spalten: (Anzahl, Breite, Abstand).
    pub cols: (usize, f32, f32),
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

/// Bricht einfachen Text (Zellen, Fußnoten, Formen) in Zeilen der Breite `w`; `\n` erzwingt einen Umbruch.
pub fn wrap_simple(text: &str, font: FontKey, size: f32, w: f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        let mut lw = 0.0f32;
        for word in para.split(' ') {
            let ww: f32 = word.chars().map(|c| fonts::advance(font, c, size)).sum();
            let sp = if line.is_empty() { 0.0 } else { fonts::advance(font, ' ', size) };
            if !line.is_empty() && lw + sp + ww > w {
                out.push(std::mem::take(&mut line));
                lw = 0.0;
            } else if !line.is_empty() {
                line.push(' ');
                lw += sp;
            }
            // Überlange Wörter zeichenweise umbrechen
            for c in word.chars() {
                let cw = fonts::advance(font, c, size);
                if lw + cw > w && !line.is_empty() && lw > 0.0 && ww > w {
                    out.push(std::mem::take(&mut line));
                    lw = 0.0;
                }
                line.push(c);
                lw += cw;
            }
        }
        out.push(line);
    }
    out
}

/// Trennstellen eines Wortes (Zeichenindex, Trennung davor).
fn hyphen_points(word: &str) -> Vec<usize> {
    super::lang_hyph(word)
}

/// Freifläche neben umflossenen Objekten.
#[derive(Clone, Copy, Debug)]
struct Float {
    page: usize,
    col: usize,
    y0: f32,
    y1: f32,
    x0: f32,
    x1: f32,
    left: bool,
}

/// Fortschritt beim Setzen: Seite, Spalte, y.
struct Cur {
    page: usize,
    col: usize,
    y: f32,
    top: f32,
    ncol: usize,
}

impl Cur {
    fn next(&mut self) {
        self.col += 1;
        if self.col >= self.ncol {
            self.col = 0;
            self.page += 1;
        }
        self.y = self.top;
    }
    fn new_page(&mut self) {
        self.page += 1;
        self.col = 0;
        self.y = self.top;
    }
    fn fresh(&self) -> bool {
        self.y <= self.top + 0.01
    }
}

/// Setzt das ganze Dokument. Seitenabhängige Felder (Seitenzahl eines Verweises, Seitenanzahl) brauchen einen
/// zweiten Durchgang.
pub fn layout(doc: &TextDoc) -> Layout {
    let nums = Numbers::new(doc);
    let first = layout_pass(doc, &nums, None);
    let needs = doc.paras.iter().any(|p| p.runs.iter().any(|r| matches!(r.st.field, Some(Field::Pages | Field::Ref(..)))));
    if !needs {
        return first;
    }
    layout_pass(doc, &nums, Some(&first))
}

fn layout_pass(doc: &TextDoc, nums: &Numbers, prev: Option<&Layout>) -> Layout {
    let pg = doc.page;
    let top = pg.mt;
    let bottom0 = (pg.h - pg.mb).max(top + 24.0);
    let ncol = (doc.ex.cols as usize).clamp(1, 3);
    let col_w = ((pg.content_w() - COL_GAP * (ncol - 1) as f32) / ncol as f32).max(36.0);
    let col_x = |c: usize| pg.ml + c as f32 * (col_w + COL_GAP);
    let mut cur = Cur { page: 0, col: 0, y: top, top, ncol };
    let mut lines: Vec<Line> = Vec::new();
    let mut para_lines = Vec::with_capacity(doc.paras.len());
    let mut counters = [0u32; 9];
    let mut prev_list = false;
    let mut floats: Vec<Float> = Vec::new();
    // Je Seite: Höhe der Fußnoten
    let mut note_h: Vec<f32> = Vec::new();
    let mut notes: Vec<NoteBox> = Vec::new();
    let mut note_no = 0u32;
    let prev_pages = prev.map(|l| l.pages).unwrap_or(1);
    let para_page = |i: usize| prev.map(|l| l.page_of_para(i)).unwrap_or(0);
    let nfont = FontKey { fam: doc.family, bold: false, italic: false };
    let (na, nd) = fonts::metrics(nfont, NOTE_SIZE);
    let note_lh = (na + nd) * 1.1;
    let bottom_of = |note_h: &Vec<f32>, page: usize| bottom0 - note_h.get(page).copied().unwrap_or(0.0);
    // Freier Bereich (links, rechts) bei y..y+h in der aktuellen Spalte
    let avail = |floats: &Vec<Float>, cur: &Cur, y: f32, h: f32| {
        let (mut xl, mut xr) = (col_x(cur.col), col_x(cur.col) + col_w);
        for f in floats.iter().filter(|f| f.page == cur.page && f.col == cur.col && y < f.y1 && y + h > f.y0) {
            if f.left {
                xl = xl.max(f.x1 + 10.0);
            } else {
                xr = xr.min(f.x0 - 10.0);
            }
        }
        (xl, xr.max(xl + 24.0))
    };
    let clear = |floats: &Vec<Float>, cur: &mut Cur| {
        for f in floats.iter().filter(|f| f.page == cur.page && f.col == cur.col) {
            cur.y = cur.y.max(f.y1 + 4.0);
        }
    };
    for (pi, p) in doc.paras.iter().enumerate() {
        let ps = para_style(doc, p.kind);
        let first_idx = lines.len();
        // Nummerierung
        let lvl = p.indent.min(8) as usize;
        let mut marker = match p.kind {
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
            if cur.y + h > bottom_of(&note_h, cur.page) && !cur.fresh() {
                cur.new_page();
            }
            lines.push(Line { base: a, pagebreak: true, ..Line::block(pi, cur.page, cur.y, h, col_x(cur.col)) });
            para_lines.push((first_idx, first_idx));
            cur.new_page();
            continue;
        }
        // Bilder und Objekte
        let block = match (p.kind, &p.img, &p.obj) {
            (Kind::Image, Some(im), _) => Some((im.w, im.h)),
            (Kind::Object, _, Some(o)) => Some((o.w, o.h)),
            _ => None,
        };
        if let Some((bw, bh)) = block {
            let maxh = (bottom0 - top).max(24.0);
            let k = (col_w / bw.max(1.0)).min(maxh / bh.max(1.0)).min(1.0);
            let (w, h) = (bw * k, bh * k);
            let float = p.wrap != Wrap::Inline && w < col_w * 0.8;
            if !float {
                clear(&floats, &mut cur);
            }
            if !cur.fresh() && !float {
                cur.y += 4.0;
            }
            if cur.y + h > bottom_of(&note_h, cur.page) && !cur.fresh() {
                cur.next();
            }
            let (xl, xr) = if float { (col_x(cur.col), col_x(cur.col) + col_w) } else { avail(&floats, &cur, cur.y, h) };
            let free = (xr - xl - w).max(0.0);
            let x = xl + if float {
                if p.wrap == Wrap::Right { free } else { 0.0 }
            } else {
                match p.align {
                    Align::Center => free / 2.0,
                    Align::Right => free,
                    _ => 0.0,
                }
            };
            let mut l = Line::block(pi, cur.page, cur.y, h, x + w);
            if p.kind == Kind::Image {
                l.img = Some((p.img.as_ref().map(|i| i.file.clone()).unwrap_or_default(), x, w, h));
            } else {
                l.obj = Some((x, w, h));
            }
            lines.push(l);
            para_lines.push((first_idx, first_idx));
            if float {
                floats.push(Float { page: cur.page, col: cur.col, y0: cur.y, y1: cur.y + h, x0: x, x1: x + w, left: p.wrap == Wrap::Left });
            } else {
                cur.y += h + 6.0;
            }
            prev_list = false;
            continue;
        }
        // Tabellen
        if let (Kind::Table, Some(t)) = (p.kind, &p.table) {
            clear(&floats, &mut cur);
            if !cur.fresh() {
                cur.y += 4.0;
            }
            let total: f32 = t.cols.iter().sum();
            let k = (col_w / total.max(1.0)).min(1.0);
            let x0 = col_x(cur.col)
                + match p.align {
                    Align::Center => (col_w - total * k).max(0.0) / 2.0,
                    Align::Right => (col_w - total * k).max(0.0),
                    _ => 0.0,
                };
            let size = doc.size;
            let rows: Vec<(f32, Vec<CellBox>)> = t
                .rows
                .iter()
                .enumerate()
                .map(|(ri, row)| {
                    let mut x = x0;
                    let mut cells = Vec::new();
                    let mut rh = 0.0f32;
                    for (ci, c) in row.iter().enumerate() {
                        let w = t.cols.get(ci).copied().unwrap_or(40.0) * k;
                        let head = t.header && ri == 0;
                        let font = FontKey { fam: doc.family, bold: c.b || head, italic: c.i };
                        let (a, d) = fonts::metrics(font, size);
                        let lh = (a + d) * 1.15;
                        let tl = wrap_simple(&c.text, font, size, (w - 2.0 * CELL_PAD).max(8.0));
                        let mut out = Vec::new();
                        for (li, s) in tl.into_iter().enumerate() {
                            let tw: f32 = s.chars().map(|ch| fonts::advance(font, ch, size)).sum();
                            let free = (w - 2.0 * CELL_PAD - tw).max(0.0);
                            let dx = match c.align {
                                Align::Center => free / 2.0,
                                Align::Right => free,
                                _ => 0.0,
                            };
                            out.push((x + CELL_PAD + dx, CELL_PAD + li as f32 * lh + a + (lh - a - d) * 0.3, s));
                        }
                        rh = rh.max(out.len().max(1) as f32 * lh + 2.0 * CELL_PAD);
                        let bg = if c.bg != 0 {
                            c.bg
                        } else if head {
                            0xFFEEF0F4
                        } else if t.stripes && ri % 2 == 0 {
                            0xFFF6F7F9
                        } else {
                            0
                        };
                        cells.push(CellBox { x, w, bg, font, size, color: if c.color != 0 { c.color } else { INK }, lines: out });
                        x += w;
                    }
                    (rh, cells)
                })
                .collect();
            for (ri, (rh, cells)) in rows.iter().enumerate() {
                if cur.y + rh > bottom_of(&note_h, cur.page) && !cur.fresh() {
                    cur.next();
                    if t.header && ri > 0 {
                        let (hh, hc) = &rows[0];
                        lines.push(Line { row: Some(RowBox { r: 0, repeat: true, cells: hc.clone() }), ..Line::block(pi, cur.page, cur.y, *hh, x0) });
                        cur.y += hh;
                    }
                }
                lines.push(Line { row: Some(RowBox { r: ri, repeat: false, cells: cells.clone() }), ..Line::block(pi, cur.page, cur.y, *rh, x0) });
                cur.y += rh;
            }
            para_lines.push((first_idx, lines.len() - 1));
            cur.y += 8.0;
            prev_list = false;
            continue;
        }
        // Beschriftung: „Abbildung 2: “ vor dem Text
        if p.kind == Kind::Caption {
            marker = nums.caption_prefix(pi);
        }
        let caption = p.kind == Kind::Caption;
        let chars = p.chars();
        let styles: Vec<RStyle> = p
            .runs
            .iter()
            .map(|r| {
                let mut rs = resolve(&ps, &r.st);
                if matches!(r.st.field, Some(Field::Note(_))) && r.st.va == VAlign::Base {
                    let size = r.st.size.unwrap_or(ps.size);
                    rs.size = size * 0.65;
                    rs.dy = -size * 0.33;
                }
                if r.st.field.is_some() && r.st.hl == 0 && r.st.cmt == 0 && matches!(r.st.field, Some(Field::Merge(_) | Field::Choice(..) | Field::Ref(..))) {
                    rs.hl = 0xFFE8EDF7;
                }
                rs
            })
            .collect();
        // Felder: angezeigter Text je Platzhalter
        let page_now = cur.page;
        let disp: Vec<Option<String>> = chars
            .iter()
            .map(|(c, ri)| match (&p.runs[*ri].st.field, *c == OBJ) {
                (Some(f), true) => {
                    if let Field::Note(_) = f {
                        note_no += 1;
                    }
                    let cx = FieldCx { doc, nums, page: page_now, pages: prev_pages, para_page: &para_page, note: note_no };
                    Some(field_text(f, &cx))
                }
                _ => None,
            })
            .collect();
        let width_of = |s: &str, rs: &RStyle| s.chars().map(|c| fonts::advance(rs.font, c, rs.size)).sum::<f32>();
        let adv: Vec<f32> = chars
            .iter()
            .enumerate()
            .map(|(i, (c, ri))| match &disp[i] {
                Some(d) => width_of(d, &styles[*ri]),
                None if *c == '\t' => 0.0,
                None => fonts::advance(styles[*ri].font, *c, styles[*ri].size),
            })
            .collect();
        let indent = indent_of(p);
        let n = chars.len();
        let mstyle = {
            let rs = resolve(&ps, &p.style_at(0));
            RStyle { u: false, s: false, hl: 0, link: false, dy: 0.0, color: if rs.link { ps.color } else { rs.color }, ..rs }
        };
        let lead = if caption { marker.as_deref().map(|m| width_of(m, &mstyle)).unwrap_or(0.0) } else { 0.0 };
        let hy_w = fonts::advance(styles.first().map(|s| s.font).unwrap_or(nfont), '-', ps.size);
        // Höhe einer Zeile aus den größten Zeichen
        let line_metrics = |s: usize, e: usize| {
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
            (a, d)
        };
        let mut s = 0;
        let mut li = 0;
        loop {
            if li == 0 && !cur.fresh() {
                cur.y += ps.before;
            }
            // Höhe vorab aus dem Absatzanfang schätzen (für den freien Bereich neben Objekten)
            let (ea, ed) = line_metrics(s, (s + 1).min(n));
            let est_h = (ea + ed) * ls;
            if cur.y + est_h > bottom_of(&note_h, cur.page) && !cur.fresh() {
                cur.next();
            }
            let (xl, xr) = avail(&floats, &cur, cur.y, est_h);
            let x0 = xl + indent + if li == 0 { lead } else { 0.0 };
            let maxw = (xr - x0).max(24.0);
            // Umbruch dieser Zeile
            let (mut w, mut i, mut brk) = (0.0f32, s, None);
            while i < n {
                let c = chars[i].0;
                let cw = if c == '\t' && disp[i].is_none() { tab_w(w) } else { adv[i] };
                if w + cw > maxw && i > s && c != ' ' {
                    break;
                }
                w += cw;
                i += 1;
                if disp[i - 1].is_none() && (c == ' ' || c == '-' || c == '\t' || c == '/') {
                    brk = Some(i);
                }
            }
            let mut hyph = false;
            let mut e = if i < n { brk.filter(|b| *b > s).unwrap_or(i) } else { n };
            if i < n && doc.ex.hyph {
                // Silbentrennung im überstehenden Wort
                let ws = brk.unwrap_or(s).max(s);
                let mut we = ws;
                while we < n && chars[we].0.is_alphabetic() {
                    we += 1;
                }
                if we > i && we - ws >= 5 {
                    let word: String = chars[ws..we].iter().map(|(c, _)| *c).collect();
                    let base: f32 = adv[s..ws].iter().sum::<f32>();
                    let mut best = None;
                    for hp in hyphen_points(&word) {
                        let cut = ws + hp;
                        if cut <= s || cut > i {
                            continue;
                        }
                        let ww: f32 = base + adv[ws..cut].iter().sum::<f32>();
                        if ww + hy_w <= maxw {
                            best = Some(cut);
                        }
                    }
                    if let Some(cut) = best {
                        e = cut;
                        hyph = true;
                    }
                }
            }
            let (a, d) = line_metrics(s, e);
            let h = (a + d) * ls;
            let base = a + (h - a - d) * 0.3;
            // Fußnoten in dieser Zeile
            let mut new_notes: Vec<(u32, Vec<String>, Pos)> = Vec::new();
            {
                let mut k = 0u32;
                for (ci, (c, ri)) in chars.iter().enumerate() {
                    if *c == OBJ {
                        if let Some(Field::Note(t)) = &p.runs[*ri].st.field {
                            k += 1;
                            if ci >= s && ci < e {
                                let num = disp[ci].as_deref().and_then(|d| d.parse().ok()).unwrap_or(k);
                                new_notes.push((num, wrap_simple(&format!("{num}  {t}"), nfont, NOTE_SIZE, pg.content_w()), Pos::new(pi, ci)));
                            }
                        }
                    }
                }
            }
            let need = |page: usize, note_h: &Vec<f32>| -> f32 {
                let add: f32 = new_notes.iter().map(|(_, l, _)| l.len() as f32 * note_lh + 2.0).sum();
                if add > 0.0 && note_h.get(page).copied().unwrap_or(0.0) == 0.0 {
                    add + 10.0
                } else {
                    add
                }
            };
            if cur.y + h > bottom_of(&note_h, cur.page) - need(cur.page, &note_h) && !cur.fresh() {
                cur.next();
            }
            // x-Positionen
            let mut rel = Vec::with_capacity(e - s + 1);
            let mut w = 0.0f32;
            rel.push(0.0);
            for i in s..e {
                w += if chars[i].0 == '\t' && disp[i].is_none() { tab_w(w) } else { adv[i] };
                rel.push(w);
            }
            let mut trim = e;
            while trim > s && chars[trim - 1].0 == ' ' {
                trim -= 1;
            }
            let used = rel[trim - s] + if hyph { hy_w } else { 0.0 };
            let last = e >= n;
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
                    if let Some(d) = &disp[k] {
                        if !d.is_empty() {
                            words.push((xs[k - s], d.clone()));
                        }
                        k += 1;
                        continue;
                    }
                    if matches!(chars[k].0, ' ' | '\t') {
                        k += 1;
                        continue;
                    }
                    let st = k;
                    while k < j && !matches!(chars[k].0, ' ' | '\t') && disp[k].is_none() {
                        k += 1;
                    }
                    words.push((xs[st - s], chars[st..k].iter().map(|(c, _)| *c).collect()));
                }
                let x1 = if j == e { xs[trim.max(i) - s].max(xs[i - s]) } else { xs[j - s] };
                segs.push(Seg { x0: xs[i - s], x1, st: styles[ri].clone(), words });
                i = j;
            }
            if hyph {
                if let Some(sg) = segs.last_mut() {
                    let x = xs[e - s];
                    sg.words.push((x, "-".into()));
                    sg.x1 = x + hy_w;
                }
            }
            let mk = (li == 0).then(|| marker.clone()).flatten().filter(|m| !m.is_empty()).map(|m| {
                let mw = width_of(&m, &mstyle);
                let mx = if caption {
                    x0 - lead + off
                } else if mw <= LIST_GAP - 4.0 {
                    x0 - LIST_GAP
                } else {
                    // breite Nummern ragen nach links hinaus
                    x0 - mw - 4.0
                };
                Seg { x0: mx, x1: mx + mw, st: mstyle.clone(), words: vec![(mx, m)] }
            });
            if !new_notes.is_empty() {
                let add = need(cur.page, &note_h);
                if note_h.len() <= cur.page {
                    note_h.resize(cur.page + 1, 0.0);
                }
                note_h[cur.page] += add;
                for (num, l, pos) in new_notes {
                    let h = l.len() as f32 * note_lh + 2.0;
                    notes.push(NoteBox { page: cur.page, num, y: 0.0, h, x: pg.ml, lines: l.into_iter().map(|t| (0.0, t)).collect(), pos });
                }
            }
            lines.push(Line { para: pi, start: s, end: e, page: cur.page, y: cur.y, h, base, xs, segs, marker: mk, first: li == 0, last, pagebreak: false, img: None, obj: None, row: None, hyph });
            cur.y += h;
            li += 1;
            s = e;
            if s >= n {
                break;
            }
        }
        cur.y += ps.after;
        para_lines.push((first_idx, lines.len() - 1));
    }
    // Fußnoten unten auf ihren Seiten stapeln
    let pages = cur.page + 1;
    for page in 0..pages {
        let total: f32 = notes.iter().filter(|n| n.page == page).map(|n| n.h).sum();
        let mut y = bottom0 - total;
        for nb in notes.iter_mut().filter(|n| n.page == page) {
            nb.y = y;
            for (i, l) in nb.lines.iter_mut().enumerate() {
                l.0 = y + na + i as f32 * note_lh;
            }
            y += nb.h;
        }
    }
    Layout { lines, pages, page: pg, para_lines, notes, cols: (ncol, col_w, COL_GAP) }
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
