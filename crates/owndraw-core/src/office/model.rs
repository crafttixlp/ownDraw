//! Dokumentmodell der Textverarbeitung: Absätze aus Textläufen mit Zeichenformat, Absatzformat (Vorlage,
//! Ausrichtung, Einzug, Zeilenabstand) und Seite. Positionen zählen Zeichen (`char`), nicht Bytes.
//!
//! JSON (`text.json` im `.owndraw`-Container):
//! `{"v":1,"page":{w,h,ml,mr,mt,mb},"font":"serif","size":11,"header":"…","footer":"…",
//!   "paras":[{"k":"h1","a":"c","in":1,"ls":1.5,"r":[{"t":"Text","b":1,"i":1,"u":1,"s":1,"sz":14,"c":-16777216,
//!   "hl":-256,"f":"sans","va":"sup","ln":"https://…"}]}]}`
//! Unbekannte Felder werden beim Lesen ignoriert. Kopf- und Fußzeile dürfen `{seite}` und `{seiten}` enthalten.

pub use super::fonts::Family;
use serde_json::{json, Map, Value};

/// Hoch-/Tiefstellung.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VAlign {
    #[default]
    Base,
    Sup,
    Sub,
}

/// Zeichenformat eines Laufs. `None`/0 = vom Absatz bzw. Dokument übernehmen.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CStyle {
    pub b: bool,
    pub i: bool,
    pub u: bool,
    pub s: bool,
    pub size: Option<f32>,
    /// Textfarbe ARGB, 0 = automatisch.
    pub color: u32,
    /// Hervorhebung ARGB, 0 = keine.
    pub hl: u32,
    pub fam: Option<Family>,
    pub va: VAlign,
    pub link: Option<String>,
}

/// Absatzvorlage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Body,
    Title,
    H1,
    H2,
    H3,
    Quote,
    Bullet,
    Number,
    /// Seitenumbruch (leerer Absatz, danach beginnt eine neue Seite).
    PageBreak,
    /// Bild als eigener Absatz (`Para::img`).
    Image,
}

impl Kind {
    pub const ALL: [Kind; 10] = [Kind::Body, Kind::Title, Kind::H1, Kind::H2, Kind::H3, Kind::Quote, Kind::Bullet, Kind::Number, Kind::PageBreak, Kind::Image];
    pub fn name(self) -> &'static str {
        match self {
            Kind::Body => "p",
            Kind::Title => "title",
            Kind::H1 => "h1",
            Kind::H2 => "h2",
            Kind::H3 => "h3",
            Kind::Quote => "quote",
            Kind::Bullet => "ul",
            Kind::Number => "ol",
            Kind::PageBreak => "pb",
            Kind::Image => "img",
        }
    }
    pub fn from_name(s: &str) -> Kind {
        Kind::ALL.into_iter().find(|k| k.name() == s).unwrap_or_default()
    }
    /// Anzeigename (Formatvorlagen-Liste).
    pub fn label(self) -> &'static str {
        match self {
            Kind::Body => "Standardtext",
            Kind::Title => "Titel",
            Kind::H1 => "Überschrift 1",
            Kind::H2 => "Überschrift 2",
            Kind::H3 => "Überschrift 3",
            Kind::Quote => "Zitat",
            Kind::Bullet => "Aufzählung",
            Kind::Number => "Nummerierung",
            Kind::PageBreak => "Seitenumbruch",
            Kind::Image => "Bild",
        }
    }
    pub fn is_heading(self) -> bool {
        matches!(self, Kind::Title | Kind::H1 | Kind::H2 | Kind::H3)
    }
    /// Absatz ohne Text (Seitenumbruch, Bild): Tippen legt dahinter einen neuen Absatz an.
    pub fn is_block(self) -> bool {
        matches!(self, Kind::PageBreak | Kind::Image)
    }
    pub fn is_list(self) -> bool {
        matches!(self, Kind::Bullet | Kind::Number)
    }
    /// Gliederungsebene (Titel 0, Überschrift n = n), sonst None.
    pub fn outline_level(self) -> Option<u8> {
        match self {
            Kind::Title => Some(0),
            Kind::H1 => Some(1),
            Kind::H2 => Some(2),
            Kind::H3 => Some(3),
            _ => None,
        }
    }
}

/// Ausrichtung.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

