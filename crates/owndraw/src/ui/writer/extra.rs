//! Erweiterte Funktionen des Textdokuments (wie Word): Tabellen, Formen, Textfelder, Zeichnungen, Formeln,
//! Diagramme, Textumfluss, Beschriftungen, Spalten, Fußnoten, Querverweise, Zitate, Verzeichnisse, Kommentare,
//! Änderungen nachverfolgen, Versionen, Vergleichen, Rechtschreibung, Silbentrennung, Autokorrektur, Seriendruck,
//! Formularfelder, Vorlagen, Designs, Wasserzeichen, Schutz, Umschläge und Etiketten.
//! Logik liegt im Kern (`office::tools`, `office::lang` …), hier Zustand, Befehle, Fenster und Seitenleiste.

use super::{Cmd, Edit, Writer};
use crate::app::Shared;
use crate::style::{self, c, pal, sp};
use egui::{CornerRadius, Margin, Sense, Stroke, Vec2};
use owndraw_core::office::fields::Numbers;
use owndraw_core::office::objects::{protect_hash, ChartKind, Comment, Field, MergeData, Obj, ObjKind, RefShow, Reply, Shape, Source, Stroke as DStroke, Table, Wrap, OBJ};
use owndraw_core::office::{lang, tools, Align, CStyle, Kind, Para, Pos};
use std::collections::HashMap;

/// Offene Zelle zum Bearbeiten.
#[derive(Clone, Debug, PartialEq)]
pub struct CellEdit {
    pub p: usize,
    pub r: usize,
    pub c: usize,
    pub text: String,
    pub focus: bool,
}

/// Seitenleiste rechts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Comments,
    Changes,
}

/// Fenster der erweiterten Funktionen.
#[derive(Clone, Debug, PartialEq)]
pub enum XDlg {
    Table { r: usize, c: usize },
    Obj { p: usize, o: Obj },
    Note { at: Pos, text: String, new: bool },
    Comment(String),
    Cite { pick: Option<u32>, s: Source },
    CrossRef { show: RefShow },
    Caption { label: String, text: String },
    Watermark(String),
    Protect(String),
    Versions { name: String },
    Merge(String),
    MergeField(String),
    Choice(String),
    Templates,
    Theme,
    Envelope(String, String),
    Labels(String, usize, usize),
    Lang,
    Index(String),
    Formats,
}

/// Zustand der erweiterten Funktionen.
pub struct XState {
    pub dlg: Option<XDlg>,
    pub cell: Option<CellEdit>,
    pub panel: Option<Panel>,
    /// Objekt-Texturen je Absatz: (Prüfwert, Textur).
    pub tex: HashMap<usize, (u64, egui::TextureHandle)>,
    pub spell: bool,
    pub autocorr: bool,
    init: bool,
    /// Rechtschreibfehler (Bereiche) und Stand.
    pub miss: Vec<(Pos, Pos)>,
    miss_rev: u64,
    /// Antworten im Kommentarfeld (Kommentar-ID → Text).
    pub reply: HashMap<u32, String>,
    /// Zeichnen im Zeichnungsfenster: Farbe, Breite, laufender Strich.
    pub pen: (u32, f32),
    pub cur_stroke: Option<DStroke>,
    /// Zelle, an der Tabellenbefehle wirken (Absatz, Zeile, Spalte).
    pub last_cell: Option<(usize, usize, usize)>,
    /// Nächster PDF-Export ist der Serienbrief.
    pub merge_pdf: bool,
}

impl Default for XState {
    fn default() -> Self {
        XState { dlg: None, cell: None, panel: None, tex: HashMap::new(), spell: true, autocorr: true, init: false, miss: Vec::new(), miss_rev: 0, reply: HashMap::new(), pen: (0xFF1D1D24, 2.0), cur_stroke: None, last_cell: None, merge_pdf: false }
    }
}

/// Neue Befehle in der Befehlssuche (Befehl, Name, Tastenkürzel, Suchwörter).
pub const XCMDS: &[(Cmd, &str, &str, &str)] = &[
    (Cmd::Table, "Tabelle einfügen", "", "raster zeilen spalten grid"),
    (Cmd::TableFromText, "Text in Tabelle umwandeln", "", "tabelle konvertieren"),
    (Cmd::RowAbove, "Zeile oberhalb einfügen", "", "tabelle"),
    (Cmd::RowBelow, "Zeile unterhalb einfügen", "", "tabelle"),
    (Cmd::ColLeft, "Spalte links einfügen", "", "tabelle"),
    (Cmd::ColRight, "Spalte rechts einfügen", "", "tabelle"),
    (Cmd::DelRow, "Zeile löschen", "", "tabelle entfernen"),
    (Cmd::DelCol, "Spalte löschen", "", "tabelle entfernen"),
    (Cmd::DelTable, "Tabelle löschen", "", "entfernen"),
    (Cmd::TableHeader, "Kopfzeile der Tabelle", "", "überschrift wiederholen"),
    (Cmd::TableStripes, "Gebänderte Zeilen", "", "streifen tabelle"),
    (Cmd::TableBorder, "Tabellenrahmen ein/aus", "", "linien gitter"),
    (Cmd::SortAsc, "Tabelle aufsteigend sortieren", "", "a-z"),
    (Cmd::SortDesc, "Tabelle absteigend sortieren", "", "z-a"),
    (Cmd::TableToText, "Tabelle in Text umwandeln", "", "konvertieren"),
    (Cmd::Shape(Shape::TextBox), "Textfeld", "", "textbox kasten"),
    (Cmd::Shape(Shape::Rect), "Form: Rechteck", "", "shape viereck"),
    (Cmd::Shape(Shape::Round), "Form: Abgerundetes Rechteck", "", "shape"),
    (Cmd::Shape(Shape::Ellipse), "Form: Ellipse", "", "shape kreis oval"),
    (Cmd::Shape(Shape::Triangle), "Form: Dreieck", "", "shape"),
    (Cmd::Shape(Shape::Diamond), "Form: Raute", "", "shape"),
    (Cmd::Shape(Shape::Arrow), "Form: Pfeil", "", "shape"),
    (Cmd::Shape(Shape::Star), "Form: Stern", "", "shape"),
    (Cmd::Shape(Shape::Line), "Form: Linie", "", "shape strich"),
    (Cmd::Shape(Shape::Callout), "Form: Sprechblase", "", "shape legende"),
    (Cmd::Drawing, "Zeichnung", "", "skizze malen stift"),
    (Cmd::Formula, "Formel", "Alt+=", "gleichung mathe latex"),
    (Cmd::Chart, "Diagramm", "", "chart balken säulen kreis linie"),
    (Cmd::EditObject, "Objekt bearbeiten", "", "form formel diagramm ändern"),
    (Cmd::Wrap(Wrap::Inline), "Umfluss: Mit Text in Zeile", "", "textumbruch"),
    (Cmd::Wrap(Wrap::Left), "Umfluss: Links, Text rechts daneben", "", "textumbruch quadrat"),
    (Cmd::Wrap(Wrap::Right), "Umfluss: Rechts, Text links daneben", "", "textumbruch quadrat"),
    (Cmd::Caption, "Beschriftung einfügen", "", "abbildung tabelle nummer"),
    (Cmd::Columns(1), "Spalten: eine", "", "layout"),
    (Cmd::Columns(2), "Spalten: zwei", "", "layout zeitung"),
    (Cmd::Columns(3), "Spalten: drei", "", "layout zeitung"),
    (Cmd::Footnote, "Fußnote einfügen", "Strg+Alt+F", "anmerkung endnote"),
    (Cmd::CrossRef, "Querverweis", "", "verweis siehe abbildung"),
    (Cmd::Cite, "Zitat einfügen", "", "quelle literatur zitieren"),
    (Cmd::Bibliography, "Literaturverzeichnis", "", "quellen bibliografie"),
    (Cmd::IndexMark, "Stichwort markieren", "Alt+Umschalt+X", "index eintrag"),
    (Cmd::Index, "Stichwortverzeichnis", "", "index register"),
    (Cmd::Figures, "Abbildungsverzeichnis", "", "beschriftungen verzeichnis"),
    (Cmd::PageField, "Feld: Seitenzahl", "", "seite nummer"),
    (Cmd::PagesField, "Feld: Anzahl Seiten", "", "seiten"),
    (Cmd::UpdateFields, "Felder aktualisieren", "F9", "datum aktualisieren"),
    (Cmd::Comment, "Kommentar einfügen", "Strg+Alt+M", "notiz anmerkung"),
    (Cmd::Comments, "Kommentare anzeigen", "", "seitenleiste"),
    (Cmd::Track, "Änderungen nachverfolgen", "", "überarbeiten tracking"),
    (Cmd::Changes, "Änderungen anzeigen", "", "überarbeitung liste"),
    (Cmd::AcceptHere, "Änderung annehmen", "", "übernehmen"),
    (Cmd::RejectHere, "Änderung ablehnen", "", "verwerfen"),
    (Cmd::AcceptAll, "Alle Änderungen annehmen", "", "übernehmen"),
    (Cmd::RejectAll, "Alle Änderungen ablehnen", "", "verwerfen"),
    (Cmd::Versions, "Versionsverlauf", "", "version wiederherstellen vergleichen"),
    (Cmd::SaveVersion, "Version speichern", "", "stand sichern"),
    (Cmd::Spell, "Rechtschreibung prüfen", "F7", "fehler wörterbuch"),
    (Cmd::Hyphen, "Silbentrennung", "", "trennen umbruch"),
    (Cmd::AutoCorrect, "Autokorrektur", "", "ersetzen tippfehler anführungszeichen"),
    (Cmd::Language, "Sprache und Prüfung", "", "wörterbuch einstellungen"),
    (Cmd::MergeData, "Seriendruck: Daten", "", "serienbrief csv adressen"),
    (Cmd::MergeField, "Seriendruck: Feld einfügen", "", "serienbrief platzhalter"),
    (Cmd::MergeExport, "Seriendruck: Alle als PDF", "", "serienbrief drucken"),
    (Cmd::CheckBox, "Kontrollkästchen", "", "formular checkbox ankreuzen"),
    (Cmd::Choice, "Auswahlfeld", "", "formular dropdown liste"),
    (Cmd::Templates, "Vorlagen", "", "brief bericht lebenslauf rechnung"),
    (Cmd::Theme, "Design", "", "farben schriften stil"),
    (Cmd::Watermark, "Wasserzeichen", "", "entwurf vertraulich"),
    (Cmd::Protect, "Dokument schützen", "", "schreibschutz kennwort sperren"),
    (Cmd::Envelope, "Umschlag", "", "brief adresse kuvert"),
    (Cmd::Labels, "Etiketten", "", "aufkleber adressen"),
    (Cmd::ExportOther, "Speichern als ODT, RTF, HTML, Markdown", "", "export libreoffice opendocument website text"),
    (Cmd::ImportOther, "Datei einfügen (ODT, RTF, HTML, Markdown, Text)", "", "import öffnen libreoffice"),
];

