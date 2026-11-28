//! Objekte und Zusatzdaten des Textdokuments: Tabellen, Formen/Textfelder, Zeichnungen, Formeln, Diagramme,
//! Felder (Seitenzahl, Querverweis, Zitat, Seriendruck, Formular), Kommentare, Änderungen, Quellen, Versionen.
//!
//! Felder und Fußnoten stehen als ein Platzhalterzeichen [`OBJ`] im Text; was angezeigt wird, berechnet der Satz.
//! JSON jeweils kurz, unbekannte Felder werden ignoriert.

use super::model::{Align, Run};
use serde_json::{json, Map, Value};

/// Platzhalterzeichen für Felder und Fußnoten-Zeichen im Text.
pub const OBJ: char = '\u{FFFC}';

pub(crate) fn num(v: &Value, k: &str) -> Option<f32> {
    v.get(k).and_then(Value::as_f64).map(|x| x as f32).filter(|x| x.is_finite())
}
pub(crate) fn st(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
pub(crate) fn col(v: &Value, k: &str) -> u32 {
    v.get(k).and_then(Value::as_i64).map(|c| c as i32 as u32).unwrap_or(0)
}
pub(crate) fn u(v: &Value, k: &str) -> u32 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0).min(u32::MAX as u64) as u32
}
pub(crate) fn flag(v: &Value, k: &str) -> bool {
    v.get(k).is_some_and(|x| x.as_i64() == Some(1) || x.as_bool() == Some(true))
}
fn cj(c: u32) -> Value {
    json!(c as i32)
}

// ---------------------------------------------------------------- Textumfluss

/// Wie Text um ein Bild/Objekt fließt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Wrap {
    /// Eigene Zeile (Text darüber und darunter).
    #[default]
    Inline,
    /// Objekt links, Text fließt rechts daneben.
    Left,
    /// Objekt rechts, Text fließt links daneben.
    Right,
}

impl Wrap {
    pub fn name(self) -> &'static str {
        match self {
            Wrap::Inline => "",
            Wrap::Left => "l",
            Wrap::Right => "r",
        }
    }
    pub fn from_name(s: &str) -> Wrap {
        match s {
            "l" => Wrap::Left,
            "r" => Wrap::Right,
            _ => Wrap::Inline,
        }
    }
}

// ---------------------------------------------------------------- Felder

/// Was ein Querverweis zeigt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RefShow {
    /// „Abbildung 2“ bzw. Überschriften-Text.
    #[default]
    Label,
    /// Seitenzahl des Ziels.
    Page,
    /// Ganzer Text des Ziels.
    Text,
}

/// Feld im Text (Platzhalter [`OBJ`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    /// Aktuelle Seitenzahl.
    Page,
    /// Anzahl Seiten.
    Pages,
    /// Datum (beim Einfügen festgehalten, „Felder aktualisieren“ erneuert es).
    Date(String),
    /// Verweis auf einen Absatz mit `id` (Überschrift, Beschriftung).
    Ref(u32, RefShow),
    /// Zitat einer Quelle.
    Cite(u32),
    /// Seriendruckfeld (Spaltenname der Datenquelle).
    Merge(String),
    /// Kontrollkästchen.
    Check(bool),
    /// Auswahlfeld: Möglichkeiten, gewählte.
    Choice(Vec<String>, usize),
    /// Stichwort für das Stichwortverzeichnis (unsichtbar).
    Index(String),
    /// Fußnote mit Text (Zeichen wird automatisch nummeriert).
    Note(String),
}