impl Align {
    fn name(self) -> &'static str {
        match self {
            Align::Left => "l",
            Align::Center => "c",
            Align::Right => "r",
            Align::Justify => "j",
        }
    }
    fn from_name(s: &str) -> Align {
        match s {
            "c" => Align::Center,
            "r" => Align::Right,
            "j" => Align::Justify,
            _ => Align::Left,
        }
    }
}

/// Textlauf: Text mit einheitlichem Format.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Run {
    pub text: String,
    pub st: CStyle,
}

/// Absatz.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Para {
    pub kind: Kind,
    pub align: Align,
    /// Einzugsstufe (0..=8).
    pub indent: u8,
    /// Zeilenabstand als Faktor, 0 = Vorgabe (1,15).
    pub line: f32,
    pub runs: Vec<Run>,
    /// Bild (nur bei `Kind::Image`).
    pub img: Option<Img>,
}

/// Bild im Textdokument: Datei im Bildspeicher des Dokuments, Größe in pt.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Img {
    pub file: String,
    pub w: f32,
    pub h: f32,
}

impl Para {
    /// Bild-Absatz.
    pub fn image(file: &str, w: f32, h: f32) -> Para {
        Para { kind: Kind::Image, img: Some(Img { file: file.into(), w, h }), align: Align::Center, ..Para::new(Kind::Image) }
    }
}

/// Seite in pt (1/72 Zoll).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSpec {
    pub w: f32,
    pub h: f32,
    pub ml: f32,
    pub mr: f32,
    pub mt: f32,
    pub mb: f32,
}

impl Default for PageSpec {
    /// A4 hoch, Ränder 2,5 cm.
    fn default() -> Self {
        PageSpec { w: 595.28, h: 841.89, ml: 70.87, mr: 70.87, mt: 70.87, mb: 70.87 }
    }
}

impl PageSpec {
    pub fn content_w(&self) -> f32 {
        (self.w - self.ml - self.mr).max(36.0)
    }
}

/// Position im Text: Absatz + Zeichen im Absatz.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Pos {
    pub p: usize,
    pub o: usize,
}

impl Pos {
    pub fn new(p: usize, o: usize) -> Pos {
        Pos { p, o }
    }
}

/// Bereich (sortiert).
pub fn ordered(a: Pos, b: Pos) -> (Pos, Pos) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

impl Para {
    pub fn new(kind: Kind) -> Para {
        Para { kind, runs: vec![Run::default()], ..Para::default() }
    }

    pub fn with_text(kind: Kind, text: &str) -> Para {
        Para { kind, runs: vec![Run { text: text.into(), st: CStyle::default() }], ..Para::default() }
    }

    /// Länge in Zeichen.
    pub fn len(&self) -> usize {
        self.runs.iter().map(|r| r.text.chars().count()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.iter().all(|r| r.text.is_empty())
    }

    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }

    /// Zeichen als Liste (mit Laufindex).
    pub fn chars(&self) -> Vec<(char, usize)> {
        let mut v = Vec::new();
        for (i, r) in self.runs.iter().enumerate() {
            v.extend(r.text.chars().map(|c| (c, i)));
        }
        v
    }

    /// Teilt Läufe so, dass an Zeichenposition `o` eine Laufgrenze liegt; liefert den Index des Laufs ab `o`.
    fn cut(&mut self, o: usize) -> usize {
        let mut acc = 0;
        for i in 0..self.runs.len() {
            let n = self.runs[i].text.chars().count();
            if o == acc {
                return i;
            }
            if o < acc + n {
                let k = o - acc;
                let b = self.runs[i].text.char_indices().nth(k).map(|(b, _)| b).unwrap_or(self.runs[i].text.len());
                let rest = self.runs[i].text.split_off(b);
                let st = self.runs[i].st.clone();
                self.runs.insert(i + 1, Run { text: rest, st });
                return i + 1;
            }
            acc += n;
        }
        self.runs.len()
    }

    /// Zeichenformat an der Schreibstelle `o` (Format des Zeichens davor, am Anfang das erste).
    pub fn style_at(&self, o: usize) -> CStyle {
        let mut acc = 0;
        let mut last = None;
        for r in &self.runs {
            let n = r.text.chars().count();
            if n > 0 {
                if o <= acc + n && o > acc {
                    return r.st.clone();
                }
                if o == 0 {
                    return r.st.clone();
                }
                last = Some(&r.st);
            }
            acc += n;
        }
        last.cloned().or_else(|| self.runs.first().map(|r| r.st.clone())).unwrap_or_default()
    }