/// Befehle, die nichts ändern (gehen auch bei geschütztem Dokument).
pub fn is_readonly_cmd(c: Cmd) -> bool {
    use Cmd::*;
    matches!(c, Copy | SelectAll | Find | WordCount | Shortcuts | Outline | ZoomIn | ZoomOut | ZoomReset | ExportPdf | ExportDocx | Print | SaveCopy | Help | Back | Comments | Changes | Protect | Spell | Language | MergeExport | ExportOther | Versions)
}

fn now_iso() -> String {
    crate::session::rfc3339_now()
}

fn today() -> String {
    let d = now_iso();
    format!("{}.{}.{}", &d[8..10], &d[5..7], &d[0..4])
}

/// Name für Kommentare und Änderungen: Kontoname, sonst „Ich“.
pub fn author(sh: &Shared) -> String {
    if let Some(n) = sh.settings.extra.get("writer_author").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
        return n.trim().to_string();
    }
    crate::net::with(|n| n.me.as_ref().map(|m| if m.name.is_empty() { m.email.clone() } else { m.name.clone() })).filter(|s| !s.is_empty()).unwrap_or_else(|| "Ich".into())
}

fn flag(sh: &Shared, k: &str, def: bool) -> bool {
    sh.settings.extra.get(k).and_then(|v| v.as_bool()).unwrap_or(def)
}

impl Writer {
    pub fn protected(&self) -> bool {
        !self.td.ex.protect.is_empty()
    }

    /// Einmal je Öffnen: Einstellungen übernehmen; Rechtschreibung aktuell halten.
    pub(super) fn x_frame(&mut self, sh: &mut Shared) {
        if !self.x.init {
            self.x.init = true;
            self.x.spell = flag(sh, "writer_spell", true);
            self.x.autocorr = flag(sh, "writer_autocorrect", true);
            if let Some(v) = sh.settings.extra.get("writer_words").and_then(|v| v.as_array()) {
                lang::set_user_words(v.iter().filter_map(|x| x.as_str().map(String::from)).collect());
            }
        }
        if self.x.spell && self.x.miss_rev != self.rev {
            self.x.miss_rev = self.rev;
            let mut v = Vec::new();
            for (pi, p) in self.td.paras.iter().enumerate() {
                if p.kind.is_block() {
                    continue;
                }
                let t = p.text();
                for (a, b) in lang::misspelled(&t) {
                    // Nicht direkt am Cursor (Wort wird gerade getippt)
                    if pi == self.cur.p && b == self.cur.o {
                        continue;
                    }
                    v.push((Pos::new(pi, a), Pos::new(pi, b)));
                }
            }
            self.x.miss = v;
        }
        if !self.x.spell {
            self.x.miss.clear();
        }
    }

    /// Absatz am Cursor, falls Bild, Objekt oder Tabelle (ausgewählt).
    pub fn sel_block(&self) -> Option<usize> {
        (!self.has_sel() && self.td.paras.get(self.cur.p).is_some_and(|p| matches!(p.kind, Kind::Image | Kind::Object | Kind::Table))).then_some(self.cur.p)
    }

    /// Autokorrektur beim Tippen eines Zeichens; true = Zeichen ist verbraucht.
    pub(super) fn autocorrect(&mut self, ch: char) -> bool {
        if !self.x.autocorr || self.has_sel() {
            return false;
        }
        let p = &self.td.paras[self.cur.p.min(self.td.paras.len() - 1)];
        let before: String = p.text().chars().take(self.cur.o).collect();
        let Some(cor) = lang::autocorrect(&before, ch) else { return false };
        let st = self.td.style_at(self.cur);
        let a = Pos::new(self.cur.p, self.cur.o.saturating_sub(cor.remove));
        self.cur = self.td.delete(a, self.cur);
        self.cur = if self.td.ex.track {
            let rev = self.td.ex.rev(&self.x_author(), &now_iso());
            tools::track_insert(&mut self.td, self.cur, &cor.insert, &st, rev)
        } else {
            self.td.insert_text(self.cur, &cor.insert, &st)
        };
        self.anchor = self.cur;
        cor.consume_typed
    }

    pub(super) fn x_author(&self) -> String {
        self.author.clone()
    }

    pub(super) fn track_rev(&mut self) -> u32 {
        let a = self.x_author();
        self.td.ex.rev(&a, &now_iso())
    }

    fn insert_block(&mut self, para: Para, now: f64) {
        self.insert_paras(&[para], now);
        self.cur = Pos::new(self.cur.p.saturating_sub(1), 0);
        self.anchor = self.cur;
    }

    fn insert_field(&mut self, f: Field, now: f64) {
        if self.protected() {
            return;
        }
        self.begin(Edit::Other, now);
        self.delete_sel();
        let mut st = self.td.style_at(self.cur);
        if self.td.ex.track {
            st.ins = self.track_rev();
        }
        self.cur = tools::insert_field(&mut self.td, self.cur, f, &st);
        self.anchor = self.cur;
        self.changed(now);
    }

    fn table_mut(&mut self, p: usize) -> Option<&mut Table> {
        self.td.paras.get_mut(p).and_then(|q| q.table.as_mut())
    }

    /// Zelle für Tabellenbefehle.
    fn cur_cell(&self) -> Option<(usize, usize, usize)> {
        if let Some(e) = &self.x.cell {
            return Some((e.p, e.r, e.c));
        }
        let p = self.sel_block().filter(|p| self.td.paras[*p].kind == Kind::Table)?;
        Some(self.x.last_cell.filter(|l| l.0 == p).unwrap_or((p, 0, 0)))
    }

    /// Offene Zelle ins Dokument schreiben.
    pub(super) fn commit_cell(&mut self, now: f64) {
        if let Some(e) = self.x.cell.clone() {
            let changed = self.td.paras.get(e.p).and_then(|q| q.table.as_ref()).and_then(|t| t.rows.get(e.r)).and_then(|r| r.get(e.c)).is_some_and(|c| c.text != e.text);
            if changed {
                self.begin(Edit::Type, now);
                if let Some(c) = self.table_mut(e.p).and_then(|t| t.rows.get_mut(e.r)).and_then(|r| r.get_mut(e.c)) {
                    c.text = e.text.clone();
                }
                self.changed(now);
            }
        }
    }

    /// Zelle öffnen.
    pub(super) fn open_cell(&mut self, p: usize, r: usize, c: usize) {
        let Some(t) = self.td.paras.get(p).and_then(|q| q.table.as_ref()) else { return };
        let (r, c) = (r.min(t.rows.len() - 1), c.min(t.ncols() - 1));
        let text = t.rows[r][c].text.clone();
        self.x.cell = Some(CellEdit { p, r, c, text, focus: true });
        self.x.last_cell = Some((p, r, c));
        self.cur = Pos::new(p, 0);
        self.anchor = self.cur;
    }

    /// Tab in der Zelle: nächste Zelle, am Ende neue Zeile.
    pub(super) fn cell_step(&mut self, back: bool, now: f64) {
        self.commit_cell(now);
        let Some(e) = self.x.cell.clone() else { return };
        let Some(t) = self.td.paras.get(e.p).and_then(|q| q.table.as_ref()) else { return };
        let (nr, nc) = (t.rows.len(), t.ncols());
        let i = e.r * nc + e.c;
        let j = if back { i.saturating_sub(1) } else { i + 1 };
        if j >= nr * nc {
            self.begin(Edit::Other, now);
            if let Some(t) = self.table_mut(e.p) {
                t.insert_row(nr);
            }
            self.changed(now);
        }
        self.open_cell(e.p, j / nc, j % nc);
    }

    fn table_op(&mut self, now: f64, f: impl FnOnce(&mut Table, usize, usize) -> Option<(usize, usize)>) {
        self.commit_cell(now);
        let Some((p, r, c)) = self.cur_cell() else { return };
        self.begin(Edit::Other, now);
        let next = self.table_mut(p).and_then(|t| f(t, r, c));
        self.changed(now);
        match (next, self.x.cell.is_some()) {
            (Some((r, c)), true) => self.open_cell(p, r, c),
            (Some((r, c)), false) => self.x.last_cell = Some((p, r, c)),
            _ => self.x.cell = None,
        }
    }

    fn remove_block(&mut self, p: usize, now: f64) {
        self.x.cell = None;
        self.remove_para(p, now);
    }

    fn set_wrap(&mut self, wr: Wrap, now: f64) {
        if let Some(p) = self.sel_block().filter(|p| matches!(self.td.paras[*p].kind, Kind::Image | Kind::Object)) {
            self.begin(Edit::Other, now);
            self.td.paras[p].wrap = wr;
            self.changed(now);
        }
    }

    fn ensure_id(&mut self, p: usize) -> u32 {
        if self.td.paras[p].id == 0 {
            self.td.paras[p].id = self.td.ex.id();
        }
        self.td.paras[p].id
    }

