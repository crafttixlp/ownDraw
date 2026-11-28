//! Textdokumente in andere Formate schreiben und von dort lesen: ODT, RTF, HTML, Markdown und reiner Text.
//! Gemeinsam ist allen Ausgaben: Felder werden ausgefüllt (siehe [`super::fields`]), gelöschte Änderungen entfallen,
//! Fußnoten stehen im Text (ODT, RTF) bzw. als Endnoten (HTML, Markdown, Text). Der Import ist tolerant: Unbekanntes
//! wird übersprungen, der Text bleibt erhalten.

use super::fields::{field_text, FieldCx, Numbers};
use super::fonts::Family;
use super::model::{Align, CStyle, Img, Kind, Para, Run, TextDoc, VAlign};
use super::objects::{Cell, Field, Obj, ObjKind, Table, OBJ};
use super::render::{self, Images};
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;

/// Dateiformat eines Textdokuments (Austausch mit anderen Programmen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFmt {
    Odt,
    Rtf,
    Html,
    Md,
    Txt,
}

impl TextFmt {
    pub const ALL: [TextFmt; 5] = [TextFmt::Odt, TextFmt::Rtf, TextFmt::Html, TextFmt::Md, TextFmt::Txt];
    pub fn ext(self) -> &'static str {
        match self {
            TextFmt::Odt => "odt",
            TextFmt::Rtf => "rtf",
            TextFmt::Html => "html",
            TextFmt::Md => "md",
            TextFmt::Txt => "txt",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            TextFmt::Odt => "OpenDocument-Text (.odt)",
            TextFmt::Rtf => "Rich Text (.rtf)",
            TextFmt::Html => "Webseite (.html)",
            TextFmt::Md => "Markdown (.md)",
            TextFmt::Txt => "Reiner Text (.txt)",
        }
    }
    pub fn from_ext(e: &str) -> Option<TextFmt> {
        match e.trim().trim_start_matches('.').to_ascii_lowercase().as_str() {
            "odt" => Some(TextFmt::Odt),
            "rtf" => Some(TextFmt::Rtf),
            "html" | "htm" | "xhtml" => Some(TextFmt::Html),
            "md" | "markdown" => Some(TextFmt::Md),
            "txt" | "text" => Some(TextFmt::Txt),
            _ => None,
        }
    }
    pub fn mime(self) -> &'static str {
        match self {
            TextFmt::Odt => "application/vnd.oasis.opendocument.text",
            TextFmt::Rtf => "application/rtf",
            TextFmt::Html => "text/html",
            TextFmt::Md => "text/markdown",
            TextFmt::Txt => "text/plain",
        }
    }
}

/// Schreibt das Dokument im gewünschten Format.
pub fn export(doc: &TextDoc, fmt: TextFmt, title: &str, imgs: &Images) -> Result<Vec<u8>, String> {
    match fmt {
        TextFmt::Odt => odt_export(doc, title, imgs),
        TextFmt::Rtf => Ok(rtf_export(doc, title, imgs).into_bytes()),
        TextFmt::Html => Ok(html_export(doc, title, imgs).into_bytes()),
        TextFmt::Md => Ok(md_export(doc, imgs).into_bytes()),
        TextFmt::Txt => Ok(txt_export(doc).into_bytes()),
    }
}

/// Liest ein Dokument im angegebenen Format.
pub fn import(bytes: &[u8], fmt: TextFmt) -> Result<(TextDoc, Images), String> {
    if bytes.len() > 256 << 20 {
        return Err("Die Datei ist zu groß.".into());
    }
    match fmt {
        TextFmt::Odt => odt_import(bytes),
        TextFmt::Rtf => rtf_import(bytes),
        TextFmt::Html => Ok(html_import(&decode_text(bytes))),
        TextFmt::Md => Ok(md_import(&decode_text(bytes))),
        TextFmt::Txt => Ok(txt_import(&decode_text(bytes))),
    }
}

// ------------------------------------------------------------------ Hilfen

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' => {}
            c => o.push(c),
        }
    }
    o
}

fn hex6(c: u32) -> String {
    format!("#{:06x}", c & 0xFF_FFFF)
}

/// "#rgb", "#rrggbb", "rgb(r,g,b)" oder ein Grundfarbname → ARGB.
fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim().to_ascii_lowercase();
    let rgb = |r: u32, g: u32, b: u32| Some(0xFF00_0000 | (r << 16) | (g << 8) | b);
    if let Some(h) = s.strip_prefix('#') {
        return match h.len() {
            3 => {
                let v = u32::from_str_radix(h, 16).ok()?;
                rgb(((v >> 8) & 15) * 17, ((v >> 4) & 15) * 17, (v & 15) * 17)
            }
            6 | 8 => {
                let v = u32::from_str_radix(&h[..6], 16).ok()?;
                Some(0xFF00_0000 | v)
            }
            _ => None,
        };
    }
    if let Some(r) = s.strip_prefix("rgb(").or_else(|| s.strip_prefix("rgba(")) {
        let v: Vec<u32> = r.trim_end_matches(')').split(',').take(3).filter_map(|x| x.trim().parse::<f32>().ok()).map(|x| x.clamp(0.0, 255.0) as u32).collect();
        return if v.len() == 3 { rgb(v[0], v[1], v[2]) } else { None };
    }
    match s.as_str() {
        "black" => rgb(0, 0, 0),
        "white" => rgb(255, 255, 255),
        "red" => rgb(255, 0, 0),
        "green" => rgb(0, 128, 0),
        "blue" => rgb(0, 0, 255),
        "yellow" => rgb(255, 255, 0),
        "gray" | "grey" => rgb(128, 128, 128),
        "orange" => rgb(255, 165, 0),
        "purple" => rgb(128, 0, 128),
        _ => None,
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_enc(d: &[u8]) -> String {
    let mut o = String::with_capacity(d.len() / 3 * 4 + 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        o.push(B64[(n >> 18) as usize & 63] as char);
        o.push(B64[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    o
}

fn b64_dec(s: &str) -> Option<Vec<u8>> {
    let mut o = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            o.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    (!o.is_empty()).then_some(o)
}

/// MIME-Typ und Endung nach den ersten Bytes.
fn sniff(b: &[u8]) -> (&'static str, &'static str) {
    if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        ("image/png", "png")
    } else if b.starts_with(&[0xFF, 0xD8]) {
        ("image/jpeg", "jpg")
    } else if b.starts_with(b"GIF8") {
        ("image/gif", "gif")
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        ("image/webp", "webp")
    } else if b.starts_with(b"BM") {
        ("image/bmp", "bmp")
    } else {
        ("image/png", "png")
    }
}

fn data_uri(b: &[u8]) -> String {
    format!("data:{};base64,{}", sniff(b).0, b64_enc(b))
}

/// Text aus Bytes: BOM entfernen, UTF-8, UTF-16 (mit BOM), sonst Windows-1252.
fn decode_text(b: &[u8]) -> String {
    if let Some(r) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(r).into_owned();
    }
    for (bom, le) in [(&[0xFF, 0xFE], true), (&[0xFE, 0xFF], false)] {
        if let Some(r) = b.strip_prefix(bom) {
            let u = r.chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) });
            return char::decode_utf16(u).map(|c| c.unwrap_or('\u{FFFD}')).collect();
        }
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| cp1252(c)).collect(),
    }
}

fn cp1252(c: u8) -> char {
    const T: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    if (0x80..0xA0).contains(&c) {
        T[(c - 0x80) as usize]
    } else {
        c as char
    }
}

/// Bild für den Import einfügen: nur lesbare Rasterbilder; Größe in pt (fehlt sie, aus den Pixeln).
fn add_image(store: &mut crate::doc::ImageStore, bytes: Vec<u8>, w: Option<f32>, h: Option<f32>) -> Option<Img> {
    let fmt = image::guess_format(&bytes).ok()?;
    let ext = fmt.extensions_str().first().copied().unwrap_or("png");
    let (mut w, mut h) = match (w, h) {
        (Some(w), Some(h)) if w >= 1.0 && h >= 1.0 => (w, h),
        _ => {
            let pm = render::decode(&bytes)?;
            let (pw, ph) = (pm.width() as f32 * 0.75, pm.height() as f32 * 0.75);
            match (w, h) {
                (Some(w), _) if w >= 1.0 => (w, w * ph / pw),
                (_, Some(h)) if h >= 1.0 => (h * pw / ph, h),
                _ => (pw, ph),
            }
        }
    };
    let max = super::model::PageSpec::default().content_w();
    if w > max {
        h *= max / w;
        w = max;
    }
    let file = store.add(bytes, ext);
    Some(Img { file, w: w.clamp(4.0, 5000.0), h: h.clamp(4.0, 5000.0) })
}

fn pic_para(img: Img, align: Align) -> Para {
    Para { align, img: Some(img), ..Para::new(Kind::Image) }
}

/// Länge mit Einheit (cm, mm, in, pt, pc, px) in pt.
fn len_pt(s: &str) -> Option<f32> {
    let s = s.trim();
    let i = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(s.len());
    let v: f32 = s[..i].parse().ok()?;
    let k = match s[i..].trim() {
        "cm" => 28.346457,
        "mm" => 2.8346457,
        "in" => 72.0,
        "pc" => 12.0,
        "px" => 0.75,
        "pt" | "" => 1.0,
        _ => return None,
    };
    Some(v * k)
}

fn al_css(a: Align) -> Option<&'static str> {
    match a {
        Align::Left => None,
        Align::Center => Some("center"),
        Align::Right => Some("right"),
        Align::Justify => Some("justify"),
    }
}

fn align_from(s: &str) -> Option<Align> {
    match s.trim().to_ascii_lowercase().as_str() {
        "center" => Some(Align::Center),
        "right" | "end" => Some(Align::Right),
        "justify" => Some(Align::Justify),
        "left" | "start" => Some(Align::Left),
        _ => None,
    }
}

/// Text eines Objekts, wenn es sich nicht als Bild ausgeben lässt.
fn obj_text(o: &Obj) -> String {
    match &o.kind {
        ObjKind::Formula { src } => format!("[Formel: {src}]"),
        ObjKind::Chart { title, .. } if !title.trim().is_empty() => format!("[Diagramm: {}]", title.trim()),
        ObjKind::Chart { .. } => "[Diagramm]".into(),
        ObjKind::Drawing { .. } => "[Zeichnung]".into(),
        ObjKind::Shape { text, shape, .. } => {
            if text.trim().is_empty() {
                format!("[{}]", shape.label())
            } else {
                text.clone()
            }
        }
    }
}

/// Teil eines Absatzes nach dem Auflösen von Feldern.
enum Seg {
    T(String, CStyle),
    Note(u32, String),
    Page,
    Pages,
}

/// Gemeinsamer Zustand der Ausgaben.
struct Cx<'a> {
    doc: &'a TextDoc,
    imgs: &'a Images,
    nums: Numbers,
    note: u32,
}

impl<'a> Cx<'a> {
    fn new(doc: &'a TextDoc, imgs: &'a Images) -> Cx<'a> {
        Cx { doc, imgs, nums: Numbers::new(doc), note: 0 }
    }

    /// Absatz als Folge von Teilen (Beschriftungsnummer vorangestellt, Felder ausgefüllt, Gelöschtes weg).
    fn segs(&mut self, pi: usize) -> Vec<Seg> {
        let doc = self.doc;
        let p = &doc.paras[pi];
        let mut v = Vec::new();
        if let Some(pre) = self.nums.caption_prefix(pi) {
            v.push(Seg::T(pre, CStyle::default()));
        }
        let pp = |_: usize| 0usize;
        for r in &p.runs {
            if r.st.del > 0 {
                continue;
            }
            let Some(f) = &r.st.field else {
                let t = r.text.replace(OBJ, "");
                if !t.is_empty() {
                    v.push(Seg::T(t, r.st.clone()));
                }
                continue;
            };
            let st = CStyle { field: None, ..r.st.clone() };
            for _ in r.text.chars() {
                match f {
                    Field::Note(t) => {
                        self.note += 1;
                        v.push(Seg::Note(self.note, t.clone()));
                    }
                    Field::Page => v.push(Seg::Page),
                    Field::Pages => v.push(Seg::Pages),
                    Field::Index(_) => {}
                    _ => {
                        let cx = FieldCx { doc, nums: &self.nums, page: 0, pages: 1, para_page: &pp, note: self.note };
                        let t = field_text(f, &cx);
                        if !t.is_empty() {
                            v.push(Seg::T(t, st.clone()));
                        }
                    }
                }
            }
        }
        v
    }

    /// Bild eines Bild- oder Objekt-Absatzes (Bytes, Breite, Höhe in pt). Formeln bleiben auf Wunsch Text.
    fn picture(&self, p: &Para, formula_text: bool) -> Option<(Cow<'a, [u8]>, f32, f32)> {
        match p.kind {
            Kind::Image => {
                let im = p.img.as_ref()?;
                Some((Cow::Borrowed(self.imgs.get(&im.file)?.as_slice()), im.w, im.h))
            }
            Kind::Object => {
                let o = p.obj.as_ref()?;
                if formula_text && matches!(o.kind, ObjKind::Formula { .. }) {
                    return None;
                }
                render::object_png(o, 2.0).map(|b| (Cow::Owned(b), o.w, o.h))
            }
            _ => None,
        }
    }
}

fn seg_plain(segs: &[Seg]) -> String {
    let mut s = String::new();
    for g in segs {
        match g {
            Seg::T(t, _) => s.push_str(t),
            Seg::Note(n, _) => s.push_str(&format!("[{n}]")),
            Seg::Page | Seg::Pages => s.push('#'),
        }
    }
    s
}

/// Auf-/Zu-Tags einer Listenart (HTML, ODT).
struct ListTags {
    open: fn(bool) -> &'static str,
    close: fn(bool) -> &'static str,
    li: &'static str,
    li_end: &'static str,
}

/// Offene Listen (je Ebene: nummeriert?) beim Schreiben verschachtelter Listen.
#[derive(Default)]
struct Lists(Vec<bool>);

impl Lists {
    /// Beginnt einen Eintrag auf Ebene `level`; schließt und öffnet Listen nach Bedarf.
    fn enter(&mut self, level: usize, ordered: bool, t: &ListTags, o: &mut String) {
        while self.0.len() > level + 1 {
            let ord = self.0.pop().unwrap_or(false);
            o.push_str(t.li_end);
            o.push_str((t.close)(ord));
        }
        if self.0.len() == level + 1 {
            o.push_str(t.li_end);
            if self.0[level] != ordered {
                let ord = self.0.pop().unwrap_or(false);
                o.push_str((t.close)(ord));
            }
        }
        while self.0.len() < level + 1 {
            let ord = self.0.len() == level && ordered;
            o.push_str((t.open)(ord));
            self.0.push(ord);
            if self.0.len() < level + 1 {
                o.push_str(t.li);
            }
        }
        o.push_str(t.li);
    }
    fn close_all(&mut self, t: &ListTags, o: &mut String) {
        while let Some(ord) = self.0.pop() {
            o.push_str(t.li_end);
            o.push_str((t.close)(ord));
        }
    }
}

/// Zähler für nummerierte Listen (Markdown, RTF).
#[derive(Default)]
struct Counter([u32; 9]);

impl Counter {
    fn next(&mut self, level: usize, ordered: bool) -> u32 {
        let l = level.min(8);
        for c in &mut self.0[l + 1..] {
            *c = 0;
        }
        if ordered {
            self.0[l] += 1;
        } else {
            self.0[l] = 0;
        }
        self.0[l]
    }
    fn reset(&mut self) {
        self.0 = [0; 9];
    }
}

/// Zelle beim Lesen: sammelt Text und stellt fest, ob alles fett/kursiv ist.
#[derive(Default)]
struct CellB {
    text: String,
    n: usize,
    nb: usize,
    ni: usize,
    bg: u32,
    color: u32,
    align: Align,
    th: bool,
}

impl CellB {
    fn add(&mut self, s: &str, st: &CStyle) {
        let n = s.chars().filter(|c| !c.is_whitespace()).count();
        self.text.push_str(s);
        self.n += n;
        if st.b {
            self.nb += n;
        }
        if st.i {
            self.ni += n;
        }
        if self.color == 0 && st.color != 0 && n > 0 {
            self.color = st.color;
        }
    }
    fn done(self) -> Cell {
        Cell { text: self.text.trim().to_string(), b: self.n > 0 && self.nb == self.n, i: self.n > 0 && self.ni == self.n, align: self.align, bg: self.bg, color: self.color }
    }
}

/// Tabelle aus gelesenen Zeilen (Spaltenbreite gleichmäßig auf die Textbreite verteilt).
fn make_table(rows: Vec<Vec<Cell>>, header: bool) -> Option<Para> {
    let n = rows.iter().map(Vec::len).max().unwrap_or(0).clamp(0, 20);
    if n == 0 || rows.is_empty() {
        return None;
    }
    let w = super::model::PageSpec::default().content_w();
    let mut rows = rows;
    rows.truncate(200);
    let mut t = Table { cols: vec![w / n as f32; n], rows, header, stripes: false, border: 0xFFB8BCC6 };
    for r in &mut t.rows {
        r.truncate(n);
    }
    t.fix();
    Some(Para { table: Some(t), ..Para::new(Kind::Table) })
}

/// Kopfzeilen-Tönung, die der Export selbst setzt (beim Import wieder entfernt).
const HEAD_BG: u32 = 0xFFE8EAF0;

/// Formel-Absatz.
fn formula_para(src: &str) -> Para {
    let w = (src.chars().count() as f32 * 8.0).clamp(40.0, 400.0);
    Para { obj: Some(Obj { kind: ObjKind::Formula { src: src.into() }, w, h: 28.0 }), ..Para::new(Kind::Object) }
}

/// "Abbildung 2: Text" → ("Abbildung", "Text") für Beschriftungen beim Import.
fn strip_caption(runs: &mut [Run]) -> Option<String> {
    let t = &runs.first()?.text;
    let (lab, rest) = t.split_once(": ")?;
    let (word, num) = lab.rsplit_once(' ')?;
    if word.is_empty() || num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (word, rest) = (word.to_string(), rest.to_string());
    runs[0].text = rest;
    Some(word)
}

/// Baustein für das Lesen von Textformaten: Absätze, laufender Absatz, Bilder.
#[derive(Default)]
struct Bld {
    paras: Vec<Para>,
    runs: Vec<Run>,
    store: crate::doc::ImageStore,
}

impl Bld {
    fn push(&mut self, t: &str, st: &CStyle) {
        if t.is_empty() {
            return;
        }
        match self.runs.last_mut() {
            Some(l) if l.st == *st => l.text.push_str(t),
            _ => self.runs.push(Run { text: t.into(), st: st.clone() }),
        }
    }
    fn has_text(&self) -> bool {
        self.runs.iter().any(|r| !r.text.is_empty())
    }
    /// Beendet den laufenden Absatz (leer nur mit `force`).
    fn flush(&mut self, kind: Kind, align: Align, indent: u8, force: bool) {
        let mut runs = std::mem::take(&mut self.runs);
        if let Some(l) = runs.last_mut() {
            l.text = l.text.trim_end_matches(' ').to_string();
        }
        if runs.iter().all(|r| r.text.is_empty()) && !force {
            return;
        }
        let cap = if kind == Kind::Caption { strip_caption(&mut runs) } else { None };
        let mut p = Para { kind, align, indent: if kind.is_list() { indent.min(8) } else { 0 }, runs, cap: cap.unwrap_or_default(), ..Para::default() };
        p.normalize();
        self.paras.push(p);
    }
    fn image(&mut self, bytes: Vec<u8>, w: Option<f32>, h: Option<f32>, align: Align) {
        if let Some(im) = add_image(&mut self.store, bytes, w, h) {
            self.paras.push(pic_para(im, align));
        }
    }
    fn finish(mut self) -> (TextDoc, Images) {
        self.flush(Kind::Body, Align::Left, 0, false);
        let mut d = TextDoc::new();
        if !self.paras.is_empty() {
            d.paras = self.paras;
        }
        (d, self.store.files)
    }
}

// ------------------------------------------------------------------ HTML-Zerlegung

enum Tok {
    Text(String),
    Open(String, Vec<(String, String)>),
    Close(String),
}

fn entity(n: &str) -> Option<char> {
    if let Some(h) = n.strip_prefix('#') {
        let v = match h.strip_prefix(['x', 'X']) {
            Some(x) => u32::from_str_radix(x, 16),
            None => h.parse(),
        };
        return v.ok().and_then(char::from_u32);
    }
    Some(match n {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "auml" => 'ä',
        "ouml" => 'ö',
        "uuml" => 'ü',
        "Auml" => 'Ä',
        "Ouml" => 'Ö',
        "Uuml" => 'Ü',
        "szlig" => 'ß',
        "euro" => '€',
        "hellip" => '…',
        "ndash" => '–',
        "mdash" => '—',
        "copy" => '©',
        "reg" => '®',
        "laquo" => '«',
        "raquo" => '»',
        "bdquo" => '„',
        "ldquo" => '“',
        "rdquo" => '”',
        "lsquo" => '‘',
        "rsquo" => '’',
        "bull" => '•',
        "middot" => '·',
        "deg" => '°',
        "times" => '×',
        "larr" => '←',
        "rarr" => '→',
        _ => return None,
    })
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut o = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        o.push_str(&rest[..p]);
        rest = &rest[p..];
        if let Some(e) = rest[1..].find(';').filter(|e| *e <= 10) {
            if let Some(c) = entity(&rest[1..1 + e]) {
                o.push(c);
                rest = &rest[e + 2..];
                continue;
            }
        }
        o.push('&');
        rest = &rest[1..];
    }
    o.push_str(rest);
    o
}