    /// Fügt Text (ohne Zeilenumbruch) mit Format ein.
    pub fn insert(&mut self, o: usize, s: &str, st: &CStyle) {
        if s.is_empty() {
            return;
        }
        let o = o.min(self.len());
        let i = self.cut(o);
        self.runs.insert(i, Run { text: s.to_string(), st: st.clone() });
        self.normalize();
    }

    /// Löscht Zeichen a..b.
    pub fn delete(&mut self, a: usize, b: usize) {
        let n = self.len();
        let (a, b) = (a.min(n), b.min(n));
        if a >= b {
            return;
        }
        let i = self.cut(a);
        let j = self.cut(b);
        self.runs.drain(i..j);
        self.normalize();
    }

    /// Abschnitt a..b als eigener Absatz (gleiches Absatzformat).
    pub fn slice(&self, a: usize, b: usize) -> Para {
        let mut p = self.clone();
        let n = p.len();
        p.delete(b.min(n), n);
        p.delete(0, a);
        p
    }

    /// Schneidet ab `o` ab und liefert den Rest als neuen Absatz.
    pub fn split_off(&mut self, o: usize) -> Para {
        let i = self.cut(o.min(self.len()));
        let rest: Vec<Run> = self.runs.drain(i..).collect();
        let st = self.style_at(o);
        let mut p = Para { kind: self.kind, align: self.align, indent: self.indent, line: self.line, runs: rest, img: None };
        if p.runs.iter().all(|r| r.text.is_empty()) {
            p.runs = vec![Run { text: String::new(), st }];
        }
        self.normalize();
        p.normalize();
        p
    }

    /// Hängt die Läufe eines anderen Absatzes an.
    pub fn append(&mut self, other: Para) {
        self.runs.extend(other.runs);
        self.normalize();
    }

    /// Ändert das Zeichenformat von a..b.
    pub fn restyle(&mut self, a: usize, b: usize, f: &dyn Fn(&mut CStyle)) {
        let (a, b) = (a.min(self.len()), b.min(self.len()));
        if a >= b {
            return;
        }
        let i = self.cut(a);
        let j = self.cut(b);
        for r in &mut self.runs[i..j] {
            f(&mut r.st);
        }
        self.normalize();
    }

    /// Fasst gleiche Läufe zusammen, entfernt leere (mindestens ein Lauf bleibt).
    pub fn normalize(&mut self) {
        let keep = self.runs.first().map(|r| r.st.clone()).unwrap_or_default();
        let mut out: Vec<Run> = Vec::with_capacity(self.runs.len());
        for r in self.runs.drain(..) {
            if r.text.is_empty() {
                continue;
            }
            match out.last_mut() {
                Some(l) if l.st == r.st => l.text.push_str(&r.text),
                _ => out.push(r),
            }
        }
        if out.is_empty() {
            out.push(Run { text: String::new(), st: keep });
        }
        self.runs = out;
    }

    fn to_json(&self) -> Value {
        let mut o = Map::new();
        if self.kind != Kind::Body {
            o.insert("k".into(), json!(self.kind.name()));
        }
        if self.align != Align::Left {
            o.insert("a".into(), json!(self.align.name()));
        }
        if self.indent > 0 {
            o.insert("in".into(), json!(self.indent));
        }
        if self.line > 0.0 {
            o.insert("ls".into(), json!(self.line as f64));
        }
        o.insert("r".into(), Value::Array(self.runs.iter().map(run_json).collect()));
        if let Some(i) = &self.img {
            o.insert("img".into(), json!({"f": i.file, "w": i.w as f64, "h": i.h as f64}));
        }
        Value::Object(o)
    }

    fn from_json(v: &Value) -> Para {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
        let mut p = Para {
            kind: Kind::from_name(s("k")),
            align: Align::from_name(s("a")),
            indent: v.get("in").and_then(Value::as_u64).unwrap_or(0).min(8) as u8,
            line: v.get("ls").and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite() && *x >= 0.5 && *x <= 4.0).unwrap_or(0.0),
            runs: v.get("r").and_then(Value::as_array).map(|a| a.iter().map(run_from).collect()).unwrap_or_default(),
            img: None,
        };
        if p.kind == Kind::Image {
            let im = v.get("img");
            let f = |k: &str| im.and_then(|i| i.get(k)).and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite() && *x >= 4.0 && *x <= 5000.0);
            match (im.and_then(|i| i.get("f")).and_then(Value::as_str), f("w"), f("h")) {
                (Some(file), Some(w), Some(h)) if !file.is_empty() => p.img = Some(Img { file: file.into(), w, h }),
                _ => p.kind = Kind::Body,
            }
            p.runs = vec![Run::default()];
        }
        p.normalize();
        p
    }
}