    /// Ziele für Querverweise: (Absatz, Anzeige).
    pub fn ref_targets(&self) -> Vec<(usize, String)> {
        let nums = Numbers::new(&self.td);
        self.td
            .paras
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                if p.kind == Kind::Caption {
                    Some((i, format!("{}{}", nums.caption_prefix(i).unwrap_or_default(), p.text().replace(OBJ, ""))))
                } else if p.kind.is_heading() && !p.text().trim().is_empty() {
                    Some((i, p.text().replace(OBJ, "")))
                } else {
                    None
                }
            })
            .collect()
    }

    fn add_comment(&mut self, text: String, now: f64, sh: &Shared) {
        if text.trim().is_empty() {
            return;
        }
        let (mut a, mut b) = self.sel();
        if a == b {
            (a, b) = self.td.word_at(self.cur);
        }
        if a == b {
            b = self.td.next(a);
        }
        self.begin(Edit::Other, now);
        let id = self.td.ex.id();
        self.td.ex.comments.push(Comment { id, author: author(sh), time: now_iso(), text: text.trim().into(), replies: Vec::new(), done: false });
        self.td.restyle(a, b, &move |s: &mut CStyle| s.cmt = id);
        self.x.panel = Some(super::extra::Panel::Comments);
        self.changed(now);
    }

    fn delete_comment(&mut self, id: u32, now: f64) {
        self.begin(Edit::Other, now);
        self.td.ex.comments.retain(|c| c.id != id);
        let (a, b) = (Pos::default(), self.td.end());
        self.td.restyle(a, b, &move |s: &mut CStyle| {
            if s.cmt == id {
                s.cmt = 0;
            }
        });
        self.changed(now);
    }

    fn jump(&mut self, p: Pos, now: f64) {
        self.set_cur(p, false, now);
        self.focus_req = true;
    }

    fn resolve_rev(&mut self, all: bool, accept: bool, sh: &mut Shared) {
        let now = sh.now;
        let id = if all { None } else { tools::rev_at(&self.td, self.cur).or_else(|| tools::rev_at(&self.td, self.td.prev(self.cur))) };
        if !all && id.is_none() {
            sh.toast("Hier ist keine Änderung");
            return;
        }
        self.begin(Edit::Other, now);
        let n = if accept { tools::accept(&mut self.td, id) } else { tools::reject(&mut self.td, id) };
        self.cur = self.td.clamp(self.cur);
        self.anchor = self.cur;
        self.changed(now);
        if all {
            sh.toast(if n == 0 { "Keine Änderungen" } else if accept { "Alle Änderungen angenommen" } else { "Alle Änderungen abgelehnt" });
        }
    }

    fn replace_all_doc(&mut self, td: owndraw_core::office::TextDoc, now: f64) {
        self.begin(Edit::Other, now);
        let versions = std::mem::take(&mut self.td.ex.versions);
        self.td = td;
        if self.td.ex.versions.is_empty() {
            self.td.ex.versions = versions;
        }
        self.cur = Pos::default();
        self.anchor = self.cur;
        self.x.cell = None;
        self.changed(now);
    }

    fn insert_generated(&mut self, paras: Vec<Para>, now: f64) {
        let c = self.cur;
        self.cur = Pos::new(c.p, 0);
        self.anchor = self.cur;
        self.insert_paras(&paras, now);
    }
}

/// Erweiterte Befehle; false = nicht zuständig.
/// Funktion im Feature-Katalog, zu der ein neuer Befehl gehört (Admin kann sie sperren).
pub fn feature_of(c: Cmd) -> Option<&'static str> {
    use owndraw_core::features as f;
    use Cmd::*;
    Some(match c {
        Table | TableFromText | RowAbove | RowBelow | ColLeft | ColRight | DelRow | DelCol | DelTable | TableHeader | TableStripes | TableBorder | SortAsc | SortDesc | TableToText | Shape(_) | Drawing | Formula | Chart | EditObject => f::OFFICE_OBJECTS,
        Footnote | CrossRef | Cite | Bibliography | IndexMark | Index | Figures => f::OFFICE_REFS,
        Comment | Comments | Track | Changes | AcceptHere | RejectHere | AcceptAll | RejectAll | Versions | SaveVersion => f::OFFICE_REVIEW,
        MergeData | MergeField | MergeExport | CheckBox | Choice | Envelope | Labels => f::OFFICE_MAIL,
        ExportOther | ImportOther => f::OFFICE_FORMATS,
        _ => return None,
    })
}

pub fn run(w: &mut Writer, c: Cmd, sh: &mut Shared, _ctx: &egui::Context) -> bool {
    if feature_of(c).is_some_and(crate::locks::locked) {
        sh.toast("Diese Funktion hat der Administrator ausgeschaltet");
        return true;
    }
    let now = sh.now;
    let cw = w.td.page.content_w() / (w.td.ex.cols.max(1) as f32);
    match c {
        Cmd::Table => w.x.dlg = Some(XDlg::Table { r: 3, c: 3 }),
        Cmd::TableFromText => {
            let (a, b) = w.sel();
            let t = if w.has_sel() { w.td.text_range(a, b) } else { w.cur_para().text() };
            if t.trim().is_empty() {
                sh.toast("Markiere Zeilen mit Tabulatoren oder Semikolons");
                return true;
            }
            w.begin(Edit::Other, now);
            let (a, b) = if w.has_sel() { (a, b) } else { (Pos::new(w.cur.p, 0), Pos::new(w.cur.p, w.cur_para().len())) };
            w.cur = w.td.delete(a, b);
            w.anchor = w.cur;
            let tb = Table::from_text(&t, cw);
            w.insert_block(Para { table: Some(tb), ..Para::new(Kind::Table) }, now);
        }
        Cmd::RowAbove => w.table_op(now, |t, r, c| {
            t.insert_row(r);
            Some((r, c))
        }),
        Cmd::RowBelow => w.table_op(now, |t, r, c| {
            t.insert_row(r + 1);
            Some((r + 1, c))
        }),
        Cmd::ColLeft => w.table_op(now, |t, r, c| {
            t.insert_col(c);
            Some((r, c))
        }),
        Cmd::ColRight => w.table_op(now, |t, r, c| {
            t.insert_col(c + 1);
            Some((r, c + 1))
        }),
        Cmd::DelRow => {
            if w.cur_cell().and_then(|(p, ..)| w.td.paras[p].table.as_ref()).is_some_and(|t| t.rows.len() == 1) {
                return run(w, Cmd::DelTable, sh, _ctx);
            }
            w.table_op(now, |t, r, c| {
                t.delete_row(r);
                Some((r.min(t.rows.len() - 1), c))
            })
        }
        Cmd::DelCol => {
            if w.cur_cell().and_then(|(p, ..)| w.td.paras[p].table.as_ref()).is_some_and(|t| t.ncols() == 1) {
                return run(w, Cmd::DelTable, sh, _ctx);
            }
            w.table_op(now, |t, r, c| {
                t.delete_col(c);
                Some((r, c.min(t.ncols() - 1)))
            })
        }
        Cmd::DelTable => {
            if let Some((p, ..)) = w.cur_cell() {
                w.remove_block(p, now);
            }
        }
        Cmd::TableHeader => w.table_op(now, |t, r, c| {
            t.header = !t.header;
            Some((r, c))
        }),
        Cmd::TableStripes => w.table_op(now, |t, r, c| {
            t.stripes = !t.stripes;
            Some((r, c))
        }),
        Cmd::TableBorder => w.table_op(now, |t, r, c| {
            t.border = if t.border == 0 { 0xFFB8BCC6 } else { 0 };
            Some((r, c))
        }),
        Cmd::SortAsc | Cmd::SortDesc => {
            let desc = c == Cmd::SortDesc;
            w.table_op(now, move |t, r, col| {
                t.sort_by(col, desc);
                Some((r, col))
            })
        }
        Cmd::TableToText => {
            if let Some((p, ..)) = w.cur_cell() {
                w.commit_cell(now);
                let text = w.td.paras[p].table.as_ref().map(|t| t.to_text()).unwrap_or_default();
                w.x.cell = None;
                w.begin(Edit::Other, now);
                w.td.paras[p] = Para::new(Kind::Body);
                w.cur = Pos::new(p, 0);
                w.cur = w.td.insert_text(w.cur, &text, &CStyle::default());
                w.anchor = w.cur;
                w.changed(now);
            }
        }
        Cmd::Shape(s) => {
            let mut o = Obj::shape(s);
            if s == Shape::TextBox {
                if let ObjKind::Shape { text, .. } = &mut o.kind {
                    *text = if w.has_sel() { w.sel_text() } else { String::new() };
                }
            }
            let at = w.cur.p;
            w.insert_block(Para { obj: Some(o.clone()), wrap: if s == Shape::TextBox { Wrap::Right } else { Wrap::Inline }, ..Para::new(Kind::Object) }, now);
            let _ = at;
            w.x.dlg = Some(XDlg::Obj { p: w.cur.p, o });
        }
        Cmd::Drawing => {
            let o = Obj { kind: ObjKind::Drawing { strokes: Vec::new() }, w: cw.min(360.0), h: 200.0 };
            w.insert_block(Para { obj: Some(o.clone()), align: Align::Center, ..Para::new(Kind::Object) }, now);
            w.x.dlg = Some(XDlg::Obj { p: w.cur.p, o });
        }
        Cmd::Formula => {
            let src = if w.has_sel() { w.sel_text() } else { "a^2 + b^2 = c^2".into() };
            let (fw, fh) = owndraw_core::office::draw::formula_size(&src, 16.0);
            let o = Obj { kind: ObjKind::Formula { src }, w: fw.max(20.0), h: fh.max(14.0) };
            w.insert_block(Para { obj: Some(o.clone()), align: Align::Center, ..Para::new(Kind::Object) }, now);
            w.x.dlg = Some(XDlg::Obj { p: w.cur.p, o });
        }
        Cmd::Chart => {
            let o = Obj::chart();
            w.insert_block(Para { obj: Some(o.clone()), align: Align::Center, ..Para::new(Kind::Object) }, now);
            w.x.dlg = Some(XDlg::Obj { p: w.cur.p, o });
        }
        Cmd::EditObject => match w.sel_block().and_then(|p| w.td.paras[p].obj.clone().map(|o| (p, o))) {
            Some((p, o)) => w.x.dlg = Some(XDlg::Obj { p, o }),
            None => sh.toast("Wähle zuerst eine Form, Formel, Zeichnung oder ein Diagramm"),
        },
        Cmd::Wrap(wr) => {
            if w.sel_block().is_none() {
                sh.toast("Wähle zuerst ein Bild oder Objekt");
            }
            w.set_wrap(wr, now);
        }
        Cmd::Caption => {
            let label = match w.sel_block().map(|p| w.td.paras[p].kind) {
                Some(Kind::Table) => "Tabelle",
                Some(Kind::Object) if matches!(w.td.paras[w.cur.p].obj.as_ref().map(|o| &o.kind), Some(ObjKind::Formula { .. })) => "Formel",
                _ => "Abbildung",
            };
            w.x.dlg = Some(XDlg::Caption { label: label.into(), text: String::new() });
        }
        Cmd::Columns(n) => {
            w.begin(Edit::Other, now);
            w.td.ex.cols = n.clamp(1, 3);
            w.changed(now);
        }
        Cmd::Footnote => {
            let at = if w.has_sel() { w.sel().1 } else { w.cur };
            w.x.dlg = Some(XDlg::Note { at, text: String::new(), new: true });
        }
        Cmd::CrossRef => {
            if w.ref_targets().is_empty() {
                sh.toast("Keine Ziele: füge Überschriften oder Beschriftungen ein");
            } else {
                w.x.dlg = Some(XDlg::CrossRef { show: RefShow::Label });
            }
        }
        Cmd::Cite => w.x.dlg = Some(XDlg::Cite { pick: w.td.ex.sources.last().map(|s| s.id), s: Source::default() }),
        Cmd::Bibliography => {
            let v = tools::bibliography(&w.td);
            w.insert_generated(v, now);
        }
        Cmd::IndexMark => {
            let t = if w.has_sel() { w.sel_text() } else { let (a, b) = w.td.word_at(w.cur); w.td.text_range(a, b) };
            w.x.dlg = Some(XDlg::Index(t.trim().replace('\n', " ")));
        }
        Cmd::Index => {
            let lay = w.layout().clone();
            let v = tools::index(&w.td, &lay);
            w.insert_generated(v, now);
        }
        Cmd::Figures => {
            let lay = w.layout().clone();
            let label = if w.td.paras.iter().any(|p| p.kind == Kind::Caption && p.cap == "Abbildung") || !w.td.paras.iter().any(|p| p.kind == Kind::Caption) { "Abbildung" } else { "Tabelle" };
            let v = tools::figures(&w.td, &lay, label);
            w.insert_generated(v, now);
        }
        Cmd::PageField => w.insert_field(Field::Page, now),
        Cmd::PagesField => w.insert_field(Field::Pages, now),
        Cmd::UpdateFields => {
            w.begin(Edit::Other, now);
            let n = tools::update_fields(&mut w.td, &today());
            w.changed(now);
            sh.toast(&if n == 0 { "Felder sind aktuell".to_string() } else { format!("{n} Felder aktualisiert") });
        }
        Cmd::Comment => w.x.dlg = Some(XDlg::Comment(String::new())),
        Cmd::Comments => w.x.panel = if w.x.panel == Some(Panel::Comments) { None } else { Some(Panel::Comments) },
        Cmd::Changes => w.x.panel = if w.x.panel == Some(Panel::Changes) { None } else { Some(Panel::Changes) },
        Cmd::Track => {
            w.begin(Edit::Other, now);
            w.td.ex.track = !w.td.ex.track;
            w.changed(now);
            sh.toast(if w.td.ex.track { "Änderungen werden nachverfolgt" } else { "Nachverfolgen aus" });
            if w.td.ex.track {
                w.x.panel = Some(Panel::Changes);
            }
        }
        Cmd::AcceptHere => w.resolve_rev(false, true, sh),
        Cmd::RejectHere => w.resolve_rev(false, false, sh),
        Cmd::AcceptAll => w.resolve_rev(true, true, sh),
        Cmd::RejectAll => w.resolve_rev(true, false, sh),
        Cmd::SaveVersion => {
            w.begin(Edit::Other, now);
            let n = format!("Version {}", w.td.ex.versions.len() + 1);
            tools::save_version(&mut w.td, &n, &now_iso());
            w.changed(now);
            sh.toast("Version gespeichert");
        }
        Cmd::Versions => w.x.dlg = Some(XDlg::Versions { name: String::new() }),
        Cmd::Spell => {
            w.x.spell = !w.x.spell;
            w.x.miss_rev = 0;
            sh.settings.extra.insert("writer_spell".into(), serde_json::json!(w.x.spell));
            let n = if w.x.spell {
                w.x_frame(sh);
                w.x.miss.len()
            } else {
                0
            };
            sh.toast(&if !w.x.spell { "Rechtschreibprüfung aus".to_string() } else if n == 0 { "Keine Rechtschreibfehler gefunden".into() } else { format!("{n} mögliche Fehler rot unterstrichen – Rechtsklick zeigt Vorschläge") });
            if let Some((a, _)) = w.x.miss.iter().find(|(a, _)| *a >= w.cur).or(w.x.miss.first()).copied() {
                w.jump(a, now);
            }
        }
        Cmd::Hyphen => {
            w.begin(Edit::Other, now);
            w.td.ex.hyph = !w.td.ex.hyph;
            w.changed(now);
        }
        Cmd::AutoCorrect => {
            w.x.autocorr = !w.x.autocorr;
            sh.settings.extra.insert("writer_autocorrect".into(), serde_json::json!(w.x.autocorr));
            sh.toast(if w.x.autocorr { "Autokorrektur an" } else { "Autokorrektur aus" });
        }
        Cmd::Language => w.x.dlg = Some(XDlg::Lang),
        Cmd::MergeData => {
            let csv = w.td.ex.merge.as_ref().map(|m| std::iter::once(m.head.join(";")).chain(m.rows.iter().map(|r| r.join(";"))).collect::<Vec<_>>().join("\n")).unwrap_or_else(|| "Anrede;Vorname;Name;Straße;Ort\nFrau;Anna;Müller;Hauptstr. 1;10115 Berlin\nHerr;Bernd;Schulz;Ringweg 5;50667 Köln".into());
            w.x.dlg = Some(XDlg::Merge(csv));
        }
        Cmd::MergeField => {
            if w.td.ex.merge.is_none() {
                sh.toast("Lege zuerst die Daten fest (Seriendruck: Daten)");
                return run(w, Cmd::MergeData, sh, _ctx);
            }
            w.x.dlg = Some(XDlg::MergeField(String::new()));
        }
        Cmd::MergeExport => {
            if w.td.ex.merge.as_ref().is_none_or(|m| m.rows.is_empty()) {
                sh.toast("Keine Seriendruck-Daten");
                return true;
            }
            w.x.merge_pdf = true;
            sh.reqs.push(crate::app::Req::PickSave { purpose: crate::app::PickPurpose::Export(owndraw_core::export::formats::Fmt::Pdf), name: format!("{} (Serienbrief).pdf", crate::app::safe_file_name(w.name())), kind: crate::platform::FileKind::Export(owndraw_core::export::formats::Fmt::Pdf) });
        }
        Cmd::CheckBox => w.insert_field(Field::Check(false), now),
        Cmd::Choice => w.x.dlg = Some(XDlg::Choice("Ja\nNein\nVielleicht".into())),
        Cmd::Templates => w.x.dlg = Some(XDlg::Templates),
        Cmd::Theme => w.x.dlg = Some(XDlg::Theme),
        Cmd::Watermark => w.x.dlg = Some(XDlg::Watermark(if w.td.ex.watermark.is_empty() { "ENTWURF".into() } else { w.td.ex.watermark.clone() })),
        Cmd::Protect => w.x.dlg = Some(XDlg::Protect(String::new())),
        Cmd::Envelope => w.x.dlg = Some(XDlg::Envelope("Vorname Nachname\nStraße 1\n12345 Ort".into(), "Frau Beispiel\nMusterweg 2\n54321 Stadt".into())),
        Cmd::ExportOther => w.x.dlg = Some(XDlg::Formats),
        Cmd::ImportOther => sh.reqs.push(crate::app::Req::PickOpen { purpose: crate::app::PickPurpose::TextInsert, kinds: vec![crate::platform::FileKind::Any] }),
        Cmd::Labels => w.x.dlg = Some(XDlg::Labels("Vorname Nachname\nStraße 1\n12345 Ort".into(), 3, 8)),
        _ => return false,
    }
    true
}