impl Field {
    pub fn to_json(&self) -> Value {
        match self {
            Field::Page => json!({"t": "page"}),
            Field::Pages => json!({"t": "pages"}),
            Field::Date(d) => json!({"t": "date", "v": d}),
            Field::Ref(id, s) => json!({"t": "ref", "id": id, "s": match s { RefShow::Label => "l", RefShow::Page => "p", RefShow::Text => "t" }}),
            Field::Cite(id) => json!({"t": "cite", "id": id}),
            Field::Merge(n) => json!({"t": "merge", "v": n}),
            Field::Check(b) => json!({"t": "check", "v": *b as i32}),
            Field::Choice(o, i) => json!({"t": "choice", "o": o, "v": i}),
            Field::Index(t) => json!({"t": "index", "v": t}),
            Field::Note(t) => json!({"t": "note", "v": t}),
        }
    }
    pub fn from_json(v: &Value) -> Option<Field> {
        Some(match v.get("t")?.as_str()? {
            "page" => Field::Page,
            "pages" => Field::Pages,
            "date" => Field::Date(st(v, "v")),
            "ref" => Field::Ref(
                u(v, "id"),
                match v.get("s").and_then(Value::as_str) {
                    Some("p") => RefShow::Page,
                    Some("t") => RefShow::Text,
                    _ => RefShow::Label,
                },
            ),
            "cite" => Field::Cite(u(v, "id")),
            "merge" => Field::Merge(st(v, "v")),
            "check" => Field::Check(flag(v, "v")),
            "choice" => {
                let o: Vec<String> = v.get("o").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
                let i = (u(v, "v") as usize).min(o.len().saturating_sub(1));
                Field::Choice(o, i)
            }
            "index" => Field::Index(st(v, "v")),
            "note" => Field::Note(st(v, "v")),
            _ => return None,
        })
    }
    /// Unsichtbar (keine Breite).
    pub fn hidden(&self) -> bool {
        matches!(self, Field::Index(_))
    }
}

// ---------------------------------------------------------------- Tabellen

/// Tabellenzelle: Text (darf Zeilenumbrüche enthalten) mit Zellformat.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Cell {
    pub text: String,
    pub b: bool,
    pub i: bool,
    pub align: Align,
    /// Hintergrund ARGB, 0 = keiner.
    pub bg: u32,
    /// Textfarbe ARGB, 0 = automatisch.
    pub color: u32,
}

impl Cell {
    pub fn new(t: &str) -> Cell {
        Cell { text: t.into(), ..Cell::default() }
    }
    fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("t".into(), json!(self.text));
        if self.b {
            o.insert("b".into(), json!(1));
        }
        if self.i {
            o.insert("i".into(), json!(1));
        }
        if self.align != Align::Left {
            o.insert("a".into(), json!(self.align.name()));
        }
        if self.bg != 0 {
            o.insert("bg".into(), cj(self.bg));
        }
        if self.color != 0 {
            o.insert("c".into(), cj(self.color));
        }
        Value::Object(o)
    }
    fn from_json(v: &Value) -> Cell {
        Cell { text: st(v, "t"), b: flag(v, "b"), i: flag(v, "i"), align: Align::from_name(v.get("a").and_then(Value::as_str).unwrap_or("")), bg: col(v, "bg"), color: col(v, "c") }
    }
}

/// Tabelle (eigener Absatz `Kind::Table`). Spaltenbreiten in pt.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub cols: Vec<f32>,
    pub rows: Vec<Vec<Cell>>,
    /// Erste Zeile als Kopfzeile (fett, getönt, wiederholt sich auf jeder Seite).
    pub header: bool,
    /// Zeilen abwechselnd getönt.
    pub stripes: bool,
    /// Rahmenfarbe ARGB, 0 = ohne Rahmen.
    pub border: u32,
}