fn run_json(r: &Run) -> Value {
    let st = &r.st;
    let mut o = Map::new();
    o.insert("t".into(), json!(r.text));
    for (k, b) in [("b", st.b), ("i", st.i), ("u", st.u), ("s", st.s)] {
        if b {
            o.insert(k.into(), json!(1));
        }
    }
    if let Some(sz) = st.size {
        o.insert("sz".into(), json!(sz as f64));
    }
    if st.color != 0 {
        o.insert("c".into(), json!(st.color as i32));
    }
    if st.hl != 0 {
        o.insert("hl".into(), json!(st.hl as i32));
    }
    if let Some(f) = st.fam {
        o.insert("f".into(), json!(f.name()));
    }
    match st.va {
        VAlign::Sup => {
            o.insert("va".into(), json!("sup"));
        }
        VAlign::Sub => {
            o.insert("va".into(), json!("sub"));
        }
        VAlign::Base => {}
    }
    if let Some(l) = &st.link {
        o.insert("ln".into(), json!(l));
    }
    Value::Object(o)
}

fn color(v: Option<&Value>) -> u32 {
    v.and_then(Value::as_i64).map(|c| c as i32 as u32).unwrap_or(0)
}

fn run_from(v: &Value) -> Run {
    let flag = |k: &str| v.get(k).is_some_and(|x| x.as_i64() == Some(1) || x.as_bool() == Some(true));
    Run {
        text: v.get("t").and_then(Value::as_str).unwrap_or("").replace(['\n', '\r'], " "),
        st: CStyle {
            b: flag("b"),
            i: flag("i"),
            u: flag("u"),
            s: flag("s"),
            size: v.get("sz").and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite() && *x >= 1.0 && *x <= 400.0),
            color: color(v.get("c")),
            hl: color(v.get("hl")),
            fam: v.get("f").and_then(Value::as_str).map(Family::from_name),
            va: match v.get("va").and_then(Value::as_str) {
                Some("sup") => VAlign::Sup,
                Some("sub") => VAlign::Sub,
                _ => VAlign::Base,
            },
            link: v.get("ln").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from),
        },
    }
}

/// Ein Textdokument.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDoc {
    pub paras: Vec<Para>,
    pub page: PageSpec,
    /// Grundschrift und -größe (Standardtext).
    pub family: Family,
    pub size: f32,
    pub header: String,
    pub footer: String,
}

impl Default for TextDoc {
    fn default() -> Self {
        TextDoc { paras: vec![Para::new(Kind::Body)], page: PageSpec::default(), family: Family::Serif, size: 11.0, header: String::new(), footer: String::new() }
    }
}

/// Wortzeichen (für Doppelklick, Strg+Pfeil, Wörter zählen).
pub fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '\''
}

impl TextDoc {
    pub fn new() -> TextDoc {
        TextDoc::default()
    }

    /// Position am Ende.
    pub fn end(&self) -> Pos {
        let p = self.paras.len().saturating_sub(1);
        Pos::new(p, self.paras.get(p).map(Para::len).unwrap_or(0))
    }

    /// Klemmt eine Position in das Dokument.
    pub fn clamp(&self, pos: Pos) -> Pos {
        if self.paras.is_empty() {
            return Pos::default();
        }
        let p = pos.p.min(self.paras.len() - 1);
        Pos::new(p, pos.o.min(self.paras[p].len()))
    }

    fn ensure(&mut self) {
        if self.paras.is_empty() {
            self.paras.push(Para::new(Kind::Body));
        }
    }

    /// Zeichenformat für neuen Text an `pos`.
    pub fn style_at(&self, pos: Pos) -> CStyle {
        let mut st = self.paras.get(pos.p).map(|p| p.style_at(pos.o)).unwrap_or_default();
        // Links setzen sich beim Weiterschreiben nicht fort
        st.link = None;
        st
    }