// ---------------------------------------------------------------- Kontextleiste

fn chip(ui: &mut egui::Ui, label: &str, on: bool, tip: &str) -> bool {
    let p = pal();
    let g = ui.painter().layout_no_wrap(label.into(), egui::FontId::proportional(13.0), c(p.text));
    let (r, resp) = ui.allocate_exact_size(Vec2::new(g.size().x + 16.0, 28.0), Sense::click());
    let bg = if on { p.active } else if resp.hovered() { p.hover } else { 0 };
    if bg != 0 {
        ui.painter().rect_filled(r, CornerRadius::same(6), c(bg));
    }
    ui.painter().galley(egui::Pos2::new(r.left() + 8.0, r.center().y - g.size().y / 2.0), g, c(p.text));
    resp.on_hover_text(tip).clicked()
}

/// Leiste über der Seite, wenn eine Tabelle, ein Bild oder ein Objekt ausgewählt ist.
pub fn context_bar(w: &mut Writer, ui: &mut egui::Ui, sh: &mut Shared) {
    let ctx = ui.ctx().clone();
    let Some(p) = w.x.cell.as_ref().map(|e| e.p).or(w.sel_block()) else { return };
    let Some(para) = w.td.paras.get(p).cloned() else { return };
    let mut cmd = None;
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        match para.kind {
            Kind::Table => {
                ui.label(style::hint("Tabelle"));
                for (cm, l, tip) in [(Cmd::RowAbove, "+ Zeile oben", "Zeile oberhalb einfügen"), (Cmd::RowBelow, "+ Zeile unten", "Zeile unterhalb einfügen"), (Cmd::ColLeft, "+ Spalte links", "Spalte links einfügen"), (Cmd::ColRight, "+ Spalte rechts", "Spalte rechts einfügen"), (Cmd::DelRow, "− Zeile", "Zeile löschen"), (Cmd::DelCol, "− Spalte", "Spalte löschen")] {
                    if chip(ui, l, false, tip) {
                        cmd = Some(cm);
                    }
                }
                ui.separator();
                let t = para.table.clone().unwrap_or_else(|| Table::new(1, 1, 100.0));
                if chip(ui, "Kopfzeile", t.header, "Erste Zeile hervorheben und auf jeder Seite wiederholen") {
                    cmd = Some(Cmd::TableHeader);
                }
                if chip(ui, "Streifen", t.stripes, "Gebänderte Zeilen") {
                    cmd = Some(Cmd::TableStripes);
                }
                if chip(ui, "Rahmen", t.border != 0, "Linien ein/aus") {
                    cmd = Some(Cmd::TableBorder);
                }
                if chip(ui, "A–Z", false, "Nach dieser Spalte aufsteigend sortieren") {
                    cmd = Some(Cmd::SortAsc);
                }
                if chip(ui, "Z–A", false, "Absteigend sortieren") {
                    cmd = Some(Cmd::SortDesc);
                }
                // Zellformat
                if let Some((_, r, cc)) = w.cur_cell() {
                    let cell = t.rows.get(r).and_then(|x| x.get(cc)).cloned().unwrap_or_default();
                    ui.separator();
                    if chip(ui, "F", cell.b, "Zelle fett") {
                        w.cell_fmt(sh.now, |c| c.b = !c.b);
                    }
                    for (a, l) in [(Align::Left, "Links"), (Align::Center, "Mitte"), (Align::Right, "Rechts")] {
                        if chip(ui, l, cell.align == a, "Zelle ausrichten") {
                            w.cell_fmt(sh.now, move |c| c.align = a);
                        }
                    }
                    let fr = ui.add(egui::Button::new("Farbe").min_size(Vec2::new(0.0, 28.0))).on_hover_text("Zellhintergrund");
                    egui::Popup::menu(&fr).show(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.set_max_width(200.0);
                            for col in [0u32, 0xFFFDE68A, 0xFFBBF7D0, 0xFFBFDBFE, 0xFFFBCFE8, 0xFFFED7AA, 0xFFE5E7EB, 0xFF1F2937] {
                                let (r, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                                if col == 0 {
                                    ui.painter().circle_stroke(r.center(), 10.0, Stroke::new(1.0, c(0xFF9CA3AF)));
                                } else {
                                    ui.painter().circle_filled(r.center(), 10.0, c(col));
                                }
                                if resp.clicked() {
                                    w.cell_fmt(sh.now, move |c| {
                                        c.bg = col;
                                        c.color = if col == 0xFF1F2937 { 0xFFFFFFFF } else { 0 };
                                    });
                                    ui.close();
                                }
                            }
                        });
                    });
                }
                ui.separator();
                if chip(ui, "Beschriftung", false, "„Tabelle 1: …“ darunter") {
                    cmd = Some(Cmd::Caption);
                }
                if chip(ui, "Löschen", false, "Tabelle löschen") {
                    cmd = Some(Cmd::DelTable);
                }
            }
            Kind::Image | Kind::Object => {
                let name = para.obj.as_ref().map(|o| o.label()).unwrap_or("Bild");
                ui.label(style::hint(name));
                if para.kind == Kind::Object && chip(ui, "Bearbeiten", false, "Doppelklick öffnet es auch") {
                    cmd = Some(Cmd::EditObject);
                }
                ui.separator();
                ui.label(style::hint("Umfluss"));
                for (wr, l, tip) in [(Wrap::Inline, "In Zeile", "Eigene Zeile, Text darüber und darunter"), (Wrap::Left, "Links", "Objekt links, Text fließt rechts daneben"), (Wrap::Right, "Rechts", "Objekt rechts, Text fließt links daneben")] {
                    if chip(ui, l, para.wrap == wr, tip) {
                        cmd = Some(Cmd::Wrap(wr));
                    }
                }
                if para.wrap == Wrap::Inline {
                    ui.separator();
                    for (a, l) in [(Align::Left, "Links"), (Align::Center, "Mitte"), (Align::Right, "Rechts")] {
                        if chip(ui, l, para.align == a, "Ausrichten") {
                            cmd = Some(Cmd::Align(a));
                        }
                    }
                }
                ui.separator();
                if chip(ui, "Beschriftung", false, "„Abbildung 1: …“ darunter") {
                    cmd = Some(Cmd::Caption);
                }
                if chip(ui, "Löschen", false, "Entfernen (Entf)") {
                    w.remove_block(p, sh.now);
                }
            }
            _ => {}
        }
    });
    if let Some(cm) = cmd {
        w.run(cm, sh, &ctx);
    }
}