impl Table {
    /// Leere Tabelle mit `r` Zeilen, `c` Spalten auf Breite `w`.
    pub fn new(r: usize, c: usize, w: f32) -> Table {
        let (r, c) = (r.clamp(1, 200), c.clamp(1, 20));
        Table { cols: vec![w / c as f32; c], rows: vec![vec![Cell::default(); c]; r], header: false, stripes: false, border: 0xFFB8BCC6 }
    }
    pub fn ncols(&self) -> usize {
        self.cols.len()
    }
    /// Alle Zeilen auf Spaltenzahl bringen.
    pub fn fix(&mut self) {
        if self.cols.is_empty() {
            self.cols.push(100.0);
        }
        if self.rows.is_empty() {
            self.rows.push(Vec::new());
        }
        let n = self.cols.len();
        for r in &mut self.rows {
            r.resize(n, Cell::default());
        }
        for c in &mut self.cols {
            if !c.is_finite() || *c < 12.0 {
                *c = 12.0;
            }
        }
    }
    pub fn insert_row(&mut self, at: usize) {
        let n = self.ncols();
        self.rows.insert(at.min(self.rows.len()), vec![Cell::default(); n]);
    }
    pub fn delete_row(&mut self, r: usize) {
        if self.rows.len() > 1 && r < self.rows.len() {
            self.rows.remove(r);
        }
    }
    /// Spalte einfügen; Breite wird von den anderen genommen (Gesamtbreite bleibt).
    pub fn insert_col(&mut self, at: usize) {
        let total: f32 = self.cols.iter().sum();
        let n = self.cols.len() + 1;
        let at = at.min(self.cols.len());
        let w = total / n as f32;
        let k = (total - w) / total.max(1.0);
        for c in &mut self.cols {
            *c *= k;
        }
        self.cols.insert(at, w);
        for r in &mut self.rows {
            r.insert(at, Cell::default());
        }
    }
    pub fn delete_col(&mut self, c: usize) {
        if self.cols.len() > 1 && c < self.cols.len() {
            let w = self.cols.remove(c);
            let n = self.cols.len() as f32;
            for x in &mut self.cols {
                *x += w / n;
            }
            for r in &mut self.rows {
                if c < r.len() {
                    r.remove(c);
                }
            }
        }
    }
    /// Tabelle aus Text: Zeilen durch `\n`, Spalten durch Tab oder `;`.
    pub fn from_text(s: &str, w: f32) -> Table {
        let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
        let sep = if s.contains('\t') { '\t' } else { ';' };
        let rows: Vec<Vec<Cell>> = lines.iter().map(|l| l.split(sep).map(|c| Cell::new(c.trim())).collect()).collect();
        let n = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let mut t = Table { cols: vec![w / n as f32; n], rows, header: true, stripes: false, border: 0xFFB8BCC6 };
        t.fix();
        t
    }
    /// Text als Tab-getrennte Zeilen.
    pub fn to_text(&self) -> String {
        self.rows.iter().map(|r| r.iter().map(|c| c.text.replace('\n', " ")).collect::<Vec<_>>().join("\t")).collect::<Vec<_>>().join("\n")
    }
    /// Spaltenbreiten auf neue Gesamtbreite skalieren.
    pub fn fit(&mut self, w: f32) {
        let total: f32 = self.cols.iter().sum();
        if total > 0.0 {
            let k = w / total;
            for c in &mut self.cols {
                *c *= k;
            }
        }
    }
    /// Sortiert die Zeilen (ohne Kopfzeile) nach Spalte `c`; Zahlen numerisch.
    pub fn sort_by(&mut self, c: usize, desc: bool) {
        let start = self.header as usize;
        if self.rows.len() <= start + 1 {
            return;
        }
        let key = |r: &Vec<Cell>| r.get(c).map(|x| x.text.trim().replace(',', ".")).unwrap_or_default();
        self.rows[start..].sort_by(|a, b| {
            let (ka, kb) = (key(a), key(b));
            let o = match (ka.parse::<f64>(), kb.parse::<f64>()) {
                (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                _ => ka.to_lowercase().cmp(&kb.to_lowercase()),
            };
            if desc {
                o.reverse()
            } else {
                o
            }
        });
    }
    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("cols".into(), Value::Array(self.cols.iter().map(|c| json!(*c as f64)).collect()));
        o.insert("rows".into(), Value::Array(self.rows.iter().map(|r| Value::Array(r.iter().map(Cell::to_json).collect())).collect()));
        if self.header {
            o.insert("hd".into(), json!(1));
        }
        if self.stripes {
            o.insert("st".into(), json!(1));
        }
        o.insert("bd".into(), cj(self.border));
        Value::Object(o)
    }
    pub fn from_json(v: &Value) -> Option<Table> {
        let cols: Vec<f32> = v.get("cols")?.as_array()?.iter().filter_map(|x| x.as_f64()).map(|x| (x as f32).clamp(12.0, 2000.0)).take(20).collect();
        let rows: Vec<Vec<Cell>> = v.get("rows")?.as_array()?.iter().take(1000).map(|r| r.as_array().map(|a| a.iter().map(Cell::from_json).collect()).unwrap_or_default()).collect();
        let mut t = Table { cols, rows, header: flag(v, "hd"), stripes: flag(v, "st"), border: if v.get("bd").is_some() { col(v, "bd") } else { 0xFFB8BCC6 } };
        t.fix();
        Some(t)
    }
}

// ---------------------------------------------------------------- Objekte