const VOID: [&str; 8] = ["br", "img", "hr", "meta", "link", "input", "col", "wbr"];

fn parse_tag(inner: &str, out: &mut Vec<Tok>) {
    let closing = inner.starts_with('/');
    let mut t = inner.trim_start_matches('/').trim();
    let selfclose = t.ends_with('/');
    if selfclose {
        t = t[..t.len() - 1].trim_end();
    }
    let b = t.as_bytes();
    let end = b.iter().position(|c| c.is_ascii_whitespace() || *c == b'/').unwrap_or(b.len());
    let name = t[..end].to_ascii_lowercase();
    if name.is_empty() {
        return;
    }
    if closing {
        out.push(Tok::Close(name));
        return;
    }
    let mut attrs = Vec::new();
    let mut i = end;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let k0 = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        let key = t[k0..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut val = String::new();
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                let v0 = i + 1;
                i = v0;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                val = unescape(&t[v0..i]);
                i = (i + 1).min(b.len());
            } else {
                let v0 = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                val = unescape(&t[v0..i]);
            }
        }
        if !key.is_empty() {
            attrs.push((key, val));
        }
    }
    let void = VOID.contains(&name.as_str());
    out.push(Tok::Open(name.clone(), attrs));
    if selfclose && !void {
        out.push(Tok::Close(name));
    }
}

fn tokenize(s: &str) -> Vec<Tok> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut ts) = (0, 0);
    let text = |out: &mut Vec<Tok>, a: usize, z: usize| {
        if z > a {
            out.push(Tok::Text(unescape(&s[a..z])));
        }
    };
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let rest = &s[i..];
        if rest.starts_with("<!--") {
            text(&mut out, ts, i);
            i = rest.find("-->").map(|e| i + e + 3).unwrap_or(b.len());
            ts = i;
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            text(&mut out, ts, i);
            i = rest.find('>').map(|e| i + e + 1).unwrap_or(b.len());
            ts = i;
            continue;
        }
        let nb = b.get(i + 1).copied().unwrap_or(0);
        if !(nb.is_ascii_alphabetic() || nb == b'/') {
            i += 1;
            continue;
        }
        text(&mut out, ts, i);
        let mut j = i + 1;
        let mut q = 0u8;
        while j < b.len() {
            let c = b[j];
            if q != 0 {
                if c == q {
                    q = 0;
                }
            } else if c == b'"' || c == b'\'' {
                q = c;
            } else if c == b'>' {
                break;
            }
            j += 1;
        }
        let inner = &s[i + 1..j.min(b.len())];
        i = (j + 1).min(b.len());
        ts = i;
        parse_tag(inner, &mut out);
        if let Some(Tok::Open(n, _)) = out.last() {
            if n == "script" || n == "style" {
                let lower = s[i..].to_ascii_lowercase();
                let end = lower.find(&format!("</{n}")).map(|e| i + e).unwrap_or(b.len());
                i = end;
                ts = i;
            }
        }
    }
    text(&mut out, ts, b.len());
    out
}

fn attr<'a>(a: &'a [(String, String)], k: &str) -> Option<&'a str> {
    a.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

/// Wert einer CSS-Eigenschaft aus einem `style`-Attribut.
fn css<'a>(style: &'a str, name: &str) -> Option<&'a str> {
    style.split(';').find_map(|d| {
        let (k, v) = d.split_once(':')?;
        (k.trim().eq_ignore_ascii_case(name)).then(|| v.trim())
    })
}

fn css_style(style: &str, st: &mut CStyle) {
    for d in style.split(';') {
        let Some((k, v)) = d.split_once(':') else { continue };
        let v = v.trim().to_ascii_lowercase();
        match k.trim().to_ascii_lowercase().as_str() {
            "color" => {
                if let Some(c) = parse_color(&v) {
                    st.color = c;
                }
            }
            "background-color" | "background" => {
                if let Some(c) = parse_color(v.split_whitespace().next().unwrap_or("")) {
                    st.hl = c;
                }
            }
            "font-weight" => st.b = v == "bold" || v == "bolder" || v.parse::<u32>().is_ok_and(|n| n >= 600),
            "font-style" => st.i = v == "italic" || v == "oblique",
            "text-decoration" | "text-decoration-line" => {
                st.u |= v.contains("underline");
                st.s |= v.contains("line-through");
            }
            "font-size" => {
                if let Some(pt) = len_pt(&v).filter(|p| (1.0..=400.0).contains(p)) {
                    st.size = Some(pt);
                }
            }
            "font-family" => {
                if ["sans", "arial", "helvetica", "verdana", "calibri"].iter().any(|f| v.contains(f)) {
                    st.fam = Some(Family::Sans);
                } else if ["serif", "times", "georgia", "garamond"].iter().any(|f| v.contains(f)) {
                    st.fam = Some(Family::Serif);
                }
            }
            "vertical-align" => {
                if v == "super" {
                    st.va = VAlign::Sup;
                } else if v == "sub" {
                    st.va = VAlign::Sub;
                }
            }
            _ => {}
        }
    }
}