    /// Fügt Text ein (`\n` beginnt einen neuen Absatz) und liefert die Position dahinter.
    pub fn insert_text(&mut self, pos: Pos, s: &str, st: &CStyle) -> Pos {
        self.ensure();
        let mut pos = self.clamp(pos);
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        for (i, part) in s.split('\n').enumerate() {
            if i > 0 {
                pos = self.split(pos);
            }
            if part.is_empty() {
                continue;
            }
            if self.paras[pos.p].kind.is_block() {
                // In einen Seitenumbruch wird nicht geschrieben: dahinter einen Absatz anlegen
                self.paras.insert(pos.p + 1, Para::new(Kind::Body));
                pos = Pos::new(pos.p + 1, 0);
            }
            let clean: String = part.chars().map(|c| if c.is_control() && c != '\t' { ' ' } else { c }).collect();
            self.paras[pos.p].insert(pos.o, &clean, st);
            pos.o += clean.chars().count();
        }
        pos
    }

    /// Neuer Absatz an `pos` (Enter). Überschriften gehen in Standardtext über, leere Listenpunkte enden.
    pub fn split(&mut self, pos: Pos) -> Pos {
        self.ensure();
        let pos = self.clamp(pos);
        let para = &mut self.paras[pos.p];
        let at_end = pos.o >= para.len();
        let mut rest = para.split_off(pos.o);
        if para.kind.is_block() {
            rest.kind = Kind::Body;
            rest.align = Align::Left;
        }
        if at_end && (para.kind.is_heading() || para.kind == Kind::Quote) {
            rest.kind = Kind::Body;
            rest.align = Align::Left;
            for r in &mut rest.runs {
                r.st = CStyle::default();
            }
        }
        self.paras.insert(pos.p + 1, rest);
        Pos::new(pos.p + 1, 0)
    }

    /// Löscht a..b (über Absätze hinweg) und liefert die Position.
    pub fn delete(&mut self, a: Pos, b: Pos) -> Pos {
        self.ensure();
        let (a, b) = ordered(self.clamp(a), self.clamp(b));
        if a == b {
            return a;
        }
        if a.p == b.p {
            self.paras[a.p].delete(a.o, b.o);
            return a;
        }
        let mut tail = self.paras[b.p].clone();
        tail.delete(0, b.o);
        self.paras[a.p].delete(a.o, usize::MAX);
        self.paras.drain(a.p + 1..=b.p);
        let first_empty = self.paras[a.p].is_empty() && a.o == 0;
        if first_empty && self.paras[a.p].kind.is_block() {
            self.paras[a.p].kind = tail.kind;
            self.paras[a.p].align = tail.align;
            self.paras[a.p].img = None;
        }
        self.paras[a.p].append(tail);
        a
    }

    /// Ausschnitt a..b als Absätze (für Kopieren).
    pub fn slice(&self, a: Pos, b: Pos) -> Vec<Para> {
        let (a, b) = ordered(self.clamp(a), self.clamp(b));
        if a.p == b.p {
            return vec![self.paras[a.p].slice(a.o, b.o)];
        }
        let mut v = vec![self.paras[a.p].slice(a.o, usize::MAX)];
        v.extend(self.paras[a.p + 1..b.p].iter().cloned());
        v.push(self.paras[b.p].slice(0, b.o));
        v
    }

    /// Reiner Text von a..b (Absätze durch `\n` getrennt).
    pub fn text_range(&self, a: Pos, b: Pos) -> String {
        self.slice(a, b).iter().map(Para::text).collect::<Vec<_>>().join("\n")
    }