/// Form (auch Textfeld).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Rect,
    Round,
    Ellipse,
    Triangle,
    Diamond,
    Arrow,
    Star,
    Line,
    Callout,
    TextBox,
}

impl Shape {
    pub const ALL: [Shape; 10] = [Shape::TextBox, Shape::Rect, Shape::Round, Shape::Ellipse, Shape::Triangle, Shape::Diamond, Shape::Arrow, Shape::Star, Shape::Line, Shape::Callout];
    pub fn name(self) -> &'static str {
        match self {
            Shape::Rect => "rect",
            Shape::Round => "round",
            Shape::Ellipse => "ellipse",
            Shape::Triangle => "tri",
            Shape::Diamond => "diamond",
            Shape::Arrow => "arrow",
            Shape::Star => "star",
            Shape::Line => "line",
            Shape::Callout => "callout",
            Shape::TextBox => "text",
        }
    }
    pub fn from_name(s: &str) -> Shape {
        Shape::ALL.into_iter().find(|x| x.name() == s).unwrap_or_default()
    }
    pub fn label(self) -> &'static str {
        match self {
            Shape::Rect => "Rechteck",
            Shape::Round => "Abgerundetes Rechteck",
            Shape::Ellipse => "Ellipse",
            Shape::Triangle => "Dreieck",
            Shape::Diamond => "Raute",
            Shape::Arrow => "Pfeil",
            Shape::Star => "Stern",
            Shape::Line => "Linie",
            Shape::Callout => "Sprechblase",
            Shape::TextBox => "Textfeld",
        }
    }
}

/// Strich einer Zeichnung; Punkte relativ zur Objektgröße 0..1.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Stroke {
    pub color: u32,
    /// Breite in pt.
    pub w: f32,
    pub pts: Vec<f32>,
}

/// Diagrammart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ChartKind {
    #[default]
    Bar,
    Line,
    Pie,
}

impl ChartKind {
    pub fn name(self) -> &'static str {
        match self {
            ChartKind::Bar => "bar",
            ChartKind::Line => "line",
            ChartKind::Pie => "pie",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ChartKind::Bar => "Säulen",
            ChartKind::Line => "Linie",
            ChartKind::Pie => "Kreis",
        }
    }
}

/// Inhalt eines Objekts.
#[derive(Clone, Debug, PartialEq)]
pub enum ObjKind {
    Shape { shape: Shape, fill: u32, stroke: u32, sw: f32, text: String, tcol: u32 },
    Drawing { strokes: Vec<Stroke> },
    Formula { src: String },
    /// Diagramm aus einer kleinen Datentabelle: erste Zeile Reihen-Namen, erste Spalte Beschriftungen.
    Chart { kind: ChartKind, title: String, data: Vec<Vec<String>> },
}

/// Objekt als eigener Absatz (`Kind::Object`), Größe in pt.
#[derive(Clone, Debug, PartialEq)]
pub struct Obj {
    pub kind: ObjKind,
    pub w: f32,
    pub h: f32,
}