impl Writer {
    fn cell_fmt(&mut self, now: f64, f: impl FnOnce(&mut owndraw_core::office::objects::Cell)) {
        self.commit_cell(now);
        let Some((p, r, cc)) = self.cur_cell() else { return };
        self.begin(Edit::Other, now);
        if let Some(cell) = self.table_mut(p).and_then(|t| t.rows.get_mut(r)).and_then(|x| x.get_mut(cc)) {
            f(cell);
        }
        self.changed(now);
    }
}

// ---------------------------------------------------------------- Seitenleiste (Kommentare, Änderungen)

pub fn side_panel(w: &mut Writer, ui: &mut egui::Ui, sh: &mut Shared) {
    let now = sh.now;
    let Some(panel) = w.x.panel else { return };
    ui.horizontal(|ui| {
        for (p, l) in [(Panel::Comments, "Kommentare"), (Panel::Changes, "Änderungen")] {
            if chip(ui, l, panel == p, "") {
                w.x.panel = Some(p);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("×").on_hover_text("Schließen").clicked() {
                w.x.panel = None;
            }
        });
    });
    style::separator(ui);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match panel {
        Panel::Comments => {
            if style::primary_btn(ui, "+ Kommentar", 32.0).on_hover_text("Strg+Alt+M – zur Markierung oder zum Wort am Cursor").clicked() {
                w.x.dlg = Some(XDlg::Comment(String::new()));
            }
            ui.add_space(sp::S);
            // Reihenfolge nach Stelle im Text
            let mut pos: HashMap<u32, Pos> = HashMap::new();
            for (pi, p) in w.td.paras.iter().enumerate() {
                let mut acc = 0;
                for r in &p.runs {
                    if r.st.cmt > 0 {
                        pos.entry(r.st.cmt).or_insert(Pos::new(pi, acc));
                    }
                    acc += r.text.chars().count();
                }
            }
            let mut list = w.td.ex.comments.clone();
            list.sort_by_key(|c| pos.get(&c.id).copied().unwrap_or(Pos::new(usize::MAX, 0)));
            if list.is_empty() {
                ui.label(style::hint("Noch keine Kommentare. Markiere Text und klicke „+ Kommentar“."));
            }
            for cm in list {
                let pl = pal();
                let fr = egui::Frame::new().fill(c(if cm.done { pl.hover } else { pl.card })).stroke(Stroke::new(1.0, c(pl.line))).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(8));
                fr.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let head = ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&cm.author).strong());
                        ui.label(style::hint(cm.time.get(..10).unwrap_or("")));
                    });
                    if head.response.interact(Sense::click()).clicked() {
                        if let Some(p) = pos.get(&cm.id) {
                            w.jump(*p, now);
                        }
                    }
                    ui.label(&cm.text);
                    for r in &cm.replies {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(format!("Antwort {}:", r.author)).strong());
                            ui.label(&r.text);
                        });
                    }
                    let mut txt = w.x.reply.get(&cm.id).cloned().unwrap_or_default();
                    let te = ui.add(egui::TextEdit::singleline(&mut txt).hint_text("Antworten …").desired_width(f32::INFINITY));
                    let send = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    w.x.reply.insert(cm.id, txt.clone());
                    ui.horizontal(|ui| {
                        if (send || (!txt.trim().is_empty() && ui.small_button("Senden").clicked())) && !txt.trim().is_empty() {
                            w.begin(Edit::Other, now);
                            let a = author(sh);
                            if let Some(c) = w.td.ex.comments.iter_mut().find(|c| c.id == cm.id) {
                                c.replies.push(Reply { author: a, time: now_iso(), text: txt.trim().into() });
                            }
                            w.x.reply.remove(&cm.id);
                            w.changed(now);
                        }
                        if ui.small_button(if cm.done { "Wieder öffnen" } else { "Erledigt" }).clicked() {
                            w.begin(Edit::Other, now);
                            if let Some(c) = w.td.ex.comments.iter_mut().find(|c| c.id == cm.id) {
                                c.done = !c.done;
                            }
                            w.changed(now);
                        }
                        if ui.small_button("Löschen").clicked() {
                            w.delete_comment(cm.id, now);
                        }
                    });
                });
                ui.add_space(sp::S);
            }
        }
        Panel::Changes => {
            let mut track = w.td.ex.track;
            if ui.checkbox(&mut track, "Änderungen nachverfolgen").changed() {
                w.run(Cmd::Track, sh, &ui.ctx().clone());
            }
            ui.horizontal(|ui| {
                if ui.button("Alle annehmen").clicked() {
                    w.resolve_rev(true, true, sh);
                }
                if ui.button("Alle ablehnen").clicked() {
                    w.resolve_rev(true, false, sh);
                }
            });
            ui.add_space(sp::S);
            let ch = tools::changes(&w.td);
            if ch.is_empty() {
                ui.label(style::hint("Keine Änderungen. Schalte „nachverfolgen“ ein – dann wird jede Änderung farbig markiert."));
            }
            for (id, p, ins, text) in ch {
                let who = w.td.ex.revs.iter().find(|r| r.id == id).map(|r| r.author.clone()).unwrap_or_default();
                let pl = pal();
                egui::Frame::new().stroke(Stroke::new(1.0, c(pl.line))).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let r = ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&who).strong());
                        ui.label(style::hint(if ins { "eingefügt" } else { "gelöscht" }));
                    });
                    if r.response.interact(Sense::click()).clicked() {
                        w.jump(p, now);
                    }
                    let t: String = text.replace(OBJ, "•").chars().take(120).collect();
                    let rt = egui::RichText::new(t).color(c(owndraw_core::office::layout::rev_color(id)));
                    ui.label(if ins { rt.underline() } else { rt.strikethrough() });
                    ui.horizontal(|ui| {
                        if ui.small_button("Annehmen").clicked() {
                            w.begin(Edit::Other, now);
                            tools::accept(&mut w.td, Some(id));
                            w.cur = w.td.clamp(w.cur);
                            w.anchor = w.cur;
                            w.changed(now);
                        }
                        if ui.small_button("Ablehnen").clicked() {
                            w.begin(Edit::Other, now);
                            tools::reject(&mut w.td, Some(id));
                            w.cur = w.td.clamp(w.cur);
                            w.anchor = w.cur;
                            w.changed(now);
                        }
                    });
                });
                ui.add_space(sp::S);
            }
        }
    });
}

// ---------------------------------------------------------------- Rechtsklick (Rechtschreibung, Tabelle, Kommentar)

/// Ergänzt das Rechtsklick-Menü; true = Menü ist vollständig (nichts weiter anzeigen).
pub fn context_menu(w: &mut Writer, ui: &mut egui::Ui, sh: &mut Shared) -> bool {
    let now = sh.now;
    let ctx = ui.ctx().clone();
    if let Some((a, b)) = w.x.miss.iter().find(|(a, b)| a.p == w.cur.p && a.o <= w.cur.o && w.cur.o <= b.o).copied() {
        let word = w.td.text_range(a, b);
        let sug = lang::suggest(&word, 5);
        if sug.is_empty() {
            ui.label(style::hint("Keine Vorschläge"));
        }
        for s in sug {
            if ui.button(egui::RichText::new(&s).strong()).clicked() {
                w.begin(Edit::Other, now);
                w.anchor = a;
                w.cur = b;
                w.delete_sel();
                let st = w.td.style_at(w.cur);
                w.cur = w.td.insert_text(w.cur, &s, &st);
                w.anchor = w.cur;
                w.changed(now);
                ui.close();
            }
        }
        if ui.button(format!("„{word}“ zum Wörterbuch hinzufügen")).clicked() {
            lang::add_user_word(&word);
            sh.settings.extra.insert("writer_words".into(), serde_json::json!(lang::user_words()));
            w.x.miss_rev = 0;
            ui.close();
        }
        style::separator(ui);
    }
    if w.cur_cell().is_some() {
        for (cm, l) in [(Cmd::RowAbove, "Zeile oberhalb einfügen"), (Cmd::RowBelow, "Zeile unterhalb einfügen"), (Cmd::ColLeft, "Spalte links einfügen"), (Cmd::ColRight, "Spalte rechts einfügen"), (Cmd::DelRow, "Zeile löschen"), (Cmd::DelCol, "Spalte löschen"), (Cmd::DelTable, "Tabelle löschen")] {
            if ui.button(l).clicked() {
                w.run(cm, sh, &ctx);
                ui.close();
            }
        }
        return true;
    }
    if tools::rev_at(&w.td, w.cur).is_some() {
        if ui.button("Änderung annehmen").clicked() {
            w.run(Cmd::AcceptHere, sh, &ctx);
            ui.close();
        }
        if ui.button("Änderung ablehnen").clicked() {
            w.run(Cmd::RejectHere, sh, &ctx);
            ui.close();
        }
        style::separator(ui);
    }
    if ui.button("Kommentar einfügen").clicked() {
        w.run(Cmd::Comment, sh, &ctx);
        ui.close();
    }
    false
}