    /// Fügt kopierte Absätze an `pos` ein und liefert die Position dahinter.
    pub fn insert_paras(&mut self, pos: Pos, frag: &[Para]) -> Pos {
        self.ensure();
        let pos = self.clamp(pos);
        if frag.is_empty() {
            return pos;
        }
        if frag.iter().any(|p| p.kind.is_block()) {
            // Bilder/Umbrüche bleiben eigene Absätze: an der Stelle teilen und dazwischen einsetzen
            let tail = self.paras[pos.p].split_off(pos.o);
            let mut idx = pos.p + 1;
            if self.paras[pos.p].is_empty() && !self.paras[pos.p].kind.is_block() {
                self.paras.remove(pos.p);
                idx -= 1;
            }
            for f in frag {
                self.paras.insert(idx, f.clone());
                idx += 1;
            }
            // Leeren Rest nur behalten, wenn danach nichts Beschreibbares mehr folgt
            if !tail.is_empty() || self.paras.get(idx).is_none_or(|n| n.kind.is_block()) {
                self.paras.insert(idx, tail);
            }
            return Pos::new(idx, 0);
        }
        if frag.len() == 1 {
            let mut pos = pos;
            for r in &frag[0].runs {
                self.paras[pos.p].insert(pos.o, &r.text, &r.st);
                pos.o += r.text.chars().count();
            }
            return pos;
        }
        let tail = self.paras[pos.p].split_off(pos.o);
        let first = &frag[0];
        let target_empty = self.paras[pos.p].is_empty();
        if target_empty {
            self.paras[pos.p].kind = first.kind;
            self.paras[pos.p].align = first.align;
            self.paras[pos.p].indent = first.indent;
        }
        self.paras[pos.p].append(first.clone());
        let mut idx = pos.p;
        for m in &frag[1..frag.len() - 1] {
            idx += 1;
            self.paras.insert(idx, m.clone());
        }
        idx += 1;
        let mut last = frag[frag.len() - 1].clone();
        let o = last.len();
        last.append(tail);
        self.paras.insert(idx, last);
        Pos::new(idx, o)
    }

    /// Zeichenformat von a..b ändern.
    pub fn restyle(&mut self, a: Pos, b: Pos, f: &dyn Fn(&mut CStyle)) {
        let (a, b) = ordered(self.clamp(a), self.clamp(b));
        for p in a.p..=b.p {
            let s = if p == a.p { a.o } else { 0 };
            let e = if p == b.p { b.o } else { usize::MAX };
            self.paras[p].restyle(s, e, f);
        }
    }

    /// Absatzformat aller Absätze in a..b ändern.
    pub fn repara(&mut self, a: Pos, b: Pos, f: &dyn Fn(&mut Para)) {
        let (a, b) = ordered(self.clamp(a), self.clamp(b));
        for p in a.p..=b.p {
            f(&mut self.paras[p]);
        }
    }

    /// Zeichen an `pos` (dahinter), sofern vorhanden.
    pub fn char_at(&self, pos: Pos) -> Option<char> {
        self.paras.get(pos.p)?.text().chars().nth(pos.o)
    }

    /// Vorherige Position (über Absätze hinweg).
    pub fn prev(&self, pos: Pos) -> Pos {
        let pos = self.clamp(pos);
        if pos.o > 0 {
            Pos::new(pos.p, pos.o - 1)
        } else if pos.p > 0 {
            Pos::new(pos.p - 1, self.paras[pos.p - 1].len())
        } else {
            pos
        }
    }

    /// Nächste Position.
    pub fn next(&self, pos: Pos) -> Pos {
        let pos = self.clamp(pos);
        if pos.o < self.paras[pos.p].len() {
            Pos::new(pos.p, pos.o + 1)
        } else if pos.p + 1 < self.paras.len() {
            Pos::new(pos.p + 1, 0)
        } else {
            pos
        }
    }

    /// Wortanfang links von `pos` (Strg+Links).
    pub fn word_left(&self, pos: Pos) -> Pos {
        let pos = self.clamp(pos);
        if pos.o == 0 {
            return self.prev(pos);
        }
        let ch: Vec<char> = self.paras[pos.p].text().chars().collect();
        let mut o = pos.o;
        while o > 0 && !is_word(ch[o - 1]) {
            o -= 1;
        }
        while o > 0 && is_word(ch[o - 1]) {
            o -= 1;
        }
        Pos::new(pos.p, o)
    }

    /// Wortende rechts von `pos` (Strg+Rechts).
    pub fn word_right(&self, pos: Pos) -> Pos {
        let pos = self.clamp(pos);
        let ch: Vec<char> = self.paras[pos.p].text().chars().collect();
        if pos.o >= ch.len() {
            return self.next(pos);
        }
        let mut o = pos.o;
        while o < ch.len() && is_word(ch[o]) {
            o += 1;
        }
        while o < ch.len() && !is_word(ch[o]) {
            o += 1;
        }
        Pos::new(pos.p, o)
    }