impl Obj {
    pub fn shape(s: Shape) -> Obj {
        let (fill, stroke, w, h) = match s {
            Shape::TextBox => (0, 0xFF6B7280, 220.0, 70.0),
            Shape::Line | Shape::Arrow => (0xFF3B82F6, 0xFF1D4ED8, 180.0, 50.0),
            _ => (0xFFDBEAFE, 0xFF1D4ED8, 160.0, 100.0),
        };
        Obj { kind: ObjKind::Shape { shape: s, fill, stroke, sw: 1.5, text: String::new(), tcol: 0 }, w, h }
    }
    pub fn chart() -> Obj {
        let d = |r: &[&str]| r.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        Obj { kind: ObjKind::Chart { kind: ChartKind::Bar, title: "Diagramm".into(), data: vec![d(&["", "2025", "2026"]), d(&["Q1", "12", "15"]), d(&["Q2", "18", "21"]), d(&["Q3", "9", "14"]), d(&["Q4", "22", "26"])] }, w: 320.0, h: 200.0 }
    }
    /// Kurzname (Kontextleiste, Barrierefreiheit).
    pub fn label(&self) -> &'static str {
        match &self.kind {
            ObjKind::Shape { shape, .. } => shape.label(),
            ObjKind::Drawing { .. } => "Zeichnung",
            ObjKind::Formula { .. } => "Formel",
            ObjKind::Chart { .. } => "Diagramm",
        }
    }
    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("w".into(), json!(self.w as f64));
        o.insert("h".into(), json!(self.h as f64));
        match &self.kind {
            ObjKind::Shape { shape, fill, stroke, sw, text, tcol } => {
                o.insert("t".into(), json!("shape"));
                o.insert("s".into(), json!(shape.name()));
                o.insert("fill".into(), cj(*fill));
                o.insert("stroke".into(), cj(*stroke));
                o.insert("sw".into(), json!(*sw as f64));
                o.insert("text".into(), json!(text));
                if *tcol != 0 {
                    o.insert("tc".into(), cj(*tcol));
                }
            }
            ObjKind::Drawing { strokes } => {
                o.insert("t".into(), json!("draw"));
                o.insert("strokes".into(), Value::Array(strokes.iter().map(|s| json!({"c": s.color as i32, "w": s.w as f64, "p": s.pts.iter().map(|v| (*v as f64 * 1000.0).round() / 1000.0).collect::<Vec<_>>()})).collect()));
            }
            ObjKind::Formula { src } => {
                o.insert("t".into(), json!("formula"));
                o.insert("src".into(), json!(src));
            }
            ObjKind::Chart { kind, title, data } => {
                o.insert("t".into(), json!("chart"));
                o.insert("k".into(), json!(kind.name()));
                o.insert("title".into(), json!(title));
                o.insert("data".into(), json!(data));
            }
        }
        Value::Object(o)
    }
    pub fn from_json(v: &Value) -> Option<Obj> {
        let w = num(v, "w").unwrap_or(100.0).clamp(4.0, 5000.0);
        let h = num(v, "h").unwrap_or(100.0).clamp(4.0, 5000.0);
        let kind = match v.get("t")?.as_str()? {
            "shape" => ObjKind::Shape { shape: Shape::from_name(&st(v, "s")), fill: col(v, "fill"), stroke: col(v, "stroke"), sw: num(v, "sw").unwrap_or(1.5).clamp(0.0, 40.0), text: st(v, "text"), tcol: col(v, "tc") },
            "draw" => ObjKind::Drawing {
                strokes: v
                    .get("strokes")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|s| Stroke { color: col(s, "c"), w: num(s, "w").unwrap_or(2.0).clamp(0.1, 60.0), pts: s.get("p").and_then(Value::as_array).map(|p| p.iter().filter_map(Value::as_f64).map(|x| x as f32).collect()).unwrap_or_default() })
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            "formula" => ObjKind::Formula { src: st(v, "src") },
            "chart" => ObjKind::Chart {
                kind: match v.get("k").and_then(Value::as_str) {
                    Some("line") => ChartKind::Line,
                    Some("pie") => ChartKind::Pie,
                    _ => ChartKind::Bar,
                },
                title: st(v, "title"),
                data: v.get("data").and_then(Value::as_array).map(|a| a.iter().map(|r| r.as_array().map(|c| c.iter().map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).collect()).unwrap_or_default()).collect()).unwrap_or_default(),
            },
            _ => return None,
        };
        Some(Obj { kind, w, h })
    }
}

/// Zahlenwerte eines Diagramms: (Beschriftungen, Reihen (Name, Werte)).
pub fn chart_series(data: &[Vec<String>]) -> (Vec<String>, Vec<(String, Vec<f32>)>) {
    let ncol = data.iter().map(Vec::len).max().unwrap_or(0);
    let labels: Vec<String> = data.iter().skip(1).map(|r| r.first().cloned().unwrap_or_default()).collect();
    let mut series = Vec::new();
    for c in 1..ncol.max(1) {
        let name = data.first().and_then(|r| r.get(c)).cloned().unwrap_or_default();
        let vals = data.iter().skip(1).map(|r| r.get(c).and_then(|x| x.trim().replace(',', ".").parse::<f32>().ok()).filter(|x| x.is_finite()).unwrap_or(0.0)).collect();
        series.push((name, vals));
    }
    (labels, series)
}

// ---------------------------------------------------------------- Zusammenarbeit

/// Antwort auf einen Kommentar.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Reply {
    pub author: String,
    pub time: String,
    pub text: String,
}