// ---------------------------------------------------------------- Fenster

fn ok_cancel(ui: &mut egui::Ui, ok: &str) -> (bool, bool) {
    let mut r = (false, false);
    ui.add_space(sp::M);
    ui.horizontal(|ui| {
        r.0 = style::primary_btn(ui, ok, 36.0).clicked();
        r.1 = style::plain_btn(ui, "Abbrechen", 36.0).clicked();
    });
    r
}

fn color_row(ui: &mut egui::Ui, label: &str, v: &mut u32, none: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        let mut cols = vec![0xFF1D1D24, 0xFF1D4ED8, 0xFF15803D, 0xFFB91C1C, 0xFFF59E0B, 0xFF7C3AED, 0xFFDBEAFE, 0xFFDCFCE7, 0xFFFEF3C7, 0xFFFCE7F3, 0xFFFFFFFF];
        if none {
            cols.insert(0, 0);
        }
        for col in cols {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
            if col == 0 {
                ui.painter().circle_stroke(r.center(), 9.0, Stroke::new(1.0, c(0xFF9CA3AF)));
                ui.painter().line_segment([r.center() + Vec2::new(-6.0, 6.0), r.center() + Vec2::new(6.0, -6.0)], Stroke::new(1.0, c(0xFF9CA3AF)));
            } else {
                ui.painter().circle_filled(r.center(), 9.0, c(col));
                ui.painter().circle_stroke(r.center(), 9.0, Stroke::new(0.5, c(0xFF9CA3AF)));
            }
            if *v == col {
                ui.painter().circle_stroke(r.center(), 11.5, Stroke::new(2.0, c(pal().accent)));
            }
            if resp.clicked() {
                *v = col;
            }
        }
    });
}