    /// Wort um `pos` (Doppelklick).
    pub fn word_at(&self, pos: Pos) -> (Pos, Pos) {
        let pos = self.clamp(pos);
        let ch: Vec<char> = self.paras[pos.p].text().chars().collect();
        let (mut a, mut b) = (pos.o, pos.o);
        let w = |i: usize| ch.get(i).copied().is_some_and(is_word);
        if !w(pos.o) && pos.o > 0 && w(pos.o - 1) {
            a -= 1;
            b = a;
        }
        if w(a) {
            while a > 0 && w(a - 1) {
                a -= 1;
            }
            while w(b) {
                b += 1;
            }
        } else if b < ch.len() {
            b += 1;
        }
        (Pos::new(pos.p, a), Pos::new(pos.p, b))
    }

    /// Anzahl Wörter.
    pub fn words(&self) -> usize {
        self.paras.iter().map(|p| p.text().split(|c: char| !is_word(c) && c != '-').filter(|w| w.chars().any(char::is_alphanumeric)).count()).sum()
    }

    /// Anzahl Zeichen (ohne Absatzenden).
    pub fn char_count(&self) -> usize {
        self.paras.iter().map(Para::len).sum()
    }

    /// Alle Fundstellen von `q` (ohne Groß-/Kleinschreibung).
    pub fn find(&self, q: &str) -> Vec<(Pos, Pos)> {
        let q: Vec<char> = q.chars().flat_map(char::to_lowercase).collect();
        if q.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (pi, p) in self.paras.iter().enumerate() {
            // Kleinbuchstaben einzeln (Länge bleibt gleich, sonst Zeichen unverändert)
            let t: Vec<char> = p.text().chars().map(|c| {
                let mut l = c.to_lowercase();
                match (l.next(), l.next()) {
                    (Some(x), None) => x,
                    _ => c,
                }
            }).collect();
            let mut i = 0;
            while i + q.len() <= t.len() {
                if t[i..i + q.len()] == q[..] {
                    out.push((Pos::new(pi, i), Pos::new(pi, i + q.len())));
                    i += q.len();
                } else {
                    i += 1;
                }
            }
        }
        out
    }

    /// Text für Kopf-/Fußzeile mit Seitenzahlen.
    pub fn hf_text(s: &str, page: usize, pages: usize) -> String {
        s.replace("{seite}", &(page + 1).to_string()).replace("{seiten}", &pages.to_string())
    }

    pub fn to_json(&self) -> Value {
        let p = &self.page;
        json!({
            "v": 1,
            "page": {"w": p.w as f64, "h": p.h as f64, "ml": p.ml as f64, "mr": p.mr as f64, "mt": p.mt as f64, "mb": p.mb as f64},
            "font": self.family.name(),
            "size": self.size as f64,
            "header": self.header,
            "footer": self.footer,
            "paras": Value::Array(self.paras.iter().map(Para::to_json).collect()),
        })
    }