/// Kommentar zu einer Textstelle (Läufe mit `CStyle::cmt == id`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Comment {
    pub id: u32,
    pub author: String,
    pub time: String,
    pub text: String,
    pub replies: Vec<Reply>,
    pub done: bool,
}

impl Comment {
    fn to_json(&self) -> Value {
        json!({"id": self.id, "a": self.author, "tm": self.time, "t": self.text, "done": self.done as i32,
            "re": self.replies.iter().map(|r| json!({"a": r.author, "tm": r.time, "t": r.text})).collect::<Vec<_>>()})
    }
    fn from_json(v: &Value) -> Comment {
        Comment {
            id: u(v, "id"),
            author: st(v, "a"),
            time: st(v, "tm"),
            text: st(v, "t"),
            done: flag(v, "done"),
            replies: v.get("re").and_then(Value::as_array).map(|a| a.iter().map(|r| Reply { author: st(r, "a"), time: st(r, "tm"), text: st(r, "t") }).collect()).unwrap_or_default(),
        }
    }
}

/// Eine nachverfolgte Änderung (Läufe mit `ins`/`del` == id).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Rev {
    pub id: u32,
    pub author: String,
    pub time: String,
}

/// Literaturquelle.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Source {
    pub id: u32,
    pub author: String,
    pub title: String,
    pub year: String,
    pub publisher: String,
    pub url: String,
}

impl Source {
    /// Kurzzitat „(Müller 2020)“.
    pub fn cite(&self) -> String {
        let a = self.author.split([',', ';']).next().unwrap_or("").trim();
        let a = a.rsplit(' ').next().unwrap_or(a);
        match (a.is_empty(), self.year.trim().is_empty()) {
            (true, _) => format!("({})", self.title.trim()),
            (false, true) => format!("({a})"),
            (false, false) => format!("({a} {})", self.year.trim()),
        }
    }
    /// Eintrag im Literaturverzeichnis.
    pub fn entry(&self) -> String {
        let mut s = String::new();
        if !self.author.trim().is_empty() {
            s.push_str(self.author.trim());
            s.push(' ');
        }
        if !self.year.trim().is_empty() {
            s.push_str(&format!("({}). ", self.year.trim()));
        } else if !s.is_empty() {
            s.push_str(". ");
        }
        s.push_str(self.title.trim());
        s.push('.');
        if !self.publisher.trim().is_empty() {
            s.push_str(&format!(" {}.", self.publisher.trim()));
        }
        if !self.url.trim().is_empty() {
            s.push_str(&format!(" {}", self.url.trim()));
        }
        s
    }
}

/// Gespeicherte Version (ganzer Text als JSON, ohne Versionen).
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub name: String,
    pub time: String,
    pub data: Value,
}

/// Datenquelle für den Seriendruck: Spaltennamen und Zeilen.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MergeData {
    pub head: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl MergeData {
    /// CSV (Trenner `;`, `,` oder Tab, Anführungszeichen erlaubt), erste Zeile = Spaltennamen.
    pub fn from_csv(s: &str) -> MergeData {
        let first = s.lines().next().unwrap_or("");
        let sep = [';', '\t', ','].into_iter().max_by_key(|c| first.matches(*c).count()).unwrap_or(';');
        let mut rows: Vec<Vec<String>> = Vec::new();
        let (mut row, mut cell, mut q) = (Vec::new(), String::new(), false);
        let mut it = s.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '"' if q && it.peek() == Some(&'"') => {
                    cell.push('"');
                    it.next();
                }
                '"' => q = !q,
                c if c == sep && !q => row.push(std::mem::take(&mut cell)),
                '\r' if !q => {}
                '\n' if !q => {
                    row.push(std::mem::take(&mut cell));
                    rows.push(std::mem::take(&mut row));
                }
                c => cell.push(c),
            }
        }
        if !cell.is_empty() || !row.is_empty() {
            row.push(cell);
            rows.push(row);
        }
        rows.retain(|r| r.iter().any(|c| !c.trim().is_empty()));
        if rows.is_empty() {
            return MergeData::default();
        }
        let head = rows.remove(0).into_iter().map(|h| h.trim().to_string()).collect();
        MergeData { head, rows }
    }
    pub fn value(&self, row: usize, name: &str) -> Option<&str> {
        let c = self.head.iter().position(|h| h.eq_ignore_ascii_case(name))?;
        self.rows.get(row)?.get(c).map(|s| s.trim())
    }
}