pub fn dialogs(w: &mut Writer, ctx: &egui::Context, sh: &mut Shared) {
    let Some(mut d) = w.x.dlg.clone() else { return };
    let now = sh.now;
    let mut close = false;
    let maxw = (ctx.content_rect().width() - 40.0).min(if matches!(d, XDlg::Obj { .. } | XDlg::Versions { .. }) { 560.0 } else { 440.0 });
    let title = match &d {
        XDlg::Table { .. } => "Tabelle einfügen",
        XDlg::Obj { o, .. } => o.label(),
        XDlg::Note { new: true, .. } => "Fußnote einfügen",
        XDlg::Note { .. } => "Fußnote",
        XDlg::Comment(_) => "Kommentar",
        XDlg::Cite { .. } => "Zitat einfügen",
        XDlg::CrossRef { .. } => "Querverweis",
        XDlg::Caption { .. } => "Beschriftung",
        XDlg::Watermark(_) => "Wasserzeichen",
        XDlg::Protect(_) => "Dokument schützen",
        XDlg::Versions { .. } => "Versionsverlauf",
        XDlg::Merge(_) => "Seriendruck: Daten",
        XDlg::MergeField(_) => "Seriendruck: Feld",
        XDlg::Choice(_) => "Auswahlfeld",
        XDlg::Templates => "Vorlagen",
        XDlg::Theme => "Design",
        XDlg::Envelope(..) => "Umschlag",
        XDlg::Labels(..) => "Etiketten",
        XDlg::Lang => "Sprache und Prüfung",
        XDlg::Index(_) => "Stichwort",
        XDlg::Formats => "Speichern als …",
    };
    let m = egui::Modal::new(egui::Id::unique("w-xdlg")).frame(style::card().inner_margin(Margin::same(sp::XL as i8))).show(ctx, |ui| {
        ui.set_width(maxw);
        ui.label(style::title(title));
        ui.add_space(sp::M);
        match &mut d {
            XDlg::Table { r, c: cc } => {
                ui.label(style::hint("Größe wählen: Feld anklicken"));
                let (cell, n, m) = (22.0, 10usize, 8usize);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(cell * n as f32, cell * m as f32), Sense::click());
                if let Some(h) = resp.hover_pos() {
                    *cc = (((h.x - rect.left()) / cell) as usize + 1).min(n);
                    *r = (((h.y - rect.top()) / cell) as usize + 1).min(m);
                }
                for y in 0..m {
                    for x in 0..n {
                        let rr = egui::Rect::from_min_size(rect.min + Vec2::new(x as f32 * cell, y as f32 * cell), Vec2::splat(cell - 3.0));
                        let on = x < *cc && y < *r;
                        ui.painter().rect_filled(rr, CornerRadius::same(3), c(if on { (pal().accent & 0x00FF_FFFF) | 0x88 << 24 } else { pal().hover }));
                    }
                }
                ui.label(format!("{} × {}  (Spalten × Zeilen)", cc, r));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(cc).range(1..=20).prefix("Spalten "));
                    ui.add(egui::DragValue::new(r).range(1..=200).prefix("Zeilen "));
                });
                let (ok, ca) = ok_cancel(ui, "Einfügen");
                if ok || resp.clicked() {
                    let cw = w.td.page.content_w() / (w.td.ex.cols.max(1) as f32);
                    let mut t = Table::new(*r, *cc, cw);
                    t.header = *r > 1;
                    w.insert_block(Para { table: Some(t), ..Para::new(Kind::Table) }, now);
                    w.open_cell(w.cur.p, 0, 0);
                    close = true;
                }
                close |= ca;
            }
            XDlg::Obj { p, o } => {
                obj_editor(w, ui, o, sh);
                let (ok, ca) = ok_cancel(ui, "Übernehmen");
                if ok {
                    w.begin(Edit::Other, now);
                    if let Some(q) = w.td.paras.get_mut(*p).filter(|q| q.kind == Kind::Object) {
                        // Formeln: Größe aus dem Inhalt
                        if let ObjKind::Formula { src } = &o.kind {
                            let (fw, fh) = owndraw_core::office::draw::formula_size(src, 16.0);
                            let k = q.obj.as_ref().map(|x| x.h / formula_size_h(x)).unwrap_or(1.0).clamp(0.5, 4.0);
                            o.w = fw * k;
                            o.h = fh * k;
                        }
                        q.obj = Some(o.clone());
                    }
                    w.x.tex.remove(p);
                    w.changed(now);
                    close = true;
                }
                close |= ca;
            }
            XDlg::Note { at, text, new } => {
                let te = ui.add(egui::TextEdit::multiline(text).hint_text("Text der Fußnote").desired_rows(3).desired_width(f32::INFINITY));
                if text.is_empty() {
                    te.request_focus();
                }
                ui.label(style::hint("Die Nummer setzt OwnDraw selbst; die Fußnote steht unten auf der Seite."));
                ui.add_space(sp::M);
                ui.horizontal(|ui| {
                    if style::primary_btn(ui, "Übernehmen", 36.0).clicked() && !text.trim().is_empty() {
                        if *new {
                            w.cur = *at;
                            w.anchor = *at;
                            w.insert_field(Field::Note(text.trim().into()), now);
                        } else {
                            w.begin(Edit::Other, now);
                            tools::set_field_at(&mut w.td, *at, Field::Note(text.trim().into()));
                            w.changed(now);
                        }
                        close = true;
                    }
                    if !*new && style::plain_btn(ui, "Fußnote löschen", 36.0).clicked() {
                        w.begin(Edit::Other, now);
                        w.td.delete(*at, Pos::new(at.p, at.o + 1));
                        w.changed(now);
                        close = true;
                    }
                    if style::plain_btn(ui, "Abbrechen", 36.0).clicked() {
                        close = true;
                    }
                });
            }
            XDlg::Comment(t) => {
                let te = ui.add(egui::TextEdit::multiline(t).hint_text("Kommentar …").desired_rows(3).desired_width(f32::INFINITY));
                if t.is_empty() {
                    te.request_focus();
                }
                ui.label(style::hint(&format!("Als „{}“ – gilt für die Markierung bzw. das Wort am Cursor.", author(sh))));
                let (ok, ca) = ok_cancel(ui, "Kommentieren");
                if ok {
                    let t = t.clone();
                    w.add_comment(t, now, sh);
                    close = true;
                }
                close |= ca;
            }
            XDlg::Cite { pick, s } => {
                if !w.td.ex.sources.is_empty() {
                    ui.label("Vorhandene Quelle");
                    for q in w.td.ex.sources.clone() {
                        ui.radio_value(pick, Some(q.id), format!("{} {}", q.cite(), q.title));
                    }
                    ui.radio_value(pick, None, "Neue Quelle …");
                    ui.add_space(sp::S);
                }
                if pick.is_none() || w.td.ex.sources.is_empty() {
                    *pick = None;
                    for (l, v, h) in [("Autor(en)", &mut s.author, "Nachname, Vorname"), ("Titel", &mut s.title, ""), ("Jahr", &mut s.year, "2026"), ("Verlag / Zeitschrift", &mut s.publisher, ""), ("Link", &mut s.url, "https://…")] {
                        ui.label(l);
                        ui.add(egui::TextEdit::singleline(v).hint_text(h).desired_width(f32::INFINITY));
                    }
                }
                ui.label(style::hint("„Literaturverzeichnis“ (Einfügen) listet alle Quellen auf."));
                let (ok, ca) = ok_cancel(ui, "Zitieren");
                if ok {
                    let id = match pick {
                        Some(id) => *id,
                        None => {
                            w.begin(Edit::Other, now);
                            let id = w.td.ex.id();
                            w.td.ex.sources.push(Source { id, ..s.clone() });
                            id
                        }
                    };
                    w.insert_field(Field::Cite(id), now);
                    close = true;
                }
                close |= ca;
            }
            XDlg::CrossRef { show } => {
                ui.horizontal(|ui| {
                    ui.label("Zeigt:");
                    ui.radio_value(show, RefShow::Label, "Nummer/Titel");
                    ui.radio_value(show, RefShow::Page, "Seite");
                    ui.radio_value(show, RefShow::Text, "Ganzer Text");
                });
                ui.add_space(sp::S);
                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    for (p, t) in w.ref_targets() {
                        if ui.button(t).clicked() {
                            let id = w.ensure_id(p);
                            w.insert_field(Field::Ref(id, *show), now);
                            close = true;
                        }
                    }
                });
                ui.label(style::hint("Verweise passen sich an, wenn sich Nummern oder Seiten ändern."));
                if style::plain_btn(ui, "Abbrechen", 36.0).clicked() {
                    close = true;
                }
            }
            XDlg::Caption { label, text } => {
                ui.horizontal(|ui| {
                    for l in ["Abbildung", "Tabelle", "Formel"] {
                        ui.radio_value(label, l.to_string(), l);
                    }
                });
                let te = ui.add(egui::TextEdit::singleline(text).hint_text("Beschreibung").desired_width(f32::INFINITY));
                if text.is_empty() {
                    te.request_focus();
                }
                ui.label(style::hint("Die Nummer zählt OwnDraw automatisch („Abbildung 1: …“)."));
                let (ok, ca) = ok_cancel(ui, "Einfügen");
                if ok {
                    w.begin(Edit::Other, now);
                    let at = w.cur.p + 1;
                    let mut cp = Para::with_text(Kind::Caption, text.trim());
                    cp.cap = label.clone();
                    cp.align = if w.sel_block().is_some() { w.td.paras[w.cur.p].align } else { Align::Left };
                    cp.id = w.td.ex.id();
                    let at = at.min(w.td.paras.len());
                    w.td.paras.insert(at, cp);
                    w.cur = Pos::new(at, w.td.paras[at].len());
                    w.anchor = w.cur;
                    w.changed(now);
                    close = true;
                }
                close |= ca;
            }
            XDlg::Watermark(t) => {
                ui.add(egui::TextEdit::singleline(t).hint_text("z. B. ENTWURF, VERTRAULICH").desired_width(f32::INFINITY));
                ui.add_space(sp::M);
                ui.horizontal(|ui| {
                    if style::primary_btn(ui, "Übernehmen", 36.0).clicked() {
                        w.begin(Edit::Other, now);
                        w.td.ex.watermark = t.trim().to_string();
                        w.changed(now);
                        close = true;
                    }
                    if !w.td.ex.watermark.is_empty() && style::plain_btn(ui, "Entfernen", 36.0).clicked() {
                        w.begin(Edit::Other, now);
                        w.td.ex.watermark.clear();
                        w.changed(now);
                        close = true;
                    }
                    if style::plain_btn(ui, "Abbrechen", 36.0).clicked() {
                        close = true;
                    }
                });
            }
            XDlg::Protect(pw) => {
                let on = w.protected();
                ui.label(if on { "Das Dokument ist geschützt. Gib das Kennwort ein, um es wieder zu bearbeiten." } else { "Schützt vor versehentlichem Ändern. Lesen, Kopieren und Drucken bleiben möglich." });
                ui.add(egui::TextEdit::singleline(pw).password(true).hint_text("Kennwort (optional)").desired_width(f32::INFINITY));
                let (ok, ca) = ok_cancel(ui, if on { "Schutz aufheben" } else { "Schützen" });
                if ok {
                    if on {
                        if protect_hash(pw) == w.td.ex.protect || (pw.is_empty() && w.td.ex.protect == "1") {
                            w.td.ex.protect.clear();
                            w.changed(now);
                            sh.toast("Schutz aufgehoben");
                            close = true;
                        } else {
                            sh.toast("Falsches Kennwort");
                        }
                    } else {
                        w.begin(Edit::Other, now);
                        w.td.ex.protect = protect_hash(pw);
                        w.changed(now);
                        sh.toast("Dokument geschützt");
                        close = true;
                    }
                }
                close |= ca;
            }
            XDlg::Versions { name } => {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(name).hint_text("Name der neuen Version").desired_width(240.0));
                    if ui.button("Version speichern").clicked() {
                        w.begin(Edit::Other, now);
                        let n = if name.trim().is_empty() { format!("Version {}", w.td.ex.versions.len() + 1) } else { name.trim().to_string() };
                        tools::save_version(&mut w.td, &n, &now_iso());
                        w.changed(now);
                        name.clear();
                    }
                });
                ui.add_space(sp::S);
                if w.td.ex.versions.is_empty() {
                    ui.label(style::hint("Noch keine Versionen gespeichert."));
                }
                let mut act = None;
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for (i, v) in w.td.ex.versions.iter().enumerate().rev() {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&v.name).strong());
                            ui.label(style::hint(&v.time.replace('T', " ").chars().take(16).collect::<String>()));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Wiederherstellen").clicked() {
                                    act = Some((i, false));
                                }
                                if ui.small_button("Vergleichen").on_hover_text("Unterschiede zum jetzigen Stand als Änderungen zeigen").clicked() {
                                    act = Some((i, true));
                                }
                            });
                        });
                    }
                });
                match act {
                    Some((i, false)) => {
                        w.begin(Edit::Other, now);
                        tools::restore_version(&mut w.td, i, &now_iso());
                        w.cur = Pos::default();
                        w.anchor = w.cur;
                        w.changed(now);
                        sh.toast("Version wiederhergestellt (vorheriger Stand ist als Version gesichert)");
                        close = true;
                    }
                    Some((i, true)) => {
                        if let Some(old) = tools::version_doc(&w.td, i) {
                            let mut cmp = tools::compare(&old, &w.td, &author(sh), &now_iso());
                            cmp.ex.versions = w.td.ex.versions.clone();
                            w.replace_all_doc(cmp, now);
                            w.x.panel = Some(Panel::Changes);
                            sh.toast("Unterschiede als Änderungen markiert – annehmen oder ablehnen");
                            close = true;
                        }
                    }
                    None => {}
                }
                if style::plain_btn(ui, "Schließen", 36.0).clicked() {
                    close = true;
                }
            }
            XDlg::Merge(csv) => {
                ui.label("Datenquelle (CSV, erste Zeile = Spaltennamen; aus Excel/Tabellen einfügen geht auch):");
                ui.add(egui::TextEdit::multiline(csv).font(egui::TextStyle::Monospace).desired_rows(8).desired_width(f32::INFINITY));
                let md = MergeData::from_csv(csv);
                ui.label(style::hint(&format!("{} Spalten, {} Datensätze", md.head.len(), md.rows.len())));
                let (ok, ca) = ok_cancel(ui, "Übernehmen");
                if ok {
                    w.begin(Edit::Other, now);
                    w.td.ex.merge = (!md.head.is_empty()).then_some(md);
                    w.changed(now);
                    close = true;
                    sh.toast("Jetzt Felder einfügen: Einfügen › Seriendruck: Feld");
                }
                close |= ca;
            }
            XDlg::MergeField(_) => {
                let head = w.td.ex.merge.as_ref().map(|m| m.head.clone()).unwrap_or_default();
                ui.horizontal_wrapped(|ui| {
                    for h in head {
                        if ui.button(format!("«{h}»")).clicked() {
                            w.insert_field(Field::Merge(h), now);
                        }
                    }
                });
                ui.label(style::hint("„Seriendruck: Alle als PDF“ erzeugt je Datensatz eine Seite."));
                if style::plain_btn(ui, "Fertig", 36.0).clicked() {
                    close = true;
                }
            }
            XDlg::Choice(t) => {
                ui.label("Möglichkeiten (eine je Zeile):");
                ui.add(egui::TextEdit::multiline(t).desired_rows(4).desired_width(f32::INFINITY));
                ui.label(style::hint("Klick auf das Feld wählt die nächste Möglichkeit."));
                let (ok, ca) = ok_cancel(ui, "Einfügen");
                if ok {
                    let o: Vec<String> = t.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
                    if !o.is_empty() {
                        w.insert_field(Field::Choice(o, 0), now);
                    }
                    close = true;
                }
                close |= ca;
            }
            XDlg::Templates => {
                ui.label(style::hint("Ersetzt den Inhalt (Rückgängig mit Strg+Z)."));
                ui.horizontal_wrapped(|ui| {
                    for t in tools::TEMPLATES {
                        if ui.add(egui::Button::new(t).min_size(Vec2::new(120.0, 40.0))).clicked() {
                            let mut td = tools::template(t);
                            td.ex.versions = w.td.ex.versions.clone();
                            w.replace_all_doc(td, now);
                            close = true;
                        }
                    }
                });
                if style::plain_btn(ui, "Abbrechen", 36.0).clicked() {
                    close = true;
                }
            }
            XDlg::Theme => {
                for (i, (n, col, fam)) in owndraw_core::office::layout::THEMES.iter().enumerate() {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
                    let pl = pal();
                    if w.td.ex.theme as usize == i || resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(6), c(if w.td.ex.theme as usize == i { pl.active } else { pl.hover }));
                    }
                    ui.painter().circle_filled(r.left_center() + Vec2::new(18.0, 0.0), 9.0, c(*col));
                    let f = super::view::font_id(owndraw_core::office::fonts::FontKey { fam: *fam, bold: true, italic: false }, 15.0);
                    ui.painter().text(r.left_center() + Vec2::new(36.0, 0.0), egui::Align2::LEFT_CENTER, format!("{n} – Überschrift"), f, c(*col));
                    if resp.clicked() {
                        w.begin(Edit::Other, now);
                        w.td.ex.theme = i as u8;
                        w.changed(now);
                    }
                }
                if style::plain_btn(ui, "Fertig", 36.0).clicked() {
                    close = true;
                }
            }
            XDlg::Envelope(s, r) => {
                ui.label("Absender");
                ui.add(egui::TextEdit::multiline(s).desired_rows(3).desired_width(f32::INFINITY));
                ui.label("Empfänger");
                ui.add(egui::TextEdit::multiline(r).desired_rows(4).desired_width(f32::INFINITY));
                ui.label(style::hint("Format DIN lang (DL). Ersetzt den Inhalt (Rückgängig mit Strg+Z)."));
                let (ok, ca) = ok_cancel(ui, "Umschlag erstellen");
                if ok {
                    let td = tools::envelope(s, r);
                    w.replace_all_doc(td, now);
                    close = true;
                }
                close |= ca;
            }
            XDlg::Labels(t, cols, rows) => {
                ui.label("Text auf jedem Etikett");
                ui.add(egui::TextEdit::multiline(t).desired_rows(3).desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(cols).range(1..=6).prefix("Spalten "));
                    ui.add(egui::DragValue::new(rows).range(1..=20).prefix("Zeilen "));
                });
                ui.label(style::hint("A4-Bogen. Ersetzt den Inhalt (Rückgängig mit Strg+Z)."));
                let (ok, ca) = ok_cancel(ui, "Etiketten erstellen");
                if ok {
                    let td = tools::labels(t, *cols, *rows);
                    w.replace_all_doc(td, now);
                    close = true;
                }
                close |= ca;
            }
            XDlg::Lang => {
                let ctx2 = ui.ctx().clone();
                let mut s = w.x.spell;
                if ui.checkbox(&mut s, "Rechtschreibung prüfen (Deutsch und Englisch, rot unterstrichen)").changed() {
                    w.run(Cmd::Spell, sh, &ctx2);
                }
                let mut h = w.td.ex.hyph;
                if ui.checkbox(&mut h, "Silbentrennung (dieses Dokument)").changed() {
                    w.run(Cmd::Hyphen, sh, &ctx2);
                }
                let mut a = w.x.autocorr;
                if ui.checkbox(&mut a, "Autokorrektur („…“, –, ©, Tippfehler, Satzanfang groß)").changed() {
                    w.run(Cmd::AutoCorrect, sh, &ctx2);
                }
                ui.add_space(sp::S);
                let words = lang::user_words();
                ui.label(format!("Eigenes Wörterbuch: {} Wörter", words.len()));
                if !words.is_empty() {
                    ui.label(style::hint(&words.iter().take(30).cloned().collect::<Vec<_>>().join(", ")));
                    if ui.small_button("Wörterbuch leeren").clicked() {
                        lang::set_user_words(Vec::new());
                        sh.settings.extra.insert("writer_words".into(), serde_json::json!([]));
                        w.x.miss_rev = 0;
                    }
                }
                ui.add_space(sp::S);
                let mut name = sh.settings.extra.get("writer_author").and_then(|v| v.as_str()).unwrap_or("").to_string();
                ui.label("Name bei Kommentaren und Änderungen");
                if ui.add(egui::TextEdit::singleline(&mut name).hint_text("leer = Kontoname").desired_width(f32::INFINITY)).changed() {
                    sh.settings.extra.insert("writer_author".into(), serde_json::json!(name));
                    w.author = author(sh);
                }
                if style::plain_btn(ui, "Fertig", 36.0).clicked() {
                    close = true;
                }
            }
            XDlg::Formats => {
                use owndraw_core::office::convert::TextFmt;
                for tf in TextFmt::ALL {
                    if ui.add(egui::Button::new(tf.label()).min_size(Vec2::new(ui.available_width(), 36.0))).clicked() {
                        sh.reqs.push(crate::app::Req::PickSave { purpose: crate::app::PickPurpose::TextExport(tf), name: format!("{}.{}", crate::app::safe_file_name(w.name()), tf.ext()), kind: crate::platform::FileKind::Any });
                        close = true;
                    }
                }
                ui.label(style::hint("PDF und Word findest du direkt im Menü „Datei“."));
                if style::plain_btn(ui, "Abbrechen", 36.0).clicked() {
                    close = true;
                }
            }
            XDlg::Index(t) => {
                ui.add(egui::TextEdit::singleline(t).hint_text("Stichwort").desired_width(f32::INFINITY));
                ui.label(style::hint("Unsichtbare Marke; „Stichwortverzeichnis“ listet alle mit Seitenzahl."));
                let (ok, ca) = ok_cancel(ui, "Markieren");
                if ok && !t.trim().is_empty() {
                    let at = if w.has_sel() { w.sel().1 } else { w.td.word_at(w.cur).1 };
                    w.cur = at;
                    w.anchor = at;
                    w.insert_field(Field::Index(t.trim().into()), now);
                    close = true;
                }
                close |= ca;
            }
        }
    });
    if m.should_close() {
        close = true;
    }
    if close {
        w.x.dlg = None;
        w.x.cur_stroke = None;
        w.focus_req = w.x.cell.is_none();
    } else if w.x.dlg.is_some() {
        w.x.dlg = Some(d);
    }
}