    pub fn from_json(v: &Value) -> TextDoc {
        let d = PageSpec::default();
        let pg = v.get("page");
        let f = |k: &str, def: f32, lo: f32, hi: f32| pg.and_then(|p| p.get(k)).and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite() && *x >= lo && *x <= hi).unwrap_or(def);
        let page = PageSpec { w: f("w", d.w, 72.0, 5000.0), h: f("h", d.h, 72.0, 5000.0), ml: f("ml", d.ml, 0.0, 1000.0), mr: f("mr", d.mr, 0.0, 1000.0), mt: f("mt", d.mt, 0.0, 1000.0), mb: f("mb", d.mb, 0.0, 1000.0) };
        let mut t = TextDoc {
            paras: v.get("paras").and_then(Value::as_array).map(|a| a.iter().map(Para::from_json).collect()).unwrap_or_default(),
            page,
            family: Family::from_name(v.get("font").and_then(Value::as_str).unwrap_or("serif")),
            size: v.get("size").and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite() && *x >= 4.0 && *x <= 96.0).unwrap_or(11.0),
            header: v.get("header").and_then(Value::as_str).unwrap_or("").replace('\n', " "),
            footer: v.get("footer").and_then(Value::as_str).unwrap_or("").replace('\n', " "),
        };
        t.ensure();
        t
    }

    /// Ganzer Text (Absätze durch `\n`).
    pub fn plain_text(&self) -> String {
        self.paras.iter().map(Para::text).collect::<Vec<_>>().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> CStyle {
        CStyle { b: true, ..CStyle::default() }
    }

    #[test]
    fn einfuegen_loeschen_teilen() {
        let mut d = TextDoc::new();
        let p = d.insert_text(Pos::default(), "Hallo Welt", &CStyle::default());
        assert_eq!(p, Pos::new(0, 10));
        let p = d.insert_text(Pos::new(0, 5), "\nneue", &bold());
        assert_eq!(d.paras.len(), 2);
        assert_eq!(d.paras[0].text(), "Hallo");
        assert_eq!(d.paras[1].text(), "neue Welt");
        assert_eq!(p, Pos::new(1, 4));
        assert_eq!(d.paras[1].runs.len(), 2);
        assert!(d.paras[1].runs[0].st.b);
        let p = d.delete(Pos::new(0, 3), Pos::new(1, 2));
        assert_eq!(p, Pos::new(0, 3));
        assert_eq!(d.plain_text(), "Halue Welt");
        assert_eq!(d.paras.len(), 1);
        // Umlaute (Zeichen statt Bytes)
        let mut e = TextDoc::new();
        e.insert_text(Pos::default(), "Größe", &CStyle::default());
        e.delete(Pos::new(0, 2), Pos::new(0, 3));
        assert_eq!(e.plain_text(), "Grße");
    }

    #[test]
    fn format_und_normalisieren() {
        let mut d = TextDoc::new();
        d.insert_text(Pos::default(), "abcdef", &CStyle::default());
        d.restyle(Pos::new(0, 1), Pos::new(0, 3), &|s| s.b = true);
        assert_eq!(d.paras[0].runs.len(), 3);
        d.restyle(Pos::new(0, 0), Pos::new(0, 6), &|s| s.b = true);
        assert_eq!(d.paras[0].runs.len(), 1);
        assert!(d.style_at(Pos::new(0, 3)).b);
    }

    #[test]
    fn ueberschrift_enter_wird_standardtext() {
        let mut d = TextDoc::new();
        d.insert_text(Pos::default(), "Titel", &CStyle::default());
        d.paras[0].kind = Kind::H1;
        let p = d.split(Pos::new(0, 5));
        assert_eq!(d.paras[p.p].kind, Kind::Body);
        d.paras[1].kind = Kind::Bullet;
        let p = d.split(p);
        assert_eq!(d.paras[p.p].kind, Kind::Bullet);
    }

    #[test]
    fn kopieren_einfuegen_mehrere_absaetze() {
        let mut d = TextDoc::new();
        d.insert_text(Pos::default(), "eins\nzwei\ndrei", &CStyle::default());
        let frag = d.slice(Pos::new(0, 2), Pos::new(2, 2));
        assert_eq!(frag.len(), 3);
        let mut e = TextDoc::new();
        e.insert_text(Pos::default(), "AB", &CStyle::default());
        let p = e.insert_paras(Pos::new(0, 1), &frag);
        assert_eq!(e.plain_text(), "Ans\nzwei\ndrB");
        assert_eq!(p, Pos::new(2, 2));
    }

    #[test]
    fn json_rundreise_und_suche() {
        let mut d = TextDoc::new();
        d.insert_text(Pos::default(), "Beet und Beet", &CStyle { color: 0xFFFF0000, fam: Some(Family::Sans), va: VAlign::Sup, ..bold() });
        d.paras[0].kind = Kind::H2;
        d.paras[0].align = Align::Justify;
        d.paras[0].line = 1.5;
        d.footer = "Seite {seite} von {seiten}".into();
        let back = TextDoc::from_json(&d.to_json());
        assert_eq!(back, d);
        assert_eq!(d.find("beet").len(), 2);
        assert_eq!(d.words(), 3);
        assert_eq!(TextDoc::hf_text(&d.footer, 1, 3), "Seite 2 von 3");
        let leer = TextDoc::from_json(&json!({}));
        assert_eq!(leer.paras.len(), 1);
    }

    #[test]
    fn woerter_navigieren() {
        let mut d = TextDoc::new();
        d.insert_text(Pos::default(), "ein Wort hier", &CStyle::default());
        assert_eq!(d.word_at(Pos::new(0, 5)), (Pos::new(0, 4), Pos::new(0, 8)));
        assert_eq!(d.word_right(Pos::new(0, 0)), Pos::new(0, 4));
        assert_eq!(d.word_left(Pos::new(0, 8)), Pos::new(0, 4));
    }
}