/// Zusatzdaten des Dokuments (alles außer Absätzen und Seite).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Extras {
    pub comments: Vec<Comment>,
    pub revs: Vec<Rev>,
    /// Änderungen nachverfolgen ist an.
    pub track: bool,
    pub sources: Vec<Source>,
    pub versions: Vec<Version>,
    pub watermark: String,
    /// Schreibschutz: leer = aus, sonst Prüfsumme des Kennworts (oder "1" ohne Kennwort).
    pub protect: String,
    pub merge: Option<MergeData>,
    /// Silbentrennung beim Satz.
    pub hyph: bool,
    /// Textspalten (0/1 = eine, bis 3).
    pub cols: u8,
    /// Design (0 = Standard).
    pub theme: u8,
    /// Nächste freie ID für Absätze, Kommentare, Änderungen, Quellen.
    pub next_id: u32,
}

impl Extras {
    pub fn is_default(&self) -> bool {
        self.cols <= 1 && *self == Extras { cols: self.cols, ..Extras::default() }
    }
    pub fn id(&mut self) -> u32 {
        self.next_id = self.next_id.max(1) + 1;
        self.next_id
    }
    pub fn rev(&mut self, author: &str, time: &str) -> u32 {
        // Gleiche Person, gleiche Minute: gleiche Änderung
        if let Some(r) = self.revs.last() {
            if r.author == author && r.time.get(..16) == time.get(..16) {
                return r.id;
            }
        }
        let id = self.id();
        self.revs.push(Rev { id, author: author.into(), time: time.into() });
        id
    }
    pub fn source(&self, id: u32) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }
    pub fn comment(&self, id: u32) -> Option<&Comment> {
        self.comments.iter().find(|c| c.id == id)
    }
    pub fn write(&self, o: &mut Map<String, Value>) {
        if !self.comments.is_empty() {
            o.insert("comments".into(), Value::Array(self.comments.iter().map(Comment::to_json).collect()));
        }
        if !self.revs.is_empty() {
            o.insert("revs".into(), Value::Array(self.revs.iter().map(|r| json!({"id": r.id, "a": r.author, "tm": r.time})).collect()));
        }
        if self.track {
            o.insert("track".into(), json!(1));
        }
        if !self.sources.is_empty() {
            o.insert("sources".into(), Value::Array(self.sources.iter().map(|s| json!({"id": s.id, "a": s.author, "t": s.title, "y": s.year, "p": s.publisher, "u": s.url})).collect()));
        }
        if !self.versions.is_empty() {
            o.insert("versions".into(), Value::Array(self.versions.iter().map(|v| json!({"n": v.name, "tm": v.time, "d": v.data})).collect()));
        }
        if !self.watermark.is_empty() {
            o.insert("wm".into(), json!(self.watermark));
        }
        if !self.protect.is_empty() {
            o.insert("protect".into(), json!(self.protect));
        }
        if let Some(m) = &self.merge {
            o.insert("merge".into(), json!({"h": m.head, "r": m.rows}));
        }
        if self.hyph {
            o.insert("hyph".into(), json!(1));
        }
        if self.cols > 1 {
            o.insert("cols".into(), json!(self.cols));
        }
        if self.theme > 0 {
            o.insert("theme".into(), json!(self.theme));
        }
        if self.next_id > 0 {
            o.insert("nid".into(), json!(self.next_id));
        }
    }
    pub fn read(v: &Value) -> Extras {
        let arr = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        let strs = |x: &Value| x.as_array().map(|a| a.iter().map(|s| s.as_str().unwrap_or("").to_string()).collect::<Vec<_>>()).unwrap_or_default();
        Extras {
            comments: arr("comments").iter().map(Comment::from_json).collect(),
            revs: arr("revs").iter().map(|r| Rev { id: u(r, "id"), author: st(r, "a"), time: st(r, "tm") }).collect(),
            track: flag(v, "track"),
            sources: arr("sources").iter().map(|s| Source { id: u(s, "id"), author: st(s, "a"), title: st(s, "t"), year: st(s, "y"), publisher: st(s, "p"), url: st(s, "u") }).collect(),
            versions: arr("versions").iter().take(50).map(|x| Version { name: st(x, "n"), time: st(x, "tm"), data: x.get("d").cloned().unwrap_or(Value::Null) }).collect(),
            watermark: st(v, "wm"),
            protect: st(v, "protect"),
            merge: v.get("merge").map(|m| MergeData { head: m.get("h").map(strs).unwrap_or_default(), rows: m.get("r").and_then(Value::as_array).map(|a| a.iter().map(strs).collect()).unwrap_or_default() }),
            hyph: flag(v, "hyph"),
            cols: (u(v, "cols") as u8).min(3),
            theme: (u(v, "theme") as u8).min(8),
            next_id: u(v, "nid"),
        }
    }
}