fn formula_size_h(o: &Obj) -> f32 {
    match &o.kind {
        ObjKind::Formula { src } => owndraw_core::office::draw::formula_size(src, 16.0).1.max(1.0),
        _ => o.h.max(1.0),
    }
}

/// Bearbeiten eines Objekts (Form, Formel, Diagramm, Zeichnung) mit Vorschau.
fn obj_editor(w: &mut Writer, ui: &mut egui::Ui, o: &mut Obj, sh: &mut Shared) {
    let _ = sh;
    match &mut o.kind {
        ObjKind::Shape { shape, fill, stroke, sw, text, tcol } => {
            ui.horizontal_wrapped(|ui| {
                for s in Shape::ALL {
                    ui.selectable_value(shape, s, s.label());
                }
            });
            ui.label("Text in der Form");
            ui.add(egui::TextEdit::multiline(text).desired_rows(3).desired_width(f32::INFINITY));
            color_row(ui, "Füllung", fill, true);
            color_row(ui, "Linie", stroke, true);
            color_row(ui, "Schrift", tcol, true);
            ui.add(egui::Slider::new(sw, 0.0..=8.0).text("Linienstärke").clamping(egui::SliderClamping::Never));
        }
        ObjKind::Formula { src } => {
            ui.label("Formel (LaTeX, z. B. \\frac{a}{b}, x^2, \\sqrt{x}, \\sum_{i=1}^n)");
            ui.add(egui::TextEdit::multiline(src).font(egui::TextStyle::Monospace).desired_rows(3).desired_width(f32::INFINITY));
            ui.horizontal_wrapped(|ui| {
                for (l, ins) in [("Bruch", "\\frac{a}{b}"), ("Wurzel", "\\sqrt{x}"), ("Hoch", "x^{2}"), ("Index", "x_{i}"), ("Summe", "\\sum_{i=1}^{n}"), ("Integral", "\\int_{a}^{b}"), ("π", "\\pi"), ("≤", "\\le"), ("→", "\\to"), ("Matrix", "\\begin{pmatrix}a & b\\\\ c & d\\end{pmatrix}")] {
                    if ui.small_button(l).clicked() {
                        src.push_str(ins);
                    }
                }
            });
            let f = owndraw_core::math::layout(src, 16.0);
            if !f.ok() {
                ui.colored_label(c(0xFFE5484D), f.errors.join("; "));
            }
        }
        ObjKind::Chart { kind, title, data } => {
            ui.horizontal(|ui| {
                for k in [ChartKind::Bar, ChartKind::Line, ChartKind::Pie] {
                    ui.selectable_value(kind, k, k.label());
                }
            });
            ui.add(egui::TextEdit::singleline(title).hint_text("Titel").desired_width(f32::INFINITY));
            ui.label(style::hint("Daten: erste Zeile = Reihen, erste Spalte = Beschriftungen"));
            let nc = data.iter().map(Vec::len).max().unwrap_or(1);
            egui::ScrollArea::both().max_height(180.0).show(ui, |ui| {
                egui::Grid::new("chart-grid").spacing(Vec2::splat(2.0)).show(ui, |ui| {
                    for row in data.iter_mut() {
                        row.resize(nc, String::new());
                        for cell in row.iter_mut() {
                            ui.add(egui::TextEdit::singleline(cell).desired_width(64.0));
                        }
                        ui.end_row();
                    }
                });
            });
            ui.horizontal(|ui| {
                if ui.small_button("+ Zeile").clicked() {
                    data.push(vec![String::new(); nc]);
                }
                if ui.small_button("+ Reihe").clicked() {
                    for r in data.iter_mut() {
                        r.push(String::new());
                    }
                }
                if data.len() > 2 && ui.small_button("− Zeile").clicked() {
                    data.pop();
                }
                if nc > 2 && ui.small_button("− Reihe").clicked() {
                    for r in data.iter_mut() {
                        r.pop();
                    }
                }
            });
        }
        ObjKind::Drawing { strokes } => {
            let pl = pal();
            ui.horizontal_wrapped(|ui| {
                for col in [0xFF1D1D24, 0xFF1D4ED8, 0xFFB91C1C, 0xFF15803D, 0xFFF59E0B, 0xFF7C3AED] {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                    ui.painter().circle_filled(r.center(), 9.0, c(col));
                    if w.x.pen.0 == col {
                        ui.painter().circle_stroke(r.center(), 11.5, Stroke::new(2.0, c(pl.accent)));
                    }
                    if resp.clicked() {
                        w.x.pen.0 = col;
                    }
                }
                ui.add(egui::Slider::new(&mut w.x.pen.1, 0.5..=12.0).text("Stärke").clamping(egui::SliderClamping::Never));
                if ui.small_button("Rückgängig").clicked() {
                    strokes.pop();
                }
                if ui.small_button("Leeren").clicked() {
                    strokes.clear();
                }
            });
            let aspect = o.h / o.w.max(1.0);
            let cw = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(cw, (cw * aspect).clamp(120.0, 360.0)), Sense::drag());
            ui.painter().rect_filled(rect, CornerRadius::same(4), c(0xFFFFFFFF));
            ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, c(pl.line)), egui::StrokeKind::Inside);
            let to = |x: f32, y: f32| egui::Pos2::new(rect.left() + x * rect.width(), rect.top() + y * rect.height());
            let scale = rect.width() / o.w.max(1.0);
            for s in strokes.iter().chain(w.x.cur_stroke.iter()) {
                let pts: Vec<egui::Pos2> = s.pts.chunks_exact(2).map(|q| to(q[0], q[1])).collect();
                ui.painter().add(egui::Shape::line(pts, Stroke::new(s.w * scale, c(s.color))));
            }
            if let Some(p) = resp.interact_pointer_pos() {
                let (x, y) = (((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0), ((p.y - rect.top()) / rect.height()).clamp(0.0, 1.0));
                if resp.drag_started() || w.x.cur_stroke.is_none() && resp.dragged() {
                    w.x.cur_stroke = Some(DStroke { color: w.x.pen.0, w: w.x.pen.1, pts: vec![x, y] });
                } else if resp.dragged() {
                    if let Some(s) = &mut w.x.cur_stroke {
                        s.pts.extend([x, y]);
                    }
                }
            }
            if resp.drag_stopped() {
                if let Some(s) = w.x.cur_stroke.take() {
                    strokes.push(s);
                }
            }
            ui.label(style::hint("Mit Maus, Finger oder Stift zeichnen."));
        }
    }
    // Vorschau
    if !matches!(o.kind, ObjKind::Drawing { .. }) {
        ui.add_space(sp::S);
        if let Some(pm) = owndraw_core::office::render::object_pixmap(o, 1.5) {
            let img = egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data());
            let t = ui.ctx().load_texture("w-obj-preview", img, egui::TextureOptions::LINEAR);
            let s = Vec2::new(o.w, o.h) * (ui.available_width() / o.w.max(1.0)).min(220.0 / o.h.max(1.0)).min(1.5);
            ui.image((t.id(), s));
        }
    }
}