/// Fußnotenabschnitt (`<section class="footnotes">`) herauslösen: Kennung → Text.
fn extract_notes(toks: &mut Vec<Tok>) -> HashMap<String, String> {
    let mut notes = HashMap::new();
    let Some(a) = toks.iter().position(|t| matches!(t, Tok::Open(n, at) if n == "section" && attr(at, "class").is_some_and(|c| c.contains("footnotes")))) else { return notes };
    let mut depth = 0;
    let mut z = toks.len();
    for (i, t) in toks.iter().enumerate().skip(a) {
        match t {
            Tok::Open(n, _) if n == "section" => depth += 1,
            Tok::Close(n) if n == "section" => {
                depth -= 1;
                if depth == 0 {
                    z = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    let mut cur: Option<(String, String)> = None;
    let mut skip_a = false;
    for t in &toks[a..z] {
        match t {
            Tok::Open(n, at) if n == "li" => cur = attr(at, "id").map(|i| (i.to_string(), String::new())),
            Tok::Open(n, at) if n == "a" => skip_a = attr(at, "href").is_some_and(|h| h.starts_with("#fnref")),
            Tok::Close(n) if n == "a" => skip_a = false,
            Tok::Text(s) if !skip_a => {
                if let Some((_, t)) = &mut cur {
                    t.push_str(s);
                }
            }
            Tok::Close(n) if n == "li" => {
                if let Some((id, t)) = cur.take() {
                    notes.insert(id, t.split_whitespace().collect::<Vec<_>>().join(" "));
                }
            }
            _ => {}
        }
    }
    toks.drain(a..z);
    notes
}

// ------------------------------------------------------------------ HTML

const HTML_LISTS: ListTags = ListTags {
    open: |o| if o { "<ol>" } else { "<ul>" },
    close: |o| if o { "</ol>" } else { "</ul>" },
    li: "<li>",
    li_end: "</li>",
};

fn html_run(t: &str, st: &CStyle) -> String {
    let mut s = esc(t);
    match st.va {
        VAlign::Sup => s = format!("<sup>{s}</sup>"),
        VAlign::Sub => s = format!("<sub>{s}</sub>"),
        VAlign::Base => {}
    }
    for (on, tag) in [(st.s, "s"), (st.u, "u"), (st.i, "i"), (st.b, "b")] {
        if on {
            s = format!("<{tag}>{s}</{tag}>");
        }
    }
    let mut c = String::new();
    if st.color != 0 {
        c.push_str(&format!("color:{};", hex6(st.color)));
    }
    if st.hl != 0 {
        c.push_str(&format!("background-color:{};", hex6(st.hl)));
    }
    if let Some(sz) = st.size {
        c.push_str(&format!("font-size:{}pt;", (sz * 10.0).round() / 10.0));
    }
    if let Some(f) = st.fam {
        c.push_str(if f == Family::Sans { "font-family:Arial,sans-serif;" } else { "font-family:'Times New Roman',serif;" });
    }
    if !c.is_empty() {
        s = format!("<span style=\"{c}\">{s}</span>");
    }
    if let Some(l) = &st.link {
        s = format!("<a href=\"{}\">{s}</a>", esc(l));
    }
    s
}

fn html_inline(segs: &[Seg], notes: &mut Vec<(u32, String)>) -> String {
    let mut o = String::new();
    for g in segs {
        match g {
            Seg::T(t, st) => o.push_str(&html_run(t, st)),
            Seg::Note(n, t) => {
                notes.push((*n, t.clone()));
                o.push_str(&format!("<sup><a id=\"fnref{n}\" href=\"#fn{n}\">{n}</a></sup>"));
            }
            Seg::Page | Seg::Pages => o.push('#'),
        }
    }
    o
}

fn html_cell_text(s: &str) -> String {
    esc(s).replace('\n', "<br>")
}

fn html_table(t: &Table) -> String {
    let mut o = String::from(if t.border != 0 { "<table>\n" } else { "<table class=\"nb\">\n" });
    for (ri, row) in t.rows.iter().enumerate() {
        o.push_str("<tr>");
        for c in row {
            let tag = if t.header && ri == 0 { "th" } else { "td" };
            let mut st = String::new();
            if let Some(a) = al_css(c.align) {
                st.push_str(&format!("text-align:{a};"));
            }
            if c.bg != 0 {
                st.push_str(&format!("background-color:{};", hex6(c.bg)));
            }
            if c.color != 0 {
                st.push_str(&format!("color:{};", hex6(c.color)));
            }
            if c.b {
                st.push_str("font-weight:bold;");
            }
            if c.i {
                st.push_str("font-style:italic;");
            }
            let st = if st.is_empty() { String::new() } else { format!(" style=\"{st}\"") };
            o.push_str(&format!("<{tag}{st}>{}</{tag}>", html_cell_text(&c.text)));
        }
        o.push_str("</tr>\n");
    }
    o.push_str("</table>\n");
    o
}

fn html_export(doc: &TextDoc, title: &str, imgs: &Images) -> String {
    let mut cx = Cx::new(doc, imgs);
    let mut body = String::new();
    let mut notes: Vec<(u32, String)> = Vec::new();
    let mut lists = Lists::default();
    let body_font = if doc.family == Family::Sans { "Arial,Helvetica,sans-serif" } else { "'Times New Roman',Times,serif" };
    for (pi, p) in doc.paras.iter().enumerate() {
        if !p.kind.is_list() {
            lists.close_all(&HTML_LISTS, &mut body);
        }
        let al = al_css(p.align).map(|a| format!(" style=\"text-align:{a}\"")).unwrap_or_default();
        match p.kind {
            Kind::PageBreak => body.push_str("<div class=\"pagebreak\" style=\"page-break-after:always\"></div>\n"),
            Kind::Table => {
                if let Some(t) = &p.table {
                    body.push_str(&html_table(t));
                }
            }
            Kind::Image | Kind::Object => {
                if let Some((b, w, h)) = cx.picture(p, true) {
                    let a = al_css(p.align).map(|a| format!(" style=\"text-align:{a}\"")).unwrap_or_default();
                    body.push_str(&format!("<p{a}><img src=\"{}\" width=\"{}\" height=\"{}\" alt=\"\"></p>\n", data_uri(&b), (w * 4.0 / 3.0).round(), (h * 4.0 / 3.0).round()));
                } else if let Some(o) = &p.obj {
                    match &o.kind {
                        ObjKind::Formula { src } => body.push_str(&format!("<p{al}><code class=\"formula\">{}</code></p>\n", esc(src))),
                        _ => body.push_str(&format!("<p{al}>{}</p>\n", esc(&obj_text(o)))),
                    }
                }
            }
            k => {
                let segs = cx.segs(pi);
                let inner = html_inline(&segs, &mut notes);
                if k.is_list() {
                    let a = al_css(p.align).map(|a| format!(" style=\"text-align:{a}\"")).unwrap_or_default();
                    lists.enter((p.indent as usize).min(8), k == Kind::Number, &HTML_LISTS, &mut body);
                    if !a.is_empty() {
                        // Ausrichtung an den Eintrag hängen
                        let at = body.len() - "<li>".len();
                        body.replace_range(at.., &format!("<li{a}>"));
                    }
                    body.push_str(&inner);
                    continue;
                }
                let inner = if inner.is_empty() { "<br>".to_string() } else { inner };
                let s = match k {
                    Kind::Title => format!("<h1 class=\"title\"{al}>{inner}</h1>"),
                    Kind::H1 => format!("<h1{al}>{inner}</h1>"),
                    Kind::H2 => format!("<h2{al}>{inner}</h2>"),
                    Kind::H3 => format!("<h3{al}>{inner}</h3>"),
                    Kind::Quote => format!("<blockquote{al}>{inner}</blockquote>"),
                    Kind::Caption => format!("<p class=\"caption\"{al}>{inner}</p>"),
                    _ => format!("<p{al}>{inner}</p>"),
                };
                body.push_str(&s);
                body.push('\n');
            }
        }
    }
    lists.close_all(&HTML_LISTS, &mut body);
    if !notes.is_empty() {
        body.push_str("<section class=\"footnotes\"><hr><ol>\n");
        for (n, t) in &notes {
            body.push_str(&format!("<li id=\"fn{n}\">{} <a href=\"#fnref{n}\">↩</a></li>\n", esc(t)));
        }
        body.push_str("</ol></section>\n");
    }
    format!(
        "<!DOCTYPE html>\n<html lang=\"de\">\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n<style>\nbody{{max-width:46em;margin:2em auto;padding:0 1em;font-family:{body_font};font-size:{}pt;line-height:1.5;color:#1f2430}}\nh1,h2,h3{{font-family:Arial,Helvetica,sans-serif}}\nblockquote{{margin:1em 0 1em 1em;padding-left:1em;border-left:3px solid #b8bcc6;font-style:italic;color:#4a4a55}}\n.caption{{font-style:italic;color:#4a4a55}}\ntable{{border-collapse:collapse;margin:1em 0}}\ntd,th{{border:1px solid #b8bcc6;padding:4px 8px;vertical-align:top}}\nth{{background:#e8eaf0}}\ntable.nb td,table.nb th{{border:none}}\n.footnotes{{font-size:90%}}\n</style>\n</head>\n<body>\n{body}</body>\n</html>\n",
        esc(title),
        (doc.size * 10.0).round() / 10.0
    )
}

/// Laufender Zustand des HTML-Imports.
struct Hi {
    b: Bld,
    stack: Vec<(String, CStyle)>,
    lists: Vec<bool>,
    quote: u32,
    blk: Option<Kind>,
    align: Align,
    indent: u8,
    pre: bool,
    skip_name: Option<String>,
    skip_a: bool,
    notes: HashMap<String, String>,
    had_br: bool,
    split: bool,
    tb: Option<HTable>,
    tdepth: u32,
}

#[derive(Default)]
struct HTable {
    rows: Vec<Vec<Cell>>,
    row: Option<Vec<Cell>>,
    cell: Option<CellB>,
    head_row: bool,
    all_th: bool,
}

impl Hi {
    fn kind(&self) -> Kind {
        self.blk.unwrap_or(if self.quote > 0 { Kind::Quote } else { Kind::Body })
    }
    fn style(&self) -> CStyle {
        self.stack.last().map(|(_, s)| s.clone()).unwrap_or_default()
    }
    fn flush(&mut self, force: bool) {
        let (k, a, i) = (self.kind(), self.align, self.indent);
        self.b.flush(k, a, i, force);
    }
    fn cell_nl(&mut self) {
        if let Some(c) = self.tb.as_mut().and_then(|t| t.cell.as_mut()) {
            if !c.text.is_empty() && !c.text.ends_with('\n') {
                c.text.push('\n');
            }
        }
    }
    fn end_cell(&mut self) {
        if let Some(t) = &mut self.tb {
            if let Some(c) = t.cell.take() {
                if !c.th {
                    t.all_th = false;
                }
                let th = c.th;
                let mut cell = c.done();
                if th {
                    cell.b = false;
                }
                t.row.get_or_insert_with(Vec::new).push(cell);
            }
        }
    }
    fn end_row(&mut self) {
        self.end_cell();
        if let Some(t) = &mut self.tb {
            if let Some(r) = t.row.take() {
                if t.rows.is_empty() {
                    t.head_row = t.all_th;
                }
                t.rows.push(r);
                t.all_th = true;
            }
        }
    }
    fn end_table(&mut self) {
        self.end_row();
        if let Some(t) = self.tb.take() {
            if let Some(p) = make_table(t.rows, t.head_row) {
                self.b.paras.push(p);
            }
        }
    }

    fn text(&mut self, t: String) {
        if self.skip_name.is_some() || self.skip_a {
            return;
        }
        let st = self.style();
        if let Some(c) = self.tb.as_mut().and_then(|t| t.cell.as_mut()) {
            let mut s = collapse_ws(&t);
            if c.text.is_empty() || c.text.ends_with('\n') {
                s = s.trim_start().to_string();
            }
            c.add(&s, &st);
            return;
        }
        if self.pre {
            for (i, line) in t.split('\n').enumerate() {
                if i > 0 {
                    self.flush(true);
                }
                self.b.push(line.trim_end_matches('\r'), &st);
            }
            return;
        }
        let mut s = collapse_ws(&t);
        if !self.b.has_text() {
            s = s.trim_start().to_string();
        }
        self.b.push(&s, &st);
    }

    fn open(&mut self, name: String, at: Vec<(String, String)>) {
        let n = name.as_str();
        // stillschweigend geschlossene Elemente
        if n == "p" || n == "li" {
            let stop = |s: &str| matches!(s, "ul" | "ol" | "table" | "td" | "th");
            if self.stack.iter().rev().take_while(|(s, _)| !stop(s)).any(|(s, _)| s == n) {
                self.close(name.clone());
            }
        }
        let style_attr = attr(&at, "style").unwrap_or("");
        if let Some(al) = attr(&at, "align").and_then(align_from).or_else(|| css(style_attr, "text-align").and_then(align_from)) {
            if !matches!(n, "td" | "th" | "table" | "tr") {
                self.align = al;
            }
        }
        match n {
            "head" | "title" => self.skip_name = Some(name.clone()),
            "br" => {
                if self.tb.as_ref().is_some_and(|t| t.cell.is_some()) {
                    if let Some(c) = self.tb.as_mut().and_then(|t| t.cell.as_mut()) {
                        c.text.push('\n');
                    }
                } else if self.b.has_text() {
                    self.flush(false);
                    self.split = true;
                } else {
                    self.had_br = true;
                }
                return;
            }
            "hr" => {
                self.flush(false);
                if attr(&at, "class").is_some_and(|c| c.contains("pagebreak")) || style_attr.contains("page-break") {
                    self.b.paras.push(Para::new(Kind::PageBreak));
                }
                return;
            }
            "img" => {
                self.img(&at);
                return;
            }
            "div" => {
                self.flush(false);
                if attr(&at, "class").is_some_and(|c| c.contains("pagebreak")) || css(style_attr, "page-break-after").is_some_and(|v| v == "always") || css(style_attr, "page-break-before").is_some_and(|v| v == "always") {
                    self.b.paras.push(Para::new(Kind::PageBreak));
                }
                self.cell_nl();
            }
            "p" => {
                self.flush(false);
                self.cell_nl();
                self.had_br = false;
                self.split = false;
                if attr(&at, "class").is_some_and(|c| c.contains("caption")) {
                    self.blk = Some(Kind::Caption);
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.flush(false);
                self.blk = Some(match n {
                    "h1" if attr(&at, "class").is_some_and(|c| c.contains("title")) => Kind::Title,
                    "h1" => Kind::H1,
                    "h2" => Kind::H2,
                    _ => Kind::H3,
                });
            }
            "figcaption" => {
                self.flush(false);
                self.blk = Some(Kind::Caption);
            }
            "blockquote" => {
                self.flush(false);
                self.quote += 1;
            }
            "ul" | "ol" => {
                self.flush(false);
                self.lists.push(n == "ol");
                self.indent = (self.lists.len() - 1).min(8) as u8;
            }
            "li" => {
                self.flush(false);
                self.blk = Some(if self.lists.last().copied().unwrap_or(false) { Kind::Number } else { Kind::Bullet });
                self.indent = self.lists.len().saturating_sub(1).min(8) as u8;
            }
            "pre" => {
                self.flush(false);
                self.pre = true;
            }
            "table" => {
                if self.tb.is_some() {
                    self.tdepth += 1;
                } else {
                    self.flush(false);
                    self.tb = Some(HTable { all_th: true, ..HTable::default() });
                }
            }
            "tr" if self.tdepth == 0 => {
                self.end_row();
                if let Some(t) = &mut self.tb {
                    t.row = Some(Vec::new());
                }
            }
            "td" | "th" if self.tdepth == 0 => {
                self.end_cell();
                if self.tb.is_some() {
                    let mut c = CellB { th: n == "th", ..CellB::default() };
                    if let Some(al) = attr(&at, "align").and_then(align_from).or_else(|| css(style_attr, "text-align").and_then(align_from)) {
                        c.align = al;
                    }
                    let mut tmp = CStyle::default();
                    css_style(style_attr, &mut tmp);
                    c.bg = css(style_attr, "background-color").or(attr(&at, "bgcolor")).and_then(parse_color).unwrap_or(0);
                    c.color = tmp.color;
                    if let Some(t) = &mut self.tb {
                        if t.row.is_none() {
                            t.row = Some(Vec::new());
                        }
                        t.cell = Some(c);
                    }
                }
            }
            "a" => {
                if let Some(h) = attr(&at, "href").and_then(|h| h.strip_prefix('#')).filter(|h| h.starts_with("fn")) {
                    if let Some(t) = self.notes.get(h).cloned() {
                        self.b.runs.push(Run { text: OBJ.to_string(), st: CStyle { field: Some(Field::Note(t)), ..CStyle::default() } });
                        self.skip_a = true;
                    }
                }
            }
            _ => {}
        }
        if !VOID.contains(&n) {
            let st = tag_style(n, &at, &self.style());
            self.stack.push((name, st));
        }
    }

    fn img(&mut self, at: &[(String, String)]) {
        let src = attr(at, "src").unwrap_or("");
        let dim = |k: &str| attr(at, k).filter(|v| !v.contains('%')).and_then(|v| if v.trim().parse::<f32>().is_ok() { len_pt(&format!("{}px", v.trim())) } else { len_pt(v) });
        if let Some(d) = src.strip_prefix("data:") {
            if let Some((meta, data)) = d.split_once(',') {
                if meta.contains("base64") {
                    if let Some(bytes) = b64_dec(data) {
                        if self.tb.is_some() {
                            return;
                        }
                        self.flush(false);
                        let al = self.align;
                        self.b.image(bytes, dim("width"), dim("height"), al);
                        return;
                    }
                }
            }
        }
        if let Some(alt) = attr(at, "alt").filter(|a| !a.trim().is_empty()) {
            let st = self.style();
            self.text(format!("[{alt}]"));
            let _ = st;
        }
    }

    fn close(&mut self, name: String) {
        let n = name.as_str();
        match n {
            "head" | "title" => {
                if self.skip_name.as_deref() == Some(n) {
                    self.skip_name = None;
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "figcaption" => {
                self.flush(true);
                self.blk = None;
                self.align = Align::Left;
            }
            "p" => {
                let force = self.had_br && !self.split && self.lists.is_empty() && self.tb.is_none();
                self.flush(force);
                self.cell_nl();
                self.had_br = false;
                self.split = false;
                if self.blk == Some(Kind::Caption) {
                    self.blk = None;
                }
                self.align = Align::Left;
            }
            "li" => {
                self.flush(false);
                self.blk = None;
                self.align = Align::Left;
            }
            "ul" | "ol" => {
                self.flush(false);
                self.lists.pop();
                self.indent = self.lists.len().saturating_sub(1).min(8) as u8;
                self.blk = self.lists.last().map(|o| if *o { Kind::Number } else { Kind::Bullet });
            }
            "blockquote" => {
                self.flush(false);
                self.quote = self.quote.saturating_sub(1);
            }
            "div" => {
                self.flush(false);
                self.cell_nl();
                self.align = Align::Left;
            }
            "pre" => {
                self.flush(false);
                self.pre = false;
            }
            "td" | "th" if self.tdepth == 0 => self.end_cell(),
            "tr" if self.tdepth == 0 => self.end_row(),
            "table" => {
                if self.tdepth > 0 {
                    self.tdepth -= 1;
                } else {
                    self.end_table();
                }
            }
            "a" => self.skip_a = false,
            _ => {}
        }
        if let Some(i) = self.stack.iter().rposition(|(s, _)| s == n) {
            self.stack.truncate(i);
        }
    }
}

fn collapse_ws(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut sp = false;
    for c in s.chars() {
        if c.is_whitespace() && c != '\u{a0}' {
            if !sp {
                o.push(' ');
            }
            sp = true;
        } else {
            o.push(if c == '\u{a0}' { ' ' } else { c });
            sp = false;
        }
    }
    o
}

/// Zeichenformat nach dem Öffnen eines Elements.
fn tag_style(name: &str, at: &[(String, String)], base: &CStyle) -> CStyle {
    let mut st = base.clone();
    match name {
        "b" | "strong" => st.b = true,
        "i" | "em" | "cite" | "dfn" | "var" => st.i = true,
        "u" | "ins" => st.u = true,
        "s" | "strike" | "del" => st.s = true,
        "sup" => st.va = VAlign::Sup,
        "sub" => st.va = VAlign::Sub,
        "mark" => st.hl = 0xFFFFFF00,
        "a" => {
            if let Some(h) = attr(at, "href").filter(|h| !h.starts_with("#fn") && !h.trim().is_empty()) {
                st.link = Some(h.to_string());
            }
        }
        "font" => {
            if let Some(c) = attr(at, "color").and_then(parse_color) {
                st.color = c;
            }
        }
        _ => {}
    }
    if let Some(s) = attr(at, "style") {
        css_style(s, &mut st);
    }
    st
}

fn html_import(s: &str) -> (TextDoc, Images) {
    let mut toks = tokenize(s);
    let notes = extract_notes(&mut toks);
    let mut h = Hi {
        b: Bld::default(),
        stack: Vec::new(),
        lists: Vec::new(),
        quote: 0,
        blk: None,
        align: Align::Left,
        indent: 0,
        pre: false,
        skip_name: None,
        skip_a: false,
        notes,
        had_br: false,
        split: false,
        tb: None,
        tdepth: 0,
    };
    for t in toks {
        match t {
            Tok::Text(s) => h.text(s),
            Tok::Open(n, a) => h.open(n, a),
            Tok::Close(n) => h.close(n),
        }
    }
    h.end_table();
    h.flush(false);
    h.b.finish()
}

// ------------------------------------------------------------------ Markdown

fn md_esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '*' | '_' | '`' | '[' | ']' | '~' | '<' | '$' | '|') {
            o.push('\\');
        }
        o.push(c);
    }
    o
}

/// Anfang einer Zeile, der sonst als Auszeichnung gelesen würde.
fn md_block_esc(s: String) -> String {
    match s.chars().next() {
        Some('#' | '>' | '+' | '-') => format!("\\{s}"),
        Some(c) if c.is_ascii_digit() => {
            let d = s.chars().take_while(char::is_ascii_digit).count();
            if matches!(s[d..].chars().next(), Some('.' | ')')) {
                format!("{}\\{}", &s[..d], &s[d..])
            } else {
                s
            }
        }
        _ => s,
    }
}

fn md_run(t: &str, st: &CStyle) -> String {
    let core = t.trim();
    if core.is_empty() {
        return t.to_string();
    }
    let lead = &t[..t.len() - t.trim_start().len()];
    let trail = &t[t.trim_end().len()..];
    let mut s = md_esc(core);
    match st.va {
        VAlign::Sup => s = format!("<sup>{s}</sup>"),
        VAlign::Sub => s = format!("<sub>{s}</sub>"),
        VAlign::Base => {}
    }
    if st.u {
        s = format!("<u>{s}</u>");
    }
    if st.s {
        s = format!("~~{s}~~");
    }
    if st.i {
        s = format!("*{s}*");
    }
    if st.b {
        s = format!("**{s}**");
    }
    if let Some(l) = &st.link {
        s = format!("[{s}]({})", l.replace(' ', "%20").replace(')', "%29").replace('(', "%28"));
    }
    format!("{lead}{s}{trail}")
}

fn md_inline(segs: &[Seg]) -> String {
    let mut o = String::new();
    for g in segs {
        match g {
            Seg::T(t, st) => o.push_str(&md_run(t, st)),
            Seg::Note(n, _) => o.push_str(&format!("[^{n}]")),
            Seg::Page | Seg::Pages => o.push('#'),
        }
    }
    o
}

fn md_export(doc: &TextDoc, imgs: &Images) -> String {
    let mut cx = Cx::new(doc, imgs);
    let mut out = String::new();
    let mut notes: Vec<(u32, String)> = Vec::new();
    let mut counter = Counter::default();
    let has_title = doc.paras.iter().any(|p| p.kind == Kind::Title);
    let mut prev_list = false;
    let block = |out: &mut String, s: &str, list: bool, prev_list: &mut bool| {
        if !out.is_empty() && !(list && *prev_list) {
            out.push('\n');
        }
        out.push_str(s);
        out.push('\n');
        *prev_list = list;
    };
    for (pi, p) in doc.paras.iter().enumerate() {
        if !p.kind.is_list() {
            counter.reset();
        }
        match p.kind {
            Kind::PageBreak => block(&mut out, "\\pagebreak", false, &mut prev_list),
            Kind::Table => {
                if let Some(t) = &p.table {
                    block(&mut out, md_table(t).trim_end(), false, &mut prev_list);
                }
            }
            Kind::Image | Kind::Object => {
                if let Some(o) = p.obj.as_ref().filter(|o| matches!(o.kind, ObjKind::Formula { .. })) {
                    if let ObjKind::Formula { src } = &o.kind {
                        block(&mut out, &format!("$${src}$$"), false, &mut prev_list);
                    }
                } else if let Some((b, _, _)) = cx.picture(p, true) {
                    block(&mut out, &format!("![]({})", data_uri(&b)), false, &mut prev_list);
                } else if let Some(o) = &p.obj {
                    let t = obj_text(o);
                    block(&mut out, &if t.starts_with('[') { t } else { md_esc(&t) }, false, &mut prev_list);
                }
            }
            k => {
                let segs = cx.segs(pi);
                for g in &segs {
                    if let Seg::Note(n, t) = g {
                        notes.push((*n, t.clone()));
                    }
                }
                let text = md_inline(&segs);
                let lvl = if has_title { 1 } else { 0 };
                let line = match k {
                    Kind::Title => format!("# {text}"),
                    Kind::H1 => format!("{} {text}", "#".repeat(1 + lvl)),
                    Kind::H2 => format!("{} {text}", "#".repeat(2 + lvl)),
                    Kind::H3 => format!("{} {text}", "#".repeat(3 + lvl)),
                    Kind::Quote => format!("> {}", md_block_esc(text)),
                    Kind::Bullet | Kind::Number => {
                        let l = (p.indent as usize).min(8);
                        let n = counter.next(l, k == Kind::Number);
                        let mark = if k == Kind::Number { format!("{n}.") } else { "-".into() };
                        format!("{}{mark} {text}", "    ".repeat(l))
                    }
                    _ => md_block_esc(text),
                };
                if text_is_blank(&line, k) {
                    continue;
                }
                block(&mut out, &line, k.is_list(), &mut prev_list);
            }
        }
    }
    if !notes.is_empty() {
        out.push('\n');
        for (n, t) in &notes {
            out.push_str(&format!("[^{n}]: {}\n", t.replace('\n', " ")));
        }
    }
    out
}

/// Leerer Absatz (kein Eintrag nötig).
fn text_is_blank(line: &str, k: Kind) -> bool {
    (!k.is_list() && !k.is_heading() && line.trim().is_empty()) || (k == Kind::Quote && line.trim() == ">")
}

fn md_table(t: &Table) -> String {
    let n = t.ncols().max(1);
    let cell = |c: &Cell| {
        let st = CStyle { b: c.b, i: c.i, ..CStyle::default() };
        c.text.split('\n').map(|l| md_run(l, &st)).collect::<Vec<_>>().join("<br>")
    };
    let row = |r: &[Cell]| format!("| {} |", (0..n).map(|i| r.get(i).map(cell).unwrap_or_default()).collect::<Vec<_>>().join(" | "));
    let mut o = String::new();
    for (i, r) in t.rows.iter().enumerate() {
        o.push_str(&row(r));
        o.push('\n');
        if i == 0 {
            let seps: Vec<&str> = (0..n)
                .map(|c| match t.rows.get(1).or(t.rows.first()).and_then(|r| r.get(c)).map(|x| x.align).unwrap_or_default() {
                    Align::Center => ":---:",
                    Align::Right => "---:",
                    _ => "---",
                })
                .collect();
            o.push_str(&format!("| {} |\n", seps.join(" | ")));
        }
    }
    o
}

/// Schließende Markierung `m` (k Zeichen) ab `from` suchen.
fn md_close(c: &[char], from: usize, m: char, k: usize) -> Option<usize> {
    let mut j = from;
    while j + k <= c.len() {
        if c[j] == '\\' {
            j += 2;
            continue;
        }
        if c[j] == m && j > from {
            let run = c[j..].iter().take_while(|x| **x == m).count();
            if run >= k && !c[j - 1].is_whitespace() {
                let after = c.get(j + k).copied();
                if m != '_' || !after.is_some_and(char::is_alphanumeric) {
                    // bei k == 1 Doppelmarkierungen überspringen
                    if k == 1 && run >= 2 {
                        j += run;
                        continue;
                    }
                    return Some(j);
                }
            }
            j += run.max(1);
            continue;
        }
        j += 1;
    }
    None
}

fn md_starts(c: &[char], i: usize, s: &str) -> bool {
    s.chars().enumerate().all(|(k, ch)| c.get(i + k) == Some(&ch))
}

/// Klammerpaar ab `i` (c[i] ist die öffnende) → Index der schließenden.
fn md_bracket(c: &[char], i: usize, open: char, close: char) -> Option<usize> {
    let mut d = 0;
    let mut j = i;
    while j < c.len() {
        if c[j] == '\\' {
            j += 2;
            continue;
        }
        if c[j] == open {
            d += 1;
        } else if c[j] == close {
            d -= 1;
            if d == 0 {
                return Some(j);
            }
        }
        j += 1;
    }
    None
}

fn md_inl(c: &[char], st: &CStyle, notes: &HashMap<String, String>, out: &mut Vec<Run>) {
    let mut lit = String::new();
    let mut i = 0;
    macro_rules! put {
        () => {
            if !lit.is_empty() {
                out.push(Run { text: std::mem::take(&mut lit), st: st.clone() });
            }
        };
    }
    while i < c.len() {
        let ch = c[i];
        match ch {
            '\\' if c.get(i + 1).is_some_and(char::is_ascii_punctuation) => {
                lit.push(c[i + 1]);
                i += 2;
            }
            '`' => match c[i + 1..].iter().position(|x| *x == '`') {
                Some(e) if e > 0 => {
                    lit.extend(&c[i + 1..i + 1 + e]);
                    i += e + 2;
                }
                _ => {
                    lit.push(ch);
                    i += 1;
                }
            },
            '*' | '_' | '~' => {
                let run = c[i..].iter().take_while(|x| **x == ch).count();
                let k = if ch == '~' { if run >= 2 { 2 } else { 0 } } else { run.min(3) };
                let next_ok = c.get(i + k).is_some_and(|x| !x.is_whitespace());
                let prev_ok = ch != '_' || i == 0 || !c[i - 1].is_alphanumeric();
                let close = if k > 0 && next_ok && prev_ok { md_close(c, i + k, ch, k) } else { None };
                match close {
                    Some(j) => {
                        put!();
                        let mut s2 = st.clone();
                        match (ch, k) {
                            ('~', _) => s2.s = true,
                            (_, 1) => s2.i = true,
                            (_, 2) => s2.b = true,
                            _ => {
                                s2.b = true;
                                s2.i = true;
                            }
                        }
                        md_inl(&c[i + k..j], &s2, notes, out);
                        i = j + k;
                    }
                    None => {
                        let n = run.max(1);
                        lit.extend(std::iter::repeat_n(ch, n));
                        i += n;
                    }
                }
            }
            '!' | '[' => {
                let img = ch == '!' && c.get(i + 1) == Some(&'[');
                let o = if img { i + 1 } else { i };
                if ch == '!' && !img {
                    lit.push(ch);
                    i += 1;
                    continue;
                }
                if c.get(o + 1) == Some(&'^') && !img {
                    if let Some(e) = c[o..].iter().position(|x| *x == ']') {
                        let id: String = c[o + 2..o + e].iter().collect();
                        if let Some(t) = notes.get(&id) {
                            put!();
                            out.push(Run { text: OBJ.to_string(), st: CStyle { field: Some(Field::Note(t.clone())), ..CStyle::default() } });
                            i = o + e + 1;
                            continue;
                        }
                    }
                }
                let close = md_bracket(c, o, '[', ']');
                let link = close.filter(|j| c.get(j + 1) == Some(&'(')).and_then(|j| md_bracket(c, j + 1, '(', ')').map(|e| (j, e)));
                match link {
                    Some((j, e)) => {
                        put!();
                        let url: String = c[j + 2..e].iter().collect();
                        let url = url.trim();
                        let url = url.split(" \"").next().unwrap_or(url).trim().trim_matches(['<', '>']).to_string();
                        if img {
                            let alt: String = c[o + 1..j].iter().collect();
                            if !alt.trim().is_empty() {
                                out.push(Run { text: alt, st: st.clone() });
                            }
                        } else {
                            let mut s2 = st.clone();
                            if !url.is_empty() {
                                s2.link = Some(url);
                            }
                            md_inl(&c[o + 1..j], &s2, notes, out);
                        }
                        i = e + 1;
                    }
                    None => {
                        lit.push(ch);
                        i += 1;
                    }
                }
            }
            '<' => {
                let tag = [("<u>", "</u>"), ("<sup>", "</sup>"), ("<sub>", "</sub>")].into_iter().find(|(o, _)| md_starts(c, i, o));
                if let Some((o, cl)) = tag {
                    let a = i + o.len();
                    let end = (a..c.len()).find(|&j| md_starts(c, j, cl));
                    if let Some(e) = end {
                        put!();
                        let mut s2 = st.clone();
                        match o {
                            "<u>" => s2.u = true,
                            "<sup>" => s2.va = VAlign::Sup,
                            _ => s2.va = VAlign::Sub,
                        }
                        md_inl(&c[a..e], &s2, notes, out);
                        i = e + cl.len();
                        continue;
                    }
                }
                if md_starts(c, i, "<br>") || md_starts(c, i, "<br/>") || md_starts(c, i, "<br />") {
                    lit.push('\n');
                    i += c[i..].iter().position(|x| *x == '>').unwrap_or(0) + 1;
                    continue;
                }
                // <https://…>
                if let Some(e) = c[i..].iter().position(|x| *x == '>') {
                    let u: String = c[i + 1..i + e].iter().collect();
                    if (u.starts_with("http://") || u.starts_with("https://") || u.starts_with("mailto:")) && !u.contains(' ') {
                        put!();
                        let s2 = CStyle { link: Some(u.clone()), ..st.clone() };
                        out.push(Run { text: u, st: s2 });
                        i += e + 1;
                        continue;
                    }
                }
                lit.push(ch);
                i += 1;
            }
            _ => {
                lit.push(ch);
                i += 1;
            }
        }
    }
    put!();
}

fn md_runs(s: &str, notes: &HashMap<String, String>) -> Vec<Run> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    md_inl(&c, &CStyle::default(), notes, &mut out);
    for r in &mut out {
        r.text = r.text.replace('\n', " ");
    }
    out
}

/// Zeilen einer Tabellenzeile `| a | b |` (maskierte `|` bleiben).
fn md_cells(l: &str) -> Vec<String> {
    let l = l.trim();
    let l = l.strip_prefix('|').unwrap_or(l);
    let l = if l.ends_with('|') && !l.ends_with("\\|") { &l[..l.len() - 1] } else { l };
    let mut v = vec![String::new()];
    let mut esc = false;
    for c in l.chars() {
        if esc {
            if c != '|' {
                v.last_mut().unwrap().push('\\');
            }
            v.last_mut().unwrap().push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '|' {
            v.push(String::new());
        } else {
            v.last_mut().unwrap().push(c);
        }
    }
    v.into_iter().map(|s| s.trim().to_string()).collect()
}

fn md_is_sep(l: &str) -> bool {
    let t = l.trim();
    t.contains('-') && t.contains('|') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// Listenmarke am Zeilenanfang: (Einrückung, Länge der Marke, nummeriert).
fn md_list_mark(l: &str) -> Option<(usize, usize, bool)> {
    let ind: usize = l.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum();
    let t = l.trim_start();
    let b = t.as_bytes();
    if b.len() >= 2 && matches!(b[0], b'-' | b'*' | b'+') && b[1] == b' ' {
        return Some((ind, 2, false));
    }
    let d = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if d > 0 && d < 10 && b.len() > d + 1 && matches!(b[d], b'.' | b')') && b[d + 1] == b' ' {
        return Some((ind, d + 2, true));
    }
    None
}

fn md_hr(l: &str) -> bool {
    let t: String = l.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3 && ['-', '*', '_'].iter().any(|m| t.chars().all(|c| c == *m))
}

#[allow(unused_assignments)]
fn md_import(s: &str) -> (TextDoc, Images) {
    let mut notes: HashMap<String, String> = HashMap::new();
    let mut lines: Vec<&str> = Vec::new();
    for l in s.lines() {
        match l.strip_prefix("[^").and_then(|r| r.split_once("]:")) {
            Some((id, t)) => {
                notes.insert(id.to_string(), t.trim().to_string());
            }
            None => lines.push(l),
        }
    }
    // „# Titel“ + „## Kapitel“: einzelne Ebene 1 ist der Titel
    let heads: Vec<usize> = lines.iter().filter_map(|l| md_heading(l).map(|h| h.0)).collect();
    let first_one = heads.first() == Some(&1);
    let title_mode = first_one && heads.iter().filter(|h| **h == 1).count() == 1 && heads.contains(&2);
    let mut b = Bld::default();
    let mut buf = String::new();
    let mut bkind = Kind::Body;
    let mut bind = 0u8;
    let mut stack: Vec<usize> = Vec::new();
    let mut i = 0;
    let mut code = false;
    macro_rules! flush {
        () => {
            if !buf.trim().is_empty() {
                let mut runs = md_runs(buf.trim(), &notes);
                let cap = if bkind == Kind::Caption { strip_caption(&mut runs) } else { None };
                let mut p = Para { kind: bkind, indent: if bkind.is_list() { bind } else { 0 }, runs, cap: cap.unwrap_or_default(), ..Para::default() };
                p.normalize();
                b.paras.push(p);
            }
            if !bkind.is_list() {
                stack.clear();
            }
            buf.clear();
            bkind = Kind::Body;
        };
    }
    while i < lines.len() {
        let l = lines[i];
        i += 1;
        let t = l.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            flush!();
            code = !code;
            continue;
        }
        if code {
            b.paras.push(Para::with_text(Kind::Body, l));
            continue;
        }
        if t.is_empty() {
            flush!();
            continue;
        }
        if t == "\\pagebreak" || t == "\\newpage" {
            flush!();
            b.paras.push(Para::new(Kind::PageBreak));
            continue;
        }
        if let Some((lv, text)) = md_heading(l) {
            flush!();
            let kind = match (title_mode, lv) {
                (true, 1) => Kind::Title,
                (true, 2) | (false, 1) => Kind::H1,
                (true, 3) | (false, 2) => Kind::H2,
                _ => Kind::H3,
            };
            let mut p = Para { kind, runs: md_runs(&text, &notes), ..Para::default() };
            p.normalize();
            b.paras.push(p);
            continue;
        }
        if md_hr(t) && !t.starts_with('|') {
            flush!();
            continue;
        }
        if t.starts_with("$$") {
            let one = t.len() > 4 && t.ends_with("$$");
            let src = if one {
                Some(t[2..t.len() - 2].trim().to_string())
            } else if t == "$$" {
                let mut v = Vec::new();
                while i < lines.len() && lines[i].trim() != "$$" {
                    v.push(lines[i].trim());
                    i += 1;
                }
                i += 1;
                Some(v.join(" "))
            } else {
                None
            };
            if let Some(src) = src.filter(|s| !s.is_empty()) {
                flush!();
                b.paras.push(formula_para(&src));
                continue;
            }
        }
        if t.starts_with("![") && t.ends_with(')') && t.contains("](data:") {
            if let Some(d) = t.split_once("](data:").map(|x| x.1.trim_end_matches(')')) {
                if let Some(bytes) = d.split_once(',').and_then(|x| b64_dec(x.1)) {
                    flush!();
                    b.image(bytes, None, None, Align::Center);
                    continue;
                }
            }
        }
        if t.contains('|') && i < lines.len() && md_is_sep(lines[i]) && !md_is_sep(t) {
            flush!();
            let head = md_cells(t);
            let aligns: Vec<Align> = md_cells(lines[i])
                .iter()
                .map(|c| match (c.starts_with(':'), c.ends_with(':')) {
                    (true, true) => Align::Center,
                    (false, true) => Align::Right,
                    _ => Align::Left,
                })
                .collect();
            i += 1;
            let mut rows = vec![head];
            while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                rows.push(md_cells(lines[i]));
                i += 1;
            }
            let rows: Vec<Vec<Cell>> = rows
                .into_iter()
                .map(|r| {
                    r.iter()
                        .enumerate()
                        .map(|(ci, s)| {
                            let mut cb = CellB { align: aligns.get(ci).copied().unwrap_or_default(), ..CellB::default() };
                            for run in md_runs(s, &notes) {
                                cb.add(&run.text, &run.st);
                            }
                            cb.done()
                        })
                        .collect()
                })
                .collect();
            if let Some(p) = make_table(rows, true) {
                b.paras.push(p);
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix('>') {
            if bkind != Kind::Quote {
                flush!();
            }
            let q = rest.strip_prefix(' ').unwrap_or(rest);
            if q.trim().is_empty() {
                flush!();
                continue;
            }
            bkind = Kind::Quote;
            if !buf.is_empty() {
                buf.push(' ');
            }
            buf.push_str(q.trim());
            continue;
        }
        if let Some((ind, n, ordered)) = md_list_mark(l) {
            flush!();
            while stack.last().is_some_and(|x| ind < *x) {
                stack.pop();
            }
            if stack.last().is_none_or(|x| ind > *x) {
                stack.push(ind);
            }
            bind = (stack.len() - 1).min(8) as u8;
            bkind = if ordered { Kind::Number } else { Kind::Bullet };
            let text = l.trim_start()[n..].trim();
            let text = text.strip_prefix("[ ] ").map(|r| format!("□ {r}")).or_else(|| text.strip_prefix("[x] ").or(text.strip_prefix("[X] ")).map(|r| format!("■ {r}"))).unwrap_or_else(|| text.to_string());
            buf.push_str(&text);
            continue;
        }
        // Fortsetzung bzw. normaler Absatz
        if !buf.is_empty() {
            buf.push(' ');
        }
        buf.push_str(t);
        if l.ends_with("  ") || l.ends_with('\\') {
            if l.ends_with('\\') {
                buf.pop();
            }
            flush!();
        }
    }
    flush!();
    b.finish()
}

/// Überschrift „## Text“ → (Ebene, Text).
fn md_heading(l: &str) -> Option<(usize, String)> {
    let t = l.trim_start();
    let n = t.chars().take_while(|c| *c == '#').count();
    if n == 0 || n > 6 || l.len() - t.len() > 3 {
        return None;
    }
    let rest = &t[n..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((n, rest.trim().trim_end_matches('#').trim_end().to_string()))
}

// ------------------------------------------------------------------ Reiner Text

fn txt_export(doc: &TextDoc) -> String {
    let imgs = Images::new();
    let mut cx = Cx::new(doc, &imgs);
    let mut out: Vec<String> = Vec::new();
    let mut notes: Vec<(u32, String)> = Vec::new();
    let mut counter = Counter::default();
    for (pi, p) in doc.paras.iter().enumerate() {
        if !p.kind.is_list() {
            counter.reset();
        }
        match p.kind {
            Kind::PageBreak => out.push("\u{c}".into()),
            Kind::Table => {
                if let Some(t) = &p.table {
                    for r in &t.rows {
                        out.push(r.iter().map(|c| c.text.replace('\n', " ")).collect::<Vec<_>>().join("\t"));
                    }
                }
            }
            Kind::Image => out.push("[Bild]".into()),
            Kind::Object => {
                if let Some(o) = &p.obj {
                    out.push(obj_text(o));
                }
            }
            k => {
                let segs = cx.segs(pi);
                for g in &segs {
                    if let Seg::Note(n, t) = g {
                        notes.push((*n, t.clone()));
                    }
                }
                let t = seg_plain(&segs);
                out.push(match k {
                    Kind::Bullet => format!("{}• {t}", "  ".repeat((p.indent as usize).min(8))),
                    Kind::Number => {
                        let l = (p.indent as usize).min(8);
                        format!("{}{}. {t}", "  ".repeat(l), counter.next(l, true))
                    }
                    Kind::Quote => format!("> {t}"),
                    _ => t,
                });
            }
        }
    }
    if !notes.is_empty() {
        out.push(String::new());
        out.push("Fußnoten:".into());
        for (n, t) in notes {
            out.push(format!("[{n}] {t}"));
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

fn txt_import(s: &str) -> (TextDoc, Images) {
    let mut lines: Vec<&str> = s.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    if lines.last() == Some(&"") && lines.len() > 1 {
        lines.pop();
    }
    let mut d = TextDoc::new();
    d.paras = Vec::new();
    for l in lines {
        for (i, part) in l.split('\u{c}').enumerate() {
            if i > 0 {
                d.paras.push(Para::new(Kind::PageBreak));
            }
            if i == 0 || !part.is_empty() {
                let clean: String = part.chars().filter(|c| !c.is_control() || *c == '\t').collect();
                d.paras.push(Para::with_text(Kind::Body, &clean));
            }
        }
    }
    if d.paras.is_empty() {
        d.paras.push(Para::new(Kind::Body));
    }
    (d, Images::new())
}

// ------------------------------------------------------------------ Kopf-/Fußzeile

enum Hf {
    T(String),
    Page,
    Pages,
}

/// Kopf-/Fußzeilentext in Teile zerlegen (`{seite}`, `{seiten}`).
fn hf_parts(s: &str) -> Vec<Hf> {
    let mut v = Vec::new();
    let mut rest = s;
    loop {
        let (pos, len, h) = match (rest.find("{seite}"), rest.find("{seiten}")) {
            (Some(a), Some(b)) if b < a => (b, 8, Hf::Pages),
            (Some(a), _) => (a, 7, Hf::Page),
            (None, Some(b)) => (b, 8, Hf::Pages),
            (None, None) => {
                if !rest.is_empty() {
                    v.push(Hf::T(rest.to_string()));
                }
                return v;
            }
        };
        if pos > 0 {
            v.push(Hf::T(rest[..pos].to_string()));
        }
        v.push(h);
        rest = &rest[pos + len..];
    }
}

// ------------------------------------------------------------------ ODT: Schreiben

const ODT_NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0""#;

const ODT_LISTS: ListTags = ListTags {
    open: |o| if o { r#"<text:list text:style-name="LN">"# } else { r#"<text:list text:style-name="LB">"# },
    close: |_| "</text:list>",
    li: "<text:list-item>",
    li_end: "</text:list-item>",
};

/// Automatische Formatvorlagen, nach Inhalt zusammengefasst.
#[derive(Default)]
struct Auto {
    items: String,
    map: HashMap<String, String>,
}

impl Auto {
    fn get(&mut self, prefix: &str, key: String, xml: impl FnOnce(&str) -> String) -> String {
        let key = format!("{prefix}|{key}");
        if let Some(n) = self.map.get(&key) {
            return n.clone();
        }
        let name = format!("{prefix}{}", self.map.len() + 1);
        self.items.push_str(&xml(&name));
        self.map.insert(key, name.clone());
        name
    }
}

fn odt_text_props(st: &CStyle) -> String {
    let mut p = String::new();
    if st.b {
        p.push_str(r#" fo:font-weight="bold""#);
    }
    if st.i {
        p.push_str(r#" fo:font-style="italic""#);
    }
    if st.u {
        p.push_str(r#" style:text-underline-style="solid" style:text-underline-width="auto" style:text-underline-color="font-color""#);
    }
    if st.s {
        p.push_str(r#" style:text-line-through-style="solid""#);
    }
    match st.va {
        VAlign::Sup => p.push_str(r#" style:text-position="super 58%""#),
        VAlign::Sub => p.push_str(r#" style:text-position="sub 58%""#),
        VAlign::Base => {}
    }
    if st.color != 0 {
        p.push_str(&format!(r#" fo:color="{}""#, hex6(st.color)));
    }
    if st.hl != 0 {
        p.push_str(&format!(r#" fo:background-color="{}""#, hex6(st.hl)));
    }
    if let Some(sz) = st.size {
        p.push_str(&format!(r#" fo:font-size="{}pt""#, (sz * 10.0).round() / 10.0));
    }
    if let Some(f) = st.fam {
        p.push_str(&format!(r#" fo:font-family="{}""#, f.word_name()));
    }
    p
}

/// Text mit Tabulatoren und mehrfachen Leerzeichen.
fn odt_text(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut prev_space = true;
    for c in s.chars() {
        match c {
            '\t' => {
                o.push_str("<text:tab/>");
                prev_space = false;
            }
            '\n' => {
                o.push_str("<text:line-break/>");
                prev_space = false;
            }
            ' ' => {
                o.push_str(if prev_space { "<text:s/>" } else { " " });
                prev_space = true;
            }
            c => {
                if (c as u32) >= 0x20 {
                    o.push_str(&esc(c.encode_utf8(&mut [0; 4])));
                }
                prev_space = false;
            }
        }
    }
    o
}

fn odt_inline(segs: &[Seg], auto: &mut Auto) -> String {
    let mut o = String::new();
    for g in segs {
        match g {
            Seg::T(t, st) => {
                let props = odt_text_props(st);
                let mut s = odt_text(t);
                if !props.is_empty() {
                    let name = auto.get("T", props.clone(), |n| format!(r#"<style:style style:name="{n}" style:family="text"><style:text-properties{props}/></style:style>"#));
                    s = format!(r#"<text:span text:style-name="{name}">{s}</text:span>"#);
                }
                if let Some(l) = &st.link {
                    s = format!(r#"<text:a xlink:type="simple" xlink:href="{}">{s}</text:a>"#, esc(l));
                }
                o.push_str(&s);
            }
            Seg::Note(n, t) => o.push_str(&format!(r#"<text:note text:id="ftn{n}" text:note-class="footnote"><text:note-citation>{n}</text:note-citation><text:note-body><text:p text:style-name="Standard">{}</text:p></text:note-body></text:note>"#, odt_text(t))),
            Seg::Page => o.push_str(r#"<text:page-number text:select-page="current">1</text:page-number>"#),
            Seg::Pages => o.push_str("<text:page-count>1</text:page-count>"),
        }
    }
    o
}

fn odt_hf(s: &str) -> String {
    hf_parts(s)
        .iter()
        .map(|h| match h {
            Hf::T(t) => odt_text(t),
            Hf::Page => r#"<text:page-number text:select-page="current">1</text:page-number>"#.to_string(),
            Hf::Pages => "<text:page-count>1</text:page-count>".to_string(),
        })
        .collect()
}

/// Absatzvorlage mit Ausrichtung (und Seitenumbruch danach) als automatische Vorlage.
fn odt_pstyle(auto: &mut Auto, base: &str, align: Align, brk: bool) -> String {
    if align == Align::Left && !brk {
        return base.to_string();
    }
    let mut pp = String::new();
    if let Some(a) = al_css(align) {
        pp.push_str(&format!(r#" fo:text-align="{a}""#));
    }
    if brk {
        pp.push_str(r#" fo:break-after="page""#);
    }
    auto.get("P", format!("{base}|{pp}"), |n| format!(r#"<style:style style:name="{n}" style:family="paragraph" style:parent-style-name="{base}"><style:paragraph-properties{pp}/></style:style>"#))
}

fn odt_table(t: &Table, auto: &mut Auto, n: usize) -> String {
    let total: f32 = t.cols.iter().sum();
    let tname = auto.get("Tbl", format!("{n}"), |nm| format!(r#"<style:style style:name="{nm}" style:family="table"><style:table-properties style:width="{total:.1}pt" table:align="left"/></style:style>"#));
    let mut o = format!(r#"<table:table table:name="Tabelle{n}" table:style-name="{tname}">"#);
    for w in &t.cols {
        let cn = auto.get("Col", format!("{w:.1}"), |nm| format!(r#"<style:style style:name="{nm}" style:family="table-column"><style:table-column-properties style:column-width="{w:.1}pt"/></style:style>"#));
        o.push_str(&format!(r#"<table:table-column table:style-name="{cn}"/>"#));
    }
    let border = if t.border != 0 { format!("0.5pt solid {}", hex6(t.border)) } else { "none".into() };
    for (ri, row) in t.rows.iter().enumerate() {
        let hdr = t.header && ri == 0;
        if hdr {
            o.push_str("<table:table-header-rows>");
        }
        o.push_str("<table:table-row>");
        for c in row {
            let bg = if c.bg != 0 {
                c.bg
            } else if hdr {
                HEAD_BG
            } else {
                0
            };
            let bgp = if bg != 0 { format!(r#" fo:background-color="{}""#, hex6(bg)) } else { String::new() };
            let cs = auto.get("Ce", format!("{bgp}|{border}"), |nm| format!(r#"<style:style style:name="{nm}" style:family="table-cell"><style:table-cell-properties fo:padding="0.1cm" fo:border="{border}"{bgp}/></style:style>"#));
            let st = CStyle { b: c.b || hdr, i: c.i, color: c.color, ..CStyle::default() };
            let tp = odt_text_props(&st);
            let ps = auto.get("CP", format!("{tp}|{:?}", c.align), |nm| {
                let al = al_css(c.align).map(|a| format!(r#" fo:text-align="{a}""#)).unwrap_or_default();
                format!(r#"<style:style style:name="{nm}" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties{al}/><style:text-properties{tp}/></style:style>"#)
            });
            o.push_str(&format!(r#"<table:table-cell table:style-name="{cs}" office:value-type="string">"#));
            for line in c.text.split('\n') {
                o.push_str(&format!(r#"<text:p text:style-name="{ps}">{}</text:p>"#, odt_text(line)));
            }
            o.push_str("</table:table-cell>");
        }
        o.push_str("</table:table-row>");
        if hdr {
            o.push_str("</table:table-header-rows>");
        }
    }
    o.push_str("</table:table>");
    o
}

fn odt_export(doc: &TextDoc, title: &str, imgs: &Images) -> Result<Vec<u8>, String> {
    let mut cx = Cx::new(doc, imgs);
    let mut auto = Auto::default();
    let mut body = String::new();
    let mut lists = Lists::default();
    let mut pics: Vec<(String, Vec<u8>)> = Vec::new();
    let mut ntab = 0;
    for (pi, p) in doc.paras.iter().enumerate() {
        if !p.kind.is_list() {
            lists.close_all(&ODT_LISTS, &mut body);
        }
        match p.kind {
            Kind::PageBreak => {
                let s = odt_pstyle(&mut auto, "Standard", Align::Left, true);
                body.push_str(&format!(r#"<text:p text:style-name="{s}"/>"#));
            }
            Kind::Table => {
                if let Some(t) = &p.table {
                    ntab += 1;
                    body.push_str(&odt_table(t, &mut auto, ntab));
                }
            }
            Kind::Image | Kind::Object => {
                if let Some((b, w, h)) = cx.picture(p, false) {
                    let name = format!("Pictures/img{}.{}", pics.len() + 1, sniff(&b).1);
                    let s = odt_pstyle(&mut auto, "Standard", p.align, false);
                    body.push_str(&format!(
                        r#"<text:p text:style-name="{s}"><draw:frame draw:name="Bild{}" text:anchor-type="as-char" svg:width="{w:.1}pt" svg:height="{h:.1}pt"><draw:image xlink:href="{name}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/></draw:frame></text:p>"#,
                        pics.len() + 1
                    ));
                    pics.push((name, b.into_owned()));
                } else if let Some(o) = &p.obj {
                    let s = odt_pstyle(&mut auto, "Standard", p.align, false);
                    body.push_str(&format!(r#"<text:p text:style-name="{s}">{}</text:p>"#, odt_text(&obj_text(o))));
                }
            }
            k => {
                let segs = cx.segs(pi);
                let inner = odt_inline(&segs, &mut auto);
                let (base, lvl) = match k {
                    Kind::Title => ("Title", 0),
                    Kind::H1 => ("Heading_20_1", 1),
                    Kind::H2 => ("Heading_20_2", 2),
                    Kind::H3 => ("Heading_20_3", 3),
                    Kind::Quote => ("Quotations", 0),
                    Kind::Caption => ("Caption", 0),
                    _ => ("Standard", 0),
                };
                let s = odt_pstyle(&mut auto, base, p.align, false);
                if k.is_list() {
                    lists.enter((p.indent as usize).min(8), k == Kind::Number, &ODT_LISTS, &mut body);
                }
                if lvl > 0 {
                    body.push_str(&format!(r#"<text:h text:style-name="{s}" text:outline-level="{lvl}">{inner}</text:h>"#));
                } else {
                    body.push_str(&format!(r#"<text:p text:style-name="{s}">{inner}</text:p>"#));
                }
            }
        }
    }
    lists.close_all(&ODT_LISTS, &mut body);
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content {ODT_NS} office:version="1.2"><office:automatic-styles>{}</office:automatic-styles><office:body><office:text>{body}</office:text></office:body></office:document-content>"#,
        auto.items
    );
    let styles = odt_styles(doc);
    let meta = format!(r#"<?xml version="1.0" encoding="UTF-8"?><office:document-meta {ODT_NS} office:version="1.2"><office:meta><dc:title>{}</dc:title><meta:generator>OwnDraw</meta:generator></office:meta></office:document-meta>"#, esc(title));
    let mut manifest = String::from(r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/>"#);
    for (n, b) in &pics {
        manifest.push_str(&format!(r#"<manifest:file-entry manifest:full-path="{n}" manifest:media-type="{}"/>"#, sniff(b).0));
    }
    manifest.push_str("</manifest:manifest>");
    let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut put = |name: &str, data: &[u8], opt: SimpleFileOptions| -> Result<(), String> {
        z.start_file(name, opt).map_err(|e| e.to_string())?;
        z.write_all(data).map_err(|e| e.to_string())
    };
    put("mimetype", b"application/vnd.oasis.opendocument.text", stored)?;
    put("META-INF/manifest.xml", manifest.as_bytes(), deflated)?;
    put("content.xml", content.as_bytes(), deflated)?;
    put("styles.xml", styles.as_bytes(), deflated)?;
    put("meta.xml", meta.as_bytes(), deflated)?;
    for (n, b) in &pics {
        put(n, b, stored)?;
    }
    Ok(z.finish().map_err(|e| e.to_string())?.into_inner())
}

/// styles.xml: Formatvorlagen, Listen, Seite, Kopf- und Fußzeile.
fn odt_styles(doc: &TextDoc) -> String {
    let font = doc.family.word_name();
    let sans = Family::Sans.word_name();
    let size = doc.size;
    let cap = (size - 1.5).max(6.0);
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles {ODT_NS} office:version="1.2"><office:styles><style:style style:name="Standard" style:family="paragraph" style:class="text"><style:paragraph-properties fo:margin-top="0cm" fo:margin-bottom="0.2cm" fo:line-height="115%"/><style:text-properties fo:font-family="{font}" fo:font-size="{size}pt"/></style:style>"#
    );
    let head = |name: &str, disp: &str, lvl: u8, sz: f32, mt: f32, mb: f32| {
        let o = if lvl > 0 { format!(r#" style:default-outline-level="{lvl}""#) } else { String::new() };
        format!(r#"<style:style style:name="{name}" style:display-name="{disp}" style:family="paragraph" style:parent-style-name="Standard" style:next-style-name="Standard"{o}><style:paragraph-properties fo:margin-top="{:.2}cm" fo:margin-bottom="{:.2}cm" fo:keep-with-next="always"/><style:text-properties fo:font-family="{sans}" fo:font-size="{sz}pt" fo:font-weight="bold"/></style:style>"#, mt / 28.35, mb / 28.35)
    };
    s.push_str(&head("Title", "Title", 0, 26.0, 0.0, 14.0));
    s.push_str(&head("Heading_20_1", "Heading 1", 1, 18.0, 18.0, 6.0));
    s.push_str(&head("Heading_20_2", "Heading 2", 2, 14.0, 14.0, 4.0));
    s.push_str(&head("Heading_20_3", "Heading 3", 3, 12.0, 10.0, 3.0));
    s.push_str(&format!(r##"<style:style style:name="Quotations" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-left="1cm" fo:margin-top="0.15cm"/><style:text-properties fo:font-style="italic" fo:color="#4a4a55"/></style:style><style:style style:name="Caption" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-top="0.07cm" fo:margin-bottom="0.35cm"/><style:text-properties fo:font-family="{sans}" fo:font-size="{cap}pt" fo:font-style="italic" fo:color="#4a4a55"/></style:style>"##));
    for (name, bullet) in [("LB", true), ("LN", false)] {
        s.push_str(&format!(r#"<text:list-style style:name="{name}">"#));
        for l in 0..9usize {
            let pos = 0.635 + 0.635 * (l as f32 + 1.0);
            let props = format!(r#"<style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" text:list-tab-stop-position="{pos:.3}cm" fo:text-indent="-0.635cm" fo:margin-left="{pos:.3}cm"/></style:list-level-properties>"#);
            if bullet {
                s.push_str(&format!(r#"<text:list-level-style-bullet text:level="{}" text:bullet-char="{}">{props}</text:list-level-style-bullet>"#, l + 1, ["•", "◦", "▪"][l % 3]));
            } else {
                let (f, suf) = [("1", "."), ("a", ")"), ("i", ".")][l % 3];
                s.push_str(&format!(r#"<text:list-level-style-number text:level="{}" style:num-suffix="{suf}" style:num-format="{f}">{props}</text:list-level-style-number>"#, l + 1));
            }
        }
        s.push_str("</text:list-style>");
    }
    let p = &doc.page;
    let (hf, ff) = (!doc.header.is_empty(), !doc.footer.is_empty());
    let hs = if hf { r#"<style:header-style><style:header-footer-properties fo:min-height="0cm" fo:margin-bottom="0.3cm"/></style:header-style>"# } else { "" };
    let fs = if ff { r#"<style:footer-style><style:header-footer-properties fo:min-height="0cm" fo:margin-top="0.3cm"/></style:footer-style>"# } else { "" };
    s.push_str(&format!(
        r#"</office:styles><office:automatic-styles><style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="{:.2}pt" fo:page-height="{:.2}pt" fo:margin-top="{:.2}pt" fo:margin-bottom="{:.2}pt" fo:margin-left="{:.2}pt" fo:margin-right="{:.2}pt"/>{hs}{fs}</style:page-layout></office:automatic-styles><office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1">"#,
        p.w, p.h, p.mt, p.mb, p.ml, p.mr
    ));
    if hf {
        s.push_str(&format!(r#"<style:header><text:p text:style-name="Standard">{}</text:p></style:header>"#, odt_hf(&doc.header)));
    }
    if ff {
        s.push_str(&format!(r#"<style:footer><text:p text:style-name="Standard">{}</text:p></style:footer>"#, odt_hf(&doc.footer)));
    }
    s.push_str("</style:master-page></office:master-styles></office:document-styles>");
    s
}

// ------------------------------------------------------------------ ODT: Lesen

struct OSt {
    parent: Option<String>,
    family: String,
    auto: bool,
    props: HashMap<String, String>,
}

struct OCx<'a> {
    styles: HashMap<String, OSt>,
    lists: HashMap<String, Vec<bool>>,
    z: std::cell::RefCell<zip::ZipArchive<Cursor<&'a [u8]>>>,
    store: std::cell::RefCell<crate::doc::ImageStore>,
}

type XNode<'a, 'b> = roxmltree::Node<'a, 'b>;

fn at<'a>(n: XNode<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name).map(|a| a.value())
}

fn lname<'a>(n: XNode<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn read_entry(z: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut e = z.by_name(name).ok()?;
    let mut s = String::new();
    e.by_ref().take(256 << 20).read_to_string(&mut s).ok()?;
    Some(s)
}

fn is_sans(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["arial", "helvetica", "sans", "calibri", "verdana", "tahoma", "segoe", "liberation sans", "dejavu sans"].iter().any(|f| n.contains(f))
}

impl<'a> OCx<'a> {
    fn load_styles(&mut self, doc: &roxmltree::Document) {
        for n in doc.descendants().filter(|n| n.is_element()) {
            match lname(n) {
                "style" | "default-style" => {
                    let name = if lname(n) == "default-style" { Some("\0default") } else { at(n, "name") };
                    let Some(name) = name else { continue };
                    let mut props = HashMap::new();
                    for c in n.children().filter(|c| c.is_element()) {
                        let pfx = match lname(c) {
                            "text-properties" => "t:",
                            "paragraph-properties" => "p:",
                            "table-cell-properties" => "c:",
                            "table-column-properties" => "k:",
                            _ => continue,
                        };
                        for a in c.attributes() {
                            props.insert(format!("{pfx}{}", a.name()), a.value().to_string());
                        }
                    }
                    let family = at(n, "family").unwrap_or("").to_string();
                    let name = if name == "\0default" && family != "paragraph" { continue } else { name };
                    let auto = n.parent().is_some_and(|p| lname(p) == "automatic-styles");
                    self.styles.insert(name.to_string(), OSt { parent: at(n, "parent-style-name").map(String::from), family, auto, props });
                }
                "list-style" => {
                    let Some(name) = at(n, "name") else { continue };
                    let mut v = vec![true; 9];
                    for l in n.children().filter(|c| c.is_element()) {
                        let i = at(l, "level").and_then(|x| x.parse::<usize>().ok()).unwrap_or(1).clamp(1, 9) - 1;
                        v[i] = lname(l) != "list-level-style-number";
                    }
                    self.lists.insert(name.to_string(), v);
                }
                _ => {}
            }
        }
    }

    /// Eigenschaften einer Vorlage samt Eltern (nur automatische und Zeichenvorlagen: Absatzvorlagen wie
    /// „Überschrift“ bestimmen das Aussehen schon über die Art des Absatzes).
    fn props(&self, name: &str) -> HashMap<String, String> {
        let mut chain = Vec::new();
        let mut cur = Some(name.to_string());
        while let Some(n) = cur {
            if chain.len() > 8 {
                break;
            }
            let Some(s) = self.styles.get(&n) else { break };
            chain.push(s);
            cur = s.parent.clone();
        }
        let mut m = HashMap::new();
        for s in chain.iter().rev() {
            if s.auto || s.family == "text" {
                m.extend(s.props.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        m
    }

    fn kind_hint(&self, name: &str) -> Option<Kind> {
        let mut cur = Some(name.to_string());
        for _ in 0..8 {
            let n = cur?;
            let l = n.to_lowercase().replace("_20_", " ");
            match l.as_str() {
                "title" => return Some(Kind::Title),
                "heading 1" => return Some(Kind::H1),
                "heading 2" => return Some(Kind::H2),
                h if h.starts_with("heading ") => return Some(Kind::H3),
                "quotations" | "quote" => return Some(Kind::Quote),
                "caption" | "illustration" => return Some(Kind::Caption),
                _ => {}
            }
            cur = self.styles.get(&n).and_then(|s| s.parent.clone());
        }
        None
    }

    fn image(&self, frame: XNode, align: Align) -> Option<Para> {
        let img = frame.descendants().find(|d| d.is_element() && lname(*d) == "image")?;
        let href = at(img, "href")?.trim_start_matches("./").to_string();
        let mut bytes = Vec::new();
        self.z.borrow_mut().by_name(&href).ok()?.take(64 << 20).read_to_end(&mut bytes).ok()?;
        let (w, h) = (at(frame, "width").and_then(len_pt), at(frame, "height").and_then(len_pt));
        let im = add_image(&mut self.store.borrow_mut(), bytes, w, h)?;
        Some(pic_para(im, align))
    }

    fn inline(&self, n: XNode, st: &CStyle, out: &mut Vec<Run>, pics: &mut Vec<Para>, align: Align) {
        for c in n.children() {
            if c.is_text() {
                let t = collapse_ws(c.text().unwrap_or(""));
                if !t.is_empty() {
                    out.push(Run { text: t, st: st.clone() });
                }
                continue;
            }
            if !c.is_element() {
                continue;
            }
            match lname(c) {
                "span" => {
                    let mut s2 = st.clone();
                    if let Some(sn) = at(c, "style-name") {
                        apply_props(&self.props(sn), &mut s2);
                    }
                    self.inline(c, &s2, out, pics, align);
                }
                "a" => {
                    let mut s2 = st.clone();
                    if let Some(h) = at(c, "href") {
                        s2.link = Some(h.to_string());
                    }
                    self.inline(c, &s2, out, pics, align);
                }
                "s" => {
                    let k = at(c, "c").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(200);
                    out.push(Run { text: " ".repeat(k), st: st.clone() });
                }
                "tab" => out.push(Run { text: "\t".into(), st: st.clone() }),
                "line-break" => out.push(Run { text: " ".into(), st: st.clone() }),
                "note" => {
                    let mut t = String::new();
                    if let Some(b) = c.children().find(|x| x.is_element() && lname(*x) == "note-body") {
                        for p in b.children().filter(|x| x.is_element()) {
                            let mut runs = Vec::new();
                            self.inline(p, &CStyle::default(), &mut runs, &mut Vec::new(), Align::Left);
                            if !t.is_empty() {
                                t.push(' ');
                            }
                            t.extend(runs.iter().map(|r| r.text.as_str()));
                        }
                    }
                    out.push(Run { text: OBJ.to_string(), st: CStyle { field: Some(Field::Note(t.trim().to_string())), ..CStyle::default() } });
                }
                "page-number" => out.push(Run { text: OBJ.to_string(), st: CStyle { field: Some(Field::Page), ..CStyle::default() } }),
                "page-count" => out.push(Run { text: OBJ.to_string(), st: CStyle { field: Some(Field::Pages), ..CStyle::default() } }),
                "frame" => {
                    if let Some(p) = self.image(c, align) {
                        pics.push(p);
                    }
                }
                "annotation" | "annotation-end" | "soft-page-break" | "note-citation" | "tracked-changes" | "frame-text-box" => {}
                _ => self.inline(c, st, out, pics, align),
            }
        }
    }

    fn para(&self, n: XNode, kind: Kind, indent: u8, out: &mut Vec<Para>) {
        let sname = at(n, "style-name").unwrap_or("");
        let props = self.props(sname);
        let hint = self.kind_hint(sname);
        let kind = match (kind, hint) {
            (Kind::Body, Some(h)) => h,
            (Kind::H1 | Kind::H2 | Kind::H3, Some(Kind::Title)) => Kind::Title,
            (k, _) => k,
        };
        let align = props.get("p:text-align").and_then(|s| align_from(s)).unwrap_or_default();
        let mut base = CStyle::default();
        apply_props(&props, &mut base);
        let (mut runs, mut pics) = (Vec::new(), Vec::new());
        self.inline(n, &base, &mut runs, &mut pics, align);
        if let Some(r) = runs.iter_mut().find(|r| !r.text.is_empty()) {
            r.text = r.text.trim_start().to_string();
        }
        let brk_b = props.get("p:break-before").is_some_and(|v| v == "page");
        let brk_a = props.get("p:break-after").is_some_and(|v| v == "page");
        let empty = runs.iter().all(|r| r.text.is_empty());
        if brk_b {
            out.push(Para::new(Kind::PageBreak));
        }
        if !(empty && (!pics.is_empty() || brk_a)) {
            let cap = if kind == Kind::Caption { strip_caption(&mut runs) } else { None };
            let mut p = Para { kind, align, indent: if kind.is_list() { indent.min(8) } else { 0 }, runs, cap: cap.unwrap_or_default(), ..Para::default() };
            p.normalize();
            out.push(p);
        }
        out.extend(pics);
        if brk_a {
            out.push(Para::new(Kind::PageBreak));
        }
    }

    fn list(&self, n: XNode, level: usize, inherited: &[bool], out: &mut Vec<Para>) {
        let bl = at(n, "style-name").and_then(|s| self.lists.get(s)).cloned().unwrap_or_else(|| inherited.to_vec());
        for item in n.children().filter(|c| c.is_element() && matches!(lname(*c), "list-item" | "list-header")) {
            for c in item.children().filter(|c| c.is_element()) {
                match lname(c) {
                    "p" | "h" => {
                        let kind = if bl.get(level.min(8)).copied().unwrap_or(true) { Kind::Bullet } else { Kind::Number };
                        self.para(c, kind, level.min(8) as u8, out);
                    }
                    "list" => self.list(c, level + 1, &bl, out),
                    _ => {}
                }
            }
        }
    }

    fn cell(&self, c: XNode) -> Cell {
        let mut cb = CellB::default();
        if let Some(sn) = at(c, "style-name") {
            let p = self.props(sn);
            cb.bg = p.get("c:background-color").and_then(|v| parse_color(v)).unwrap_or(0);
        }
        let mut first = true;
        for p in c.descendants().filter(|d| d.is_element() && matches!(lname(*d), "p" | "h")) {
            let props = at(p, "style-name").map(|s| self.props(s)).unwrap_or_default();
            let mut base = CStyle::default();
            apply_props(&props, &mut base);
            if first {
                cb.align = props.get("p:text-align").and_then(|s| align_from(s)).unwrap_or_default();
            } else {
                cb.text.push('\n');
            }
            first = false;
            let mut runs = Vec::new();
            self.inline(p, &base, &mut runs, &mut Vec::new(), Align::Left);
            for r in runs {
                cb.add(&r.text.replace(OBJ, ""), &r.st);
            }
        }
        cb.done()
    }

    fn row(&self, r: XNode) -> Vec<Cell> {
        let mut cells = Vec::new();
        for c in r.children().filter(|c| c.is_element()) {
            match lname(c) {
                "table-cell" => {
                    let rep = at(c, "number-columns-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
                    let cell = self.cell(c);
                    if rep <= 20 {
                        for _ in 0..rep {
                            cells.push(cell.clone());
                        }
                    } else if !cell.text.is_empty() {
                        cells.push(cell);
                    }
                }
                "covered-table-cell" => cells.push(Cell::default()),
                _ => {}
            }
        }
        cells
    }

    fn table(&self, n: XNode, out: &mut Vec<Para>) {
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        let mut header = false;
        let mut colw: Vec<f32> = Vec::new();
        let add_rows = |r: XNode, rows: &mut Vec<Vec<Cell>>| {
            let rep = at(r, "number-rows-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
            let cells = self.row(r);
            if rep <= 50 && rows.len() < 200 {
                for _ in 0..rep {
                    rows.push(cells.clone());
                }
            }
        };
        for c in n.children().filter(|c| c.is_element()) {
            match lname(c) {
                "table-column" => {
                    let rep = at(c, "number-columns-repeated").and_then(|v| v.parse::<usize>().ok()).unwrap_or(1).min(20);
                    let w = at(c, "style-name").and_then(|s| self.styles.get(s)).and_then(|s| s.props.get("k:column-width")).and_then(|v| len_pt(v)).unwrap_or(0.0);
                    colw.extend(std::iter::repeat_n(w, rep));
                }
                "table-columns" | "table-header-columns" => {
                    for k in c.children().filter(|k| k.is_element() && lname(*k) == "table-column") {
                        let w = at(k, "style-name").and_then(|s| self.styles.get(s)).and_then(|s| s.props.get("k:column-width")).and_then(|v| len_pt(v)).unwrap_or(0.0);
                        colw.push(w);
                    }
                }
                "table-header-rows" => {
                    header = true;
                    for r in c.children().filter(|r| r.is_element() && lname(*r) == "table-row") {
                        add_rows(r, &mut rows);
                    }
                }
                "table-rows" => {
                    for r in c.children().filter(|r| r.is_element() && lname(*r) == "table-row") {
                        add_rows(r, &mut rows);
                    }
                }
                "table-row" => add_rows(c, &mut rows),
                _ => {}
            }
        }
        if header {
            for c in rows.first_mut().into_iter().flatten() {
                if c.bg == HEAD_BG {
                    c.bg = 0;
                }
                c.b = false;
            }
        }
        if let Some(mut p) = make_table(rows, header) {
            if let Some(t) = p.table.as_mut() {
                if colw.len() == t.ncols() && colw.iter().all(|w| *w > 0.0) {
                    t.cols = colw;
                    t.fit(super::model::PageSpec::default().content_w());
                }
            }
            out.push(p);
        }
    }

    fn block(&self, n: XNode, out: &mut Vec<Para>) {
        for c in n.children().filter(|c| c.is_element()) {
            match lname(c) {
                "h" => {
                    let k = match at(c, "outline-level").and_then(|v| v.parse::<u8>().ok()).unwrap_or(1) {
                        1 => Kind::H1,
                        2 => Kind::H2,
                        _ => Kind::H3,
                    };
                    self.para(c, k, 0, out);
                }
                "p" => self.para(c, Kind::Body, 0, out),
                "list" => self.list(c, 0, &[true; 9], out),
                "table" => self.table(c, out),
                "section" | "table-of-content" | "index-body" | "alphabetical-index" | "bibliography" | "illustration-index" => self.block(c, out),
                _ => {}
            }
        }
    }
}

/// Zeichenformat aus den Eigenschaften einer Vorlage.
fn apply_props(m: &HashMap<String, String>, st: &mut CStyle) {
    let g = |k: &str| m.get(k).map(String::as_str);
    if let Some(v) = g("t:font-weight") {
        st.b = v == "bold" || v.parse::<u32>().is_ok_and(|n| n >= 600);
    }
    if let Some(v) = g("t:font-style") {
        st.i = v == "italic" || v == "oblique";
    }
    if let Some(v) = g("t:text-underline-style") {
        st.u = v != "none";
    }
    if let Some(v) = g("t:text-line-through-style") {
        st.s = v != "none";
    }
    if let Some(v) = g("t:text-position") {
        let first = v.split_whitespace().next().unwrap_or("");
        let num = first.trim_end_matches('%').parse::<f32>().ok();
        st.va = if first.starts_with("super") || num.is_some_and(|x| x > 0.0) {
            VAlign::Sup
        } else if first.starts_with("sub") || num.is_some_and(|x| x < 0.0) {
            VAlign::Sub
        } else {
            VAlign::Base
        };
    }
    if let Some(c) = g("t:color").and_then(parse_color) {
        st.color = c;
    }
    if let Some(v) = g("t:background-color") {
        st.hl = if v == "transparent" { 0 } else { parse_color(v).unwrap_or(0) };
    }
    if let Some(sz) = g("t:font-size").filter(|v| !v.ends_with('%')).and_then(len_pt) {
        if (1.0..=400.0).contains(&sz) {
            st.size = Some(sz);
        }
    }
    if let Some(f) = g("t:font-family").or(g("t:font-name")) {
        st.fam = Some(if is_sans(f) { Family::Sans } else { Family::Serif });
    }
}

/// Text von Kopf-/Fußzeile mit Seitenfeldern.
fn hf_text(n: XNode, out: &mut String) {
    for c in n.children() {
        if c.is_text() {
            out.push_str(c.text().unwrap_or(""));
        } else if c.is_element() {
            match lname(c) {
                "page-number" => out.push_str("{seite}"),
                "page-count" => out.push_str("{seiten}"),
                "s" | "tab" | "line-break" => out.push(' '),
                "p" | "h" => {
                    if !out.is_empty() && !out.ends_with(' ') {
                        out.push(' ');
                    }
                    hf_text(c, out);
                }
                _ => hf_text(c, out),
            }
        }
    }
}

fn odt_import(bytes: &[u8]) -> Result<(TextDoc, Images), String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "Das ist keine OpenDocument-Datei (.odt).".to_string())?;
    let content = read_entry(&mut z, "content.xml").ok_or("In der Datei fehlt der Text (content.xml).")?;
    let styles = read_entry(&mut z, "styles.xml").unwrap_or_default();
    let opt = roxmltree::ParsingOptions { allow_dtd: false, ..Default::default() };
    let cdoc = roxmltree::Document::parse_with_options(&content, opt).map_err(|e| format!("Der Text der Datei ist beschädigt: {e}"))?;
    let mut cx = OCx { styles: HashMap::new(), lists: HashMap::new(), z: std::cell::RefCell::new(z), store: Default::default() };
    let sdoc = roxmltree::Document::parse_with_options(&styles, opt).ok();
    if let Some(s) = &sdoc {
        cx.load_styles(s);
    }
    cx.load_styles(&cdoc);
    let mut td = TextDoc::new();
    td.paras = Vec::new();
    if let Some(body) = cdoc.descendants().find(|n| n.is_element() && lname(*n) == "text" && n.parent().is_some_and(|p| lname(p) == "body")) {
        cx.block(body, &mut td.paras);
    }
    if td.paras.is_empty() {
        td.paras.push(Para::new(Kind::Body));
    }
    if let Some(sz) = cx.styles.get("\0default").or(cx.styles.get("Standard")).and_then(|s| s.props.get("t:font-size")).and_then(|v| len_pt(v)) {
        td.size = sz.clamp(4.0, 96.0);
    }
    if let Some(f) = cx.styles.get("\0default").or(cx.styles.get("Standard")).and_then(|s| s.props.get("t:font-family").or(s.props.get("t:font-name"))) {
        td.family = if is_sans(f) { Family::Sans } else { Family::Serif };
    }
    // Größe und Schrift, die dem Dokument entsprechen, sind keine Sonderformate
    for p in &mut td.paras {
        for r in &mut p.runs {
            if r.st.size.is_some_and(|s| (s - td.size).abs() < 0.05) {
                r.st.size = None;
            }
            if r.st.fam == Some(td.family) {
                r.st.fam = None;
            }
        }
        p.normalize();
    }
    if let Some(s) = &sdoc {
        if let Some(pl) = s.descendants().find(|n| n.is_element() && lname(*n) == "page-layout-properties") {
            let g = |k: &str| at(pl, k).and_then(len_pt);
            if let (Some(w), Some(h)) = (g("page-width"), g("page-height")) {
                if w >= 72.0 && h >= 72.0 {
                    td.page.w = w;
                    td.page.h = h;
                    for (k, f) in [("margin-left", &mut td.page.ml), ("margin-right", &mut td.page.mr), ("margin-top", &mut td.page.mt), ("margin-bottom", &mut td.page.mb)] {
                        if let Some(v) = g(k).filter(|v| (0.0..=w.min(h) / 2.0).contains(v)) {
                            *f = v;
                        }
                    }
                }
            }
        }
        if let Some(mp) = s.descendants().find(|n| n.is_element() && lname(*n) == "master-page") {
            for (tag, dst) in [("header", &mut td.header), ("footer", &mut td.footer)] {
                if let Some(h) = mp.children().find(|c| c.is_element() && lname(*c) == tag) {
                    let mut t = String::new();
                    hf_text(h, &mut t);
                    *dst = t.split_whitespace().collect::<Vec<_>>().join(" ");
                }
            }
        }
    }
    let files = cx.store.into_inner().files;
    Ok((td, files))
}

// ------------------------------------------------------------------ RTF: Schreiben

/// Farbtabelle (Index 0 = automatisch, Einträge ab 1).
#[derive(Default)]
struct Colors(Vec<u32>);

impl Colors {
    fn idx(&mut self, c: u32) -> usize {
        let c = c | 0xFF00_0000;
        match self.0.iter().position(|x| *x == c) {
            Some(i) => i + 1,
            None => {
                self.0.push(c);
                self.0.len()
            }
        }
    }
}

fn rtf_esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '{' | '}' => {
                o.push('\\');
                o.push(c);
            }
            '\t' => o.push_str("\\tab "),
            '\n' => o.push_str("\\line "),
            c if (c as u32) < 0x20 => {}
            c if (c as u32) < 0x80 => o.push(c),
            c => {
                let mut b = [0u16; 2];
                for u in c.encode_utf16(&mut b) {
                    o.push_str(&format!("\\u{}?", *u as i16));
                }
            }
        }
    }
    o
}

fn rtf_props(st: &CStyle, col: &mut Colors) -> String {
    let mut p = String::new();
    if let Some(f) = st.fam {
        p.push_str(if f == Family::Sans { "\\f1" } else { "\\f0" });
    }
    if st.b {
        p.push_str("\\b");
    }
    if st.i {
        p.push_str("\\i");
    }
    if st.u {
        p.push_str("\\ul");
    }
    if st.s {
        p.push_str("\\strike");
    }
    match st.va {
        VAlign::Sup => p.push_str("\\super"),
        VAlign::Sub => p.push_str("\\sub"),
        VAlign::Base => {}
    }
    if let Some(sz) = st.size {
        p.push_str(&format!("\\fs{}", (sz * 2.0).round() as i32));
    }
    if st.color != 0 {
        p.push_str(&format!("\\cf{}", col.idx(st.color)));
    } else if st.link.is_some() {
        p.push_str(&format!("\\cf{}", col.idx(0xFF1A56DB)));
    }
    if st.hl != 0 {
        p.push_str(&format!("\\highlight{}", col.idx(st.hl)));
    }
    p
}

fn rtf_field(inst: &str) -> String {
    format!("{{\\field{{\\*\\fldinst {inst}}}{{\\fldrslt 1}}}}")
}

fn rtf_inline(segs: &[Seg], col: &mut Colors) -> String {
    let mut o = String::new();
    for g in segs {
        match g {
            Seg::T(t, st) => {
                let mut st = st.clone();
                if st.link.is_some() {
                    st.u = true;
                }
                let props = rtf_props(&st, col);
                let body = if props.is_empty() { rtf_esc(t) } else { format!("{{{props} {}}}", rtf_esc(t)) };
                match &st.link {
                    Some(l) => o.push_str(&format!("{{\\field{{\\*\\fldinst HYPERLINK \"{}\"}}{{\\fldrslt {body}}}}}", rtf_esc(l))),
                    None => o.push_str(&body),
                }
            }
            Seg::Note(_, t) => o.push_str(&format!("{{\\super\\chftn}}{{\\footnote {{\\super\\chftn}} {}}}", rtf_esc(t))),
            Seg::Page => o.push_str(&rtf_field("PAGE")),
            Seg::Pages => o.push_str(&rtf_field("NUMPAGES")),
        }
    }
    o
}

fn rtf_hf(s: &str) -> String {
    hf_parts(s)
        .iter()
        .map(|h| match h {
            Hf::T(t) => rtf_esc(t),
            Hf::Page => rtf_field("PAGE"),
            Hf::Pages => rtf_field("NUMPAGES"),
        })
        .collect()
}

/// PNG oder JPEG für `\pict`; andere Formate werden zu PNG umgewandelt.
fn png_or_jpeg(b: &[u8]) -> Option<(Vec<u8>, bool)> {
    match sniff(b).1 {
        "png" if b.starts_with(&[0x89, b'P']) => Some((b.to_vec(), false)),
        "jpg" => Some((b.to_vec(), true)),
        _ => {
            let im = image::load_from_memory(b).ok()?;
            let mut o = Cursor::new(Vec::new());
            im.write_to(&mut o, image::ImageFormat::Png).ok()?;
            Some((o.into_inner(), false))
        }
    }
}

fn tw(pt: f32) -> i64 {
    (pt * 20.0).round() as i64
}

fn rtf_align(a: Align) -> &'static str {
    match a {
        Align::Left => "",
        Align::Center => "\\qc",
        Align::Right => "\\qr",
        Align::Justify => "\\qj",
    }
}

fn rtf_export(doc: &TextDoc, title: &str, imgs: &Images) -> String {
    let mut cx = Cx::new(doc, imgs);
    let mut col = Colors::default();
    let mut body = String::new();
    let mut counter = Counter::default();
    let mut pb = false;
    let mut tail_table = false;
    for (pi, p) in doc.paras.iter().enumerate() {
        if !p.kind.is_list() {
            counter.reset();
        }
        let brk = if std::mem::take(&mut pb) { "\\pagebb" } else { "" };
        tail_table = false;
        match p.kind {
            Kind::PageBreak => pb = true,
            Kind::Table => {
                if let Some(t) = &p.table {
                    rtf_table(t, &mut col, &mut body);
                    tail_table = true;
                }
            }
            Kind::Image | Kind::Object => {
                let pic = cx.picture(p, false).and_then(|(b, w, h)| png_or_jpeg(&b).map(|(d, jpeg)| (d, jpeg, w, h)));
                if let Some((data, jpeg, w, h)) = pic {
                    let hexs: String = data.iter().map(|b| format!("{b:02x}")).collect();
                    body.push_str(&format!(
                        "\\pard{brk}{}{{\\pict\\{}blip\\picw{}\\pich{}\\picwgoal{}\\pichgoal{}\n{hexs}}}\\par\n",
                        rtf_align(p.align),
                        if jpeg { "jpeg" } else { "png" },
                        (w * 4.0 / 3.0).round() as i64,
                        (h * 4.0 / 3.0).round() as i64,
                        tw(w),
                        tw(h)
                    ));
                } else if let Some(o) = &p.obj {
                    body.push_str(&format!("\\pard{brk}{} {}\\par\n", rtf_align(p.align), rtf_esc(&obj_text(o))));
                }
            }
            k => {
                let segs = cx.segs(pi);
                let inner = rtf_inline(&segs, &mut col);
                let sty = match k {
                    Kind::Title => "\\s10",
                    Kind::H1 => "\\s1\\outlinelevel0",
                    Kind::H2 => "\\s2\\outlinelevel1",
                    Kind::H3 => "\\s3\\outlinelevel2",
                    Kind::Quote => "\\s11",
                    Kind::Caption => "\\s12",
                    _ => "",
                };
                let mut list = String::new();
                if k.is_list() {
                    let l = (p.indent as usize).min(8);
                    let n = counter.next(l, k == Kind::Number);
                    list = format!("\\li{}\\fi-360", 720 * (l + 1));
                    if k == Kind::Number {
                        list.push_str(&format!("{{\\pntext\\f0 {n}.\\tab}}{{\\*\\pn\\pnlvlbody\\pnf0\\pnindent360\\pnstart1\\pndec{{\\pntxta.}}}}"));
                    } else {
                        list.push_str("{\\pntext\\f0 \\u8226?\\tab}{\\*\\pn\\pnlvlblt\\pnf0\\pnindent360\\pnhang{\\pntxtb \\u8226?}}");
                    }
                }
                body.push_str(&format!("\\pard{brk}{sty}{}{list} {inner}\\par\n", rtf_align(p.align)));
            }
        }
    }
    if pb || tail_table {
        body.push_str(if pb { "\\pard\\page\\par\n" } else { "\\pard\\par\n" });
    }
    let sans = doc.family == Family::Sans;
    let mut s = format!("{{\\rtf1\\ansi\\ansicpg1252\\deff{}\\uc1\\deflang1031{{\\fonttbl{{\\f0\\froman\\fcharset0 Times New Roman;}}{{\\f1\\fswiss\\fcharset0 Arial;}}}}", sans as u8);
    s.push_str("{\\colortbl;");
    for c in &col.0 {
        s.push_str(&format!("\\red{}\\green{}\\blue{};", (c >> 16) & 255, (c >> 8) & 255, c & 255));
    }
    s.push('}');
    let cap = ((doc.size - 1.5).max(6.0) * 2.0).round() as i32;
    s.push_str(&format!(
        "{{\\stylesheet{{\\s0 Normal;}}{{\\s1\\sbasedon0\\snext0\\f1\\b\\fs36\\sb360\\sa120 heading 1;}}{{\\s2\\sbasedon0\\snext0\\f1\\b\\fs28\\sb280\\sa80 heading 2;}}{{\\s3\\sbasedon0\\snext0\\f1\\b\\fs24\\sb200\\sa60 heading 3;}}{{\\s10\\sbasedon0\\snext0\\f1\\b\\fs52\\sa280 Title;}}{{\\s11\\sbasedon0\\i\\li360 Quote;}}{{\\s12\\sbasedon0\\f1\\i\\fs{cap} caption;}}}}"
    ));
    s.push_str(&format!("{{\\info{{\\title {}}}{{\\creator OwnDraw}}}}", rtf_esc(title)));
    let p = &doc.page;
    s.push_str(&format!("\\paperw{}\\paperh{}\\margl{}\\margr{}\\margt{}\\margb{}\\viewkind1\n", tw(p.w), tw(p.h), tw(p.ml), tw(p.mr), tw(p.mt), tw(p.mb)));
    if !doc.header.is_empty() {
        s.push_str(&format!("{{\\header\\pard {}\\par}}\n", rtf_hf(&doc.header)));
    }
    if !doc.footer.is_empty() {
        s.push_str(&format!("{{\\footer\\pard {}\\par}}\n", rtf_hf(&doc.footer)));
    }
    s.push_str(&format!("\\fs{}\n", (doc.size * 2.0).round() as i32));
    s.push_str(&body);
    s.push('}');
    s
}

fn rtf_table(t: &Table, col: &mut Colors, o: &mut String) {
    let brd = if t.border != 0 { "\\brdrs\\brdrw10" } else { "\\brdrnone" };
    for (ri, row) in t.rows.iter().enumerate() {
        let hdr = t.header && ri == 0;
        o.push_str("\\trowd\\trgaph108\\trleft0");
        if hdr {
            o.push_str("\\trhdr");
        }
        let mut x = 0.0;
        for (ci, c) in row.iter().enumerate() {
            x += t.cols.get(ci).copied().unwrap_or(80.0);
            let bg = if c.bg != 0 {
                c.bg
            } else if hdr {
                HEAD_BG
            } else {
                0
            };
            o.push_str(&format!("\\clbrdrt{brd}\\clbrdrl{brd}\\clbrdrb{brd}\\clbrdrr{brd}"));
            if bg != 0 {
                o.push_str(&format!("\\clcbpat{}", col.idx(bg)));
            }
            o.push_str(&format!("\\cellx{}", tw(x)));
        }
        o.push('\n');
        for c in row {
            let st = CStyle { b: c.b || hdr, i: c.i, color: c.color, ..CStyle::default() };
            let props = rtf_props(&st, col);
            let text = rtf_esc(&c.text);
            o.push_str(&format!("\\pard\\intbl{}{{{props} {text}}}\\cell\n", rtf_align(c.align)));
        }
        o.push_str("\\row\n");
    }
}

// ------------------------------------------------------------------ RTF: Lesen

#[derive(Clone, Copy, PartialEq, Debug)]
enum Dest {
    Normal,
    Skip,
    FontTbl,
    ColorTbl,
    FldInst,
    Note,
    Header,
    Footer,
    Pict,
    ListTbl,
    ListOvr,
}

#[derive(Clone)]
struct Gs {
    st: CStyle,
    dest: Dest,
    uc: usize,
    skip: usize,
}

#[derive(Default)]
struct Pict {
    hex: String,
    jpeg: bool,
    png: bool,
    w: Option<f32>,
    h: Option<f32>,
}

struct Ri {
    b: Bld,
    stack: Vec<Gs>,
    align: Align,
    li: i32,
    list: Option<bool>,
    ls: Option<i32>,
    ilvl: Option<usize>,
    sty: Option<i32>,
    outline: Option<u8>,
    pagebb: bool,
    after_page: bool,
    colors: Vec<u32>,
    rgb: [u32; 3],
    have_rgb: bool,
    fonts: HashMap<i32, bool>,
    cur_font: i32,
    font_name: String,
    font_hint: bool,
    deff: i32,
    fldinst: String,
    note_buf: String,
    hf_buf: String,
    pict: Pict,
    in_row: bool,
    row: Vec<Cell>,
    rows: Vec<Vec<Cell>>,
    row_hdr: bool,
    hdr: bool,
    cell: CellB,
    defs: Vec<u32>,
    cur_bg: u32,
    listbul: HashMap<i32, bool>,
    ls_map: HashMap<i32, i32>,
    first_nfc: Option<i32>,
    cur_lid: i32,
    header: String,
    footer: String,
    size_doc: Option<f32>,
    text_seen: bool,
    hi: u16,
    star: bool,
    page: [Option<f32>; 6],
}

fn hex2(b: &[u8], i: usize) -> Option<u8> {
    let s = std::str::from_utf8(b.get(i..i + 2)?).ok()?;
    u8::from_str_radix(s, 16).ok()
}

impl Ri {
    fn new() -> Ri {
        Ri {
            b: Bld::default(),
            stack: vec![Gs { st: CStyle::default(), dest: Dest::Normal, uc: 1, skip: 0 }],
            align: Align::Left,
            li: 0,
            list: None,
            ls: None,
            ilvl: None,
            sty: None,
            outline: None,
            pagebb: false,
            after_page: false,
            colors: vec![0],
            rgb: [0; 3],
            have_rgb: false,
            fonts: HashMap::new(),
            cur_font: 0,
            font_name: String::new(),
            font_hint: false,
            deff: 0,
            fldinst: String::new(),
            note_buf: String::new(),
            hf_buf: String::new(),
            pict: Pict::default(),
            in_row: false,
            row: Vec::new(),
            rows: Vec::new(),
            row_hdr: false,
            hdr: false,
            cell: CellB::default(),
            defs: Vec::new(),
            cur_bg: 0,
            listbul: HashMap::new(),
            ls_map: HashMap::new(),
            first_nfc: None,
            cur_lid: 0,
            header: String::new(),
            footer: String::new(),
            size_doc: None,
            text_seen: false,
            hi: 0,
            star: false,
            page: [None; 6],
        }
    }

    fn g(&mut self) -> &mut Gs {
        self.stack.last_mut().expect("Gruppenstapel nie leer")
    }

    fn push_group(&mut self) {
        let g = self.stack.last().cloned().expect("Gruppenstapel nie leer");
        if self.stack.len() < 200 {
            self.stack.push(Gs { skip: 0, ..g });
        } else {
            self.stack.push(Gs { dest: Dest::Skip, skip: 0, ..g });
        }
    }

    fn pop_group(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let g = self.stack.pop().expect("Gruppe");
        let parent = self.stack.last().map(|p| p.dest);
        if parent != Some(g.dest) {
            match g.dest {
                Dest::Pict => self.finish_pict(),
                Dest::Note => self.finish_note(),
                Dest::Header => {
                    if self.header.is_empty() {
                        self.header = self.hf_buf.split_whitespace().collect::<Vec<_>>().join(" ");
                    }
                    self.hf_buf.clear();
                }
                Dest::Footer => {
                    if self.footer.is_empty() {
                        self.footer = self.hf_buf.split_whitespace().collect::<Vec<_>>().join(" ");
                    }
                    self.hf_buf.clear();
                }
                _ => {}
            }
        }
    }

    fn finish_note(&mut self) {
        let t = self.note_buf.trim().to_string();
        self.note_buf.clear();
        if !self.in_row {
            self.b.runs.push(Run { text: OBJ.to_string(), st: CStyle { field: Some(Field::Note(t)), ..CStyle::default() } });
            self.text_seen = true;
        }
    }

    fn finish_pict(&mut self) {
        let p = std::mem::take(&mut self.pict);
        if !(p.png || p.jpeg) {
            return;
        }
        let h: Vec<u8> = p.hex.bytes().collect();
        let bytes: Vec<u8> = h.chunks_exact(2).filter_map(|c| hex2(c, 0)).collect();
        if bytes.is_empty() || self.in_row {
            return;
        }
        self.flush_para(false);
        let al = self.align;
        self.b.image(bytes, p.w, p.h, al);
        self.text_seen = true;
        // das Absatzende hinter dem Bild gehört zum Bild
        self.after_page = true;
    }

    /// Kind und Einzug des laufenden Absatzes nach den Absatzeigenschaften.
    fn kind(&self) -> (Kind, u8) {
        let list = match self.ls.filter(|v| *v > 0) {
            Some(ls) => Some(self.ls_map.get(&ls).and_then(|id| self.listbul.get(id)).copied().unwrap_or(true)),
            None => self.list,
        };
        if let Some(bullet) = list {
            let lvl = match (self.ls.filter(|v| *v > 0), self.ilvl) {
                (Some(_), Some(l)) => l,
                _ => (self.li.max(0) as usize / 720).saturating_sub(1),
            };
            return (if bullet { Kind::Bullet } else { Kind::Number }, lvl.min(8) as u8);
        }
        let k = match self.sty {
            Some(1) => Kind::H1,
            Some(2) => Kind::H2,
            Some(3..=9) => Kind::H3,
            Some(10) => Kind::Title,
            Some(11) => Kind::Quote,
            Some(12) => Kind::Caption,
            _ => match self.outline {
                Some(0) => Kind::H1,
                Some(1) => Kind::H2,
                Some(_) => Kind::H3,
                None => Kind::Body,
            },
        };
        (k, 0)
    }

    fn flush_para(&mut self, force: bool) {
        let (k, ind) = self.kind();
        if self.pagebb && (force || self.b.has_text()) {
            self.pagebb = false;
            self.b.paras.push(Para::new(Kind::PageBreak));
        }
        let al = self.align;
        self.b.flush(k, al, ind, force);
    }

    fn par(&mut self) {
        if self.in_row {
            if !self.cell.text.is_empty() && !self.cell.text.ends_with('\n') {
                self.cell.text.push('\n');
            }
            return;
        }
        match self.g().dest {
            Dest::Header | Dest::Footer => self.hf_buf.push(' '),
            Dest::Note => self.note_buf.push(' '),
            Dest::Normal => {
                self.end_table();
                if self.after_page && !self.b.has_text() {
                    self.after_page = false;
                    return;
                }
                self.flush_para(true);
            }
            _ => {}
        }
    }

    fn end_table(&mut self) {
        if self.in_row || self.rows.is_empty() {
            return;
        }
        let mut rows = std::mem::take(&mut self.rows);
        let hdr = std::mem::take(&mut self.hdr);
        if hdr {
            for c in rows.first_mut().into_iter().flatten() {
                if c.bg == HEAD_BG {
                    c.bg = 0;
                }
                c.b = false;
            }
        }
        if let Some(p) = make_table(rows, hdr) {
            self.b.paras.push(p);
        }
    }

    /// Text in das Ziel der aktuellen Gruppe schreiben.
    fn emit(&mut self, s: &str) {
        let g = self.g().clone();
        match g.dest {
            Dest::Normal => {
                if self.in_row {
                    self.cell.add(s, &g.st);
                } else {
                    self.end_table();
                    if !s.trim().is_empty() {
                        self.text_seen = true;
                        self.after_page = false;
                    }
                    self.b.push(s, &g.st);
                }
            }
            Dest::FldInst => self.fldinst.push_str(s),
            Dest::Note => self.note_buf.push_str(s),
            Dest::Header | Dest::Footer => self.hf_buf.push_str(s),
            Dest::Pict => self.pict.hex.extend(s.chars().filter(char::is_ascii_hexdigit)),
            Dest::ColorTbl => {
                for c in s.chars() {
                    if c == ';' {
                        let v = if self.have_rgb { 0xFF00_0000 | self.rgb[0] << 16 | self.rgb[1] << 8 | self.rgb[2] } else { 0 };
                        self.colors.push(v);
                        self.rgb = [0; 3];
                        self.have_rgb = false;
                    }
                }
            }
            Dest::FontTbl => {
                for c in s.chars() {
                    if c == ';' {
                        let sans = self.font_hint || is_sans(&self.font_name);
                        self.fonts.insert(self.cur_font, sans && !self.font_name.to_ascii_lowercase().contains("times"));
                        self.font_name.clear();
                        self.font_hint = false;
                    } else {
                        self.font_name.push(c);
                    }
                }
            }
            Dest::Skip | Dest::ListTbl | Dest::ListOvr => {}
        }
    }

    /// Sichtbare Zeichen (nach Unicode-Ersatzzeichen wird `\ucN` Zeichen übersprungen).
    fn text(&mut self, s: &str) {
        let mut out = String::new();
        for c in s.chars() {
            let g = self.g();
            if g.skip > 0 {
                g.skip -= 1;
                continue;
            }
            out.push(c);
        }
        if !out.is_empty() {
            self.emit(&out);
        }
    }

    fn unicode(&mut self, n: i32) {
        let u = (if n < 0 { n + 65536 } else { n }) as u16;
        let ch = if (0xD800..0xDC00).contains(&u) {
            self.hi = u;
            None
        } else if (0xDC00..0xE000).contains(&u) && self.hi != 0 {
            let c = char::decode_utf16([self.hi, u]).next().and_then(Result::ok);
            self.hi = 0;
            c
        } else {
            char::from_u32(u as u32)
        };
        if let Some(c) = ch {
            self.emit(&c.to_string());
        }
        let uc = self.g().uc;
        self.g().skip = uc;
    }

    fn page_marker(&mut self) {
        let dest = self.g().dest;
        match dest {
            Dest::Header | Dest::Footer => self.hf_buf.push_str("{seite}"),
            Dest::Normal if !self.in_row => {
                self.end_table();
                self.b.runs.push(Run { text: OBJ.to_string(), st: CStyle { field: Some(Field::Page), ..CStyle::default() } });
                self.text_seen = true;
            }
            _ => {}
        }
    }

    fn word(&mut self, w: &str, p: Option<i32>) {
        if self.star {
            self.star = false;
            if !matches!(w, "fldinst" | "shppict" | "pict" | "listtable" | "listoverridetable") {
                self.g().dest = Dest::Skip;
            }
        }
        match self.g().dest {
            Dest::Skip => {
                match w {
                    "pnlvlblt" => self.list = Some(true),
                    "pnlvlbody" => self.list = Some(false),
                    _ => {}
                }
                return;
            }
            Dest::ColorTbl => {
                let v = p.unwrap_or(0).clamp(0, 255) as u32;
                match w {
                    "red" => self.rgb[0] = v,
                    "green" => self.rgb[1] = v,
                    "blue" => self.rgb[2] = v,
                    _ => return,
                }
                self.have_rgb = true;
                return;
            }
            Dest::FontTbl => {
                match w {
                    "f" => self.cur_font = p.unwrap_or(0),
                    "fswiss" => self.font_hint = true,
                    _ => {}
                }
                return;
            }
            Dest::ListTbl => {
                match w {
                    "list" => self.first_nfc = None,
                    "levelnfc" if self.first_nfc.is_none() => self.first_nfc = p,
                    "listid" => {
                        self.listbul.insert(p.unwrap_or(0), self.first_nfc == Some(23));
                        self.first_nfc = None;
                    }
                    _ => {}
                }
                return;
            }
            Dest::ListOvr => {
                match w {
                    "listid" => self.cur_lid = p.unwrap_or(0),
                    "ls" => {
                        self.ls_map.insert(p.unwrap_or(0), self.cur_lid);
                    }
                    _ => {}
                }
                return;
            }
            _ => {}
        }
        let on = p != Some(0);
        match w {
            "fonttbl" => self.g().dest = Dest::FontTbl,
            "colortbl" => {
                self.colors.clear();
                self.g().dest = Dest::ColorTbl;
            }
            "listtable" => self.g().dest = Dest::ListTbl,
            "listoverridetable" => self.g().dest = Dest::ListOvr,
            "stylesheet" | "info" | "nonshppict" | "headerl" | "headerf" | "footerl" | "footerf" | "pntext" | "listtext" | "rsidtbl" | "generator" | "themedata" | "colorschememapping" | "latentstyles" | "datastore" | "revtbl" | "xmlnstbl" | "pgdsctbl" | "filetbl" | "template" | "upr" | "ud" | "fldtype" | "private" | "bkmkstart" | "bkmkend" | "xe" | "tc" | "comment" | "annotation" => {
                self.g().dest = Dest::Skip
            }
            "header" | "headerr" => self.g().dest = Dest::Header,
            "footer" | "footerr" => self.g().dest = Dest::Footer,
            "footnote" => {
                self.note_buf.clear();
                self.g().dest = Dest::Note;
            }
            "pict" => {
                self.pict = Pict::default();
                self.g().dest = Dest::Pict;
            }
            "pngblip" => self.pict.png = true,
            "jpegblip" => self.pict.jpeg = true,
            "picwgoal" => self.pict.w = p.map(|v| v as f32 / 20.0),
            "pichgoal" => self.pict.h = p.map(|v| v as f32 / 20.0),
            "field" => self.fldinst.clear(),
            "fldinst" => {
                self.fldinst.clear();
                self.g().dest = Dest::FldInst;
            }
            "fldrslt" => {
                let inst = self.fldinst.trim().to_string();
                let up = inst.to_ascii_uppercase();
                if up.starts_with("HYPERLINK") {
                    let url = inst.split('"').nth(1).map(str::to_string).or_else(|| inst.split_whitespace().nth(1).map(str::to_string));
                    if let Some(u) = url.filter(|u| !u.is_empty() && !inst.contains("\\l ")) {
                        self.g().st.link = Some(u);
                    }
                } else if up.starts_with("PAGE") {
                    self.page_marker();
                    self.g().dest = Dest::Skip;
                } else if up.starts_with("NUMPAGES") {
                    let dest = self.g().dest;
                    match dest {
                        Dest::Header | Dest::Footer => self.hf_buf.push_str("{seiten}"),
                        Dest::Normal if !self.in_row => self.b.runs.push(Run { text: OBJ.to_string(), st: CStyle { field: Some(Field::Pages), ..CStyle::default() } }),
                        _ => {}
                    }
                    self.g().dest = Dest::Skip;
                }
            }
            "chpgn" => {
                // Fußnotenzeichen ergibt keinen Text; Seitenzahl nur in Kopf-/Fußzeile
                if matches!(self.g().dest, Dest::Header | Dest::Footer) {
                    self.page_marker();
                }
            }
            "par" | "sect" => self.par(),
            "line" => {
                if self.in_row {
                    self.cell.text.push('\n');
                } else if self.g().dest == Dest::Normal {
                    self.flush_para(false);
                }
            }
            "tab" => self.emit("\t"),
            "page" => {
                if self.g().dest == Dest::Normal && !self.in_row {
                    self.flush_para(false);
                    self.b.paras.push(Para::new(Kind::PageBreak));
                    self.after_page = true;
                }
            }
            "pagebb" => self.pagebb = true,
            "pard" => {
                self.align = Align::Left;
                self.li = 0;
                self.list = None;
                self.ls = None;
                self.ilvl = None;
                self.sty = None;
                self.outline = None;
            }
            "plain" => self.g().st = CStyle::default(),
            "b" => self.g().st.b = on,
            "i" => self.g().st.i = on,
            "ul" | "uld" | "uldb" | "ulw" | "ulth" | "ulwave" | "uldash" => self.g().st.u = on,
            "ulnone" => self.g().st.u = false,
            "strike" | "striked" => self.g().st.s = on,
            "super" => self.g().st.va = VAlign::Sup,
            "sub" => self.g().st.va = VAlign::Sub,
            "nosupersub" => self.g().st.va = VAlign::Base,
            "up" => self.g().st.va = if p.unwrap_or(6) > 0 { VAlign::Sup } else { VAlign::Base },
            "dn" => self.g().st.va = if p.unwrap_or(6) > 0 { VAlign::Sub } else { VAlign::Base },
            "fs" => {
                let v = p.unwrap_or(24) as f32 / 2.0;
                if (1.0..=400.0).contains(&v) {
                    self.g().st.size = Some(v);
                    if self.stack.len() <= 2 && !self.text_seen {
                        self.size_doc = Some(v);
                    }
                }
            }
            "cf" => {
                let c = self.colors.get(p.unwrap_or(0).max(0) as usize).copied().unwrap_or(0);
                self.g().st.color = c;
            }
            "highlight" | "chcbpat" => {
                let c = self.colors.get(p.unwrap_or(0).max(0) as usize).copied().unwrap_or(0);
                self.g().st.hl = c;
            }
            "f" => {
                let sans = self.fonts.get(&p.unwrap_or(0)).copied().unwrap_or(false);
                self.g().st.fam = Some(if sans { Family::Sans } else { Family::Serif });
            }
            "deff" => self.deff = p.unwrap_or(0),
            "uc" => self.g().uc = p.unwrap_or(1).clamp(0, 8) as usize,
            "u" => self.unicode(p.unwrap_or(63)),
            "qc" => self.align = Align::Center,
            "qr" => self.align = Align::Right,
            "qj" => self.align = Align::Justify,
            "ql" => self.align = Align::Left,
            "li" => self.li = p.unwrap_or(0),
            "ls" => self.ls = p,
            "ilvl" => self.ilvl = p.map(|v| v.max(0) as usize),
            "s" => self.sty = p,
            "outlinelevel" => self.outline = p.map(|v| v.clamp(0, 8) as u8),
            "bullet" => self.emit("•"),
            "emdash" => self.emit("—"),
            "endash" => self.emit("–"),
            "lquote" => self.emit("‘"),
            "rquote" => self.emit("’"),
            "ldblquote" => self.emit("“"),
            "rdblquote" => self.emit("”"),
            "emspace" | "enspace" | "qmspace" => self.emit(" "),
            "trowd" => {
                if !self.in_row {
                    if self.rows.is_empty() {
                        self.flush_para(false);
                    }
                    self.in_row = true;
                    self.row.clear();
                    self.defs.clear();
                    self.cur_bg = 0;
                    self.row_hdr = false;
                    self.cell = CellB::default();
                }
            }
            "trhdr" => self.row_hdr = true,
            "clcbpat" => self.cur_bg = self.colors.get(p.unwrap_or(0).max(0) as usize).copied().unwrap_or(0),
            "cellx" => {
                self.defs.push(self.cur_bg);
                self.cur_bg = 0;
            }
            "cell" | "nestcell" => {
                if self.in_row {
                    let mut c = std::mem::take(&mut self.cell);
                    c.bg = self.defs.get(self.row.len()).copied().unwrap_or(0);
                    c.align = self.align;
                    self.row.push(c.done());
                }
            }
            "row" | "nestrow" => {
                if self.in_row {
                    if self.rows.is_empty() {
                        self.hdr = self.row_hdr;
                    }
                    self.rows.push(std::mem::take(&mut self.row));
                    self.in_row = false;
                }
            }
            "paperw" => self.page[0] = p.map(|v| v as f32 / 20.0),
            "paperh" => self.page[1] = p.map(|v| v as f32 / 20.0),
            "margl" => self.page[2] = p.map(|v| v as f32 / 20.0),
            "margr" => self.page[3] = p.map(|v| v as f32 / 20.0),
            "margt" => self.page[4] = p.map(|v| v as f32 / 20.0),
            "margb" => self.page[5] = p.map(|v| v as f32 / 20.0),
            _ => {}
        }
    }

    /// Steuerzeichen/-wort ab `i` (hinter dem Backslash); liefert die Position dahinter.
    fn control(&mut self, b: &[u8], mut i: usize) -> usize {
        let Some(&c) = b.get(i) else { return i };
        if !c.is_ascii_alphabetic() {
            i += 1;
            match c {
                b'\'' => {
                    if let Some(h) = hex2(b, i) {
                        i += 2;
                        let ch = cp1252(h);
                        self.text(&ch.to_string());
                    }
                }
                b'\\' | b'{' | b'}' => self.text(&(c as char).to_string()),
                b'~' => self.text(" "),
                b'_' => self.text("-"),
                b'*' => self.star = true,
                b'\n' | b'\r' => self.par(),
                _ => {}
            }
            return i;
        }
        let w0 = i;
        while i < b.len() && b[i].is_ascii_alphabetic() {
            i += 1;
        }
        let word = std::str::from_utf8(&b[w0..i]).unwrap_or("");
        let neg = b.get(i) == Some(&b'-');
        if neg {
            i += 1;
        }
        let d0 = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let param = if i > d0 {
            std::str::from_utf8(&b[d0..i.min(d0 + 9)]).ok().and_then(|s| s.parse::<i32>().ok()).map(|v| if neg { -v } else { v })
        } else {
            if neg {
                i -= 1;
            }
            None
        };
        if b.get(i) == Some(&b' ') {
            i += 1;
        }
        self.word(word, param);
        i
    }

    fn finish(mut self) -> (TextDoc, Images) {
        self.end_table();
        if self.b.has_text() {
            self.flush_para(false);
        }
        self.end_table();
        let mut td = TextDoc::new();
        td.family = if self.fonts.get(&self.deff).copied().unwrap_or(false) { Family::Sans } else { Family::Serif };
        if let Some(s) = self.size_doc {
            td.size = s.clamp(4.0, 96.0);
        }
        let mut paras = self.b.paras;
        while paras.len() > 1 && paras.last().is_some_and(|p| p.kind == Kind::Body && p.is_empty()) {
            paras.pop();
        }
        for p in &mut paras {
            for r in &mut p.runs {
                if r.st.size.is_some_and(|s| (s - td.size).abs() < 0.05) {
                    r.st.size = None;
                }
                if r.st.fam == Some(td.family) {
                    r.st.fam = None;
                }
            }
            p.normalize();
        }
        if !paras.is_empty() {
            td.paras = paras;
        }
        td.header = self.header;
        td.footer = self.footer;
        let pg = self.page;
        if let (Some(w), Some(h)) = (pg[0], pg[1]) {
            if w >= 72.0 && h >= 72.0 {
                td.page.w = w;
                td.page.h = h;
                for (v, f) in [(pg[2], &mut td.page.ml), (pg[3], &mut td.page.mr), (pg[4], &mut td.page.mt), (pg[5], &mut td.page.mb)] {
                    if let Some(v) = v.filter(|v| (0.0..=w.min(h) / 2.0).contains(v)) {
                        *f = v;
                    }
                }
            }
        }
        (td, self.b.store.files)
    }
}

fn rtf_import(bytes: &[u8]) -> Result<(TextDoc, Images), String> {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace() && *b != 0xEF && *b != 0xBB && *b != 0xBF).unwrap_or(0);
    if !bytes[start..].starts_with(b"{\\rtf") {
        return Err("Das ist keine RTF-Datei (.rtf).".into());
    }
    let mut r = Ri::new();
    let (mut i, n) = (start, bytes.len());
    while i < n {
        match bytes[i] {
            b'{' => {
                r.push_group();
                i += 1;
            }
            b'}' => {
                r.pop_group();
                i += 1;
            }
            b'\\' => i = r.control(bytes, i + 1),
            b'\r' | b'\n' => i += 1,
            _ => {
                let mut j = i;
                while j < n && !matches!(bytes[j], b'{' | b'}' | b'\\' | b'\r' | b'\n') {
                    j += 1;
                }
                let s: String = bytes[i..j].iter().map(|&c| cp1252(c)).collect();
                r.text(&s);
                i = j;
            }
        }
    }
    Ok(r.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &str, st: CStyle) -> Run {
        Run { text: t.into(), st }
    }

    fn png() -> Vec<u8> {
        let im = image::RgbaImage::from_pixel(8, 4, image::Rgba([200, 30, 30, 255]));
        let mut o = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(im).write_to(&mut o, image::ImageFormat::Png).unwrap();
        o.into_inner()
    }

    /// Dokument mit Überschrift, Fett, Link, Listen, Tabelle und Fußnote.
    fn sample() -> TextDoc {
        let mut d = TextDoc::new();
        let b = CStyle { b: true, ..CStyle::default() };
        let link = CStyle { link: Some("https://example.org/a?x=1&y=2".into()), ..CStyle::default() };
        let note = CStyle { field: Some(Field::Note("Fußnote & Co".into())), ..CStyle::default() };
        let mut body = Para::new(Kind::Body);
        body.runs = vec![run("Das ist ", CStyle::default()), run("fett", b), run(" und ein ", CStyle::default()), run("Link", link), run(".", CStyle::default()), run(&OBJ.to_string(), note)];
        let mut t = Table::new(2, 2, 300.0);
        t.header = true;
        t.rows[0][0] = Cell { text: "Kopf".into(), ..Cell::default() };
        t.rows[0][1] = Cell::new("Zahl");
        t.rows[1][0] = Cell { text: "a2".into(), b: true, bg: 0xFFFFEE99, ..Cell::default() };
        t.rows[1][1] = Cell { text: "42".into(), align: Align::Right, ..Cell::default() };
        let mut sub = Para::with_text(Kind::Bullet, "Unterpunkt");
        sub.indent = 1;
        let mut deleted = Para::with_text(Kind::Body, "Behalten");
        deleted.runs.push(run(" GELÖSCHT", CStyle { del: 1, ..CStyle::default() }));
        d.paras = vec![
            Para::with_text(Kind::H1, "Überschrift"),
            body,
            Para::with_text(Kind::Bullet, "Punkt 1"),
            sub,
            Para::with_text(Kind::Number, "Eins"),
            Para::with_text(Kind::Number, "Zwei"),
            Para { table: Some(t), ..Para::new(Kind::Table) },
            Para::with_text(Kind::Quote, "Zitat"),
            Para::new(Kind::PageBreak),
            deleted,
        ];
        d.paras[1].align = Align::Center;
        d
    }

    fn all_runs(d: &TextDoc) -> Vec<&Run> {
        d.paras.iter().flat_map(|p| p.runs.iter()).collect()
    }

    fn check(d: &TextDoc, what: &str) {
        let h = d.paras.iter().find(|p| p.kind == Kind::H1).unwrap_or_else(|| panic!("{what}: keine Überschrift: {:?}", d.paras.iter().map(|p| p.kind).collect::<Vec<_>>()));
        assert_eq!(h.text(), "Überschrift", "{what}");
        assert!(all_runs(d).iter().any(|r| r.st.b && r.text.contains("fett")), "{what}: fett fehlt");
        assert!(all_runs(d).iter().any(|r| r.st.link.as_deref() == Some("https://example.org/a?x=1&y=2") && r.text == "Link"), "{what}: Link fehlt");
        let kinds: Vec<(Kind, u8)> = d.paras.iter().filter(|p| p.kind.is_list()).map(|p| (p.kind, p.indent)).collect();
        assert_eq!(kinds, [(Kind::Bullet, 0), (Kind::Bullet, 1), (Kind::Number, 0), (Kind::Number, 0)], "{what}: Listen");
        let t = d.paras.iter().find_map(|p| p.table.as_ref()).unwrap_or_else(|| panic!("{what}: Tabelle fehlt"));
        assert_eq!((t.rows.len(), t.ncols()), (2, 2), "{what}");
        assert_eq!(t.rows[1][0].text, "a2", "{what}");
        assert!(t.header, "{what}: Kopfzeile");
        assert!(t.rows[1][0].b, "{what}: Zell-Fett");
        assert!(!t.rows[0][0].b, "{what}: Kopf nicht extra fett");
        if !what.starts_with("Markdown") {
            assert_eq!(t.rows[1][0].bg & 0xFFFFFF, 0xFFEE99, "{what}: Zellhintergrund");
        }
        assert_eq!(t.rows[0][0].bg, 0, "{what}: Kopfzeilen-Tönung");
        assert_eq!(t.rows[1][1].align, Align::Right, "{what}");
        assert!(all_runs(d).iter().all(|r| !r.text.contains("GELÖSCHT")), "{what}: Gelöschtes");
        assert!(all_runs(d).iter().any(|r| r.text.contains("Behalten")), "{what}");
        assert!(d.paras.iter().any(|p| p.kind == Kind::PageBreak), "{what}: Seitenumbruch");
        assert!(d.paras.iter().any(|p| p.kind == Kind::Quote && p.text() == "Zitat"), "{what}: Zitat");
        let n = all_runs(d).into_iter().find_map(|r| match &r.st.field {
            Some(Field::Note(t)) => Some(t.clone()),
            _ => None,
        });
        assert_eq!(n.as_deref(), Some("Fußnote & Co"), "{what}: Fußnote");
    }

    #[test]
    fn rundreise_je_format() {
        let d = sample();
        for f in [TextFmt::Odt, TextFmt::Rtf, TextFmt::Html, TextFmt::Md] {
            let bytes = export(&d, f, "Titel", &Images::new()).unwrap();
            let (back, _) = import(&bytes, f).unwrap();
            check(&back, f.label());
        }
    }

    #[test]
    fn ausrichtung_bleibt() {
        let d = sample();
        for f in [TextFmt::Odt, TextFmt::Rtf, TextFmt::Html] {
            let (back, _) = import(&export(&d, f, "T", &Images::new()).unwrap(), f).unwrap();
            let p = back.paras.iter().find(|p| p.text().starts_with("Das ist")).unwrap();
            assert_eq!(p.align, Align::Center, "{f:?}");
        }
    }

    #[test]
    fn text_rundreise() {
        let d = sample();
        let s = String::from_utf8(export(&d, TextFmt::Txt, "T", &Images::new()).unwrap()).unwrap();
        assert!(s.contains("Überschrift\n"));
        assert!(s.contains("• Punkt 1"));
        assert!(s.contains("  • Unterpunkt"));
        assert!(s.contains("2. Zwei"));
        assert!(s.contains("Kopf\tZahl"));
        assert!(s.contains("[1] Fußnote & Co"));
        assert!(!s.contains("GELÖSCHT"));
        let (back, _) = import(s.as_bytes(), TextFmt::Txt).unwrap();
        assert_eq!(back.paras[0].text(), "Überschrift");
        assert!(back.paras.iter().any(|p| p.kind == Kind::PageBreak));
        // BOM und Windows-1252
        let (b, _) = import("\u{feff}Hallo\r\nWörter\r\n".as_bytes(), TextFmt::Txt).unwrap();
        assert_eq!(b.paras.iter().map(Para::text).collect::<Vec<_>>(), ["Hallo", "Wörter"]);
        let (b, _) = import(b"Hallo\r\nW\xF6rter \x80\r\n", TextFmt::Txt).unwrap();
        assert_eq!(b.paras.iter().map(Para::text).collect::<Vec<_>>(), ["Hallo", "Wörter €"]);
        let (b, _) = import("Grüße".as_bytes(), TextFmt::Txt).unwrap();
        assert_eq!(b.paras[0].text(), "Grüße");
    }

    #[test]
    fn html_snippet() {
        let h = r#"<html><head><title>Nix</title><style>p{color:red}</style></head><body>
            <h1>Titel &amp; mehr</h1><p>Ein <b>fetter</b> und <i>schiefer</i> Satz mit <a href="http://x.de/?a=1&amp;b=2">Link</a>,
            <span style="color:#ff0000;background-color:yellow;font-weight:bold">rot</span> H<sub>2</sub>O&nbsp;m<sup>2</sup> &#228;&#x20AC;<br>neue Zeile</p>
            <ul><li>a<ul><li>b</li></ul></li><li>c</li></ul><ol><li>x</li></ol>
            <blockquote>Zitat</blockquote><p style="text-align:center"><del>weg</del> <u>unter</u></p>
            <table><tr><th>K1</th><th>K2</th></tr><tr><td style="background-color:#00ff00">1</td><td><b>2</b></td></tr></table></body></html>"#;
        let (d, _) = import(h.as_bytes(), TextFmt::Html).unwrap();
        let k: Vec<(Kind, String)> = d.paras.iter().map(|p| (p.kind, p.text())).collect();
        assert_eq!(k[0], (Kind::H1, "Titel & mehr".into()));
        assert_eq!(k[1], (Kind::Body, "Ein fetter und schiefer Satz mit Link, rot H2O m2 ä€".into()));
        assert_eq!(k[2], (Kind::Body, "neue Zeile".into()));
        let lists: Vec<(Kind, u8, String)> = d.paras.iter().filter(|p| p.kind.is_list()).map(|p| (p.kind, p.indent, p.text())).collect();
        assert_eq!(lists, [(Kind::Bullet, 0, "a".into()), (Kind::Bullet, 1, "b".into()), (Kind::Bullet, 0, "c".into()), (Kind::Number, 0, "x".into())]);
        assert!(d.paras.iter().any(|p| p.kind == Kind::Quote && p.text() == "Zitat"));
        let runs = all_runs(&d);
        assert!(runs.iter().any(|r| r.text == "fetter" && r.st.b));
        assert!(runs.iter().any(|r| r.text == "schiefer" && r.st.i));
        assert!(runs.iter().any(|r| r.text == "Link" && r.st.link.as_deref() == Some("http://x.de/?a=1&b=2")));
        assert!(runs.iter().any(|r| r.text == "rot" && r.st.b && r.st.color == 0xFFFF0000 && r.st.hl == 0xFFFFFF00));
        assert!(runs.iter().any(|r| r.text == "2" && r.st.va == VAlign::Sub));
        assert!(runs.iter().any(|r| r.text.starts_with('2') && r.st.va == VAlign::Sup || r.text == "2" && r.st.va == VAlign::Sup));
        assert!(runs.iter().any(|r| r.text == "weg" && r.st.s));
        assert!(runs.iter().any(|r| r.text == "unter" && r.st.u));
        let c = d.paras.iter().find(|p| p.text().starts_with("weg")).unwrap();
        assert_eq!(c.align, Align::Center);
        let t = d.paras.iter().find_map(|p| p.table.as_ref()).unwrap();
        assert!(t.header);
        assert_eq!(t.rows[0][1].text, "K2");
        assert_eq!(t.rows[1][0].bg, 0xFF00FF00);
        assert!(t.rows[1][1].b);
        assert!(!d.plain_text().contains("Nix") && !d.plain_text().contains("color:red"));
    }

    #[test]
    fn html_ohne_struktur() {
        let (d, _) = import(b"Nur Text <b>fett</b> ohne Tags", TextFmt::Html).unwrap();
        assert_eq!(d.plain_text(), "Nur Text fett ohne Tags");
        let (d, _) = import(b"", TextFmt::Html).unwrap();
        assert_eq!(d.paras.len(), 1);
    }

    #[test]
    fn rtf_mit_umlauten() {
        let rtf = br"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fswiss Arial;}}{\colortbl;\red255\green0\blue0;}\pard\qc Gr\'fc\'dfe \u8364? und \u228?\u-10179?\u-8704?\b fett\b0\par \cf1\ul rot\ulnone\cf0\par}";
        let (d, _) = import(rtf, TextFmt::Rtf).unwrap();
        assert_eq!(d.paras[0].text(), "Grüße € und ä\u{1F600}fett");
        assert_eq!(d.paras[0].align, Align::Center);
        assert_eq!(d.family, Family::Sans);
        assert!(d.paras[0].runs.iter().any(|r| r.text.contains("fett") && r.st.b));
        assert_eq!(d.paras[1].text(), "rot");
        assert_eq!(d.paras[1].runs[0].st.color, 0xFFFF0000);
        assert!(d.paras[1].runs[0].st.u);
        assert_eq!(d.paras.len(), 2);
        // Export mit Umlauten und Sonderzeichen
        let mut e = TextDoc::new();
        e.paras = vec![Para::with_text(Kind::Body, "Größe {a} \\ ä€\u{1F600}\tEnde")];
        let (back, _) = import(&export(&e, TextFmt::Rtf, "T", &Images::new()).unwrap(), TextFmt::Rtf).unwrap();
        assert_eq!(back.paras[0].text(), "Größe {a} \\ ä€\u{1F600}\tEnde");
        assert!(import(b"kein rtf", TextFmt::Rtf).is_err());
    }

    #[test]
    fn rtf_fremde_liste_und_tabelle() {
        let rtf = br"{\rtf1\ansi{\*\listtable{\list\listtemplateid1{\listlevel\levelnfc23}\listid11}{\list\listtemplateid2{\listlevel\levelnfc0}\listid12}}{\*\listoverridetable{\listoverride\listid11\listoverridecount0\ls1}{\listoverride\listid12\listoverridecount0\ls2}}
\pard\ls1\ilvl0{\listtext\u8226?\tab}Punkt\par\pard\ls2\ilvl1{\listtext 1.\tab}Nummer\par
\trowd\cellx1000\cellx2000\pard\intbl A\cell B\cell\row\trowd\cellx1000\cellx2000\pard\intbl C\cell D\cell\row\pard Danach\par}";
        let (d, _) = import(rtf, TextFmt::Rtf).unwrap();
        assert_eq!((d.paras[0].kind, d.paras[0].indent, d.paras[0].text()), (Kind::Bullet, 0, "Punkt".into()));
        assert_eq!((d.paras[1].kind, d.paras[1].indent), (Kind::Number, 1));
        let t = d.paras[2].table.as_ref().unwrap();
        assert_eq!((t.rows.len(), t.rows[1][1].text.as_str()), (2, "D"));
        assert_eq!(d.paras[3].text(), "Danach");
    }

    #[test]
    fn odt_stile_aus_fremder_datei() {
        // minimale ODT mit Automatikstilen, Überschrift, Liste und Bild
        let content = r##"<?xml version="1.0"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0">
<office:automatic-styles><style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold" fo:color="#ff0000"/></style:style><style:style style:name="T2" style:family="text"><style:text-properties fo:font-style="italic" style:text-position="super 58%"/></style:style><style:style style:name="P1" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:text-align="end" fo:break-before="page"/></style:style></office:automatic-styles>
<office:body><office:text><text:h text:outline-level="2">Kapitel</text:h><text:p text:style-name="P1">Hallo <text:span text:style-name="T1">rot fett</text:span><text:s text:c="2"/><text:span text:style-name="T2">hoch</text:span><text:a xlink:href="http://a.b/">Ziel</text:a></text:p>
<text:list><text:list-item><text:p>eins</text:p><text:list><text:list-item><text:p>unter</text:p></text:list-item></text:list></text:list-item></text:list></office:text></office:body></office:document-content>"##;
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        z.start_file("content.xml", SimpleFileOptions::default()).unwrap();
        z.write_all(content.as_bytes()).unwrap();
        let bytes = z.finish().unwrap().into_inner();
        let (d, _) = import(&bytes, TextFmt::Odt).unwrap();
        let k: Vec<(Kind, String)> = d.paras.iter().map(|p| (p.kind, p.text())).collect();
        assert_eq!(k[0], (Kind::H2, "Kapitel".into()));
        assert_eq!(k[1].0, Kind::PageBreak);
        assert_eq!(k[2], (Kind::Body, "Hallo rot fett  hochZiel".into()));
        assert_eq!(d.paras[2].align, Align::Right);
        assert!(d.paras[2].runs.iter().any(|r| r.text == "rot fett" && r.st.b && r.st.color == 0xFFFF0000));
        assert!(d.paras[2].runs.iter().any(|r| r.text == "hoch" && r.st.i && r.st.va == VAlign::Sup));
        assert!(d.paras[2].runs.iter().any(|r| r.text == "Ziel" && r.st.link.as_deref() == Some("http://a.b/")));
        assert_eq!((d.paras[3].kind, d.paras[3].indent, d.paras[4].indent), (Kind::Bullet, 0, 1));
        assert!(import(b"nix", TextFmt::Odt).is_err());
    }

    #[test]
    fn odt_ist_gueltiger_container() {
        let bytes = export(&sample(), TextFmt::Odt, "A <&> B", &Images::new()).unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(&bytes[..])).unwrap();
        assert_eq!(&*z.by_index(0).unwrap().name().unwrap(), "mimetype");
        for n in ["content.xml", "styles.xml", "meta.xml", "META-INF/manifest.xml"] {
            let s = read_entry(&mut z, n).unwrap();
            roxmltree::Document::parse(&s).unwrap_or_else(|e| panic!("{n}: {e}"));
        }
    }

    #[test]
    fn kopf_und_fusszeile() {
        let mut d = sample();
        d.header = "Bericht".into();
        d.footer = "Seite {seite} von {seiten}".into();
        for f in [TextFmt::Odt, TextFmt::Rtf] {
            let (b, _) = import(&export(&d, f, "T", &Images::new()).unwrap(), f).unwrap();
            assert_eq!((b.header.as_str(), b.footer.as_str()), ("Bericht", "Seite {seite} von {seiten}"), "{f:?}");
        }
        let (b, _) = import(&export(&d, TextFmt::Odt, "T", &Images::new()).unwrap(), TextFmt::Odt).unwrap();
        assert!((b.page.w - d.page.w).abs() < 0.1 && (b.page.ml - d.page.ml).abs() < 0.1);
    }

    #[test]
    fn bilder_rundreise() {
        let mut d = TextDoc::new();
        let mut imgs = Images::new();
        imgs.insert("a.png".into(), png());
        d.paras = vec![Para::with_text(Kind::Body, "Vor"), Para::image("a.png", 80.0, 40.0), Para::with_text(Kind::Body, "Nach")];
        for f in [TextFmt::Odt, TextFmt::Rtf, TextFmt::Html, TextFmt::Md] {
            let (b, files) = import(&export(&d, f, "T", &imgs).unwrap(), f).unwrap();
            let kinds: Vec<Kind> = b.paras.iter().map(|p| p.kind).collect();
            assert_eq!(kinds, [Kind::Body, Kind::Image, Kind::Body], "{f:?}");
            let im = b.paras[1].img.as_ref().unwrap();
            assert_eq!(files.len(), 1, "{f:?}");
            assert!(files.contains_key(&im.file));
            if f != TextFmt::Md {
                assert!((im.w - 80.0).abs() < 1.0 && (im.h - 40.0).abs() < 1.0, "{f:?}: {} x {}", im.w, im.h);
            }
        }
        let h = String::from_utf8(export(&d, TextFmt::Html, "T", &imgs).unwrap()).unwrap();
        assert!(h.contains("src=\"data:image/png;base64,"));
    }

    #[test]
    fn felder_formeln_objekte() {
        let mut d = TextDoc::new();
        let mut cap = Para::with_text(Kind::Caption, "Haus");
        cap.cap = "Abbildung".into();
        let f = Obj { kind: ObjKind::Formula { src: "x^2+1".into() }, w: 60.0, h: 20.0 };
        d.paras = vec![cap, Para { obj: Some(f), ..Para::new(Kind::Object) }, Para { obj: Some(Obj::chart()), ..Para::new(Kind::Object) }, Para::with_text(Kind::Body, "Seite ")];
        d.paras[3].insert(6, &OBJ.to_string(), &CStyle { field: Some(Field::Page), ..CStyle::default() });
        let md = String::from_utf8(export(&d, TextFmt::Md, "T", &Images::new()).unwrap()).unwrap();
        assert!(md.contains("Abbildung 1: Haus"), "{md}");
        assert!(md.contains("$$x^2+1$$"), "{md}");
        assert!(md.contains("[Diagramm: Diagramm]") || md.contains("data:image/png"));
        let (b, _) = import(md.as_bytes(), TextFmt::Md).unwrap();
        assert!(b.paras.iter().any(|p| matches!(p.obj.as_ref().map(|o| &o.kind), Some(ObjKind::Formula { src }) if src == "x^2+1")));
        let h = String::from_utf8(export(&d, TextFmt::Html, "T", &Images::new()).unwrap()).unwrap();
        assert!(h.contains("<code class=\"formula\">x^2+1</code>"));
        let (b, _) = import(h.as_bytes(), TextFmt::Html).unwrap();
        let c = b.paras.iter().find(|p| p.kind == Kind::Caption).unwrap();
        assert_eq!((c.cap.as_str(), c.text().as_str()), ("Abbildung", "Haus"));
        let t = String::from_utf8(export(&d, TextFmt::Txt, "T", &Images::new()).unwrap()).unwrap();
        assert!(t.contains("[Formel: x^2+1]") && t.contains("Seite #"), "{t}");
    }

    #[test]
    fn markdown_import_und_export() {
        let md = "# Titel\n\n## Kapitel\n\nText mit **fett**, *kursiv*, ~~weg~~, `code` und [Link](http://a.de/x_y_z) sowie snake_case_name und 2 * 3.\nZweite Zeile\n\n- eins\n    - zwei\n- drei\n\n1. a\n2. b\n\n> Zitat\n\n| A | B |\n|:--|--:|\n| 1 | **2** |\n\n\\pagebreak\n\nEnde[^1]\n\n[^1]: Die Note.\n";
        let (d, _) = import(md.as_bytes(), TextFmt::Md).unwrap();
        let k: Vec<(Kind, u8)> = d.paras.iter().map(|p| (p.kind, p.indent)).collect();
        assert_eq!(k, [(Kind::Title, 0), (Kind::H1, 0), (Kind::Body, 0), (Kind::Bullet, 0), (Kind::Bullet, 1), (Kind::Bullet, 0), (Kind::Number, 0), (Kind::Number, 0), (Kind::Quote, 0), (Kind::Table, 0), (Kind::PageBreak, 0), (Kind::Body, 0)]);
        assert_eq!(d.paras[2].text(), "Text mit fett, kursiv, weg, code und Link sowie snake_case_name und 2 * 3. Zweite Zeile");
        let runs = &d.paras[2].runs;
        assert!(runs.iter().any(|r| r.text == "fett" && r.st.b) && runs.iter().any(|r| r.text == "kursiv" && r.st.i) && runs.iter().any(|r| r.text == "weg" && r.st.s));
        assert!(runs.iter().any(|r| r.text == "Link" && r.st.link.as_deref() == Some("http://a.de/x_y_z")));
        let t = d.paras[9].table.as_ref().unwrap();
        assert_eq!((t.rows[1][1].text.as_str(), t.rows[1][1].b, t.rows[0][1].align), ("2", true, Align::Right));
        assert!(d.paras[11].runs.iter().any(|r| matches!(&r.st.field, Some(Field::Note(t)) if t == "Die Note.")));
        // Zeichen, die Markdown deuten würde, überleben die Rundreise
        let mut e = TextDoc::new();
        e.paras = vec![Para::with_text(Kind::Body, "# kein * Titel_ [x](y) <u> 1. $$ | `"), Para::with_text(Kind::Body, "- nicht Liste")];
        let (b, _) = import(&export(&e, TextFmt::Md, "T", &Images::new()).unwrap(), TextFmt::Md).unwrap();
        assert_eq!(b.paras.iter().map(Para::text).collect::<Vec<_>>(), ["# kein * Titel_ [x](y) <u> 1. $$ | `", "- nicht Liste"]);
        assert_eq!(b.paras.iter().map(|p| p.kind).collect::<Vec<_>>(), [Kind::Body, Kind::Body]);
    }

    #[test]
    fn markdown_titel_ebene() {
        let mut d = TextDoc::new();
        d.paras = vec![Para::with_text(Kind::Title, "T"), Para::with_text(Kind::H1, "A"), Para::with_text(Kind::H2, "B"), Para::with_text(Kind::H3, "C")];
        let (b, _) = import(&export(&d, TextFmt::Md, "T", &Images::new()).unwrap(), TextFmt::Md).unwrap();
        assert_eq!(b.paras.iter().map(|p| p.kind).collect::<Vec<_>>(), [Kind::Title, Kind::H1, Kind::H2, Kind::H3]);
    }

    #[test]
    fn formate_und_endungen() {
        for f in TextFmt::ALL {
            assert_eq!(TextFmt::from_ext(f.ext()), Some(f));
            assert_eq!(TextFmt::from_ext(&format!(".{}", f.ext().to_uppercase())), Some(f));
            assert!(f.label().contains(f.ext()) && !f.mime().is_empty());
        }
        assert_eq!(TextFmt::from_ext("htm"), Some(TextFmt::Html));
        assert_eq!(TextFmt::from_ext("docx"), None);
    }

    #[test]
    fn robust_gegen_muell() {
        for f in TextFmt::ALL {
            for bytes in [&b""[..], b"{", b"\xFF\xFE\x00", b"<<<>>>&&&;", b"{\\rtf1 {{{{\\u", b"PK\x03\x04", b"|||\n---|---\n"] {
                let _ = import(bytes, f);
            }
        }
        assert_eq!(unescape("a &amp b &lt;&#65;&#x42;&bogus; &"), "a &amp b <AB&bogus; &");
        assert_eq!(b64_dec(&b64_enc(b"abcde")).unwrap(), b"abcde");
    }
}