/// Einfache Prüfsumme für das Schutz-Kennwort (kein Geheimnis-Schutz, nur gegen versehentliches Ändern).
pub fn protect_hash(pw: &str) -> String {
    if pw.is_empty() {
        return "1".into();
    }
    let mut h: u64 = 0xcbf29ce484222325;
    for b in pw.bytes().chain(b"owndraw".iter().copied()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Läufe ohne Felder/Änderungen (für Textausgabe).
pub fn runs_plain(runs: &[Run]) -> String {
    runs.iter().filter(|r| r.st.del == 0).map(|r| r.text.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabelle_bearbeiten_und_json() {
        let mut t = Table::new(2, 3, 300.0);
        t.insert_col(1);
        assert_eq!(t.ncols(), 4);
        assert!((t.cols.iter().sum::<f32>() - 300.0).abs() < 0.1);
        t.insert_row(5);
        assert_eq!(t.rows.len(), 3);
        t.delete_col(0);
        t.rows[1][0].text = "10".into();
        t.rows[2][0].text = "9".into();
        t.sort_by(0, false);
        assert_eq!(t.rows[0][0].text, "");
        assert_eq!(t.rows[1][0].text, "9");
        let b = Table::from_json(&t.to_json()).unwrap();
        assert_eq!(b, t);
        let x = Table::from_text("a;b\n1;2;3", 200.0);
        assert_eq!((x.ncols(), x.rows.len()), (3, 2));
    }

    #[test]
    fn objekte_felder_json() {
        for o in [Obj::shape(Shape::Star), Obj::chart(), Obj { kind: ObjKind::Formula { src: "x^2".into() }, w: 40.0, h: 20.0 }, Obj { kind: ObjKind::Drawing { strokes: vec![Stroke { color: 1, w: 2.0, pts: vec![0.0, 0.5, 1.0, 0.25] }] }, w: 40.0, h: 20.0 }] {
            assert_eq!(Obj::from_json(&o.to_json()).unwrap(), o);
        }
        for f in [Field::Page, Field::Ref(3, RefShow::Page), Field::Choice(vec!["a".into(), "b".into()], 1), Field::Note("Hallo".into()), Field::Check(true)] {
            assert_eq!(Field::from_json(&f.to_json()).unwrap(), f);
        }
        let (l, s) = chart_series(&Obj::chart().kind.data());
        assert_eq!(l.len(), 4);
        assert_eq!(s[1].1[3], 26.0);
    }

    #[test]
    fn csv_und_quellen() {
        let m = MergeData::from_csv("Name;Ort\n\"Müller; Anna\";Berlin\nBob;Köln\n");
        assert_eq!(m.head, ["Name", "Ort"]);
        assert_eq!(m.value(0, "name"), Some("Müller; Anna"));
        assert_eq!(m.rows.len(), 2);
        let s = Source { author: "Anna Müller, Bob Meier".into(), year: "2020".into(), title: "Buch".into(), ..Source::default() };
        assert_eq!(s.cite(), "(Müller 2020)");
        let mut e = Extras::default();
        e.comments.push(Comment { id: 3, text: "x".into(), ..Comment::default() });
        e.cols = 2;
        let mut o = Map::new();
        e.write(&mut o);
        assert_eq!(Extras::read(&Value::Object(o)), e);
        assert!(Extras::default().is_default());
    }
}

impl ObjKind {
    /// Diagrammdaten (leer bei anderen Objekten).
    pub fn data(&self) -> Vec<Vec<String>> {
        match self {
            ObjKind::Chart { data, .. } => data.clone(),
            _ => Vec::new(),
        }
    }
}
